//! Planned file writes through digest-checked replacement, then the
//! registration through Claude Code's own command. The plan owns the
//! permission to act and interprets the returned observations.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::host_artifacts::executable::Executable;
use crate::host_doctor::placed;
use crate::process::{Process, System};
use crate::replace::{self, Failure};

use super::plan::Plan;
use super::registration::{self, Attempt, Next, Registered, Step};

/// What applying a plan did: each file write, in plan order, and the
/// registration step.
pub(crate) struct Applying {
    /// One result per write reached, in plan order.
    pub writes: Vec<Result<(), Failure>>,
    /// What became of the registration.
    pub attempt: Attempt,
}

/// Applies the file writes in order with default folder modes, stopping at the
/// first failure. A failed sync is returned too, since the rename already
/// happened. The registration launches run after every file is written, and
/// only when every write was reached.
pub(crate) fn run(
    plan: &Plan,
    latest: Option<&Value>,
    registration_file: &Path,
    executable: &Executable,
) -> Applying {
    let writes = write_files(plan);
    let reached = writes.len() == plan.writes.len()
        && writes
            .iter()
            .all(|result| matches!(result, Ok(()) | Err(Failure::Unsynced { .. })));
    let attempt = register(plan, reached, latest, registration_file, executable);
    Applying { writes, attempt }
}

fn write_files(plan: &Plan) -> Vec<Result<(), Failure>> {
    let mut results = Vec::new();
    for write in &plan.writes {
        let result = fs::create_dir_all(
            write
                .path
                .parent()
                .expect("an absolute stub path has a parent"),
        )
        .map_err(|cause| Failure::Unchanged {
            path: write.path.clone(),
            cause,
        })
        .and_then(|()| replace::replace(&write.path, &write.bytes, write.read_digest.as_deref()));
        let stopped = result.is_err();
        results.push(result);
        if stopped {
            break;
        }
    }
    results
}

fn register(
    plan: &Plan,
    reached: bool,
    latest: Option<&Value>,
    file: &Path,
    executable: &Executable,
) -> Attempt {
    let mut attempt = Attempt {
        seen: plan.registration_seen.clone(),
        removed: false,
        result: Registered::Unchanged,
    };
    if let Some(wiring) = &plan.wiring {
        attempt.result = Registered::Withheld(wiring.cause.clone());
        return attempt;
    }
    let Step::Run(launches) = &plan.registration else {
        return attempt;
    };
    if !reached {
        attempt.result = Registered::NotReached("an earlier write failed".into());
        return attempt;
    }
    let mut launches = launches.clone();
    if launches.len() == 2 {
        // A remove deletes an entry, so it never runs on evidence read before
        // the file writes: the file is read again and judged again first.
        let again = registration::recheck(file, &placed::read(file), latest, executable);
        attempt.seen = again.seen;
        match again.next {
            Next::Run(next) => launches = next,
            Next::Unchanged => return attempt,
            Next::Refused(cause) => {
                attempt.result = Registered::Refused(cause);
                return attempt;
            }
        }
    }
    let replacing = launches.len() == 2;
    for (index, launch) in launches.iter().enumerate() {
        match registration::interpret(file, System.run(launch)) {
            Ok(()) => attempt.removed |= replacing && index == 0,
            Err(cause) => {
                attempt.result = Registered::Failed(cause);
                return attempt;
            }
        }
    }
    attempt.result = if replacing {
        Registered::Replaced
    } else {
        Registered::Registered
    };
    attempt
}
