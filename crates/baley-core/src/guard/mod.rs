//! The guard's rules as pure decisions (design 0010). Nothing in this module
//! reads a file, the environment, git, the store or a clock: the hook
//! supplies every observation it judges.

mod answer;
pub mod reason;
mod scan;
mod settings;

#[cfg(test)]
mod tests;

pub use answer::{Answer, BranchObservation, commit_push_answer, powershell_answer};
pub use scan::{GitVerb, git_verb};
pub use settings::{GuardSettings, SettingsInput};
