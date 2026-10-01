//! Which project a ledger command acts on, and which remote each project is
//! checked against, from the discovered project and its `git.remote`. Pure:
//! the callers gather the facts.
use baley_core::policy::{EffectivePolicy, Value};
use baley_store::ProjectId;

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

/// Why `doctor` checks a project without a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalReason {
    /// The project is not the checkout's, and only a checkout names a remote.
    NotDiscovered,
    /// The checkout's project sets no `git.remote`.
    NoForgeRemote,
    /// A settings file could not be read, so `git.remote` may be set in it.
    SettingsUnreadable,
}

/// What `doctor` knows of the discovered project's `git.remote`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RemoteState {
    /// The setting names this remote.
    Name(String),
    /// The setting is absent.
    NotSet,
    /// A settings file could not be read, so the setting is unknown.
    Unknown,
}

/// What `doctor` checks one project against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CheckAgainst {
    /// The remote the checkout's `git.remote` names.
    Remote(String),
    /// Nothing outside the ledger.
    Local(LocalReason),
}

/// The checks `doctor` runs, and the remote to validate before any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DoctorChecks {
    /// One check per ledger project, in the ledger's order.
    pub(super) checks: Vec<(ProjectId, CheckAgainst)>,
    /// The discovered project's `git.remote`. It is validated even when that
    /// project is not in the ledger yet, since the setting is committed and
    /// a wrong name should not wait for the first anchor to be found.
    pub(super) validate: Option<String>,
}

/// Maps every ledger project to its check. The discovered project gets its
/// `git.remote`, or a local check when it sets none. Every other project,
/// `user` included, is checked locally. A discovered project absent from the
/// ledger gets no check, since the store refuses one for an unknown project.
pub(super) fn doctor_checks(
    discovered: Option<&str>,
    remote: &RemoteState,
    projects: &[(ProjectId, String)],
) -> DoctorChecks {
    let checks = projects
        .iter()
        .map(|(project, _)| {
            let against = match (discovered == Some(project.0.as_str()), remote) {
                (true, RemoteState::Name(remote)) => CheckAgainst::Remote(remote.clone()),
                (true, RemoteState::NotSet) => CheckAgainst::Local(LocalReason::NoForgeRemote),
                (true, RemoteState::Unknown) => {
                    CheckAgainst::Local(LocalReason::SettingsUnreadable)
                }
                (false, _) => CheckAgainst::Local(LocalReason::NotDiscovered),
            };
            (project.clone(), against)
        })
        .collect();
    let validate = match (discovered, remote) {
        (Some(_), RemoteState::Name(remote)) => Some(remote.clone()),
        _ => None,
    };
    DoctorChecks { checks, validate }
}

/// Matches a configured remote as one complete line.
pub(super) fn configured(stdout: &str, name: &str) -> bool {
    stdout.lines().any(|line| line == name)
}
