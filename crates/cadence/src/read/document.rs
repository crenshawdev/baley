use super::{
    ReadDomain,
    model::{DocumentIdentity, DocumentRequest, DocumentSearchRequest},
    search,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use std::path::Path;

const PART_BOUND: usize = 24_576;

fn bounded_parts(parts: Vec<Part>) -> Vec<Part> {
    parts.into_iter().flat_map(|part| {
        let mut chunks = Vec::new();
        let mut remaining = part.body.as_str();
        loop {
            let mut end = remaining.len().min(PART_BOUND);
            while !remaining.is_char_boundary(end) { end -= 1; }
            let selector = if chunks.is_empty() { part.selector.clone() }
                else { format!("{}:{}", part.selector, chunks.len() + 1) };
            chunks.push(Part { selector, title: part.title.clone(), body: remaining[..end].to_owned() });
            remaining = &remaining[end..];
            if remaining.is_empty() { break; }
        }
        chunks
    }).collect()
}

fn dispatch(root: &Path, identity: &DocumentIdentity, id: &str) -> Result<Resolved, Value> {
    use cadence::execution::{admission, dispatch::{changed_part, issue_binding}, history, model::ExecutionSnapshot};
    let fail = |error: cadence::store::Error| refusal("identity", "document-unavailable", error.to_string());
    let data = snapshot(root)?;
    let execution: ExecutionSnapshot = serde_json::from_value(data["execution"].clone())
        .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?;
    let occurrence = execution.occurrences.values().find(|occurrence| occurrence.issues.contains_key(id)
        || occurrence.active.as_ref().is_some_and(|active| active.id == id))
        .ok_or_else(|| refusal("identity", "document-not-found", "issued dispatch identity is absent"))?;
    let issue = occurrence.issues.get(id);
    let Some(active) = &occurrence.active else {
        let records = history::records(&data, occurrence.phase).map_err(fail)?;
        let slot = issue.and_then(|issue| issue.binding["tasks"].as_array()).and_then(|tasks| {
            tasks.iter().find(|task| records.iter().any(|record| record.request.task.task == task["id"]
                && matches!(record.request.event, history::Event::Close(_) | history::Event::Retirement { .. })))
        }).and_then(|task| task["id"].as_str()).map(|task| format!("task:{task}"));
        let mut answer = refusal(slot.as_deref().unwrap_or("identity"), "dispatch-superseded", "the dispatch has ended");
        answer["value"] = json!({"dispatch_id":null});
        return Err(answer);
    };
    let binding = issue_binding(&data, active).map_err(fail)?;
    if let Some(issue) = issue
        && let Some(slot) = changed_part(&issue.binding, &binding) {
            let current = occurrence.issues.iter().find(|(_, issue)| issue.binding == binding)
                .map(|(id, _)| id.clone());
            let mut answer = refusal(&slot, "dispatch-superseded", "the issued dispatch binding has changed");
            answer["value"] = json!({"dispatch_id":current});
            return Err(answer);
    }
    let publications = cadence::plan::persistence::saved(&data, active.phase).map_err(fail)?
        .ok_or_else(|| refusal("identity", "document-not-found", "dispatch publication is absent"))?;
    let publication = publications.publications.get(&active.plan)
        .ok_or_else(|| refusal("identity", "document-not-found", "dispatch plan is absent"))?;
    let records = history::records(&data, active.phase).map_err(fail)?;
    let views = history::plan_task_views(&data, &records, active.phase, active.plan).map_err(fail)?;
    let admissions = admission::records(&data, active.phase).map_err(fail)?;
    let basis = admissions.iter().find(|record| record.request_digest == binding["admission_digest"])
        .ok_or_else(|| refusal("identity", "document-not-found", "dispatch admission is absent"))?;
    let mut operational = issue.map(|issue| issue.operational.clone()).unwrap_or_else(|| json!({
        "protocol":cadence::execution::dispatch::NATIVE_PROTOCOL,
        "instructions":cadence::execution::instructions::VERSION,"phase":active.phase,"plan":active.plan,
        "occurrence":basis.request.contract.occurrence,"admission_digest":basis.request_digest,
        "set_version":binding["set_version"],"base_sha":active.base_sha,
        "expected_execution_version":active.expected_execution_version,"lease":{"files":active.files,"directories":active.directories},
        "commands":{},"continuation":null,"policy":active.policy,"route":active.route
    }));
    let head = std::process::Command::new("git").args(["rev-parse", "HEAD"])
        .current_dir(root.parent().unwrap_or(root)).output()
        .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?;
    if !head.status.success() { return Err(refusal("head", "document-unavailable", "cannot observe project head")); }
    operational["head"] = json!(String::from_utf8_lossy(&head.stdout).trim());
    operational["dispatch_id"] = json!(id);
    let mut parts = Vec::new();
    let mut identity_fields = serde_json::Map::new();
    for key in ["protocol", "instructions", "phase", "plan", "occurrence", "admission_digest",
        "expected_execution_version", "set_version", "base_sha", "head", "dispatch_id"] {
        identity_fields.insert(key.into(), operational[key].clone());
    }
    let mut add = |selector: String, body: String| parts.push(Part { title: selector.clone(), selector, body });
    add("identity".into(), Value::Object(identity_fields).to_string());
    for (slot, body) in [("goal", &publication.content.goal), ("context", &publication.content.context), ("notes", &publication.content.notes)] {
        if !body.is_empty() { add(slot.into(), body.clone()); }
    }
    let mut unfinished = Vec::new();
    let mut completed = Vec::new();
    for view in views {
        if let Some(mut done) = history::completed_view(&records, &view) {
            done["document_identity"] = json!({"kind":"task-summary","phase":view.task.phase,
                "occurrence":view.task.occurrence,"plan":view.task.plan,"task":view.task.task});
            completed.push(done);
            continue;
        }
        let mut task = publication.content.tasks.iter().find(|task| task.id == view.task.task)
            .map(serde_json::to_value).transpose().map_err(|error| refusal("part", "document-invalid", error.to_string()))?
            .unwrap_or_else(|| json!({"id":view.task.task,"verify":view.verify}));
        task["checks"] = json!(view.checks);
        task["state"] = json!(view.state);
        task["uncertainty"] = cadence::execution::runner::uncertainty(root.parent().unwrap_or(root), &records, &view).map_err(fail)?;
        task["checkpoints"] = json!(history::task_checkpoints(&records, &view.task));
        task["lease_scope"] = json!({"kind":"current-task-lease","phase":view.task.phase,
            "occurrence":view.task.occurrence,"plan":view.task.plan,"task":view.task.task});
        add(format!("task:{}", view.task.task), task.to_string());
        unfinished.push(view);
    }
    for check in cadence::execution::dispatch::admitted_checks(&data, active.phase, basis, &unfinished).map_err(fail)? {
        add(format!("check:{}", check["id"].as_str().unwrap_or_default()), check.to_string());
    }
    add("completed".into(), json!(completed).to_string());
    let suite_records = history::plan_records(&data, active.phase).map_err(fail)?;
    let suite = history::plan_project(&suite_records, &history::PlanIdentity { phase: active.phase,
        occurrence: basis.request.contract.occurrence.clone(), admission_digest: basis.request_digest.clone(), plan: active.plan });
    add("suite".into(), json!({"command":active.suite,"state":suite}).to_string());
    for key in ["continuation", "lease", "commands", "policy", "route"] {
        add(key.into(), operational[key].to_string());
    }
    let parts = bounded_parts(parts);
    let bytes = parts.iter().flat_map(|part| part.body.bytes()).collect::<Vec<_>>();
    Ok(Resolved { identity: identity.clone(), classification: "native-dispatch",
        revision: cadence::store::model::digest(&bytes), parts })
}

#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub selector: String,
    pub title: String,
    #[serde(skip)]
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct Resolved {
    pub identity: DocumentIdentity,
    pub classification: &'static str,
    pub revision: String,
    pub parts: Vec<Part>,
}

fn refusal(slot: &str, code: &str, reason: impl Into<String>) -> Value {
    Refusal::new(code, reason).rule("D-148").slot(slot).value()
}

fn snapshot(root: &Path) -> Result<Value, Value> {
    cadence::context::persistence::read_snapshot(root)
        .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?
        .map(|snapshot| snapshot.data)
        .ok_or_else(|| refusal("identity", "document-not-found", "native process authority is absent"))
}

fn roadmap(root: &Path, phase: u32) -> Result<Resolved, Value> {
    let text = std::fs::read_to_string(root.join("ROADMAP.md"))
        .map_err(|_| refusal("identity", "document-not-found", "roadmap authority is absent"))?;
    let parsed = cadence::derivation::parse_roadmap(&text)
        .map_err(|error| refusal("identity", "document-invalid", format!("roadmap authority is invalid: {error:?}")))?;
    let matches = parsed
        .phases
        .iter()
        .filter(|entry| entry.id.address() == phase.to_string())
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        let (code, reason) = if matches.is_empty() {
            ("document-not-found", "the requested roadmap phase is absent")
        } else {
            ("document-ambiguous", "the requested roadmap phase is ambiguous")
        };
        return Err(refusal("identity", code, reason));
    }
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n");
    let line = normalized
        .lines()
        .nth(matches[0].source_line - 1)
        .ok_or_else(|| refusal("identity", "document-invalid", "the selected roadmap row is unavailable"))?;
    let body = format!("{line}\n");
    Ok(Resolved {
        identity: DocumentIdentity::PhaseRoadmapRow {
            phase: std::num::NonZeroU32::new(phase).unwrap(),
        },
        classification: "canonical-roadmap-row",
        revision: cadence::store::model::digest(text.as_bytes()),
        parts: vec![Part {
            selector: "row".into(),
            title: format!("Phase {phase}"),
            body,
        }],
    })
}

pub fn resolve(root: &Path, identity: &DocumentIdentity) -> Result<Resolved, Value> {
    match identity {
        DocumentIdentity::Dispatch { id } => dispatch(root, identity, id),
        DocumentIdentity::PlanDraft { phase, plan, digest } => {
            let drafts = crate::import::drafts(root)
                .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?;
            let drafts = drafts.lock()
                .map_err(|_| refusal("identity", "document-unavailable", "drafts unavailable"))?;
            drafts.plans.get(&(phase.get(), digest.clone()))
                .and_then(|draft| draft.documents.iter().find(|document| matches!(
                    &document.identity, DocumentIdentity::PlanDraft { plan: number, .. } if number == plan
                ))).cloned()
                .ok_or_else(|| refusal("identity", "document-not-found", "held plan draft is absent"))
        }
        DocumentIdentity::ContextDraft { phase, digest } => {
            let drafts = crate::import::drafts(root)
                .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?;
            let drafts = drafts.lock()
                .map_err(|_| refusal("identity", "document-unavailable", "drafts unavailable"))?;
            drafts.contexts.get(&(phase.get(), digest.clone())).map(|draft| draft.document.clone())
                .ok_or_else(|| refusal("identity", "document-not-found", "held context draft is absent"))
        }
        DocumentIdentity::PhaseContext { phase } => {
            let data = snapshot(root)?;
            let context = cadence::context::persistence::saved(&data, phase.get())
                .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?
                .ok_or_else(|| refusal("identity", "document-not-found", "native context identity is absent"))?;
            let bytes = cadence::context::render::document(&context);
            Ok(Resolved {
                identity: identity.clone(),
                classification: "native-context",
                revision: cadence::store::model::digest(bytes.as_bytes()),
                parts: cadence::context::render::parts(&context)
                    .into_iter()
                    .map(|part| Part {
                        selector: part.selector,
                        title: part.title,
                        body: part.body,
                    })
                    .collect(),
            })
        }
        DocumentIdentity::PhasePlan { phase, plan } => {
            let data = snapshot(root)?;
            let occurrence = cadence::plan::persistence::saved(&data, phase.get())
                .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?
                .ok_or_else(|| refusal("identity", "document-not-found", "native plan occurrence is absent"))?;
            let publication = occurrence
                .publications
                .get(&plan.get())
                .ok_or_else(|| refusal("identity", "document-not-found", "native plan identity is absent"))?;
            let mut parts = cadence::plan::render::task_parts(&publication.content)
                .map_err(|error| refusal("part", "document-ambiguous", error.to_string()))?;
            for (selector, body) in [("goal", &publication.content.goal),
                ("context", &publication.content.context), ("notes", &publication.content.notes)] {
                if !body.is_empty() {
                    parts.push(cadence::plan::render::Part { selector: selector.into(), title: selector.into(), body: body.clone() });
                }
            }
            if let Some(cadence::plan::evidence::Map::Attached { items }) = &publication.content.evidence_map {
                for item in items {
                    let value = serde_json::to_value(item).map_err(|error| refusal("part", "document-invalid", error.to_string()))?;
                    let selector = format!("evidence:{}", value["id"].as_str().unwrap_or_default());
                    parts.push(cadence::plan::render::Part { title: selector.clone(), selector, body: value.to_string() });
                }
            }
            Ok(Resolved {
                identity: identity.clone(),
                classification: "native-publication",
                revision: publication.revision.clone(),
                parts: bounded_parts(parts
                    .into_iter()
                    .map(|part| Part {
                        selector: part.selector,
                        title: part.title,
                        body: part.body,
                    })
                    .collect()),
            })
        }
        DocumentIdentity::PhaseRoadmapRow { phase } => roadmap(root, phase.get()),
        DocumentIdentity::TaskSummary {
            phase,
            occurrence,
            plan,
            task,
        } => {
            if occurrence.trim().is_empty() || task.trim().is_empty() {
                return Err(refusal("identity", "document-identity", "task summary identity must be complete"));
            }
            let data = snapshot(root)?;
            let records = cadence::execution::history::records(&data, phase.get())
                .map_err(|error| refusal("identity", "document-unavailable", error.to_string()))?;
            let body = cadence::execution::render::render_native_task_row(
                &records,
                phase.get(),
                occurrence,
                plan.get(),
                task,
            )
            .map_err(|error| refusal("identity", "document-ambiguous", error.to_string()))?
            .ok_or_else(|| refusal("identity", "document-not-found", "completed task summary identity is absent"))?;
            Ok(Resolved {
                identity: identity.clone(),
                classification: "native-task-summary",
                revision: cadence::store::model::digest(body.as_bytes()),
                parts: vec![Part {
                    selector: "row".into(),
                    title: task.clone(),
                    body,
                }],
            })
        }
        DocumentIdentity::PlannerRound { .. } => {
            let report = super::measurement::resolve(root, identity)?;
            Ok(Resolved {
                identity: identity.clone(),
                classification: "claude-planner-round",
                revision: report.revision,
                parts: vec![Part {
                    selector: "report".into(),
                    title: "Claude planner round measurement".into(),
                    body: report.body,
                }],
            })
        }
    }
}

impl Resolved {
    pub fn bytes(&self) -> Vec<u8> {
        self.parts.iter().flat_map(|part| part.body.bytes()).collect()
    }

    /// Compare in the newest document's order, then report a removed part.
    pub fn first_difference(&self, previous: &Self) -> Option<String> {
        self.parts.iter().find(|part| {
            previous.parts.iter().find(|old| old.selector == part.selector)
                .is_none_or(|old| old.body != part.body)
        }).or_else(|| previous.parts.iter().find(|old| {
            !self.parts.iter().any(|part| part.selector == old.selector)
        })).map(|part| part.selector.clone())
    }
}

/// Partition by the typed slots' lengths, never by headings in authored prose.
pub fn plan_draft(
    data: &Value, content: &cadence::plan::model::Content, digest: &str,
) -> cadence::store::Result<Resolved> {
    use cadence::plan::{persistence, render};
    let retained = persistence::rendered_content(data, content)?;
    let bytes = render::document(&retained)?;
    let text = String::from_utf8(bytes.clone())
        .map_err(|error| cadence::store::Error::Invalid(error.to_string()))?;
    let mut spans = vec![("frontmatter".to_owned(), text.len() - retained.body.len())];
    let slot_len = |value: &str| value.len() + usize::from(!value.ends_with('\n')) + 1;
    spans.push(("goal".into(), "## Goal\n\n".len() + slot_len(&content.goal)));
    let truths = persistence::required_truths(data, content)?;
    spans.push(("truths".into(), "## Must be true when done\n\n".len()
        + truths.iter().map(|(id, sentence)| format!("- {id}. {sentence}\n").len()).sum::<usize>() + 1));
    spans.push(("context".into(), "## Context\n\n".len() + slot_len(&content.context)));
    spans.push(("evidence-map".into(), content.evidence_map.as_ref()
        .map(render::section).transpose()?.map_or(0, |text| text.len())));
    spans.push(("tasks-heading".into(), "## Tasks\n\n".len()));
    spans.extend(render::task_parts(&retained)?.into_iter().map(|part| (part.selector, part.body.len())));
    let used = spans.iter().map(|(_, size)| size).sum::<usize>();
    spans.push(("notes".into(), text.len().checked_sub(used)
        .ok_or_else(|| cadence::store::Error::Invalid("draft partition exceeds document".into()))?));
    let mut offset = 0;
    let mut parts = Vec::new();
    for (selector, size) in spans {
        let body = text.get(offset..offset + size)
            .ok_or_else(|| cadence::store::Error::Invalid("draft partition is invalid".into()))?.to_owned();
        offset += size;
        parts.push(Part { title: selector.clone(), selector, body });
    }
    Ok(Resolved {
        identity: DocumentIdentity::PlanDraft { phase: content.phase, plan: content.plan, digest: digest.into() },
        classification: "held-draft", revision: cadence::store::model::digest(&bytes), parts,
    })
}

pub fn context_draft(
    submission: &cadence::context::model::Submission, digest: &str,
) -> cadence::store::Result<Resolved> {
    use cadence::context::{persistence, render};
    let text = persistence::rendered(submission)?;
    let mut parts = vec![
        Part { selector: "title".into(), title: "Title".into(),
            body: format!("# Phase {}: {}\n\n", submission.phase, submission.title) },
        Part { selector: "scope".into(), title: "Scope boundary".into(),
            body: format!("## Scope boundary\n\n{}\n\n", submission.scope) },
    ];
    // Section headings belong to their first entry. Empty sections remain
    // with the preceding part so every byte has exactly one owner.
    for (heading, entries) in [
        ("Durable decisions", submission.durable_decisions.iter()
            .map(|value| (format!("durable-decision:{}", value.id), format!("- {}. {}\n", value.id, value.text))).collect::<Vec<_>>()),
        ("Decisions", submission.decisions.iter()
            .map(|value| (format!("decision:{}", value.id), format!("- {}. {}\n", value.id, value.text))).collect()),
        ("Truths", submission.truths.iter().map(|value| Ok((
            format!("truth:{}", value.id), format!("- {}. {}\n", value.id, render::sentence(value)?)
        ))).collect::<cadence::store::Result<Vec<_>>>()?),
        ("Flagged assumptions", submission.assumptions.iter().enumerate()
            .map(|(index, value)| (format!("assumption:{}", index + 1), format!("- {value}\n"))).collect()),
    ] {
        let mut prefix = format!("## {heading}\n\n");
        if entries.is_empty() {
            parts.last_mut().unwrap().body.push_str(&prefix);
        } else {
            for (selector, body) in entries {
                parts.push(Part { title: selector.clone(), selector, body: format!("{prefix}{body}") });
                prefix.clear();
            }
        }
        if heading != "Flagged assumptions" { parts.last_mut().unwrap().body.push('\n'); }
    }
    let resolved = Resolved {
        identity: DocumentIdentity::ContextDraft { phase: submission.phase, digest: digest.into() },
        classification: "held-draft", revision: cadence::store::model::digest(text.as_bytes()), parts,
    };
    if resolved.bytes() != text.as_bytes() {
        return Err(cadence::store::Error::Invalid("context draft partition differs from document".into()));
    }
    Ok(resolved)
}

pub fn catalog(root: &Path, phase: u32) -> Result<Vec<Resolved>, Value> {
    let phase = std::num::NonZeroU32::new(phase)
        .ok_or_else(|| refusal("scope", "document-identity", "phase must be positive"))?;
    let mut identities = vec![
        DocumentIdentity::PhaseContext { phase },
        DocumentIdentity::PhaseRoadmapRow { phase },
    ];
    if let Ok(data) = snapshot(root) {
        if let Ok(Some(plans)) = cadence::plan::persistence::saved(&data, phase.get()) {
            identities.extend(
                plans
                    .publications
                    .keys()
                    .filter_map(|plan| std::num::NonZeroU32::new(*plan))
                    .map(|plan| DocumentIdentity::PhasePlan { phase, plan }),
            );
        }
        if let Ok(records) = cadence::execution::history::records(&data, phase.get()) {
            for record in records {
                if matches!(record.request.event, cadence::execution::history::Event::Close(_)) {
                    let task = record.request.task;
                    if let Some(plan) = std::num::NonZeroU32::new(task.plan) {
                        identities.push(DocumentIdentity::TaskSummary {
                            phase,
                            occurrence: task.occurrence,
                            plan,
                            task: task.task,
                        });
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    for identity in identities {
        if let Ok(resolved) = resolve(root, &identity) {
            found.push(resolved);
        }
    }
    Ok(found)
}

impl ReadDomain {
    pub(super) fn document(&self, request: DocumentRequest) -> Value {
        let resolved = match resolve(&self.planning_root, &request.identity) {
            Ok(value) => value,
            Err(answer) => return answer,
        };
        let part = match request.part.as_deref() {
            None => {
                return json!({"status":"ok","kind":"document-index","bound":PART_BOUND,
                    "identity":resolved.identity,"classification":resolved.classification,"revision":resolved.revision,
                    "parts":resolved.parts.iter().map(|part| json!({"part":part.selector,"title":part.title,
                        "bytes":part.body.len()})).collect::<Vec<_>>()});
            }
            Some(part) => part,
        };
        let Some(selected) = resolved.parts.iter().find(|candidate| candidate.selector == part) else {
            return refusal("part", "document-part-not-found", "the requested part is absent from this identity");
        };
        if selected.body.len() > PART_BOUND {
            return refusal(
                "part",
                "document-part-too-large",
                format!(
                    "document part `{}` is {} bytes, exceeding the bound of {} bytes",
                    selected.selector,
                    selected.body.len(),
                    PART_BOUND
                ),
            );
        }
        json!({"status":"ok","kind":"document-slice","bound":PART_BOUND,
            "identity":resolved.identity,"classification":resolved.classification,"revision":resolved.revision,
            "part":selected.selector,"body":&selected.body})
    }
}

/// Ceiling on a document-search answer. Hits carry no bodies, so an answer
/// holds a few hundred of them and an ordinary phase never reaches it.
const SEARCH_BOUND: usize = super::slice::ANSWER_BOUND;

impl ReadDomain {
    /// Which parts of one phase's process records mention `pattern`: each
    /// hit is an identity and a part for `document`, with the matching line
    /// numbers, and never a body. Hits are in identity-then-part order.
    pub(super) fn document_search(&self, request: DocumentSearchRequest) -> Value {
        let matcher = match search::matcher(&request.pattern, request.case_insensitive.unwrap_or(false)) {
            Ok(matcher) => matcher,
            Err(answer) => return answer,
        };
        let mut searcher = search::searcher();
        let records = match catalog(&self.planning_root, request.phase.get()) {
            Ok(records) => records,
            Err(answer) => return answer,
        };
        let mut hits = Vec::new();
        for record in records {
            for part in record.parts {
                let match_lines = search::matching_lines(&mut searcher, &matcher, &part.body);
                if match_lines.is_empty() { continue; }
                hits.push(json!({"identity":record.identity,"part":part.selector,"title":part.title,
                    "classification":record.classification,"revision":record.revision,"match_lines":match_lines}));
            }
        }
        hits.sort_by(|left, right| left["identity"].to_string().cmp(&right["identity"].to_string())
            .then(left["part"].as_str().cmp(&right["part"].as_str())));
        let total = hits.len();
        let mut answer = json!({"status":"ok","kind":"document-search","bound":SEARCH_BOUND,"phase":request.phase,
            "hits":hits,"total":total,"incomplete":false,"notes":[]});
        while serde_json::to_vec(&answer).is_ok_and(|bytes| bytes.len() > SEARCH_BOUND) {
            answer["hits"].as_array_mut().unwrap().pop();
            answer["incomplete"] = json!(true);
            answer["notes"] = json!(["document-search answer was bounded; tighten the pattern"]);
        }
        answer
    }
}
