//! The guard's answer to a git commit or push (design 0010, GRD-R2 to GRD-R7).

use super::reason;
use super::scan::GitVerb;
use super::settings::{GuardSettings, SettingsInput};
use crate::policy::OnProtected;

/// What the guard says about one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Nothing to say. The host's own permission rules decide.
    Pass,
    /// Ask the owner, with the reason.
    Ask(String),
    /// Refuse the call, with the reason.
    Deny(String),
    /// Let the call through because an input could not be read. It is loud: the
    /// reason says the call went unchecked and that this is not approval.
    PassOnFailure(String),
}

/// What the hook found out about the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchObservation {
    /// Git read the branch.
    Read(String),
    /// Git failed, and the bounded `.git/HEAD` read named a branch.
    Fallback {
        /// The branch `.git/HEAD` names.
        name: String,
        /// How git failed, in words.
        failure: String,
    },
    /// No branch was read: git failed and `.git/HEAD` named none, or HEAD is
    /// detached.
    Unreadable {
        /// Why, in words.
        description: String,
    },
}
impl BranchObservation {
    /// The branch, from either source, for a reason that names it.
    fn name(&self) -> Option<&str> {
        match self {
            BranchObservation::Read(name) | BranchObservation::Fallback { name, .. } => Some(name),
            BranchObservation::Unreadable { .. } => None,
        }
    }
}

/// The words for a failed branch read, shared by the hard-fail deny and the
/// pass on failure.
fn unread(failure: &str) -> String {
    format!("git could not read the branch ({failure})")
}

/// Judges a `git commit` or `git push` from supplied observations.
///
/// `remembered` is the last complete policy's settings, if the hook has one.
/// It is used only under torn settings and only to deny.
pub fn commit_push_answer(
    verb: GitVerb,
    project_bound: bool,
    settings: &SettingsInput,
    remembered: Option<&GuardSettings>,
    branch: &BranchObservation,
) -> Answer {
    if !project_bound {
        return Answer::Pass;
    }
    if verb == GitVerb::Push {
        return Answer::Ask(reason::push_ask());
    }
    match settings {
        SettingsInput::Complete(current) => complete_commit(current, branch),
        SettingsInput::Torn(torn) => torn_commit(&torn.to_string(), remembered, branch),
    }
}

/// A commit under complete settings. `refuse` and `ask` act only on a branch
/// git read, and a name from `.git/HEAD` feeds only hard fail.
fn complete_commit(settings: &GuardSettings, branch: &BranchObservation) -> Answer {
    match branch {
        BranchObservation::Read(name) if settings.protects(name) => match settings.on_protected {
            OnProtected::Refuse => Answer::Deny(reason::refuse_deny(name)),
            OnProtected::Ask => Answer::Ask(reason::protected_ask(name)),
            OnProtected::Allow => Answer::Pass,
        },
        BranchObservation::Read(_) => Answer::Pass,
        BranchObservation::Fallback { name, failure }
            if settings.hard_fail && settings.protects(name) =>
        {
            Answer::Deny(reason::hard_fail_deny(name, &unread(failure)))
        }
        BranchObservation::Fallback { failure, .. } => {
            Answer::PassOnFailure(reason::failure_pass(&unread(failure)))
        }
        BranchObservation::Unreadable { description } => {
            Answer::PassOnFailure(reason::failure_pass(&unread(description)))
        }
    }
}

/// A commit when the settings are torn: a remembered denial still denies,
/// and anything else asks. A remembered `allow` or `ask` never relaxes it.
fn torn_commit(
    torn: &str,
    remembered: Option<&GuardSettings>,
    branch: &BranchObservation,
) -> Answer {
    if let Some(last) = remembered {
        match branch {
            BranchObservation::Read(name)
                if last.on_protected == OnProtected::Refuse && last.protects(name) =>
            {
                return Answer::Deny(reason::remembered_refuse_deny(torn, name));
            }
            BranchObservation::Fallback { name, failure }
                if last.hard_fail && last.protects(name) =>
            {
                return Answer::Deny(reason::remembered_hard_fail_deny(
                    torn,
                    &unread(failure),
                    name,
                ));
            }
            _ => {}
        }
    }
    Answer::Ask(reason::torn_ask(torn, branch.name()))
}

/// Judges a PowerShell call. Baley reads POSIX shell grammar only, so in a
/// project every PowerShell call asks, whether or not it mentions git, and
/// outside a project it passes (design 0010, GRD-R3).
///
/// It takes no command text, so nothing in the command can change the answer.
pub fn powershell_answer(project_bound: bool) -> Answer {
    if project_bound {
        Answer::Ask(reason::powershell_ask())
    } else {
        Answer::Pass
    }
}
