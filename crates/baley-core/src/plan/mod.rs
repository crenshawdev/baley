//! Plans' pure half (design 0005 sections 3 and 6, PLN-R5, PLN-R9's judge,
//! PLN-R11 to PLN-R13): the judgement of one submitted plan whole against
//! the phase's facts, the refusal codes it answers with, and the accepted
//! plan only that judgement builds; a plan draft's body, digest and target,
//! built from an accepted plan alone; and the four plan events on a
//! phase's stream, `plan.approved`, `plan.replaced`, `plan.continuation`
//! and `plan.continuation_followed`, with their payloads.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod binding;
mod capacity;
mod content;
mod coverage;
mod event;
mod evidence;
#[cfg(test)]
mod fixtures;
mod judge;
mod lease;
mod link;
mod stories;
mod submission;
mod task;

pub use event::{
    PLAN_APPROVED, PLAN_APPROVED_VERSION, PLAN_CONTINUATION, PLAN_CONTINUATION_FOLLOWED,
    PLAN_CONTINUATION_FOLLOWED_VERSION, PLAN_CONTINUATION_VERSION, PLAN_REPLACED,
    PLAN_REPLACED_VERSION, PlanApproval, continuation_followed_payload, continuation_payload,
    phase_stream, register_plan_events, replaced_payload,
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
pub use submission::{PLAN_KIND, PlanDraft, RecordedPlan, Replaces, read_plan_draft};
pub use task::Task;
