//! Whether `purge` runs the policy step (design 0003, CFG-R8, CFG-R9).

/// What `purge` does about the recorded policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgePolicy {
    /// Run the policy step and record the version in force it returns.
    RunStep,
    /// Run no step and record policy version 0.
    VersionZero,
}

/// Purge's choice from the project id discovered for the working
/// directory's checkout, if any, and the project `purge` names.
///
/// Purge is the one chain-writing command that may run outside a checkout.
/// Only a checkout of the named project has a policy for it, so any other
/// directory reads no settings file and records version 0, and a policy
/// purge cannot read never blocks a purge run elsewhere.
pub fn purge_policy(discovered: Option<&str>, named: &str) -> PurgePolicy {
    if discovered == Some(named) {
        PurgePolicy::RunStep
    } else {
        PurgePolicy::VersionZero
    }
}
