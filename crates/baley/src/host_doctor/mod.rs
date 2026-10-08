//! The runtime doctor's host checks: what Claude Code's artifacts look like
//! on this machine, gathered as observations, judged as plain values and
//! reported as owner lines with an exit code.
//!
//! The work is three steps, each public and tested apart:
//! - an [`Observation`], what was gathered;
//! - [`judge`], which turns an observation into [`Findings`];
//! - [`Report::new`], which turns findings into lines and a code.
//!
//! [`gather`] is the one place that touches the filesystem: it reads each
//! placed file once and asks about the executable, and owns no rule. The
//! readers and their judgements are in [`placed`].
//!
//! The held delivery doctor (Build 3 T17) takes the same three steps over a
//! placement map `baley install` supplies. Nothing here writes a file, the
//! ledger or Claude Code's settings, and nothing guesses where an artifact
//! belongs: an artifact whose placement is unknown is reported as not
//! installed, and that alone never raises the exit status.

pub mod placed;
pub mod report;

use std::path::{Path, PathBuf};

pub use report::Report;

use placed::{ExecutableGap, FileState, RegistrationJudgement, StubJudgement};

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

/// One path read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRead {
    /// The path, as the placement map supplied it.
    pub path: PathBuf,
    /// What the read found.
    pub state: FileState,
}

/// What was gathered when a placement map exists.
#[derive(Debug, Clone)]
pub struct Mapped {
    /// The placement map the host is judged against.
    pub map: PlacementMap,
    /// What is wrong with the map's executable, or none when it is a
    /// regular file.
    pub executable: Option<ExecutableGap>,
    /// Every distinct path of the map's expected files, read once.
    pub reads: Vec<PlacedRead>,
}

/// What was gathered about the host.
#[derive(Debug, Clone)]
pub struct Observation {
    /// What was found for the placement map, or why none could be built.
    pub host: Result<Mapped, MapFault>,
}

/// Gathers the observation for a placement map: reads each expected file
/// once and asks the filesystem about the executable. Nothing is written,
/// opened for writing or run. It owns no rule and has no unit test.
pub fn gather(placement: Result<PlacementMap, MapFault>) -> Observation {
    let host = placement.map(|map| {
        let executable = placed::observe_executable(Path::new(map.executable().as_str()));
        let mut reads: Vec<PlacedRead> = Vec::new();
        for file in map.expected_files() {
            if !reads.iter().any(|read| read.path == file.path) {
                reads.push(PlacedRead {
                    path: file.path.to_path_buf(),
                    state: placed::read(file.path),
                });
            }
        }
        Mapped {
            map,
            executable,
            reads,
        }
    });
    Observation { host }
}

/// What the judgement found out about one artifact.
#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactState {
    /// No place is known for it, so nothing was looked for.
    NotInstalled,
    /// A place is known but the observation holds no read of it.
    NotRead {
        /// The place.
        path: PathBuf,
    },
    /// A place is known and nothing is there.
    Missing {
        /// The place.
        path: PathBuf,
    },
    /// A place is known and what is there cannot be used.
    Fault {
        /// The place.
        path: PathBuf,
        /// What is wrong with it.
        fault: placed::Fault,
    },
    /// A place is known and what is there was read.
    Read {
        /// The place.
        path: PathBuf,
    },
    /// The stub is the manifest entry's, byte for byte.
    StubMatches {
        /// The place.
        path: PathBuf,
        /// The manifest entry's SHA-256, which the bytes found share.
        digest: String,
    },
    /// The stub differs from the manifest entry.
    StubDiffers {
        /// The place.
        path: PathBuf,
        /// The manifest entry's SHA-256.
        expected: String,
        /// The SHA-256 of the bytes found.
        found: String,
    },
    /// The registration document holds the entry Baley renders.
    RegistrationMatches {
        /// The place.
        path: PathBuf,
    },
    /// The registration document has no entry under Baley's key.
    RegistrationMissing {
        /// The place.
        path: PathBuf,
    },
    /// The registration document's entry runs another command or arguments.
    RegistrationDiffers {
        /// The place.
        path: PathBuf,
        /// The entry found.
        found: serde_json::Value,
    },
}

impl ArtifactState {
    /// Whether the state is a host gap, which raises the exit status. An
    /// artifact that is not installed is not one.
    pub fn is_gap(&self) -> bool {
        !matches!(
            self,
            ArtifactState::NotInstalled
                | ArtifactState::Read { .. }
                | ArtifactState::StubMatches { .. }
                | ArtifactState::RegistrationMatches { .. }
        )
    }
}

/// What a settings, hook or registration file held.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// The observation holds no read of it.
    NotRead,
    /// Nothing is there.
    Missing,
    /// It could not be used.
    Fault(placed::Fault),
    /// A JSON object.
    Object(serde_json::Value),
}

/// One registration, hook or settings file and what it held, carried for the
/// judgements that read documents.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The place.
    pub path: PathBuf,
    /// The artifacts placed in it, in artifact order.
    pub artifacts: Vec<Artifact>,
    /// What it held.
    pub content: Content,
}

/// The judged observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Findings {
    /// Why no map exists, when none could be built.
    pub no_map: Option<MapFault>,
    /// Each artifact of the map and what was found, in artifact order.
    pub artifacts: Vec<(Artifact, ArtifactState)>,
    /// What is wrong with the executable, with its path.
    pub executable: Option<(String, ExecutableGap)>,
    /// Each document the map places artifacts in.
    pub documents: Vec<Document>,
}

/// Where an artifact sits in the order the report lists them: the stubs,
/// then the registration, the hook and the settings.
fn rank(artifact: &Artifact) -> u8 {
    match artifact {
        Artifact::Stub(_) => 0,
        Artifact::Registration => 1,
        Artifact::Hook => 2,
        Artifact::Settings => 3,
    }
}

/// Judges an observation into findings.
pub fn judge(observation: &Observation) -> Findings {
    let mapped = match &observation.host {
        Ok(mapped) => mapped,
        Err(fault) => {
            return Findings {
                no_map: Some(fault.clone()),
                artifacts: Vec::new(),
                executable: None,
                documents: Vec::new(),
            };
        }
    };
    let read = |path: &Path| {
        mapped
            .reads
            .iter()
            .find(|read| read.path == path)
            .map(|read| &read.state)
    };

    let mut documents = Vec::new();
    for document in mapped.map.documents() {
        let content = match read(document.path) {
            None => Content::NotRead,
            Some(FileState::Absent) => Content::Missing,
            Some(FileState::Fault(fault)) => Content::Fault(fault.clone()),
            Some(FileState::Bytes(bytes)) => match placed::document(bytes) {
                Ok(value) => Content::Object(value),
                Err(fault) => Content::Fault(fault),
            },
        };
        documents.push(Document {
            path: document.path.to_path_buf(),
            artifacts: document.artifacts,
            content,
        });
    }

    let mut artifacts: Vec<(Artifact, ArtifactState)> = Vec::new();
    for file in mapped.map.expected_files() {
        let path = file.path.to_path_buf();
        let state = match documents
            .iter()
            .find(|document| document.artifacts.contains(&file.artifact))
        {
            Some(document) => match &document.content {
                Content::NotRead => ArtifactState::NotRead { path },
                Content::Missing => ArtifactState::Missing { path },
                Content::Fault(fault) => ArtifactState::Fault {
                    path,
                    fault: fault.clone(),
                },
                Content::Object(value) if file.artifact == Artifact::Registration => {
                    match placed::registration(value, mapped.map.executable()) {
                        RegistrationJudgement::Matches => {
                            ArtifactState::RegistrationMatches { path }
                        }
                        RegistrationJudgement::Missing => {
                            ArtifactState::RegistrationMissing { path }
                        }
                        RegistrationJudgement::Differs(found) => {
                            ArtifactState::RegistrationDiffers { path, found }
                        }
                    }
                }
                Content::Object(_) => ArtifactState::Read { path },
            },
            None => match read(file.path) {
                None => ArtifactState::NotRead { path },
                Some(FileState::Absent) => ArtifactState::Missing { path },
                Some(FileState::Fault(fault)) => ArtifactState::Fault {
                    path,
                    fault: fault.clone(),
                },
                Some(FileState::Bytes(bytes)) => match file.stub {
                    Some(entry) => match placed::stub(entry, bytes) {
                        StubJudgement::Matches => ArtifactState::StubMatches {
                            path,
                            digest: entry.digest.clone(),
                        },
                        StubJudgement::Differs { found_digest } => ArtifactState::StubDiffers {
                            path,
                            expected: entry.digest.clone(),
                            found: found_digest,
                        },
                    },
                    None => ArtifactState::Read { path },
                },
            },
        };
        artifacts.push((file.artifact, state));
    }
    artifacts.extend(
        mapped
            .map
            .not_installed()
            .into_iter()
            .map(|artifact| (artifact, ArtifactState::NotInstalled)),
    );
    artifacts.sort_by_key(|(artifact, _)| rank(artifact));

    Findings {
        no_map: None,
        artifacts,
        executable: mapped
            .executable
            .clone()
            .map(|gap| (mapped.map.executable().as_str().to_owned(), gap)),
        documents,
    }
}

/// Plain-value builders the tests of this module's files share.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) const EXECUTABLE: &str = "/usr/local/bin/baley";
    pub(crate) const HELP: &str = "/home/o/.claude/skills/bal-help/SKILL.md";
    pub(crate) const REGISTRATION: &str = "/home/o/.claude.json";
    pub(crate) const SETTINGS: &str = "/home/o/.claude/settings.json";

    pub(crate) fn executable() -> Executable {
        Executable::new(EXECUTABLE).unwrap()
    }

    /// A map with the given places and every other placement unknown.
    pub(crate) fn map(
        capture: Option<&str>,
        help: Option<&str>,
        registration: Option<&str>,
        hook: Option<&str>,
        settings: Option<&str>,
    ) -> PlacementMap {
        let place =
            |path: Option<&str>| path.map_or(Placement::Unknown, |p| Placement::At(p.into()));
        let stubs = stubs::manifest(&stubs::front_doors())
            .unwrap()
            .into_iter()
            .map(|entry| {
                let placement = match entry.identity.as_str() {
                    "bal-capture" => place(capture),
                    "bal-help" => place(help),
                    other => panic!("an unexpected front door {other}"),
                };
                (entry, placement)
            })
            .collect();
        PlacementMap::new(
            executable(),
            stubs,
            place(registration),
            place(hook),
            place(settings),
        )
        .unwrap()
    }

    /// The observation of a map whose executable is a regular file and
    /// whose files were read as given.
    pub(crate) fn observed(map: PlacementMap, reads: Vec<(&str, FileState)>) -> Observation {
        Observation {
            host: Ok(Mapped {
                map,
                executable: None,
                reads: reads
                    .into_iter()
                    .map(|(path, state)| PlacedRead {
                        path: path.into(),
                        state,
                    })
                    .collect(),
            }),
        }
    }
}
