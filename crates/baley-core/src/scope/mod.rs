//! Scope records' pure half (design 0004 sections 5 and 6): the judgement
//! of a start submission and its digest.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges, and stores any body before an event names it.

mod submission;

pub use submission::{
    DEPENDENCY_CYCLE, Description, MALFORMED_SUBMISSION, PROJECT_ALREADY_STARTED, Phase,
    STALE_DRAFT, STORY_DOUBLE_ASSIGNED, Story, StoryStatus, Submission, SubmissionRefusal,
    UNKNOWN_DRAFT, judge_submission,
};
