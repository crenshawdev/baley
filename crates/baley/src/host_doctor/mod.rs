//! The runtime doctor's host checks: what Claude Code's artifacts look like
//! on this machine, gathered as observations, judged as plain values and
//! reported as owner lines with an exit code.
//!
//! The work is three steps, each public and tested apart:
//! - an [`Observation`], what was gathered;
//! - [`judge`], which turns an observation into [`Findings`];
//! - [`Report::new`], which turns findings into lines and a code.
//!
//! The held delivery doctor (Build 3 T17) takes the same three steps over a
//! placement map `baley install` supplies. Nothing here writes a file, the
//! ledger or Claude Code's settings, and nothing guesses where an artifact
//! belongs: an artifact whose placement is unknown is reported as not
//! installed, and that alone never raises the exit status.

pub mod report;

use std::path::PathBuf;

pub use report::Report;

use crate::host_artifacts::executable::{Executable, MissingPrerequisite};
use crate::host_artifacts::placement::{Artifact, Placement, PlacementMap};
use crate::host_artifacts::stubs;

/// Why no placement map could be built for the running binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapFault {
    /// The running binary's path could not be read; the system's cause.
    PathUnreadable(String),
    /// The running binary's path is not usable as the executable the hook
    /// and the registration run.
    Executable {
        /// The path as read.
        path: PathBuf,
        /// What is wrong with it.
        refusal: MissingPrerequisite,
    },
    /// The map was refused when it was built; the reason.
    Refused(String),
}

/// The placement map the command line uses: every stub of the compiled
/// manifest, the registration, the hook and the settings all with an
/// unknown place, and the running binary as the executable. `running` is
/// what `std::env::current_exe()` returned.
pub fn all_unknown(running: std::io::Result<PathBuf>) -> Result<PlacementMap, MapFault> {
    let path = running.map_err(|error| MapFault::PathUnreadable(error.to_string()))?;
    let executable = Executable::new(&path).map_err(|refusal| MapFault::Executable {
        path: path.clone(),
        refusal,
    })?;
    let manifest = stubs::manifest(&stubs::front_doors())
        .map_err(|duplicate| MapFault::Refused(duplicate.to_string()))?;
    let stubs = manifest
        .into_iter()
        .map(|entry| (entry, Placement::Unknown))
        .collect();
    PlacementMap::new(
        executable,
        stubs,
        Placement::Unknown,
        Placement::Unknown,
        Placement::Unknown,
    )
    .map_err(|refusal| MapFault::Refused(refusal.to_string()))
}

/// What was gathered about the host.
#[derive(Debug, Clone)]
pub struct Observation {
    /// The placement map, or why none could be built.
    pub placement: Result<PlacementMap, MapFault>,
}

/// What the judgement found out about one artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactState {
    /// No place is known for it, so nothing was looked for.
    NotInstalled,
}

/// The judged observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Findings {
    /// Why no map exists, when none could be built.
    pub no_map: Option<MapFault>,
    /// Each artifact of the map and what was found, in artifact order.
    pub artifacts: Vec<(Artifact, ArtifactState)>,
}

/// Judges an observation into findings.
pub fn judge(observation: &Observation) -> Findings {
    match &observation.placement {
        Err(fault) => Findings {
            no_map: Some(fault.clone()),
            artifacts: Vec::new(),
        },
        Ok(map) => Findings {
            no_map: None,
            artifacts: map
                .not_installed()
                .into_iter()
                .map(|artifact| (artifact, ArtifactState::NotInstalled))
                .collect(),
        },
    }
}
