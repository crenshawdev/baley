//! The recorded policy's pure half (design 0003 section 6, CFG-R8, CFG-R9;
//! build-2-plan decisions 9 and 18): the `policy.effective` event and its
//! payload, the `policy` view that keeps the latest one per checkout and
//! host, and the judge of whether a payload needs recording.
//!
//! Nothing here reads a file, the store or a clock: the binary supplies
//! every policy, path, version and document it judges.

mod event;
mod judge;
mod view;

#[cfg(test)]
mod tests;

pub use event::{
    POLICY_EFFECTIVE, POLICY_EFFECTIVE_VERSION, PathNotUtf8, RecordedPolicy, effective_payload,
    recorded_policy, register_policy_events,
};
pub use judge::{PolicyJudgement, judge_policy};
pub use view::{POLICY_VIEW, PolicyProjector, policy_key, policy_spec};
