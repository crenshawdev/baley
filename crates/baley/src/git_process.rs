//! Registered git deadlines and interpretation of process observations.

use crate::guard_budget::GitGrant;
use crate::process::{Launch, Output, Process};
use std::{fmt, io, time::Duration};

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

/// A registered launch for `caller` at its exact deadline. A guard caller's
/// carries no timeout, so the validator refuses it: its time comes only from
/// a budget grant through [`guard_launch`], so its launches together never
/// outrun git's one allowance.
pub fn launch(caller: Caller) -> Launch {
    let launch = Launch::registered_git(Registration(caller)).own_group();
    match deadline(caller) {
        Deadline::Exact(timeout) => launch.timeout(timeout),
        Deadline::Guard(_) => launch,
    }
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
        // To the millisecond: a guard launch runs on whatever its budget had
        // left, which is rarely whole seconds.
        let millis = self.bound.as_millis();
        let seconds = format!("{}.{:03}", millis / 1000, millis % 1000);
        let seconds = seconds.trim_end_matches('0').trim_end_matches('.');
        write!(
            f,
            "{} exceeded git deadline of {seconds} seconds",
            self.command
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

/// Interpret one completion. Only an explicit timeout observation of a launch
/// that had a timeout is a limit, stated at the timeout that launch ran under.
pub fn finish(launch: &Launch, answer: io::Result<Output>) -> Result<Output, Error> {
    answer.map_err(|error| match launch.timeout {
        Some(bound) if error.kind() == io::ErrorKind::TimedOut => Error::Limit(Limit {
            command: format!("git {}", launch.argument_text()),
            bound,
        }),
        _ => Error::Io(error),
    })
}

pub fn run(launch: &Launch, process: &mut dyn Process) -> Result<Output, Error> {
    if launch.git_caller().is_none() {
        return Err(Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "git launch requires a registered caller",
        )));
    }
    finish(launch, process.run(launch))
}

#[cfg(test)]
mod tests;
