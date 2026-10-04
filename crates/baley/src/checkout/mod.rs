//! Checkout admission and the facts it judges (design 0001 Project identity
//! and policy, EVD-R17, ADR 0004). The module gathers a checkout's root
//! commit and remote URL through git and runs checkout admission against the
//! project's `checkout` view.

mod admit;
mod gather;
mod strip;

use std::fmt;
use std::path::Path;

use baley_core::checkout::{Checkout, ProjectIdConflict};
use baley_core::policy::EffectivePolicy;
use baley_store::{Caller, Ledger, ProjectId, RequestId, StoreError, Views};

pub use admit::{ADMIT_COMMAND, AdmitError, admit};
pub use gather::{Facts, gather, root_commit};
pub use strip::strip_user_information;

use crate::process::Process;

/// Why gathering and admitting a checkout stopped.
#[derive(Debug)]
pub enum EntryError {
    /// Git could not give the checkout's facts. The text is the refusal.
    Gather(String),
    /// Checkout admission refused the checkout as a fork, or the store failed.
    Admit(AdmitError),
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gather(text) => f.write_str(text),
            Self::Admit(error) => error.fmt(f),
        }
    }
}

/// Which refusal an [`EntryError`] is, with its parts. Every reader of
/// checkout admission's refusals, the command line and the server alike,
/// goes through [`EntryError::refusal`] so none re-matches on [`AdmitError`].
#[derive(Debug, PartialEq)]
pub enum EntryRefusal<'a> {
    /// Git could not give the checkout's facts: its refusal text.
    Git(&'a str),
    /// The checkout is a fork of another checkout of the project.
    Fork(&'a ProjectIdConflict),
    /// The store failed while checkout admission ran.
    Store(&'a StoreError),
}

impl EntryError {
    /// Reads which refusal this is, carrying its text, conflict or store error.
    pub fn refusal(&self) -> EntryRefusal<'_> {
        match self {
            Self::Gather(text) => EntryRefusal::Git(text),
            Self::Admit(AdmitError::Fork(conflict)) => EntryRefusal::Fork(conflict),
            Self::Admit(AdmitError::Store(error)) => EntryRefusal::Store(error),
        }
    }
}

/// The checkout to gather and admit: where it is, the policy whose
/// `git.remote` picks its remote, and its path as the policy step records it.
pub(crate) struct Site<'a> {
    /// The repository root, where git runs.
    pub root: &'a Path,
    /// The built policy, which names the remote. The server passes the policy
    /// that names no host, as the command line does.
    pub policy: &'a EffectivePolicy,
    /// The checkout's path text, `RecordedPolicy::checkout`.
    pub path: &'a str,
}

/// Gathers the facts of `site` and admits it under `project`, for the callers
/// that do both at once: the ledger commands, `purge`, `config set` and the
/// session server's write preparation. It prints nothing, and it runs before
/// the policy step.
///
/// `caller` goes to checkout admission unchanged. For the server, `site.policy`
/// is the policy that names no host, so the remote is the command line's, and
/// the stored version checkout admission carries is the hostless one.
pub(crate) fn gather_and_admit(
    store: &(impl Ledger + Views),
    project: &ProjectId,
    site: &Site<'_>,
    process: &mut dyn Process,
    request_id: RequestId,
    at: &str,
    caller: Option<Caller>,
) -> Result<(), EntryError> {
    let facts = gather(site.root, site.policy, process).map_err(EntryError::Gather)?;
    let checkout = Checkout {
        path: site.path.into(),
        root_commit: facts.root_commit,
        remote_url: facts.remote_url,
    };
    admit(store, project, &checkout, request_id, at, caller).map_err(EntryError::Admit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict() -> ProjectIdConflict {
        ProjectIdConflict {
            path: "/a".into(),
            remote_url: "https://example.test/a".into(),
            other_path: "/b".into(),
            other_remote_url: "https://example.test/b".into(),
        }
    }

    #[test]
    fn a_git_refusal_is_not_read_as_a_fork_or_a_store_error() {
        let error = EntryError::Gather("git said no".into());
        assert_eq!(error.refusal(), EntryRefusal::Git("git said no"));
    }

    #[test]
    fn a_fork_is_not_read_as_a_git_refusal_or_a_store_error() {
        let error = EntryError::Admit(AdmitError::Fork(conflict()));
        assert_eq!(error.refusal(), EntryRefusal::Fork(&conflict()));
    }

    #[test]
    fn a_store_error_is_not_read_as_a_git_refusal_or_a_fork() {
        let error = EntryError::Admit(AdmitError::Store(StoreError::Busy));
        assert_eq!(error.refusal(), EntryRefusal::Store(&StoreError::Busy));
    }
}
