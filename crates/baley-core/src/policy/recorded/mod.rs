//! The recorded policy's pure half (design 0003 section 6, CFG-R8, CFG-R9;
//! build-2-plan decisions 9 and 18): the `policy.effective` event and its
//! payload.
//!
//! Nothing here reads a file, the store or a clock: the binary supplies
//! every policy, path, version and document it judges.

mod event;

#[cfg(test)]
mod tests;

pub use event::{
    POLICY_EFFECTIVE, POLICY_EFFECTIVE_VERSION, PathNotUtf8, RecordedPolicy, effective_payload,
    recorded_policy, register_policy_events,
};
