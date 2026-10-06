//! The guard's rules as pure decisions (design 0010). Nothing in this module
//! reads a file, the environment, git, the store or a clock: the hook
//! supplies every observation it judges.

mod answer;
mod event;
pub mod reason;
mod recording;
mod redelivery;
mod remembered;
mod scan;
mod settings;

#[cfg(test)]
mod tests;

pub use answer::{Answer, BranchObservation, commit_push_answer, powershell_answer};
pub use event::{
    AnsweredFacts, GUARD_ANSWERED, GUARD_ANSWERED_VERSION, GUARD_COMMAND, GUARD_POLICY_RECORDED,
    GUARD_POLICY_RECORDED_VERSION, GUARD_STREAM, SettingsFact, ToolInput, answered_payload,
    input_digest, policy_recorded_payload, register_guard_events,
};
pub use recording::{AuditPrecondition, record_answer};
pub use redelivery::{GUARD_VIEW, GuardProjector, Redelivery, guard_key, guard_spec, redelivery};
pub use remembered::DenialParts;
pub use scan::{GitVerb, git_verb};
pub use settings::{GuardSettings, SettingsInput};
