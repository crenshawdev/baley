//! Registered git deadlines and interpretation of process observations.

use crate::guard_budget::GitGrant;
use crate::process::{Launch, Output, Process};
use std::{ffi::OsString, fmt, io, time::Duration};

/// The deadline of every caller that is not the guard's.
pub const OTHER_GIT_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    ExecutionOutput,
    ExecutionStatus,
    PauseRead,
    PauseIndex,
    WhyRead,
    WhyInput,
    ExecutionRunner,
    PauseMergeBase,
    RailRead,
    /// The current branch, read by the guard on its budget.
    GuardBranch,
    /// HEAD's copy of the project file, read only by the guard on its budget.
    /// The guard never reads through `ProjectHead`, so that caller keeps its
    /// exact deadline.
    GuardProjectHead,
    RailCommitInput,
    RailConfig,
    RecallHistory,
    ReadDocumentHead,
    LandingGit,
    /// Anchor tag transport.
    AnchorForge,
    /// HEAD's copy of the project file.
    ProjectHead,
    /// A checkout's root commit and remote URL, read for checkout admission.
    CheckoutFacts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Registration(Caller);

impl Registration {
    pub(crate) fn caller(&self) -> Caller {
        self.0
    }
}

/// How long one registered caller's git may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deadline {
    /// Exactly this long, every launch.
    Exact(Duration),
    /// A guard caller: above zero and at most this cap. The guard's budget
    /// picks each launch's time from what is left, so only the cap is fixed.
    Guard(Duration),
}

pub fn deadline(caller: Caller) -> Deadline {
    match caller {
        Caller::GuardBranch | Caller::GuardProjectHead => Deadline::Guard(crate::guard_budget::GIT),
        Caller::ExecutionOutput
        | Caller::ExecutionStatus
        | Caller::PauseRead
        | Caller::PauseIndex
        | Caller::WhyRead
        | Caller::WhyInput
        | Caller::ExecutionRunner
        | Caller::PauseMergeBase
        | Caller::RailRead
        | Caller::RailCommitInput
        | Caller::RailConfig
        | Caller::RecallHistory
        | Caller::ReadDocumentHead
        | Caller::LandingGit
        | Caller::AnchorForge
        | Caller::ProjectHead
        | Caller::CheckoutFacts => Deadline::Exact(OTHER_GIT_DEADLINE),
    }
}

/// A registered launch for `caller` at its deadline. A guard caller's runs to
/// its cap.
pub fn launch(caller: Caller) -> Launch {
    let (Deadline::Exact(timeout) | Deadline::Guard(timeout)) = deadline(caller);
    Launch::registered_git(Registration(caller))
        .timeout(timeout)
        .own_group()
}

/// A guard caller's launch on the time `grant` gives it.
pub fn guard_launch(caller: Caller, grant: &GitGrant) -> Launch {
    Launch::registered_git(Registration(caller))
        .timeout(grant.timeout())
        .own_group()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limit {
    pub command: String,
    pub bound: Duration,
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} exceeded git deadline of {} seconds",
            self.command,
            self.bound.as_secs()
        )
    }
}

#[derive(Debug)]
pub enum Error {
    Limit(Limit),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(limit) => limit.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// Interpret one completion; only an explicit timeout observation is a limit.
pub fn finish(
    caller: Caller,
    args: &[OsString],
    answer: io::Result<Output>,
) -> Result<Output, Error> {
    let (Deadline::Exact(bound) | Deadline::Guard(bound)) = deadline(caller);
    answer.map_err(|error| {
        if error.kind() == io::ErrorKind::TimedOut {
            Error::Limit(Limit {
                command: format!(
                    "git {}",
                    args.iter()
                        .map(|arg| arg.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                bound,
            })
        } else {
            Error::Io(error)
        }
    })
}

pub fn run(launch: &Launch, process: &mut dyn Process) -> Result<Output, Error> {
    let caller = launch.git_caller().ok_or_else(|| {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "git launch requires a registered caller",
        ))
    })?;
    finish(caller, &launch.args, process.run(launch))
}

#[cfg(test)]
mod tests;
