//! Checkout admission and the facts it judges (design 0001 Project identity
//! and policy, EVD-R17, ADR 0004). The module gathers a checkout's root
//! commit and remote URL through git and runs checkout admission against the
//! project's `checkout` view.

mod admit;
mod gather;
mod strip;

use std::fmt;
use std::path::Path;

use baley_core::checkout::Checkout;
use baley_core::policy::EffectivePolicy;
use baley_store::{Caller, Ledger, ProjectId, RequestId, Views};

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

/// The checkout to gather and admit: where it is, the policy whose
/// `git.remote` picks its remote, and its path as the policy step records it.
pub(crate) struct Site<'a> {
    /// The repository root, where git runs.
    pub root: &'a Path,
    /// The built policy, which names the remote.
    pub policy: &'a EffectivePolicy,
    /// The checkout's path text, `RecordedPolicy::checkout`.
    pub path: &'a str,
}

/// Gathers the facts of `site` and admits it under `project`, for the callers
/// that do both at once: the ledger commands, `purge` and `config set`. It
/// prints nothing, and it runs before the policy step.
///
/// `caller` goes to checkout admission unchanged.
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
