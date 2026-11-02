//! Plans' pure half (design 0005 sections 3, 5 and 6, PLN-R5, PLN-R9's
//! judge, PLN-R11 to PLN-R15, PLN-R26 and PLN-R27): the judgement of one
//! submitted plan whole against the phase's facts, the refusal codes it
//! answers with, and the accepted plan only that judgement builds; a plan
//! draft's body, digest and target, built from an accepted plan alone; the
//! judgement of a plan approval and of a replacement, with the phase facts a
//! replacement is judged under; and the six plan events on a phase's
//! stream, `plan.approved`, `plan.replaced`, `plan.continuation`,
//! `plan.continuation_followed`, `plan.check_issued` and `plan.checked`,
//! with their payloads; and the `phase_plan` view, a phase's planning
//! state, the `plan` view, each approved plan's record, and the
//! `evidence_map` view, each approved plan's map with its reading into the
//! items the judge takes, all projected from those events alone; and the
//! commitment judge, what a `phase-plan` call does with a
//! selection and whether a plan may be submitted for its phase; and the
//! plan checker's judgements, the findings a checker returns, a submit's
//! round in its check lineage and a re-check's truths, and the delta of a
//! draft and its revision that the `diff` re-check reads (PLN-R16).
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod approval;
mod binding;
mod capacity;
mod check;
mod commitment;
mod content;
mod coverage;
mod delta;
mod event;
mod evidence;
#[cfg(test)]
mod fixtures;
mod judge;
mod lease;
mod link;
mod map_view;
mod phase_plan;
mod plan_view;
mod replace;
mod stories;
mod submission;
mod task;

pub use approval::{
    ANSWER_NOT_APPLIED, DraftRead, PlanApprovalFacts, PlanApprovalRefusal, judge_plan_approval,
};
pub use check::{
    CHECKER_ORDER_PENDING, CheckRefusal, CheckRound, FINDING_SHAPE, Findings, Lineage,
    RECHECK_TRUTHS_CHANGED, THIRD_ROUND, TruthChange, judge_check_round, judge_findings,
    judge_recheck_truths,
};
pub use commitment::{
    CommitmentFacts, CommitmentRefusal, Continuation, PHASE_ACTIVE, PHASE_NOT_ACTIVE,
    PLANNER_ORDER_PENDING, PhaseNotActive, PhasePlanPath, Readiness, STORY_IN_PHASE,
    STORY_UNREFINED, SelectedStory, judge_commitment, judge_readiness, judge_submit_phase,
};
pub use delta::plan_delta;
pub use event::{
    CheckIssue, CheckResult, PLAN_APPROVED, PLAN_APPROVED_VERSION, PLAN_CHECK_ISSUED,
    PLAN_CHECK_ISSUED_VERSION, PLAN_CHECKED, PLAN_CHECKED_VERSION, PLAN_CONTINUATION,
    PLAN_CONTINUATION_FOLLOWED, PLAN_CONTINUATION_FOLLOWED_VERSION, PLAN_CONTINUATION_VERSION,
    PLAN_REPLACED, PLAN_REPLACED_VERSION, PlanApproval, PreviousCheck, TruthVersion,
    continuation_followed_payload, continuation_payload, phase_stream, register_plan_events,
    replaced_payload,
};
pub use evidence::{
    Artifact, Check, CheckTest, EvidenceItem, Expected, Level, Link, Observation, Spec,
};
pub use judge::{
    ApprovedItem, CHECK_BOUNDARY, CHECK_COMMAND, CHECK_EXPECTED, CHECK_TEST_FILE, CommittedStory,
    CurrentTruth, EVIDENCE_ITEM_CONFLICT, EVIDENCE_ITEM_TRUTH, EvidenceKind, LEASE,
    LINK_VALUE_NOT_NAMED, NO_TRUTHS, OVER_CAPACITY, Plan, PlanFacts, PlanRefusal,
    STORY_NOT_IN_PHASE, TRUTH_CHECK_LIMIT, TRUTH_VERSION_MISMATCH, TRUTH_WITHOUT_CHECK,
    TRUTH_WITHOUT_OBSERVATION, TYPED_CONTENT, UNCOVERED_TRUTH, judge_plan,
};
pub use lease::{inside_lease, plain_relative_path};
pub use map_view::{
    EVIDENCE_MAP_VIEW, EvidenceMapProjector, approved_items, evidence_map_key, evidence_map_spec,
};
pub use phase_plan::{
    ACTIVE, IN_FORCE, INACTIVE, PHASE_PLAN_STATUS_INDEX, PHASE_PLAN_VIEW, PhasePlanProjector,
    REPLACED, phase_plan_key, phase_plan_spec,
};
pub use plan_view::{APPROVED, PLAN_VIEW, PlanProjector, plan_key, plan_spec};
pub use replace::{
    ADMITTED_PLAN, NO_SUCH_PLAN, PlanInForce, ReplacementFacts, ReplacementRefusal,
    judge_replacement,
};
pub use submission::{PLAN_KIND, PlanDraft, RecordedPlan, Replaces, read_plan_draft};
pub use task::Task;
