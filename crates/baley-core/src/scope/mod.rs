//! Scope records' pure half (design 0004 sections 5 and 6): the judgement
//! of a start submission and its digest, and the six scope events.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges, and stores any body before an event names it.

mod event;
mod submission;

pub use event::{
    PHASE_DECLARED, PHASE_DECLARED_VERSION, PHASE_REORDERED, PHASE_REORDERED_VERSION,
    PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION, PROJECT_STREAM, ROADMAP_STREAM, SCOPE_APPROVED,
    SCOPE_APPROVED_VERSION, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION, START_KIND, STORY_DECLARED,
    STORY_DECLARED_VERSION, approved_payload, described_payload, phase_declared_payload,
    phase_reordered_payload, register_scope_events, story_declared_payload, submitted_payload,
};
pub use submission::{
    DEPENDENCY_CYCLE, Description, MALFORMED_SUBMISSION, PROJECT_ALREADY_STARTED, Phase,
    STALE_DRAFT, STORY_DOUBLE_ASSIGNED, Story, StoryStatus, Submission, SubmissionRefusal,
    UNKNOWN_DRAFT, judge_submission,
};
