use super::{
    ReadDomain,
    location::Capability,
    model::{DocumentIdentity, DocumentRequest, DocumentSearchRequest},
    search,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

const PART_BOUND: usize = 24_576;

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
    json!({"status":"refused","code":code,"rule":"D-148","slot":slot,"reason":reason.into()})
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
            let parts = cadence::plan::render::task_parts(&publication.content)
                .map_err(|error| refusal("part", "document-ambiguous", error.to_string()))?;
            Ok(Resolved {
                identity: identity.clone(),
                classification: "native-publication",
                revision: publication.revision.clone(),
                parts: parts
                    .into_iter()
                    .map(|part| Part {
                        selector: part.selector,
                        title: part.title,
                        body: part.body,
                    })
                    .collect(),
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
    pub(super) fn document(&mut self, request: DocumentRequest) -> Value {
        let resolved = match resolve(&self.planning_root, &request.identity) {
            Ok(value) => value,
            Err(answer) => return answer,
        };
        let (part, offset) = match request.part.as_deref() {
            None => {
                return json!({"status":"ok","kind":"document-index","bound":PART_BOUND,
                    "identity":resolved.identity,"classification":resolved.classification,"revision":resolved.revision,
                    "parts":resolved.parts.iter().map(|part| json!({"part":part.selector,"title":part.title,
                        "bytes":part.body.len()})).collect::<Vec<_>>()});
            }
            Some(token) if token.starts_with("doc-") => match self.registry.get(token) {
                Some(Capability::Document {
                    identity,
                    part,
                    revision,
                    offset,
                }) if identity == request.identity && revision == resolved.revision => (part, offset),
                _ => {
                    return refusal(
                        "part",
                        "document-continuation-not-issued",
                        "document continuation is stale, expired, or belongs to another identity",
                    );
                }
            },
            Some(part) => (part.to_owned(), 0),
        };
        let Some(selected) = resolved.parts.iter().find(|candidate| candidate.selector == part) else {
            return refusal("part", "document-part-not-found", "the requested part is absent from this identity");
        };
        if offset > selected.body.len() || !selected.body.is_char_boundary(offset) {
            return refusal("part", "document-continuation-not-issued", "document continuation offset is invalid");
        }
        let mut end = (offset + PART_BOUND).min(selected.body.len());
        while end > offset && !selected.body.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < selected.body.len();
        let continuation = truncated.then(|| {
            self.registry.document(
                resolved.identity.clone(),
                selected.selector.clone(),
                resolved.revision.clone(),
                end,
            )
        });
        json!({"status":"ok","kind":"document-slice","bound":PART_BOUND,
            "identity":resolved.identity,"classification":resolved.classification,"revision":resolved.revision,
            "part":selected.selector,"body":&selected.body[offset..end],"truncated":truncated,
            "continuation":continuation,"continue_from_byte":truncated.then_some(end)})
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
