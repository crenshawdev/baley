//! The plan checker's return (design 0005 PLN-R16 and section 6, D-06,
//! D-17): a session relays the findings its checker returned for a checker
//! order of a phase. The findings are relayed unchanged: neither the
//! session nor Baley edits, drops or adjudicates them, since adjudication
//! belongs to reviews (ADR 0019).
//!
//! The check runs before admission and records nothing: the order, the
//! findings' shape, the order's inputs still held, then the round's binding,
//! never the files or settings current at the return. An order whose chain's
//! first order is a lineage's round-2 order is round 2, bound by that
//! lineage's `plan.check_issued` (D-09, D-10), and any other is round 1, bound
//! by its own `check` input.
//! The recording then records the round's `plan.checked` with that binding
//! and, in the same transaction, what the findings buy (D-07): in round 1 a
//! blocker buys the planner's one revision order, and otherwise, round 2's
//! blockers included, the draft's question set opens and the draft is
//! previewed at once, the findings going to the owner (PLN-R16).

use std::io::Read;

use baley_core::plan::{
    CheckRefusal, CheckResult, Findings, PHASE_PLAN_VIEW, PLAN_CHECK_ISSUED, PLAN_CHECKED,
    PLAN_CHECKED_VERSION, PreviousCheck, RecordedPlan, TruthVersion, judge_findings,
    phase_plan_key, phase_stream, read_plan_draft,
};
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, RecheckScope, Role};
use baley_core::questions::{QuestionSet, Subject, judge_set, round_of};
use baley_core::work_order::{
    MATERIAL_UNAVAILABLE, NO_SUCH_WORK_ORDER, OrderInput, PENDING, WORK_ORDER_NOT_PENDING,
    WORK_ORDER_VIEW, WRONG_WORK_ORDER, phase_subject, work_order_key,
};
use baley_store::{
    Command, Decision, Hash, HistoryFilter, InstructionEvidence, Ledger, NewEvent, Observed,
    ObservedDocument, OutcomeKind, PageRequest, PayloadBody, PayloadRef, Payloads, ProjectId,
    RETENTION_STREAM, StoreError, StreamName, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::checker::{CHECK_INPUT, PLAN_INPUT, TRUTHS_INPUT};
use crate::planning::facts::{self, PhaseFacts};
use crate::planning::order::{PreparedOrder, planner_absences, prepare_check_revision};
use crate::planning::replace::facts_without;
use crate::planning::submit::{draft_members, with_preview};
use crate::questions::answer::with_now;
use crate::questions::record::open;
use crate::refine::order::order_evidence;
use crate::refine::questions::opened_facts;
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::issue::attachments;
use crate::work_order::read::{Releases, order_document, releases};

/// What the return's check read and judged, which the recording records.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedReturn {
    /// The checker order's instruction evidence, which the return's caller
    /// carries.
    pub evidence: Vec<InstructionEvidence>,
    /// The checker order returned.
    work_order: String,
    /// The phase of the plan checked.
    phase: u32,
    /// The round's binding, copied from the order.
    binding: Binding,
    /// The findings, judged and exactly as returned.
    findings: Findings,
    /// The draft's body as recorded.
    body: Value,
    /// The draft's body read back as a plan draft.
    draft: RecordedPlan,
    /// The order's `work_order` document and the `phase_plan` document that
    /// told its round, each at the sequence that produced it.
    documents: Vec<ObservedDocument>,
    /// The `retention` stream's version the check's releases were read up
    /// to.
    retention: u64,
}

/// A round's binding as it was fixed when the round was issued (design 0005
/// section 6).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Binding {
    /// The round, 1 or 2.
    round: u64,
    /// The round-1 result this round re-checks, for round 2 only.
    previous_check: Option<PreviousCheck>,
    /// The digest of the draft checked.
    pub(crate) plan_digest: Hash,
    /// The scope the round reads.
    scope: RecheckScope,
    /// The truths the round is bound to.
    truth_versions: Vec<TruthVersion>,
    /// The policy version that selected the scope.
    scope_policy_version: u64,
    /// Round 1's `plan` and `truths` inputs' references, or the material
    /// `plan.check_issued` holds for round 2.
    material: Vec<OrderInput>,
}

/// The check of a return of checker order `work_order` with `findings` as
/// relayed, run before admission and recording nothing. In order, each at
/// slot `work_order`: `no-such-work-order` when the project holds no order
/// with that id; `wrong-work-order` when it is not a checker order of a
/// phase; `work-order-not-pending` naming its state when it failed or
/// already returned. Then the findings judged by `judge_findings`,
/// `finding-shape` at slot `findings` naming the first entry that is not a
/// finding. Then `material-unavailable` at slot `work_order` naming the first
/// input of the order this project purged, since the result attaches what the
/// order holds and the session sent none of it; the way out is the order's
/// failure and a fresh planner order (D-02), which for a purged `truths` input
/// cannot run until the truth text or the stories the plan serves change. Then
/// the round's binding by [`order_binding`] and the draft read back through its
/// `plan` material, so
/// the result is bound to what was fixed at issue and never to the truths or
/// settings current now (design 0005 section 6). A binding or body that cannot
/// be read is a ledger fault; a read fault is the `failed` answer.
pub fn check<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    work_order: &str,
    findings: Vec<Value>,
) -> Result<CheckedReturn, Value> {
    let order = order_document(store, project, work_order)
        .map_err(busy)?
        .ok_or_else(no_such_work_order)?;
    let body = &order.body;
    let text = |field: &str| body.get(field).and_then(Value::as_str);
    let phase = (text("role") == Some(Role::Checker.name()))
        .then(|| text("subject").and_then(phase_of_subject))
        .flatten()
        .ok_or_else(wrong_work_order)?;
    match text("state") {
        Some(PENDING) => {}
        Some(state) => return Err(not_pending(work_order, state)),
        None => return Err(unreadable("the checker order's state cannot be read")),
    }
    let findings = judge_findings(findings).map_err(|refusal| finding_refused(&refusal))?;
    let evidence = order_evidence(body)
        .ok_or_else(|| unreadable("the checker order's instructions cannot be read"))?;
    let released = releases(store, project).map_err(busy)?;
    if let Some(input) = purged_input(body, &released) {
        return Err(material_unavailable(work_order, input));
    }
    let (binding, phase_plan) = order_binding(store, project, body, phase)?;
    let plan = binding
        .material
        .iter()
        .find(|input| input.name == PLAN_INPUT)
        .ok_or_else(|| unreadable("the checker order's material cannot be read"))?;
    let draft_body = body_at(store, &plan.body, PLAN_INPUT)?;
    let draft = read_plan_draft(&draft_body)
        .ok_or_else(|| unreadable("the checked draft no longer reads as a plan draft"))?;
    Ok(CheckedReturn {
        evidence,
        work_order: work_order.into(),
        phase,
        binding,
        findings,
        body: draft_body,
        draft,
        documents: vec![
            ObservedDocument {
                view: WORK_ORDER_VIEW.into(),
                key: work_order_key(work_order),
                produced_seq: Some(order.produced_seq),
            },
            phase_plan,
        ],
        retention: released.version,
    })
}

/// The name of the first input `order`, a `work_order` document, lists whose
/// reference this project's purges released, as the retry judge finds one. A
/// purge releases every reference the project holds to a body, the order's
/// issue included. A document that cannot be read holds none here, and the
/// binding's read answers for it.
fn purged_input<'a>(order: &'a Value, released: &Releases) -> Option<&'a str> {
    let seq = order.get("seq")?.as_u64()?;
    order.get("inputs")?.as_array()?.iter().find_map(|input| {
        let body = PayloadRef::from_value(input.get("body")?)?;
        released.find(seq, &body.hash)?;
        input.get("name")?.as_str()
    })
}

/// The binding of `order`, a checker order's `work_order` document of phase
/// `phase`, and the `phase_plan` document that told its round, as a decision
/// observes it. An order whose chain's first order, its `request`, is a
/// lineage's `recheck` order is round 2, bound by the `plan.check_issued` at
/// that lineage's `recheck` sequence, so a retry finds its first attempt's
/// binding and no setting or file of the return is read (D-09, D-10). Any
/// other order is round 1, bound by its `check` input with its `plan` and
/// `truths` inputs' references as the material. A binding that cannot be
/// read is a ledger fault; a read fault is the `failed` answer.
pub(crate) fn order_binding<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    order: &Value,
    phase: u32,
) -> Result<(Binding, ObservedDocument), Value> {
    let key = phase_plan_key(phase);
    let held = store.get(project, PHASE_PLAN_VIEW, &key).map_err(busy)?;
    let chain = order
        .get("request")
        .and_then(Value::as_str)
        .ok_or_else(|| unreadable("the checker order's chain cannot be read"))?;
    let issued = match &held {
        Some(document) => recheck_seq(&document.body, chain)
            .ok_or_else(|| unreadable("the phase's check lineages cannot be read"))?,
        None => None,
    };
    let binding = match issued {
        Some(seq) => round_two_binding(store, project, phase, chain, seq)?,
        None => round_one_binding(store, order, phase)?,
    };
    let phase_plan = ObservedDocument {
        view: PHASE_PLAN_VIEW.into(),
        key,
        produced_seq: held.as_ref().map(|document| document.produced_seq),
    };
    Ok((binding, phase_plan))
}

/// The sequence of the `plan.check_issued` of the lineage whose round-2
/// order is `chain`, if one is, or none when a lineage cannot be read.
fn recheck_seq(phase_plan: &Value, chain: &str) -> Option<Option<u64>> {
    for lineage in phase_plan.get("checks")?.as_array()? {
        let recheck = lineage.get("recheck")?;
        if !recheck.is_null() && recheck.get("work_order")?.as_str()? == chain {
            return Some(Some(recheck.get("seq")?.as_u64()?));
        }
    }
    Some(None)
}

/// The round-1 binding `order`'s own inputs hold.
fn round_one_binding<P: Payloads + ?Sized>(
    payloads: &P,
    order: &Value,
    phase: u32,
) -> Result<Binding, Value> {
    let binding_body = input_body(payloads, order, CHECK_INPUT)?;
    let material = [PLAN_INPUT, TRUTHS_INPUT]
        .into_iter()
        .map(|name| {
            input_ref(order, name).map(|body| OrderInput {
                name: name.into(),
                body,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| unreadable("the checker order's material cannot be read"))?;
    binding(&binding_body, phase, material)
        .ok_or_else(|| unreadable("the checker order's binding cannot be read"))
}

/// The round-2 binding the `plan.check_issued` at `seq` on phase `phase`'s
/// stream holds for order `chain`. One not there, or naming another order or
/// phase, is a ledger fault.
fn round_two_binding<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    phase: u32,
    chain: &str,
    seq: u64,
) -> Result<Binding, Value> {
    let filter = HistoryFilter {
        types: vec![PLAN_CHECK_ISSUED.into()],
        git_commit: None,
    };
    let page = PageRequest {
        limit: 1,
        after: None,
    };
    let stream = phase_stream(phase);
    store
        .history(project, seq..=seq, &filter, page)
        .map_err(busy)?
        .items
        .iter()
        .find(|event| event.stream == stream)
        .and_then(|event| issued_binding(&event.payload, phase, chain))
        .ok_or_else(|| unreadable("the round-2 checker order's binding cannot be read"))
}

/// What the recording does by the round and the findings, prepared outside
/// its transaction (D-07).
enum Outcome {
    /// A round-1 blocker buys the planner this revision order.
    Revise(PreparedOrder),
    /// No revision: the draft's set opens with these questions, and the
    /// receipt previews the draft with the phase's facts read now and its
    /// size before the draft, a replacement's without the plan it replaces.
    Open {
        set: QuestionSet,
        stories: Vec<u32>,
        size: usize,
        facts: PhaseFacts,
        phase_size: u32,
    },
}

/// Records the return `checked` accepted under `command`, the command its
/// caller prepared carrying the checker order and its instructions, under
/// `policy`, the effective policy, for `host` with the catalog `names`, in
/// one transaction (design 0005 section 6, D-07, D-09). It appends
/// `plan.checked` on `phase/<n>` with the round, its previous check for
/// round 2, the findings as judged, the order returned, a retry's own id
/// included, and the binding as issued, attaching its material's references
/// as they are held, each body once; the `work_order` view then marks the
/// order answered. `policy` selects nothing for round 2's binding.
///
/// With a round-1 blocker, the same transaction issues the planner's revision
/// order as `id` (`order::prepare_check_revision`), which `revision` names,
/// declaring that no planner order of the phase is pending and observing
/// its documents, and the receipt is `{status, round, digest, findings,
/// revision}`, `revision` the order's members as its issue gives them.
/// Otherwise `revision` is null, a blocker in round 2 buying none (PLN-R16),
/// and the draft's set opens on `phase/<n>` with the draft's questions, an
/// empty set closing at once; the receipt is
/// the one a recorded draft gives (`submit::draft_members`, its preview by
/// `submit::with_preview`), with its `phase_size`, `reaching` and
/// `capacity` from the phase's facts read now under the policy's capacity,
/// a replacement's size without the plan it replaces
/// (`replace::facts_without`) as the gate-off receipt counts it, and `set`,
/// `now`, `round` and `findings`. A warning never buys a round.
///
/// Every decision observes the order's `work_order` document and the
/// `phase_plan` document and expects the `retention` stream where the check
/// read it, so a second return, a failure, a moved lineage or a purge since
/// the check makes the call `ledger-busy` with nothing recorded, and a repeat
/// is judged afresh. A replay is answered from the record before any decision
/// runs, so it passes no `checked`.
pub fn record<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    checked: Option<&CheckedReturn>,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Value {
    let outcome = match checked {
        None => None,
        Some(checked) => match outcome(store, command, checked, policy, host, names, id) {
            Ok(outcome) => Some(outcome),
            Err(answer) => return answer,
        },
    };
    let at = command.recorded_at.as_str();
    let result = store.transact(command, &mut |tx| {
        let (Some(checked), Some(outcome)) = (checked, outcome.as_ref()) else {
            return Err(StoreError::Unavailable(
                "a checker's return reached its transaction without its check's read".into(),
            ));
        };
        // Before the attachments: a purged body would refuse them as if the
        // caller had sent it.
        tx.expect(&StreamName(RETENTION_STREAM.into()), checked.retention)?;
        let binding = &checked.binding;
        let revision = match outcome {
            Outcome::Revise(order) => Some(order.id()),
            Outcome::Open { .. } => None,
        };
        let result = CheckResult {
            phase: checked.phase,
            plan_digest: binding.plan_digest,
            findings: checked.findings.findings(),
            work_order: &checked.work_order,
            scope: binding.scope,
            previous_check: binding.previous_check,
            truth_versions: &binding.truth_versions,
            material: &binding.material,
            scope_policy_version: binding.scope_policy_version,
            revision,
        };
        tx.append(NewEvent {
            stream: StreamName(phase_stream(checked.phase)),
            type_name: PLAN_CHECKED.into(),
            type_version: PLAN_CHECKED_VERSION,
            git: None,
            payload: result.payload(),
            // An unchanged revision's `original` and `plan` name one body.
            attachments: attachments(&binding.material),
        })?;
        let mut observed = Observed {
            documents: checked.documents.clone(),
            ..Observed::default()
        };
        let findings = json!(checked.findings.findings());
        let answer = match outcome {
            Outcome::Revise(order) => {
                let members = order.issue_within(tx, at)?;
                observed.documents.extend_from_slice(order.documents());
                observed.absences = planner_absences(checked.phase);
                json!({"status": "ok", "round": binding.round,
                    "digest": binding.plan_digest.to_hex(), "findings": findings,
                    "revision": members})
            }
            Outcome::Open {
                set,
                stories,
                size,
                facts,
                phase_size,
            } => {
                let subject = Subject::Plan {
                    phase: checked.phase,
                    plan_digest: binding.plan_digest,
                };
                let opened = open(tx, set, &subject, at)?;
                observed.documents.extend_from_slice(facts.documents());
                let mut receipt = draft_members(
                    binding.plan_digest,
                    checked.phase,
                    stories,
                    *size,
                    *phase_size,
                    facts.facts.capacity,
                    at,
                );
                receipt.insert("set".into(), json!(opened));
                receipt.insert("round".into(), json!(binding.round));
                receipt.insert("findings".into(), findings);
                let answer = with_now(receipt, &opened, &round_of(&opened_facts(set)));
                with_preview(answer, checked.body.clone(), binding.plan_digest)
            }
        };
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive: false,
            observed,
            git: None,
        })
    });
    crate::scope::submit::answer(result, store)
}

/// What the recording of `checked` does, prepared outside its transaction:
/// the revision order as `id` for a round-1 blocker, otherwise the
/// draft's set read back from its questions with `judge_set` and the
/// phase's facts read now. A replacement draft's phase size is judged
/// without the plan it replaces, a replacement the phase no longer allows
/// refused at slot `replaces` with nothing recorded. A draft whose
/// questions or stories cannot be read is a ledger fault; a fault is the
/// answer to give.
fn outcome<S: Views + Payloads + ?Sized>(
    store: &S,
    command: &Command,
    checked: &CheckedReturn,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Result<Outcome, Value> {
    // Round 2's blockers go to the owner with the preview (PLN-R16).
    if checked.binding.previous_check.is_none() && checked.findings.has_blocker() {
        return prepare_check_revision(
            store,
            command,
            checked.phase,
            checked.binding.plan_digest,
            checked.findings.findings(),
            policy,
            host,
            names,
            id,
        )
        .map(Outcome::Revise);
    }
    let plan = &checked.draft.plan;
    let set = plan
        .get("questions")
        .and_then(|questions| judge_set(questions).ok())
        .ok_or_else(|| unreadable("the checked draft's questions cannot be read"))?;
    let stories = plan
        .get("stories")
        .and_then(Value::as_array)
        .and_then(|stories| {
            stories
                .iter()
                .map(|story| story.as_u64().and_then(|story| u32::try_from(story).ok()))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| unreadable("the checked draft's stories cannot be read"))?;
    let size = plan
        .get("tasks")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| unreadable("the checked draft's tasks cannot be read"))?;
    let facts = facts::read(
        store,
        &command.project,
        checked.phase,
        facts::capacity(policy),
    )?;
    // The gate-off receipt takes the replaced plan's tasks off the same way.
    let phase_size = match checked.draft.replaces {
        None => facts.facts.size,
        Some(replaces) => {
            facts_without(
                store,
                &command.project,
                checked.phase,
                replaces,
                &facts,
                "nothing was recorded",
            )?
            .size
        }
    };
    Ok(Outcome::Open {
        set,
        stories,
        size,
        facts,
        phase_size,
    })
}

/// The phase `subject` names when it is `phase/<n>` exactly, as
/// `phase_subject` spells it.
fn phase_of_subject(subject: &str) -> Option<u32> {
    let phase = subject.strip_prefix("phase/")?.parse::<u32>().ok()?;
    (phase > 0 && phase_subject(phase) == subject).then_some(phase)
}

/// The reference of the input `name` an order's document lists.
fn input_ref(order: &Value, name: &str) -> Option<PayloadRef> {
    order
        .get("inputs")?
        .as_array()?
        .iter()
        .find(|input| input.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|input| PayloadRef::from_value(input.get("body")?))
}

/// The input `name` an order's document lists, its body read and parsed as
/// JSON. A body not listed, no longer held or not JSON is a ledger fault.
fn input_body<P: Payloads + ?Sized>(
    payloads: &P,
    order: &Value,
    name: &str,
) -> Result<Value, Value> {
    let reference = input_ref(order, name).ok_or_else(|| missing_input(name))?;
    body_at(payloads, &reference, name)
}

/// The body `reference` names, read and parsed as JSON, for the input
/// `name`. A body no longer held or not JSON is a ledger fault.
fn body_at<P: Payloads + ?Sized>(
    payloads: &P,
    reference: &PayloadRef,
    name: &str,
) -> Result<Value, Value> {
    let missing = || missing_input(name);
    let mut reader = match payloads.open(&reference.hash).map_err(busy)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => return Err(missing()),
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|_| missing())?;
    serde_json::from_slice(&bytes).map_err(|_| missing())
}

fn missing_input(name: &str) -> Value {
    unreadable(&format!("the checker order's {name} input cannot be read"))
}

/// The round-1 binding a `check` input's body holds for phase `phase`, with
/// `material` as its material, or none when a member cannot be read, the
/// phase differs or the round is not 1.
fn binding(check: &Value, phase: u32, material: Vec<OrderInput>) -> Option<Binding> {
    // Round 2's binding is the one `plan.check_issued` records, so an order
    // holding another round is not read here.
    let round = whole(check, "round").filter(|round| *round == 1)?;
    if small(check, "phase")? != phase {
        return None;
    }
    Some(Binding {
        round,
        previous_check: None,
        plan_digest: digest(check, "plan_digest")?,
        scope: scope_of(check)?,
        truth_versions: truth_versions(check)?,
        scope_policy_version: whole(check, "scope_policy_version")?,
        material,
    })
}

/// The round-2 binding a `plan.check_issued` payload holds for order `chain`
/// of phase `phase`, or none when a member cannot be read, the order or
/// phase differs, or the round is not 2. Round 1's draft is the original, so
/// the two digests naming it must agree.
fn issued_binding(issue: &Value, phase: u32, chain: &str) -> Option<Binding> {
    let round = whole(issue, "round").filter(|round| *round == 2)?;
    if small(issue, "phase")? != phase || issue.get("work_order")?.as_str()? != chain {
        return None;
    }
    let previous = issue.get("previous_check")?;
    let previous_check = PreviousCheck {
        seq: whole(previous, "seq")?,
        plan_digest: digest(previous, "plan_digest")?,
    };
    if digest(issue, "original_plan_digest")? != previous_check.plan_digest {
        return None;
    }
    let material = issue
        .get("material")?
        .as_array()?
        .iter()
        .map(|input| {
            Some(OrderInput {
                name: input.get("name")?.as_str()?.into(),
                body: PayloadRef::from_value(input.get("body")?)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Binding {
        round,
        previous_check: Some(previous_check),
        plan_digest: digest(issue, "plan_digest")?,
        scope: scope_of(issue)?,
        truth_versions: truth_versions(issue)?,
        scope_policy_version: whole(issue, "scope_policy_version")?,
        material,
    })
}

fn whole(value: &Value, name: &str) -> Option<u64> {
    value.get(name).and_then(Value::as_u64)
}

fn small(value: &Value, name: &str) -> Option<u32> {
    whole(value, name).and_then(|n| u32::try_from(n).ok())
}

fn digest(value: &Value, name: &str) -> Option<Hash> {
    value
        .get(name)
        .and_then(Value::as_str)
        .and_then(Hash::from_hex)
}

fn scope_of(value: &Value) -> Option<RecheckScope> {
    value
        .get("scope")
        .and_then(Value::as_str)
        .and_then(RecheckScope::parse)
}

fn truth_versions(value: &Value) -> Option<Vec<TruthVersion>> {
    value
        .get("truth_versions")?
        .as_array()?
        .iter()
        .map(|truth| {
            Some(TruthVersion {
                story: small(truth, "story")?,
                truth: truth.get("truth")?.as_str()?.into(),
                version: small(truth, "version")?,
            })
        })
        .collect()
}

fn no_such_work_order() -> Value {
    Refusal::new(
        NO_SUCH_WORK_ORDER,
        "this project holds no work order with that id, so nothing was recorded",
    )
    .slot("work_order")
    .value()
}

fn wrong_work_order() -> Value {
    Refusal::new(
        WRONG_WORK_ORDER,
        "that work order is not a checker order of a phase, so these findings cannot answer it, and nothing was recorded",
    )
    .slot("work_order")
    .value()
}

/// The refusal for checker order `work_order`, which is `state` rather than
/// pending.
fn not_pending(work_order: &str, state: &str) -> Value {
    Refusal::new(
        WORK_ORDER_NOT_PENDING,
        format!(
            "checker work order {work_order} is {state}, so these findings were not recorded. A failed order is retried with `work-order-retry`, and an answered one has already returned"
        ),
    )
    .slot("work_order")
    .details(json!({"state": state}))
    .value()
}

/// The refusal for checker order `work_order`, whose input `input` this
/// project purged.
fn material_unavailable(work_order: &str, input: &str) -> Value {
    let way_out = if input == TRUTHS_INPUT {
        // A fresh order would store the same text, which a purge refuses.
        "No fresh order or check can run until the truth text or the stories the plan serves change, since a purged text cannot be stored again and a retry carries the same inputs. Then fail the order with `work-order-fail`, and a fresh `phase-plan` order starts a new lineage"
    } else {
        "Fail the order with `work-order-fail`, then a fresh `phase-plan` order starts a new lineage, whose submit gets a full first check, since a retry carries the same inputs"
    };
    Refusal::new(
        MATERIAL_UNAVAILABLE,
        format!(
            "this project purged the {input} input of checker work order {work_order}, so these findings were not recorded, and sending them again changes nothing. {way_out}"
        ),
    )
    .slot("work_order")
    .details(json!({"input": input}))
    .value()
}

/// The refusal for a findings list `judge_findings` refused.
fn finding_refused(refusal: &CheckRefusal) -> Value {
    let position = match refusal {
        CheckRefusal::FindingShape { position } => *position,
        // The findings judge refuses nothing else.
        _ => 0,
    };
    Refusal::new(
        refusal.code(),
        format!(
            "the entry at position {position}, counted from 0, is not a finding: a finding needs a `severity` of `blocker` or `warning`, a `location`, a `claim` and a `fix`, and no other member, so nothing was recorded"
        ),
    )
    .slot("findings")
    .details(refusal.details())
    .value()
}

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
}

fn unreadable(reason: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("{reason}, so nothing was recorded"),
        PLACE_LEDGER,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005
    //! PLN-R16 and sections 5 to 7, design 0008 REV-R6, D-04, D-06 to D-08,
    //! D-14, D-17 and the plan's data store C, O, PL1, PL2, P0, POFF, PB,
    //! PDF, PAJ, N0, C1, FB, FW and FN, never from running this code.

    use baley_core::policy::{EffectivePolicy, Host};
    use baley_store::Caller;
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::planning::checker::pending_checker;
    use crate::planning::checker::tests::C1;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit;
    use crate::planning::submit::tests::order_o;
    use crate::refine::order::tests::issued;
    use crate::scope::fixtures::*;

    /// V, the planner revision order a round-1 blocker issues.
    pub(crate) const V: &str = "2d3e4f50-6172-4c83-9d94-aea5f6071823";

    /// FB, a blocker.
    pub(crate) fn fb() -> Value {
        json!({"severity": "blocker", "location": "tasks[0]",
            "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    /// FW, a warning.
    pub(crate) fn fw() -> Value {
        json!({"severity": "warning", "location": "tasks[0].verify[0]",
            "claim": "The verify runs every ledger test", "fix": "Name the round test"})
    }

    /// FN, FB without its severity.
    pub(crate) fn fn_() -> Value {
        json!({"location": "tasks[0]", "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    /// Submits `plan` answering `order` with no capacity through the check
    /// and the gated recording under `policy` for Claude Code with N0, its
    /// checker order, if any, as `id`, under a command at policy version 5
    /// whose caller carries the order and its instructions, under
    /// `request_id` at `at`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gated(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        gated_replacing(store, plan, None, order, policy, id, request_id, at)
    }

    /// As [`gated`], the plan replacing `replaces` when one is named, as
    /// `plan-replace` runs its check below the session.
    #[allow(clippy::too_many_arguments)]
    fn gated_replacing(
        store: &SqliteStore,
        plan: &Value,
        replaces: Option<baley_core::plan::Replaces>,
        order: &str,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        let checked = match submit::checked(store, &project(), plan, order, replaces, None) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.submit", request_id, 8, at);
        command.policy_version = 5;
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(Caller::Server(on));
        submit::record_gated(
            store,
            &command,
            Some(&checked),
            policy,
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    #[test]
    fn a_finding_without_severity_recorded_or_the_plan_gate_read_wrongly_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let off = gated(&store, &pl1(), O, &poff(), C1, R2, T5);
        assert_eq!(off["status"], "ok", "{off}");
        assert!(off.get("preview").is_some(), "{off}");
        assert!(off["set"].is_string(), "{off}");
        assert!(off.get("check").is_none(), "{off}");
        assert!(
            issued(&store)
                .iter()
                .all(|event| event.payload["role"] != "checker")
        );

        for policy in [pb(), pdf(), paj()] {
            let (_dir, store) = store_c();
            order_o(&store, R1);
            let receipt = gated(&store, &pl1(), O, &policy, C1, R2, T5);
            assert_eq!(receipt["status"], "ok", "{receipt}");
            assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
            let checks: Vec<_> = issued(&store)
                .into_iter()
                .filter(|event| event.payload["id"] == C1)
                .collect();
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].payload["role"], "checker");
        }

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl1(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        let before = head(&store);
        let refusal = check(&store, &project(), C1, vec![fw(), fn_()]).expect_err("FN is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "finding-shape", "{refusal}");
        assert_eq!(refusal["slot"], "findings", "{refusal}");
        assert_eq!(refusal["details"], json!({"position": 1}));
        assert_eq!(head(&store), before);
        assert_eq!(
            pending_checker(&store, &project(), 1).unwrap().as_deref(),
            Some(C1)
        );
    }

    /// An id no order has.
    const UNKNOWN: &str = "5d6e7f80-91a2-4bc3-8d4e-5f6071829304";

    /// Store C with O, then PL2 through the gated recording under P0 as C1.
    /// Gives the store and X2.
    fn pl2_checking() -> (tempfile::TempDir, SqliteStore, String) {
        let (dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl2(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        let x2 = receipt["digest"].as_str().unwrap().to_owned();
        (dir, store, x2)
    }

    /// The return of `work_order` with `[FW]`, which must refuse with
    /// `code` at slot `work_order` and record nothing. Gives the refusal.
    fn refused_with(store: &SqliteStore, work_order: &str, code: &str) -> Value {
        let before = head(store);
        let refusal =
            check(store, &project(), work_order, vec![fw()]).expect_err("the return is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], "work_order", "{refusal}");
        assert_eq!(head(store), before);
        refusal
    }

    /// A round-1 `plan.checked` of C1 for the draft `digest` with `[FW]` and
    /// no revision, appended by hand under `request_id`.
    fn returned_by_hand(store: &SqliteStore, digest: &str, request_id: &str) {
        use baley_core::plan::{CheckResult, PLAN_CHECKED, PLAN_CHECKED_VERSION};
        use baley_store::{NewEvent, StreamName};

        let findings = [fw()];
        let payload = CheckResult {
            phase: 1,
            plan_digest: Hash::from_hex(digest).unwrap(),
            findings: &findings,
            work_order: C1,
            scope: RecheckScope::Full,
            previous_check: None,
            truth_versions: &[],
            material: &[],
            scope_policy_version: 5,
            revision: None,
        }
        .payload();
        append(
            store,
            request_id,
            T6,
            vec![NewEvent {
                stream: StreamName("phase/1".into()),
                type_name: PLAN_CHECKED.into(),
                type_version: PLAN_CHECKED_VERSION,
                git: None,
                payload,
                attachments: Vec::new(),
            }],
        );
    }

    #[test]
    fn a_return_for_a_planner_order_or_a_returned_check_accepted_is_caught() {
        use crate::planning::order::tests::fail_by_hand;

        let (_dir, store, _) = pl2_checking();
        refused_with(&store, UNKNOWN, "no-such-work-order");
        refused_with(&store, O, "wrong-work-order");
        fail_by_hand(&store, C1, R3);
        let failed = refused_with(&store, C1, "work-order-not-pending");
        assert_eq!(failed["details"], json!({"state": "failed"}));

        let (_dir, store, x2) = pl2_checking();
        returned_by_hand(&store, &x2, R3);
        let answered = refused_with(&store, C1, "work-order-not-pending");
        assert_eq!(answered["details"], json!({"state": "answered"}));
    }

    /// The references C1's issued order holds for its `plan` and `truths`
    /// inputs, in that order.
    pub(crate) fn c1_material(store: &SqliteStore) -> Vec<OrderInput> {
        let order = issued(store)
            .into_iter()
            .find(|event| event.payload["id"] == C1)
            .expect("C1 is issued");
        ["plan", "truths"]
            .into_iter()
            .map(|name| {
                let input = order.payload["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|input| input["name"] == name)
                    .unwrap();
                OrderInput {
                    name: name.into(),
                    body: PayloadRef::from_value(&input["body"]).unwrap(),
                }
            })
            .collect()
    }

    /// TV: story 1's `t1` and `t2`, each at version 1.
    pub(crate) fn tv() -> Vec<TruthVersion> {
        ["t1", "t2"]
            .into_iter()
            .map(|truth| TruthVersion {
                story: 1,
                truth: truth.into(),
                version: 1,
            })
            .collect()
    }

    #[test]
    fn a_round_one_binding_read_from_the_current_truths_instead_of_the_order_is_caught() {
        use crate::planning::submit::tests::as_drafted;
        use crate::refine::submit::tests::{t1, t2};

        let (_dir, store, x2) = pl2_checking();
        let mut t2_again = versioned(t2());
        t2_again["outcome"] = json!("the same round, whatever the order");
        t2_again["version"] = json!(2);
        append(
            &store,
            R3,
            T6,
            vec![refined(
                1,
                "1b1b1b1b-1b1b-4b1b-8b1b-1b1b1b1b1b1b",
                vec![versioned(t1()), t2_again],
            )],
        );

        let checked = check(&store, &project(), C1, vec![fw()]).expect("the return is checked");
        assert_eq!(
            checked.binding,
            Binding {
                round: 1,
                previous_check: None,
                plan_digest: Hash::from_hex(&x2).unwrap(),
                scope: RecheckScope::Full,
                truth_versions: tv(),
                scope_policy_version: 5,
                material: c1_material(&store),
            }
        );
        assert_eq!(checked.phase, 1);
        assert_eq!(checked.draft.plan, pl2());
        assert_eq!(checked.body, as_drafted(pl2()));
    }

    /// The `plan.check_return` command under `request_id` at `at` and policy
    /// version `version`, its caller carrying `work_order` and `checked`'s
    /// instructions.
    fn return_command(
        work_order: &str,
        checked: &CheckedReturn,
        request_id: &str,
        at: &str,
        version: u64,
    ) -> Command {
        let mut command = command("plan.check_return", request_id, 8, at);
        command.policy_version = version;
        let on = caller()
            .with_work_order(work_order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(Caller::Server(on));
        command
    }

    /// Returns `work_order` with `findings` through the check and the
    /// recording under `policy` for Claude Code with N0, a revision, if any,
    /// as `id`, under `request_id` at `at` and policy version `version`.
    /// Gives the answer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn returned(
        store: &SqliteStore,
        work_order: &str,
        findings: Vec<Value>,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
        version: u64,
    ) -> Value {
        let checked = match check(store, &project(), work_order, findings) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let command = return_command(work_order, &checked, request_id, at, version);
        record(
            store,
            &command,
            Some(&checked),
            policy,
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    #[test]
    fn a_round_one_result_recorded_off_its_order_or_with_another_binding_is_caught() {
        let (_dir, store, x2) = pl2_checking();
        let receipt = returned(&store, C1, vec![fb(), fw()], &p0(), V, R3, T6, 9);
        assert_eq!(receipt["status"], "ok", "{receipt}");

        let events = history(&store);
        let checked = of_type(&events, "plan.checked");
        assert_eq!(checked.len(), 1);
        let event = checked[0];
        assert_eq!(event.stream, "phase/1");
        let material: Vec<Value> = c1_material(&store)
            .into_iter()
            .map(|input| json!({"name": input.name, "body": input.body.to_value()}))
            .collect();
        // The store refuses a payload reference an event does not attach, so
        // this material recorded is this material attached.
        assert_eq!(
            event.payload,
            json!({"phase": 1, "plan_digest": x2, "findings": [fb(), fw()], "round": 1,
                "work_order": C1, "scope": "full",
                "truth_versions": [{"story": 1, "truth": "t1", "version": 1},
                    {"story": 1, "truth": "t2", "version": 1}],
                "material": material, "scope_policy_version": 5, "revision": V})
        );

        let order = crate::work_order::read::order(&store, &project(), C1)
            .unwrap()
            .unwrap();
        assert_eq!(order["state"], "answered", "{order}");
        assert_eq!(order["drafts"], json!([x2]), "{order}");
        let phase_plan = store
            .get(
                &project(),
                baley_core::plan::PHASE_PLAN_VIEW,
                &baley_core::plan::phase_plan_key(1),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            phase_plan.body["checks"],
            json!([{"revision": V, "first": {"seq": event.seq, "plan_digest": x2},
                "recheck": null}])
        );
    }

    #[test]
    fn a_return_landing_after_its_order_failed_is_caught() {
        use crate::planning::order::tests::fail_by_hand;

        let (_dir, store, _) = pl2_checking();
        let checked = check(&store, &project(), C1, vec![fw()]).expect("the return is checked");
        fail_by_hand(&store, C1, R3);
        let before = head(&store);
        let answer = record(
            &store,
            &return_command(C1, &checked, R4, T7, 5),
            Some(&checked),
            &p0(),
            Host::ClaudeCode,
            &n0(),
            V,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }

    /// Store C with O, then PL2 through the gated recording under P0 as
    /// C1, whose receipt must name C1 and give no preview, set or round.
    /// Gives the store and X2.
    fn pl2_gated() -> (tempfile::TempDir, SqliteStore, Hash) {
        let (dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl2(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        for member in ["preview", "set", "now"] {
            assert!(receipt.get(member).is_none(), "{member}: {receipt}");
        }
        let x2 = Hash::from_hex(receipt["digest"].as_str().unwrap()).unwrap();
        (dir, store, x2)
    }

    /// The approval's first step for `digest` under P0, which must refuse
    /// with `code` at slot `digest` and record nothing.
    fn approval_refused(store: &SqliteStore, digest: Hash, code: &str) {
        let before = head(store);
        let refusal = crate::planning::approve::judge(store, &project(), digest, &p0())
            .expect_err("the approval is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
        assert_eq!(head(store), before);
    }

    #[test]
    fn a_checked_draft_previewed_approved_mid_check_or_given_two_revisions_is_caught() {
        // Pending: round 1 has not returned.
        let (_dir, store, x2) = pl2_gated();
        approval_refused(&store, x2, "check-unfinished");
        let before = head(&store);
        let refusal = submit::check(&store, &project(), &pl3(), O, None)
            .expect_err("a submit beside the checker order is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "checker-order-pending", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": C1}));
        assert_eq!(head(&store), before);

        // Blocker: one revision order, no set.
        let (_dir, store, x2) = pl2_gated();
        let receipt = returned(&store, C1, vec![fb(), fw()], &p0(), V, R3, T6, 5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["round"], 1, "{receipt}");
        assert_eq!(receipt["digest"], x2.to_hex(), "{receipt}");
        assert_eq!(receipt["findings"], json!([fb(), fw()]));
        assert_eq!(receipt["revision"]["work_order"], V, "{receipt}");
        let events = history(&store);
        let mine: Vec<_> = events.iter().filter(|e| e.request_id.0 == R3).collect();
        let orders: Vec<_> = mine
            .iter()
            .filter(|e| e.type_name == "work_order.issued")
            .collect();
        assert_eq!(orders.len(), 1);
        let order = &orders[0].payload;
        assert_eq!(order["id"], V);
        assert_eq!(order["role"], "planner");
        assert_eq!(order["subject"], "phase/1");
        assert_eq!(order["attempt"], 1);
        assert_eq!(order["parent"], Value::Null);
        assert_eq!(
            crate::refine::order::tests::input_names(orders[0]),
            [
                "phase",
                "stories",
                "description",
                "approved_items",
                "draft",
                "findings"
            ]
        );
        assert!(mine.iter().all(|e| e.type_name != "questions.opened"));
        approval_refused(&store, x2, "check-unfinished");

        // Warnings: the set opens and the draft is previewed, no revision.
        let (_dir, store, x2) = pl2_gated();
        let receipt = returned(&store, C1, vec![fw()], &p0(), V, R3, T6, 5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["round"], 1, "{receipt}");
        assert_eq!(receipt["findings"], json!([fw()]));
        assert_eq!(
            receipt["preview"],
            crate::planning::submit::tests::as_drafted(pl2())
        );
        assert!(receipt["set"].is_string(), "{receipt}");
        assert_eq!(receipt["now"]["round"], 1, "{receipt}");
        assert_eq!(
            receipt["now"]["current"],
            json!([{"id": "q1", "question": "Which store holds the ledger?",
                "recommended": "SQLite"}])
        );
        assert!(receipt.get("revision").is_none(), "{receipt}");
        let events = history(&store);
        assert!(
            events
                .iter()
                .filter(|e| e.request_id.0 == R3)
                .all(|e| { e.type_name != "work_order.issued" || e.payload["role"] != "planner" })
        );
        approval_refused(&store, x2, "question-open");
    }

    #[test]
    fn a_replacement_previewed_with_the_replaced_plan_still_counted_is_caught() {
        use crate::planning::approve::tests::{approved, digest_of};
        use crate::planning::submit::tests::plan_submitted;

        // Plan 1, PL1 with its one task, in force.
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        let approval = approved(&store, x1, &p0(), R3, T5);
        assert_eq!(approval["plan"], 1, "{approval}");

        // PL2, also one task, replacing plan 1 under P0, then a warning back.
        let replaces = baley_core::plan::Replaces {
            plan: 1,
            digest: x1,
        };
        let submitted = gated_replacing(&store, &pl2(), Some(replaces), O, &p0(), C1, R4, T6);
        assert_eq!(submitted["check"]["work_order"], C1, "{submitted}");
        let receipt = returned(&store, C1, vec![fw()], &p0(), V, R5, T7, 5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        // Plan 1's task leaves with it, as the gate-off receipt counts it.
        assert_eq!(receipt["phase_size"], 0, "{receipt}");
        assert_eq!(receipt["reaching"], 1, "{receipt}");
    }

    /// C2, the round-2 checker order.
    pub(crate) const C2: &str = "c2c2c2c2-2a2b-4c2d-8e2f-303132333435";

    /// `plan` answering `order` through the check, the lineage step under
    /// `policy` and the gated recording under the same policy for Claude
    /// Code with N0, its checker order as `id`, under a command at policy
    /// version `version` whose caller carries the order and its
    /// instructions, under `request_id` at `at`. Gives the answer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn through_lineage(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
        version: u64,
    ) -> Value {
        let checked = submit::check(store, &project(), plan, order, None)
            .and_then(|checked| crate::planning::recheck::judge(store, &project(), checked, policy))
            .expect("the plan is checked and judged");
        let mut command = command("plan.submit", request_id, 8, at);
        command.policy_version = version;
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(Caller::Server(on));
        submit::record_gated(
            store,
            &command,
            Some(&checked),
            policy,
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    /// The lineage: store C with O; PL2 through the gated recording under
    /// P0 as C1 at policy version 5 under R2 at T5; C1's return of
    /// `[FB, FW]` as V under R3 at T6, the round-1 result's sequence being
    /// K; and PL2r under V through the gated recording under `policy` as C2
    /// at policy version `version` under R4 at T7. Gives the store, X2, X2r
    /// and K.
    pub(crate) fn lineage(
        policy: &EffectivePolicy,
        version: u64,
    ) -> (tempfile::TempDir, SqliteStore, String, String, u64) {
        let (dir, store, x2) = pl2_checking();
        let back = returned(&store, C1, vec![fb(), fw()], &p0(), V, R3, T6, 5);
        assert_eq!(back["revision"]["work_order"], V, "{back}");
        let k = of_type(&history(&store), "plan.checked")[0].seq;
        let revised = through_lineage(&store, &pl2r(), V, policy, C2, R4, T7, version);
        assert_eq!(revised["check"]["work_order"], C2, "{revised}");
        let x2r = revised["digest"].as_str().unwrap().to_owned();
        (dir, store, x2, x2r, k)
    }

    /// The issue of order `id`.
    fn issue_of(store: &SqliteStore, id: &str) -> baley_store::Event {
        issued(store)
            .into_iter()
            .find(|event| event.payload["id"] == id)
            .expect("the order is issued")
    }

    #[test]
    fn a_recheck_reading_the_scope_selected_now_instead_of_at_issue_is_caught() {
        use crate::planning::submit::tests::as_drafted;

        let (_dir, store, x2, x2r, k) = lineage(&pd(), 6);
        let events = history(&store);
        let issues = of_type(&events, "plan.check_issued");
        assert_eq!(issues.len(), 1);
        let issue = issues[0];
        let order = issue_of(&store, C2);
        assert_eq!(issue.request_id.0, R4);
        assert_eq!(order.request_id.0, R4);
        assert!(issue.seq < order.seq, "the binding is recorded first");

        let material = issue.payload["material"].clone();
        let tv = json!([{"story": 1, "truth": "t1", "version": 1},
            {"story": 1, "truth": "t2", "version": 1}]);
        let previous = json!({"seq": k, "plan_digest": x2});
        assert_eq!(
            issue.payload,
            json!({"phase": 1, "work_order": C2, "round": 2, "previous_check": previous,
                "original_plan_digest": x2, "plan_digest": x2r, "truth_versions": tv,
                "scope": "diff", "scope_policy_version": 6, "material": material})
        );
        assert_eq!(order.payload["inputs"], material);
        let names: Vec<&str> = material
            .as_array()
            .unwrap()
            .iter()
            .map(|input| input["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["original", "plan", "truths", "delta", "blockers"]);

        // `recheck` is now `full`, under a command at policy version 9.
        let receipt = returned(&store, C2, vec![fw()], &p0(), V, R5, T8, 9);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        let events = history(&store);
        let results = of_type(&events, "plan.checked");
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[1].payload,
            json!({"phase": 1, "plan_digest": x2r, "findings": [fw()], "round": 2,
                "work_order": C2, "scope": "diff", "previous_check": previous,
                "truth_versions": tv, "material": material, "scope_policy_version": 6,
                "revision": null})
        );
        assert_eq!(receipt["round"], 2, "{receipt}");
        assert_eq!(receipt["findings"], json!([fw()]));
        assert_eq!(receipt["preview"], as_drafted(pl2r()));
        assert!(receipt["set"].is_string(), "{receipt}");
        assert_eq!(receipt["now"]["round"], 1, "{receipt}");
        assert_eq!(
            receipt["now"]["current"],
            json!([{"id": "q1", "question": "Which store holds the ledger?",
                "recommended": "SQLite"}])
        );
        assert!(receipt.get("revision").is_none(), "{receipt}");
        let order = crate::work_order::read::order(&store, &project(), C2)
            .unwrap()
            .unwrap();
        assert_eq!(order["state"], "answered", "{order}");
    }

    #[test]
    fn a_round_two_blocker_buying_a_revision_is_caught() {
        use crate::planning::submit::tests::as_drafted;

        let (_dir, store, _, x2r, _) = lineage(&p0(), 5);
        let receipt = returned(&store, C2, vec![fb()], &p0(), V, R5, T8, 5);
        assert_eq!(receipt["status"], "ok", "{receipt}");

        let events = history(&store);
        let mine: Vec<_> = events.iter().filter(|e| e.request_id.0 == R5).collect();
        assert!(mine.iter().all(|e| e.type_name != "work_order.issued"));
        let result: Vec<_> = mine
            .iter()
            .filter(|e| e.type_name == "plan.checked")
            .collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].payload["round"], 2);
        assert_eq!(result[0].payload["revision"], Value::Null);
        let opened: Vec<_> = mine
            .iter()
            .filter(|e| e.type_name == "questions.opened")
            .collect();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].payload["plan_digest"], x2r.as_str());

        assert_eq!(receipt["findings"], json!([fb()]));
        assert_eq!(receipt["preview"], as_drafted(pl2r()));
        assert!(receipt["set"].is_string(), "{receipt}");
        assert!(receipt.get("revision").is_none(), "{receipt}");
    }

    #[test]
    fn a_round_two_return_of_an_unchanged_revision_attaching_one_body_twice_is_caught() {
        let (_dir, store, x2) = pl2_checking();
        let back = returned(&store, C1, vec![fb(), fw()], &p0(), V, R3, T6, 5);
        assert_eq!(back["revision"]["work_order"], V, "{back}");

        // The revision changes nothing, so the draft keeps its digest.
        let revised = through_lineage(&store, &pl2(), V, &p0(), C2, R4, T7, 5);
        assert_eq!(revised["check"]["work_order"], C2, "{revised}");
        assert_eq!(revised["digest"], x2.as_str(), "{revised}");
        let inputs = issue_of(&store, C2).payload["inputs"].clone();
        let body_of = |name: &str| {
            inputs
                .as_array()
                .unwrap()
                .iter()
                .find(|input| input["name"] == name)
                .unwrap()["body"]
                .clone()
        };
        assert_eq!(body_of("original"), body_of("plan"));

        let receipt = returned(&store, C2, vec![fw()], &p0(), V, R5, T8, 5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["round"], 2, "{receipt}");
        let events = history(&store);
        let results = of_type(&events, "plan.checked");
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].payload["round"], 2);
        assert_eq!(results[1].payload["plan_digest"], x2.as_str());
        assert_eq!(results[1].payload["work_order"], C2);
        assert_eq!(results[1].payload["material"], inputs);
    }

    #[test]
    fn a_purged_input_blamed_on_the_returned_submission_is_caught() {
        for name in ["original", "plan", "truths", "delta", "blockers"] {
            let (_dir, store, ..) = lineage(&pd(), 6);
            let held = issue_of(&store, C2);
            let input = held.payload["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|input| input["name"] == name)
                .unwrap();
            let gone = PayloadRef::from_value(&input["body"]).unwrap().hash;
            purge_in(&store, &project(), gone, "the owner purged it");

            let refusal = refused_with(&store, C2, "material-unavailable");
            assert_eq!(refusal["details"], json!({"input": name}), "{refusal}");
            let reason = refusal["reason"].as_str().unwrap();
            assert!(reason.contains(name), "{reason}");
            assert!(reason.contains("`work-order-fail`"), "{reason}");
            // A fresh order stores the same truth text, which a purge refuses.
            assert_eq!(
                reason.contains("until the truth text or the stories the plan serves change"),
                name == "truths",
                "{reason}"
            );
            assert_eq!(
                pending_checker(&store, &project(), 1).unwrap().as_deref(),
                Some(C2)
            );
        }

        // Round 1's binding is its own input, purged the same way.
        let (_dir, store, _) = pl2_checking();
        let held = issue_of(&store, C1);
        let input = held.payload["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["name"] == "check")
            .unwrap();
        let gone = PayloadRef::from_value(&input["body"]).unwrap().hash;
        purge_in(&store, &project(), gone, "the owner purged it");
        let refusal = refused_with(&store, C1, "material-unavailable");
        assert_eq!(refusal["details"], json!({"input": "check"}), "{refusal}");
    }

    #[test]
    fn a_purge_landing_after_a_returns_check_blamed_on_the_submission_is_caught() {
        let (_dir, store, ..) = lineage(&pd(), 6);
        let checked = check(&store, &project(), C2, vec![fw()]).expect("every input is held");
        let gone = c1_material(&store)[0].body.hash;
        purge_in(&store, &project(), gone, "the owner purged it");

        let before = head(&store);
        let answer = record(
            &store,
            &return_command(C2, &checked, R5, T8, 6),
            Some(&checked),
            &p0(),
            Host::ClaudeCode,
            &n0(),
            V,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }
}
