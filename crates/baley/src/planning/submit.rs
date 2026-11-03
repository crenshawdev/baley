//! `plan submit` (design 0005 section 5, PLN-R9, PLN-R11 to PLN-R13,
//! PLN-R15, PLN-R25, PLN-R27, D-02, D-03, D-05, D-13): a session submits a
//! planner's plan answering a planner order of its phase. The check runs
//! before admission and records nothing: the order, the phase's pending
//! checker order, the phase's activity, then the plan judged whole by
//! `judge_plan` over the phase's facts read from the views. The recording
//! then stores the draft and abandons the phase's earlier draft's set in
//! one transaction, so a phase holds one draft waiting for approval. Under
//! the plan gate `off` the same transaction opens the new draft's set and
//! the receipt previews the draft; under any other gate it issues round 1
//! of the plan checker's order instead (PLN-R16, D-01, D-14), and the set
//! and the preview wait for the last check.
//!
//! Build 4 T11's risk raise at plan submit still sits between the check and
//! the recording, before the checker's order is issued (design 0005 Figure
//! 5), against the digest the recording takes.

use std::num::NonZeroU32;

use baley_core::plan::{
    PHASE_PLAN_VIEW, PLAN_KIND, PhaseNotActive, Plan, PlanDraft, PlanFacts, PlanRefusal, Replaces,
    judge_plan, judge_submit_phase, phase_plan_key,
};
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, PlanGate, Role};
use baley_core::questions::{Cause, Subject, round_of};
use baley_core::scope::{
    PROJECT_STREAM, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION, submitted_payload,
};
use baley_core::work_order::{
    FAILED, NO_SUCH_WORK_ORDER, WORK_ORDER_VIEW, WRONG_WORK_ORDER, phase_subject, work_order_key,
};
use baley_store::{
    Absence, Command, Decision, InstructionEvidence, Ledger, NewEvent, Observed, ObservedDocument,
    OutcomeKind, Payloads, ProjectId, Recorded, RetentionClass, StoreError, StreamName,
    Transaction, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::parts::PART_BOUND;
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::checker::{self, checker_absences, checker_order_pending, pending_checker};
use crate::planning::{facts, replace};
use crate::questions::answer::with_now;
use crate::questions::record::{AbandonError, abandon, open};
use crate::refine::order::order_evidence;
use crate::refine::questions::opened_facts;
use crate::scope::submit::{NOT_RECORDED_BUSY, answer};
use crate::work_order::read::order_document;

/// What a refused submit did not do.
const NOT_RECORDED: &str = "nothing was recorded";

/// What the check read and accepted, which the recording records and
/// observes.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    /// The plan, judged whole and accepted.
    pub plan: Plan,
    /// The planner order's instruction evidence, which the submission's
    /// caller carries.
    pub evidence: Vec<InstructionEvidence>,
    /// The phase's size before the plan, in tasks.
    pub size: u32,
    /// The capacity the plan was judged under, or none.
    pub capacity: Option<NonZeroU32>,
    /// The approved plan a replacement replaces, or none for a new plan.
    pub replaces: Option<Replaces>,
    /// The order's document, then the documents the phase's facts came
    /// from, each at the sequence that produced it.
    documents: Vec<ObservedDocument>,
    /// What a recording relies on being absent.
    absences: Vec<Absence>,
}

impl Checked {
    /// The size the phase would reach with the plan approved.
    pub fn reaching(&self) -> u64 {
        u64::from(self.size) + self.plan.size() as u64
    }

    /// The draft the plan is recorded as: a replacement's body names the
    /// plan it replaces.
    pub fn draft(&self) -> PlanDraft {
        match self.replaces {
            None => PlanDraft::new(&self.plan),
            Some(replaces) => PlanDraft::replacing(&self.plan, replaces),
        }
    }
}

/// The check of the plan `submitted` answering work order `work_order`
/// under `capacity`, run before admission and recording nothing (D-13). In
/// order: the order, none `no-such-work-order` at slot `work_order`; a plan
/// whose `phase` cannot be read gets the plan judge's own `typed-content`
/// answer at slot `plan`, since the order is judged against that phase; an
/// order that is not a planner order of the plan's phase, or one that
/// failed, `wrong-work-order` at slot `work_order`, while a pending or
/// answered one passes, so a resubmission becomes the newest draft;
/// `checker-order-pending` at slot `operation` naming the phase's pending
/// checker order (D-03), so a draft is never replaced under its own check;
/// `phase-not-active` at slot `plan` when another phase is active (D-02);
/// then the plan judged by `judge_plan` over the phase's facts, a refusal at
/// slot `plan` with what it names. A read fault is the `failed` answer.
pub fn check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    submitted: &Value,
    work_order: &str,
    capacity: Option<NonZeroU32>,
) -> Result<Checked, Value> {
    checked(store, project, submitted, work_order, None, capacity)
}

/// The check `check` describes, shared with a replacement's: with
/// `replaces`, after `phase-not-active` the replacement is judged over the
/// phase's plans in force (D-04), and the plan is judged under the facts
/// that judgement gives, the replaced plan's tasks and items taken off.
pub(crate) fn checked<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    submitted: &Value,
    work_order: &str,
    replaces: Option<Replaces>,
    capacity: Option<NonZeroU32>,
) -> Result<Checked, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let order = order_document(store, project, work_order)
        .map_err(busy)?
        .ok_or_else(|| {
            Refusal::new(
                NO_SUCH_WORK_ORDER,
                "this project holds no work order with that id, so nothing was recorded",
            )
            .slot("work_order")
            .value()
        })?;
    let Some(phase) = phase_of(submitted) else {
        let empty = PlanFacts {
            stories: Vec::new(),
            approved: Vec::new(),
            size: 0,
            capacity: None,
        };
        // The judge refuses a phase it cannot read before anything else it
        // would take from the facts.
        let refusal =
            judge_plan(submitted, &empty)
                .err()
                .unwrap_or_else(|| PlanRefusal::TypedContent {
                    path: Some("phase".into()),
                });
        return Err(plan_refused(&refusal, "plan", NOT_RECORDED));
    };
    if !is_planner_order(&order.body, phase) {
        return Err(Refusal::new(
            WRONG_WORK_ORDER,
            format!(
                "that work order is not a live or answered planner order of phase {phase}, so this plan cannot answer it, and nothing was recorded"
            ),
        )
        .slot("work_order")
        .value());
    }
    // Refused whatever the gate, since the gate is read at the recording
    // and a late return must still find the draft its order checks.
    if let Some(pending) = pending_checker(store, project, phase).map_err(busy)? {
        return Err(checker_order_pending(phase, &pending, NOT_RECORDED));
    }
    let read = facts::read(store, project, phase, capacity)?;
    judge_submit_phase(phase, &read.active)
        .map_err(|refusal| phase_not_active(&refusal, "plan", NOT_RECORDED))?;
    let facts = match replaces {
        None => read.facts.clone(),
        Some(replaces) => {
            replace::facts_without(store, project, phase, replaces, &read, NOT_RECORDED)?
        }
    };
    let plan = judge_plan(submitted, &facts)
        .map_err(|refusal| plan_refused(&refusal, "plan", NOT_RECORDED))?;
    let evidence = order_evidence(&order.body).ok_or_else(|| {
        failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a work order whose instructions cannot be read",
            PLACE_LEDGER,
        )
    })?;
    let mut documents = vec![ObservedDocument {
        view: WORK_ORDER_VIEW.into(),
        key: work_order_key(work_order),
        produced_seq: Some(order.produced_seq),
    }];
    documents.extend_from_slice(read.documents());
    Ok(Checked {
        plan,
        evidence,
        size: facts.size,
        capacity,
        replaces,
        documents,
        absences: read.absences().to_vec(),
    })
}

/// Records the plan `checked` accepted under `command`, the command its
/// caller prepared, carrying the planner order it answers and that order's
/// instructions, in one transaction (D-03, D-05, D-13): its draft body as a
/// `record` payload and one `scope.submitted` of kind `plan` with target
/// `phase/<n>` attaching it on the `project` stream, so the `work_order`
/// view marks the order answered and lists the digest; the phase's current
/// draft's set, read from `phase_plan` here, abandoned with cause
/// `draft-replaced` when it is held and not yet abandoned; and the draft's
/// set opened on `phase/<n>` with the plan's questions, an empty set closing
/// at once. Every call records, so the same plan submitted again is the
/// newest draft again with a new set; only a retry of the same request
/// replays, before any decision runs.
/// [`record_gated`] records through this under the plan gate `off`.
///
/// The decision observes the order's and the facts' documents, so a phase
/// made active, a plan approved or a truth revised since the check makes the
/// call `ledger-busy` with nothing recorded; with no phase active at the
/// check it declares that the active index still holds none. A decision run
/// without the check's read records nothing.
///
/// The receipt: `status`, `digest`, `kind`, `phase`, `stories`, `size` (the
/// plan's tasks), `phase_size`, `reaching` (the phase's size with the plan
/// approved), `capacity` or null, `set`, `recorded_at`, `now` (round 1 in
/// the questions round's shape) and `preview`: the draft's submission when
/// the receipt stays within the part bound, otherwise
/// `{"read": {"kind": "draft", "digest": ..}}`, which `document` serves
/// whole in parts, since a receipt is never cut into parts.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    checked: Option<&Checked>,
) -> Result<Recorded, StoreError> {
    let at = command.recorded_at.as_str();
    store.transact(command, &mut |tx| {
        let Some(checked) = checked else {
            return Err(unread());
        };
        let plan = &checked.plan;
        let draft = record_draft(tx, checked, at)?;
        let subject = Subject::Plan {
            phase: plan.phase(),
            plan_digest: draft.digest(),
        };
        let id = open(tx, plan.questions(), &subject, at)?;
        let mut receipt = draft_receipt(checked, &draft, at);
        receipt.insert("set".into(), json!(id));
        let answer = with_now(receipt, &id, &round_of(&opened_facts(plan.questions())));
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: with_preview(answer, &draft),
            sensitive: false,
            observed: Observed {
                documents: checked.documents.clone(),
                absences: checked.absences.clone(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// Records the plan `checked` accepted under `command`, the command its
/// caller prepared under `policy`, the host's effective policy, for `host`
/// with the catalog `names`, reading the plan gate through
/// `EffectivePolicy::plan_gate` (D-01, D-14, D-16). Under `off` it records
/// through [`record`], unchanged, so the preview and the set follow at once.
/// Under any other gate, set or by default, one transaction records what
/// [`record`] records before the set, the draft answering its planner order
/// and the earlier draft's set abandoned, then issues round 1 of its check
/// as checker order `id` (`checker::prepare`) with the draft's digest. It
/// opens no set and gives no preview: both wait for the last check.
///
/// The decision observes the check's documents and the checker inputs'
/// and declares the check's absences and that no checker order of the
/// phase is pending, so a checker order issued since the check makes the
/// call `ledger-busy` with nothing recorded. The receipt holds `status`,
/// `digest`, `kind`, `phase`, `stories`, `size`, `phase_size`, `reaching`,
/// `capacity`, `recorded_at` and `check`, the checker order's members as
/// its issue gives them. A retry of the same request is answered from the
/// record before any decision runs, so a replay passes no `checked`.
pub fn record_gated<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    checked: Option<&Checked>,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Value {
    if policy.plan_gate().0 == PlanGate::Off {
        return answer(record(store, command, checked), store);
    }
    let prepared = match checked {
        Some(checked) => {
            let draft = checked.draft();
            match checker::prepare(
                store,
                command,
                &checked.plan,
                &draft,
                policy,
                host,
                names,
                id,
            ) {
                Ok(prepared) => Some(prepared),
                Err(answer) => return answer,
            }
        }
        None => None,
    };
    let at = command.recorded_at.as_str();
    let result = store.transact(command, &mut |tx| {
        let (Some(checked), Some(prepared)) = (checked, prepared.as_ref()) else {
            return Err(unread());
        };
        let draft = record_draft(tx, checked, at)?;
        let mut receipt = draft_receipt(checked, &draft, at);
        receipt.insert(
            "check".into(),
            Value::Object(prepared.issue_within(tx, at)?),
        );
        let mut documents = checked.documents.clone();
        documents.extend_from_slice(prepared.documents());
        let mut absences = checked.absences.clone();
        absences.extend(checker_absences(checked.plan.phase()));
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: Value::Object(receipt),
            sensitive: false,
            observed: Observed {
                documents,
                absences,
                ..Observed::default()
            },
            git: None,
        })
    });
    answer(result, store)
}

/// The fault of a decision run without the check's read.
fn unread() -> StoreError {
    StoreError::Unavailable(
        "a plan submission reached its transaction without its check's read".into(),
    )
}

/// Records `checked`'s draft inside `tx` at `at`: its body as a `record`
/// payload and its `scope.submitted` of kind `plan` on the `project` stream,
/// then the phase's current draft's set abandoned `draft-replaced` when it
/// is held and not yet abandoned. Gives the draft.
fn record_draft(
    tx: &mut dyn Transaction,
    checked: &Checked,
    at: &str,
) -> Result<PlanDraft, StoreError> {
    // Read before anything is appended, so the set found is the earlier
    // draft's, never one opened after.
    let earlier = current_set(tx, checked.plan.phase())?;
    let draft = checked.draft();
    let body = tx.put_payload(draft.body(), RetentionClass::Record)?;
    tx.append(NewEvent {
        stream: StreamName(PROJECT_STREAM.into()),
        type_name: SCOPE_SUBMITTED.into(),
        type_version: SCOPE_SUBMITTED_VERSION,
        git: None,
        payload: submitted_payload(draft.digest(), PLAN_KIND, draft.target(), &body),
        attachments: vec![body],
    })?;
    if let Some(set) = earlier {
        match abandon(tx, &set, Cause::DraftReplaced, at) {
            // A set already abandoned, or one the view no longer holds,
            // has nothing left to abandon.
            Ok(()) | Err(AbandonError::NotHeld | AbandonError::Abandoned) => {}
            Err(AbandonError::Store(error)) => return Err(error),
        }
    }
    Ok(draft)
}

/// The receipt members every recorded plan draft gives: `status`, `digest`,
/// `kind`, `phase`, `stories`, `size`, `phase_size`, `reaching`, `capacity`
/// and `recorded_at`.
fn draft_receipt(checked: &Checked, draft: &PlanDraft, at: &str) -> Map<String, Value> {
    let plan = &checked.plan;
    let mut receipt = Map::new();
    receipt.insert("status".into(), json!("ok"));
    receipt.insert("digest".into(), json!(draft.digest().to_hex()));
    receipt.insert("kind".into(), json!(PLAN_KIND));
    receipt.insert("phase".into(), json!(plan.phase()));
    receipt.insert("stories".into(), json!(plan.stories()));
    receipt.insert("size".into(), json!(plan.size()));
    receipt.insert("phase_size".into(), json!(checked.size));
    receipt.insert("reaching".into(), json!(checked.reaching()));
    receipt.insert(
        "capacity".into(),
        json!(checked.capacity.map(NonZeroU32::get)),
    );
    receipt.insert("recorded_at".into(), json!(at));
    receipt
}

/// The set of phase `phase`'s current draft, as `phase_plan` holds it inside
/// `tx`, or none. A draft that cannot be read is a ledger fault.
fn current_set(tx: &mut dyn Transaction, phase: u32) -> Result<Option<String>, StoreError> {
    let Some(document) = tx.get(PHASE_PLAN_VIEW, &phase_plan_key(phase))? else {
        return Ok(None);
    };
    match document.body.get("draft") {
        None | Some(Value::Null) => Ok(None),
        Some(draft) => match draft.get("set").and_then(Value::as_str) {
            Some(set) => Ok(Some(set.to_owned())),
            None => Err(StoreError::Unavailable(format!(
                "phase {phase}'s current plan draft cannot be read"
            ))),
        },
    }
}

/// `answer` with `preview`, the draft's submission when the whole answer
/// stays within the part bound, otherwise the `document` read of the draft.
fn with_preview(answer: Value, draft: &PlanDraft) -> Value {
    let Value::Object(mut receipt) = answer else {
        return answer;
    };
    // An accepted plan's draft body is canonical JSON.
    let submission: Value = serde_json::from_slice(draft.body()).expect("a draft body is JSON");
    receipt.insert("preview".into(), submission);
    let text = serde_json::to_string(&receipt).expect("a receipt serializes");
    if text.len() > PART_BOUND {
        receipt.insert(
            "preview".into(),
            json!({"read": {"kind": "draft", "digest": draft.digest().to_hex()}}),
        );
    }
    Value::Object(receipt)
}

/// The plan's `phase` when it is a whole number from 1 to `u32::MAX`, as
/// the plan judge reads it.
fn phase_of(submitted: &Value) -> Option<u32> {
    submitted
        .get("phase")
        .and_then(Value::as_u64)
        .and_then(|phase| u32::try_from(phase).ok())
        .filter(|phase| *phase > 0)
}

/// Whether `order` is a planner order of phase `phase` that has not failed.
fn is_planner_order(order: &Value, phase: u32) -> bool {
    let text = |field: &str| order.get(field).and_then(Value::as_str);
    text("role") == Some(Role::Planner.name())
        && text("subject") == Some(phase_subject(phase).as_str())
        && text("state") != Some(FAILED)
}

/// The refusal for `refusal`, another phase being active, at `slot`, its
/// reason ending `not_done`.
pub(crate) fn phase_not_active(refusal: &PhaseNotActive, slot: &str, not_done: &str) -> Value {
    Refusal::new(
        refusal.code(),
        format!(
            "phase {} is active, and plans are made for one phase at a time, so {not_done}. A phase stays active until it is completed or withdrawn",
            refusal.phase
        ),
    )
    .slot(slot)
    .details(refusal.details())
    .value()
}

/// The refusal for `refusal`, the plan judge's, at `slot`, its reason
/// ending `not_done` and its details naming what it is about.
pub(crate) fn plan_refused(refusal: &PlanRefusal, slot: &str, not_done: &str) -> Value {
    let reason = match refusal {
        PlanRefusal::TypedContent { path: None } => {
            format!("the plan is not an object, so {not_done}")
        }
        PlanRefusal::TypedContent { path: Some(path) } => format!(
            "the plan breaks its grammar at {path}: a member unknown, missing, of another type or out of range, or one Baley writes at approval, so {not_done}"
        ),
        PlanRefusal::Lease { id, file, .. } => format!(
            "task {id} names {file}, which lies outside the plan's files and directories, so {not_done}"
        ),
        PlanRefusal::Questions(_) => format!(
            "the plan's questions break the question set's rules, as the code says, so {not_done}. The details name the question"
        ),
        PlanRefusal::CheckCommand { id, .. } => {
            format!("check {id} has a blank command, so {not_done}")
        }
        PlanRefusal::CheckExpected { id, .. } => {
            format!("check {id} has a blank expected value, so {not_done}")
        }
        PlanRefusal::CheckTestFile { id, .. } => {
            format!("check {id} has a blank test file, so {not_done}")
        }
        PlanRefusal::CheckBoundary { id, .. } => {
            format!("check {id} has a blank boundary, so {not_done}")
        }
        PlanRefusal::NoTruths => {
            format!("no story committed to the plan's phase holds an approved truth, so {not_done}")
        }
        PlanRefusal::StoryNotInPhase { story } => format!(
            "the plan serves story {story}, which is not committed to its phase, so {not_done}"
        ),
        PlanRefusal::EvidenceItemTruth { id, .. } => format!(
            "evidence item {id} names no current truth of a story the plan serves, so {not_done}"
        ),
        PlanRefusal::TruthVersionMismatch { id, current, .. } => format!(
            "evidence item {id} names its truth at a version other than its current version, {current}, so {not_done}"
        ),
        PlanRefusal::EvidenceItemConflict { id, plan, .. } => format!(
            "evidence item {id} differs from the item approved plan {plan} holds under that id, so {not_done}"
        ),
        PlanRefusal::LinkValueNotNamed { id, .. } => {
            format!("link {id} carries a value its truth does not name, so {not_done}")
        }
        PlanRefusal::UncoveredTruth { story, truth, .. } => format!(
            "truth {truth} of story {story} has no evidence item in the phase, so {not_done}"
        ),
        PlanRefusal::TruthWithoutCheck { story, truth, .. } => format!(
            "truth {truth} of story {story} has no check among the phase's evidence items, so {not_done}"
        ),
        PlanRefusal::TruthWithoutObservation { story, truth, .. } => format!(
            "truth {truth} of story {story} needs a person to prove it and has no observation among the phase's evidence items, so {not_done}"
        ),
        PlanRefusal::TruthCheckLimit { story, truth, .. } => format!(
            "truth {truth} of story {story} has more than one check across the phase, so {not_done}"
        ),
        PlanRefusal::OverCapacity { size, .. } => format!(
            "the plan would take its phase to {size} tasks, over the phase's capacity, so {not_done}. The owner decides which of the stories it serves waits for a later phase"
        ),
    };
    let answer = Refusal::new(refusal.code(), reason).slot(slot);
    match refusal.details() {
        Some(details) => answer.details(details),
        None => answer,
    }
    .value()
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005
    //! sections 5 and 6, D-02, D-03, D-05, D-13 and the plan's data store
    //! C, P0, P2, PL1, PL2 and PL3, never from running this code.

    use baley_core::policy::Host;
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::{O, fail_by_hand, prepared};
    use crate::planning::order::{issue, prepare};
    use crate::refine::order::tests::issue_by_hand;
    use crate::scope::fixtures::*;

    /// Q, phase 2's planning order.
    pub(crate) const Q: &str = "3b4c5d6e-7f80-4a91-8b2c-3d4e5f607182";
    /// Story 1's analyzer questions order, issued by hand.
    const A: &str = "4c5d6e7f-8091-4ab2-9c3d-4e5f60718293";
    /// An id no order has.
    const UNKNOWN: &str = "5d6e7f80-91a2-4bc3-8d4e-5f6071829304";

    /// Issues phase 1's planning order O for `[1]` under `request_id` at T4.
    pub(crate) fn order_o(store: &SqliteStore, request_id: &str) {
        let (command, o) = prepared(store, &[1], O, request_id, T4);
        assert_eq!(issue(store, &command, &o)["status"], "ok");
    }

    /// Issues phase 2's planning order Q for `[2]` under `request_id` at T4.
    pub(crate) fn order_q(store: &SqliteStore, request_id: &str) {
        let command = command("phase.plan", request_id, 7, T4);
        let q = prepare(
            store,
            &command,
            2,
            &[2],
            None,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            Q,
        )
        .expect("Q is prepared");
        assert_eq!(issue(store, &command, &q)["status"], "ok");
    }

    /// Makes phase 1 active: its plan 1, PL1's map, approved by hand.
    pub(crate) fn activate_phase_1(store: &SqliteStore, request_id: &str) {
        approve_by_hand(
            store,
            request_id,
            1,
            1,
            baley_store::Hash([0x7d; 32]),
            &[1],
            1,
            &[e1(), e2()],
            Vec::new(),
        );
    }

    /// `plan` as its draft body holds it: with `kind` `plan`.
    pub(crate) fn as_drafted(plan: Value) -> Value {
        let mut plan = plan;
        plan["kind"] = json!("plan");
        plan
    }

    /// Submits `plan` answering `order` under `capacity` through the check
    /// and the recording, under a command whose caller carries the order and
    /// its instructions, as `plan-submit` runs them below the session.
    pub(crate) fn plan_submitted(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        capacity: Option<u32>,
        request_id: &str,
        at: &str,
    ) -> Value {
        let capacity = capacity.map(|capacity| NonZeroU32::new(capacity).unwrap());
        let checked = match check(store, &project(), plan, order, capacity) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.submit", request_id, 8, at);
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(baley_store::Caller::Server(on));
        crate::scope::submit::answer(record(store, &command, Some(&checked)), store)
    }

    /// The digest the `draft` view's newest index names for phase 1's plan
    /// drafts.
    pub(crate) fn newest_plan_draft(store: &SqliteStore) -> Value {
        let newest = store
            .find(
                &project(),
                baley_core::scope::DRAFT_VIEW,
                &crate::scope::approve::newest_query("plan", "phase/1"),
            )
            .unwrap();
        newest.items[0].body["digest"].clone()
    }

    #[test]
    fn a_plan_draft_lost_on_reopen_sized_by_estimate_or_left_newest_once_replaced_is_caught() {
        let (dir, store) = store_c();
        order_o(&store, R1);
        let first = plan_submitted(&store, &pl2(), O, Some(2), R2, T5);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["kind"], "plan");
        assert_eq!(first["preview"], as_drafted(pl2()));
        assert_eq!(first["size"], 1);
        assert_eq!(first["phase_size"], 0);
        assert_eq!(first["reaching"], 1);
        assert_eq!(first["capacity"], 2);
        assert_eq!(first["now"]["round"], 1);
        assert_eq!(
            first["now"]["current"],
            json!([{"id": "q1", "question": "Which store holds the ledger?",
                "recommended": "SQLite"}])
        );
        let x2 = first["digest"].as_str().unwrap().to_owned();
        let s2 = first["set"].as_str().unwrap().to_owned();
        let order = crate::work_order::read::order(&store, &project(), O)
            .unwrap()
            .unwrap();
        assert_eq!(order["state"], "answered", "{order}");
        assert_eq!(order["drafts"], json!([x2]), "{order}");

        drop(store);
        let store = reopen(&dir);
        let digest = baley_store::Hash::from_hex(&x2).unwrap();
        let crate::scope::read::Draft::Held(held) =
            crate::scope::read::draft(&store, &project(), digest).unwrap()
        else {
            panic!("X2's body is held");
        };
        assert_eq!(held.kind, "plan");
        assert_eq!(held.submission, as_drafted(pl2()));

        approve_by_hand(
            &store,
            R3,
            1,
            1,
            baley_store::Hash([0x7d; 32]),
            &[1],
            2,
            &[e1(), e2()],
            Vec::new(),
        );
        let before = head(&store);
        let over = plan_submitted(&store, &pl3(), O, Some(2), R4, T6);
        assert_eq!(over["status"], "refused", "{over}");
        assert_eq!(over["code"], "over-capacity", "{over}");
        assert_eq!(over["details"], json!({"stories": [1], "size": 3}));
        assert_eq!(head(&store), before);

        let second = plan_submitted(&store, &pl3(), O, None, R5, T7);
        assert_eq!(second["status"], "ok", "{second}");
        let x3 = second["digest"].clone();
        let events = history(&store);
        let abandoned = of_type(&events, "questions.abandoned");
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].payload["set"], s2.as_str());
        assert_eq!(abandoned[0].payload["cause"], "draft-replaced");
        assert_eq!(abandoned[0].request_id.0, R5);
        assert_eq!(newest_plan_draft(&store), x3);
        assert_ne!(x3, json!(x2));
    }

    /// The bytes of the payload `event`'s `body` references.
    fn body_bytes(store: &SqliteStore, event: &baley_store::Event) -> Vec<u8> {
        use std::io::Read;
        let hash = baley_store::Hash::from_hex(event.payload["body"]["payload"].as_str().unwrap())
            .unwrap();
        let baley_store::PayloadBody::Present(mut reader) =
            baley_store::Payloads::open(store, &hash).unwrap()
        else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn a_plan_draft_recorded_off_its_order_or_without_record_retention_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let receipt = plan_submitted(&store, &pl1(), O, None, R2, T5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        let digest = receipt["digest"].as_str().unwrap();

        let events = history(&store);
        let drafts = of_type(&events, "scope.submitted");
        let drafts: Vec<_> = drafts
            .into_iter()
            .filter(|event| event.payload["kind"] == "plan")
            .collect();
        assert_eq!(drafts.len(), 1);
        let draft = drafts[0];
        assert_eq!(draft.stream, "project");
        assert_eq!(draft.payload["target"], "phase/1");
        assert_eq!(draft.payload["digest"], digest);
        assert_eq!(draft.payload["body"]["class"], "record");
        assert_eq!(
            body_bytes(&store, draft),
            baley_store::canonical_json(&as_drafted(pl1())).unwrap()
        );
        let Some(baley_store::Caller::Server(on)) = &draft.caller else {
            panic!("a server caller: {:?}", draft.caller);
        };
        assert_eq!(on.work_order(), Some(O));
        let identities: Vec<&str> = on.instructions().iter().map(|e| e.identity()).collect();
        assert_eq!(identities, ["bal-phase-planner"]);

        let opened = of_type(&events, "questions.opened");
        let opened: Vec<_> = opened
            .into_iter()
            .filter(|event| event.payload["subject"] == "plan")
            .collect();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].stream, "phase/1");
        assert_eq!(opened[0].payload["phase"], 1);
        assert_eq!(opened[0].payload["plan_digest"], digest);
        assert_eq!(opened[0].payload["questions"], json!([]));
        assert_eq!(receipt["now"]["state"], "closed", "{receipt}");
        assert_eq!(receipt["set"], opened[0].payload["set"]);
    }

    #[test]
    fn a_plan_set_opened_on_the_roadmap_or_two_drafts_left_open_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let first = plan_submitted(&store, &pl2(), O, None, R2, T5);
        let second = plan_submitted(&store, &pl2(), O, None, R3, T6);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(second["status"], "ok", "{second}");
        assert_eq!(second["digest"], first["digest"]);
        assert_ne!(second["set"], first["set"]);

        let events = history(&store);
        let abandoned = of_type(&events, "questions.abandoned");
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].payload["set"], first["set"]);
        assert_eq!(abandoned[0].payload["cause"], "draft-replaced");
        assert_eq!(abandoned[0].stream, "phase/1");
        assert_eq!(abandoned[0].request_id.0, R3);
        let opened: Vec<_> = of_type(&events, "questions.opened")
            .into_iter()
            .filter(|event| event.request_id.0 == R3)
            .collect();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].stream, "phase/1");
        assert_eq!(opened[0].payload["plan_digest"], first["digest"]);
        assert_eq!(opened[0].payload["set"], second["set"]);

        let phase_plan = store
            .get(
                &project(),
                baley_core::plan::PHASE_PLAN_VIEW,
                &phase_plan_key(1),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            phase_plan.body["draft"],
            json!({"digest": first["digest"], "set": second["set"]})
        );
    }

    #[test]
    fn a_submit_landing_beside_a_newly_active_phase_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        let checked = check(&store, &project(), &pl1(), O, None).expect("PL1 is accepted");
        activate_phase_2(&store, R3);
        let before = head(&store);
        let mut command = command("plan.submit", R4, 8, T5);
        command.caller = Some(baley_store::Caller::Server(
            caller().with_work_order(O).unwrap(),
        ));
        let answer = crate::scope::submit::answer(record(&store, &command, Some(&checked)), &store);
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }

    /// The check of `plan` under `order` with no capacity, which must refuse
    /// with `code` at `slot` and record nothing. Gives the refusal.
    fn refused_with(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        code: &str,
        slot: &str,
    ) -> Value {
        let before = head(store);
        let refusal = check(store, &project(), plan, order, None).expect_err("the plan is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], slot, "{refusal}");
        assert_eq!(head(store), before);
        refusal
    }

    #[test]
    fn a_plan_answering_another_phases_or_a_failed_order_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        issue_by_hand(&store, A, 1, R3);

        refused_with(&store, &pl1(), UNKNOWN, "no-such-work-order", "work_order");
        refused_with(&store, &pl1(), Q, "wrong-work-order", "work_order");
        refused_with(&store, &pl1(), A, "wrong-work-order", "work_order");
        fail_by_hand(&store, O, R4);
        refused_with(&store, &pl1(), O, "wrong-work-order", "work_order");
    }

    /// Submits `plan` answering `order` with no capacity through the check
    /// and the gated recording under `policy` for Claude Code with N0, its
    /// checker order, if any, as `id`, under a command whose caller carries
    /// the order and its instructions, under `request_id` at `at`.
    fn gated_submitted(
        store: &SqliteStore,
        plan: &Value,
        policy: &baley_core::policy::EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        let checked = match check(store, &project(), plan, O, None) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        record_gated(
            store,
            &submit_command(O, &checked, request_id, at),
            Some(&checked),
            policy,
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    /// The `plan.submit` command under `request_id` at `at`, its caller
    /// carrying `order` and `checked`'s instructions.
    fn submit_command(order: &str, checked: &Checked, request_id: &str, at: &str) -> Command {
        let mut command = command("plan.submit", request_id, 8, at);
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(baley_store::Caller::Server(on));
        command
    }

    #[test]
    fn a_gated_draft_previewed_or_given_a_set_before_its_check_is_caught() {
        use crate::planning::checker::tests::{C1, C9};
        use crate::refine::order::tests::issued;

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let first = gated_submitted(&store, &pl2(), &poff(), C9, R2, T5);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["preview"], as_drafted(pl2()));
        assert!(first["set"].is_string(), "{first}");
        assert_eq!(first["now"]["round"], 1, "{first}");
        assert!(first.get("check").is_none(), "{first}");
        assert!(
            issued(&store)
                .iter()
                .all(|e| e.payload["role"] != "checker")
        );
        let x2 = first["digest"].as_str().unwrap().to_owned();
        let s2 = first["set"].as_str().unwrap().to_owned();

        let second = gated_submitted(&store, &pl3(), &p0(), C1, R3, T6);
        assert_eq!(second["status"], "ok", "{second}");
        let mut members: Vec<&str> = second
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        let mut expected = vec![
            "status",
            "digest",
            "kind",
            "phase",
            "stories",
            "size",
            "phase_size",
            "reaching",
            "capacity",
            "recorded_at",
            "check",
        ];
        expected.sort_unstable();
        assert_eq!(members, expected, "{second}");
        assert_eq!(second["check"]["work_order"], C1);
        assert_eq!(
            second["check"]["prompt"],
            format!("Read Baley work order {C1} and follow it.")
        );
        let x3 = second["digest"].as_str().unwrap().to_owned();

        let events = history(&store);
        let mine: Vec<_> = events.iter().filter(|e| e.request_id.0 == R3).collect();
        let of = |name: &str| {
            mine.iter()
                .filter(|e| e.type_name == name)
                .collect::<Vec<_>>()
        };
        let drafts = of("scope.submitted");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].payload["kind"], "plan");
        assert_eq!(drafts[0].payload["digest"], x3.as_str());
        let abandoned = of("questions.abandoned");
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].payload["set"], s2.as_str());
        assert_eq!(abandoned[0].payload["cause"], "draft-replaced");
        let orders = of("work_order.issued");
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].payload["role"], "checker");
        assert_eq!(orders[0].payload["id"], C1);
        assert!(of("questions.opened").is_empty());

        let order = crate::work_order::read::order(&store, &project(), O)
            .unwrap()
            .unwrap();
        assert_eq!(order["state"], "answered", "{order}");
        assert_eq!(order["drafts"], json!([x2, x3]), "{order}");
        assert_eq!(newest_plan_draft(&store), json!(x3));
    }

    #[test]
    fn a_gated_submit_landing_beside_a_checker_order_issued_since_its_check_is_caught() {
        use crate::planning::checker::tests::{C1, C9, issued_alone};

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let checked = check(&store, &project(), &pl1(), O, None).expect("PL1 is accepted");
        let c9 = issued_alone(&store, C9, R2, T5);
        assert_eq!(c9["status"], "ok", "{c9}");
        let before = head(&store);
        let answer = record_gated(
            &store,
            &submit_command(O, &checked, R3, T6),
            Some(&checked),
            &p0(),
            Host::ClaudeCode,
            &n0(),
            C1,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_plan_submitted_beside_a_pending_checker_order_is_caught() {
        use crate::planning::checker::tests::{C1, drafted_with_check};

        let (_dir, store) = store_c();
        order_o(&store, R1);
        drafted_with_check(&store, &pl1(), C1, R2, T5);
        let refusal = refused_with(&store, &pl3(), O, "checker-order-pending", "operation");
        assert_eq!(refusal["details"], json!({"work_order": C1}));

        fail_by_hand(&store, C1, R3);
        let checked = check(&store, &project(), &pl3(), O, None);
        assert!(checked.is_ok(), "{checked:?}");
    }

    #[test]
    fn a_plan_refused_while_its_own_phase_is_active_or_let_through_beside_another_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        activate_phase_2(&store, R3);
        let refusal = refused_with(&store, &pl1(), O, "phase-not-active", "plan");
        assert_eq!(refusal["details"], json!({"phase": 2}));

        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        activate_phase_1(&store, R3);
        let checked = check(&store, &project(), &pl1(), O, None).expect("PL1 is accepted");
        assert_eq!(checked.plan.phase(), 1);
    }
}
