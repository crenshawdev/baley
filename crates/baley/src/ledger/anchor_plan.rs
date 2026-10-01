//! Which project a ledger command acts on, and which remote each project is
//! checked against, from the discovered project and its `git.remote`. Pure:
//! the callers gather the facts.
use baley_core::policy::{EffectivePolicy, Value};

/// The setting that names the checkout's anchor remote.
const GIT_REMOTE: &str = "git.remote";

/// The remote a built policy names, `None` when `git.remote` is absent.
pub(super) fn remote_of(policy: &EffectivePolicy) -> Option<String> {
    match policy.settings.get(GIT_REMOTE)?.value.as_ref()? {
        Value::RemoteName(name) => Some(name.clone()),
        _ => None,
    }
}

/// Why a named project cannot be acted on from this directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TargetRefusal {
    /// The checkout belongs to another project.
    Differs {
        /// The project the owner named.
        named: String,
        /// The project of the checkout.
        discovered: String,
    },
    /// No project was found, so there is no checkout to hold the name.
    NoneDiscovered {
        /// The project the owner named.
        named: String,
    },
}

/// The project a single-project command acts on. The discovered project is
/// the only one a checkout can name a remote for, so a name that differs
/// from it, or is given with none discovered, is refused. With no name and
/// nothing discovered there is no project, which each command words itself.
pub(super) fn judge_target(
    discovered: Option<&str>,
    named: Option<&str>,
) -> Result<Option<String>, TargetRefusal> {
    match (discovered, named) {
        (Some(found), Some(named)) if found != named => Err(TargetRefusal::Differs {
            named: named.into(),
            discovered: found.into(),
        }),
        (None, Some(named)) => Err(TargetRefusal::NoneDiscovered {
            named: named.into(),
        }),
        (found, _) => Ok(found.map(Into::into)),
    }
}
