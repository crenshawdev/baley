//! `plan submit` (design 0005 section 5, PLN-R9, PLN-R11 to PLN-R13,
//! PLN-R15, PLN-R25, PLN-R27, D-02, D-03, D-05, D-13): a session submits a
//! planner's plan answering a planner order of its phase. The check runs
//! before admission and records nothing: the order, the phase's activity,
//! then the plan judged whole by `judge_plan` over the phase's facts read
//! from the views.

use std::num::NonZeroU32;

use baley_core::plan::{
    PhaseNotActive, Plan, PlanFacts, PlanRefusal, judge_plan, judge_submit_phase,
};
use baley_core::policy::Role;
use baley_core::work_order::{
    FAILED, NO_SUCH_WORK_ORDER, WORK_ORDER_VIEW, WRONG_WORK_ORDER, phase_subject, work_order_key,
};
use baley_store::{Absence, InstructionEvidence, ObservedDocument, ProjectId, StoreError, Views};
use serde_json::Value;

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::facts;
use crate::refine::order::order_evidence;
use crate::scope::submit::NOT_RECORDED_BUSY;
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
}

/// The check of the plan `submitted` answering work order `work_order`
/// under `capacity`, run before admission and recording nothing (D-13). In
/// order: the order, none `no-such-work-order` at slot `work_order`; a plan
/// whose `phase` cannot be read gets the plan judge's own `typed-content`
/// answer at slot `plan`, since the order is judged against that phase; an
/// order that is not a planner order of the plan's phase, or one that
/// failed, `wrong-work-order` at slot `work_order`, while a pending or
/// answered one passes, so a resubmission becomes the newest draft;
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
    let read = facts::read(store, project, phase, capacity)?;
    judge_submit_phase(phase, &read.active)
        .map_err(|refusal| phase_not_active(&refusal, "plan", NOT_RECORDED))?;
    let plan = judge_plan(submitted, &read.facts)
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
        size: read.facts.size,
        capacity,
        documents,
        absences: read.absences().to_vec(),
    })
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
