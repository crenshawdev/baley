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
//! readers and their judgements are in [`placed`]. [`protection`] runs the
//! coverage judge once over the settings and hook documents the map
//! places, with the map's protected paths as the write-only list, and checks
//! that the guard runs before all nine tools of the hook's matcher.
//! [`prerequisites`] finds the programs the sandbox needs on `PATH` for the
//! target platform, and a missing one marks the sandbox unsupported for the
//! coverage judge.
//!
//! The held delivery doctor (Build 3 T17) takes the same three steps over a
//! placement map `baley install` supplies. Nothing here writes a file, the
//! ledger or Claude Code's settings, and nothing guesses where an artifact
//! belongs: an artifact whose placement is unknown is reported as not
//! installed, and that alone never raises the exit status.

pub mod placed;
pub mod prerequisites;
pub mod protection;
pub mod report;
pub mod server_context;
pub mod store_health;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use baley_store::{Health, Ledger, ProjectId};

pub use report::Report;

use placed::{ExecutableGap, FileState, RegistrationJudgement, StubJudgement};

use crate::folders::Folders;
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
    /// regular file with an execute bit.
    pub executable: Option<ExecutableGap>,
    /// Every distinct path of the map's expected files, read once.
    pub reads: Vec<PlacedRead>,
}

/// What was gathered about the host.
#[derive(Debug, Clone)]
pub struct Observation {
    /// What was found for the placement map, or why none could be built.
    pub host: Result<Mapped, MapFault>,
    /// Baley's resolved folders, which the coverage judge protects.
    pub folders: Folders,
    /// The discovered checkout's `baley.toml`, which is write-only too.
    pub checkout_file: Option<PathBuf>,
    /// The platform and where the sandbox's programs were found.
    pub prerequisites: prerequisites::Observed,
    /// The last server call the ledger records.
    pub server: server_context::Observed,
    /// What the store's doctor returned, which the compatibility and
    /// folder lines are judged from.
    pub health: Health,
}

/// The ledger side of the gather: the store to read history from and the
/// projects the doctor already listed.
pub struct Stored<'a, S> {
    /// The store, read through the storage port's history only.
    pub store: &'a S,
    /// Every project in the ledger, in id order.
    pub projects: &'a [ProjectId],
    /// What the store's doctor already returned.
    pub health: &'a Health,
}

/// Gathers the observation for a placement map: reads each expected file
/// once, asks the filesystem about the executable and pages through the
/// ledger's history for the last server call. Nothing is written, opened for
/// writing or run. It owns no rule and has no unit test of its own; one
/// integration check runs it whole and asserts only that nothing was
/// written.
pub fn gather<S: Ledger>(
    placement: Result<PlacementMap, MapFault>,
    folders: Folders,
    checkout_file: Option<PathBuf>,
    path: Option<&OsStr>,
    os: &str,
    stored: Stored<'_, S>,
) -> Observation {
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
    Observation {
        host,
        folders,
        checkout_file,
        prerequisites: prerequisites::gather(path, os),
        server: server_context::gather(stored.store, stored.projects),
        health: stored.health.clone(),
    }
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
    /// What the settings and hook documents configure, or why that was not
    /// judged. None when there is no map.
    pub coverage: Option<Result<protection::Judged, protection::NotJudged>>,
    /// Whether the guard runs before all nine tools of the hook's matcher.
    /// None when the hook placement is unknown or its document unusable.
    pub nine_tools: Option<protection::NineTools>,
    /// The platform and the sandbox programs it needs, found on `PATH`.
    pub prerequisites: prerequisites::Judged,
    /// What the last recorded server call shows of the server's context.
    pub server: server_context::Judged,
    /// The stored epoch against this binary's, and the folders.
    pub store: store_health::Judged,
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
                coverage: None,
                nine_tools: None,
                prerequisites: prerequisites::judge(&observation.prerequisites),
                server: server_context::judge(&observation.server),
                store: store_health::judge(&observation.health, &observation.folders),
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

    let prerequisites = prerequisites::judge(&observation.prerequisites);

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
        coverage: Some(protection::judge(
            &mapped.map,
            &documents,
            &observation.folders,
            observation.checkout_file.as_deref(),
            &prerequisites.unsupported(),
        )),
        nine_tools: protection::nine_tools(&mapped.map, &documents),
        prerequisites,
        server: server_context::judge(&observation.server),
        store: store_health::judge(&observation.health, &observation.folders),
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
    pub(crate) const HOOKS: &str = "/home/o/.claude/hooks.json";
    pub(crate) const HOME: &str = "/home/o/.local/share/crenshawdev/baley";
    pub(crate) const CONFIG: &str = "/home/o/.config/crenshawdev/baley";

    pub(crate) fn executable() -> Executable {
        Executable::new(EXECUTABLE).unwrap()
    }

    pub(crate) fn folders() -> Folders {
        Folders {
            home: HOME.into(),
            config: CONFIG.into(),
        }
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

    /// The store's health with nothing to find: this binary's epoch, a clean
    /// integrity check and no projects.
    pub(crate) fn clean_health() -> Health {
        Health {
            epoch: baley_store_sqlite::EPOCH,
            scrub_pending: None,
            integrity: vec!["ok".into()],
            database_bytes: 8192,
            log_bytes: 0,
            projects: vec![],
        }
    }

    /// A Linux search that finds both sandbox programs.
    pub(crate) fn linux_with_both() -> prerequisites::Observed {
        found_on("linux", &["bwrap", "socat"])
    }

    /// A search on `os` that finds each named program in `/usr/bin`, and
    /// gives the others a `PATH` entry with nothing in it.
    pub(crate) fn found_on(os: &str, programs: &[&str]) -> prerequisites::Observed {
        let entry = |present: bool| prerequisites::Candidate {
            folder: "/usr/bin".into(),
            regular_file: present,
            mode: if present { 0o755 } else { 0 },
        };
        prerequisites::Observed {
            os: os.into(),
            searches: ["bwrap", "socat"]
                .into_iter()
                .map(|name| prerequisites::Search {
                    program: name.into(),
                    candidates: vec![entry(programs.contains(&name))],
                })
                .collect(),
        }
    }

    /// The observation of a host for which no map could be built.
    pub(crate) fn unmapped(fault: MapFault) -> Observation {
        Observation {
            host: Err(fault),
            folders: folders(),
            checkout_file: None,
            prerequisites: linux_with_both(),
            server: server_context::Observed::NoCall,
            health: clean_health(),
        }
    }

    /// The observation of a map whose executable is a runnable file and
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
            folders: folders(),
            checkout_file: None,
            prerequisites: linux_with_both(),
            server: server_context::Observed::NoCall,
            health: clean_health(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::host_artifacts::compose::compose;
    use crate::host_artifacts::coverage::{Access, Cause, Mechanism, Tool, Verdict};
    use crate::host_artifacts::{hook, security};

    #[test]
    fn a_missing_program_left_out_of_the_coverage_verdicts_is_caught() {
        let exe = executable();
        let proposal = security::propose(&folders(), &exe, &[SETTINGS.into()]).settings;
        let document = compose(None, &[proposal, hook::render(&exe)], &folders(), &exe).document;
        let bytes = serde_json::to_vec(&document).unwrap();
        for present in [["bwrap"], ["socat"]] {
            let mut observation = observed(
                map(None, None, None, Some(SETTINGS), Some(SETTINGS)),
                vec![(SETTINGS, FileState::Bytes(bytes.clone()))],
            );
            observation.prerequisites = found_on("linux", &present);
            let findings = judge(&observation);
            let coverage = findings.coverage.unwrap().unwrap().coverage;
            for tool in [Tool::Bash, Tool::Monitor, Tool::PowerShell] {
                for access in [Access::Read, Access::Write] {
                    assert_eq!(
                        coverage.verdict(tool, access),
                        Some(&Verdict::Gap(vec![Cause::Unsupported(Mechanism::Sandbox)])),
                        "{tool:?} {access:?} with only {present:?} found"
                    );
                }
            }
            for tool in [
                Tool::Read,
                Tool::Grep,
                Tool::Glob,
                Tool::Write,
                Tool::Edit,
                Tool::NotebookEdit,
            ] {
                let access = tool.accesses()[0];
                assert!(
                    matches!(
                        coverage.verdict(tool, access),
                        Some(Verdict::Covered { .. })
                    ),
                    "{tool:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod integration {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use baley_core::capture::CaptureKind;
    use baley_store::{Admin, Hash, Ledger, ProjectId, RequestId, ServerCaller};
    use serde_json::json;

    use super::fixtures::{executable, folders, map};
    use super::*;
    use crate::host_artifacts::compose::compose;
    use crate::host_artifacts::{hook, security};
    use crate::mcp::capture::{CAPTURE_COMMAND, record};
    use crate::mcp::prepare::{WriteRequest, prepared_command};

    const T0: &str = "2026-10-08T09:00:00Z";
    const A: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const B: &str = "7a2d3b5f-9c0e-4f1a-8b2c-3d4e5f6a7b8c";
    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    /// Every file under `folder` with its bytes, sorted by path.
    fn listing(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        let mut pending = vec![folder.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    found.insert(path, bytes);
                }
            }
        }
        found
    }

    /// Each listed project with the sequence of its last event.
    fn heads<S: Ledger + Admin>(store: &S) -> Vec<(ProjectId, Option<u64>)> {
        let ids: Vec<ProjectId> = Admin::projects(store)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.into_iter()
            .map(|id| {
                let head = store.head(&id).unwrap().map(|head| head.seq);
                (id, head)
            })
            .collect()
    }

    #[test]
    fn a_host_check_that_writes_the_ledger_or_a_placed_file_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (project, sub) = (root.join("project"), root.join("project/sub"));
        std::fs::create_dir_all(&sub).unwrap();

        let store =
            crate::ledger::open::store(&root.join("home"), T0, crate::ledger::open::options())
                .unwrap();
        for (id, name) in [(A, "a"), (B, "b")] {
            store
                .create_project(&ProjectId(id.into()), name, T0)
                .unwrap();
        }
        let request = WriteRequest {
            kind: baley_store::CommandKind(CAPTURE_COMMAND.into()),
            request_id: RequestId(REQUEST.into()),
            digest: Hash([1; 32]),
        };
        let caller = ServerCaller::new(
            project.to_str().unwrap(),
            sub.to_str().unwrap(),
            "claude-code",
            SESSION,
            &json!(7),
        )
        .unwrap();
        let command = prepared_command(&ProjectId(A.into()), &request, 0, T0, &caller);
        record(&store, &command, CaptureKind::Note, "a note", None).unwrap();

        let place = root.join("place");
        let help = place.join("skills/bal-help/SKILL.md");
        let settings = place.join("settings.json");
        let registration = place.join("claude.json");
        std::fs::create_dir_all(help.parent().unwrap()).unwrap();
        let exe = executable();
        let proposal = security::propose(&folders(), &exe, &[]).settings;
        let document = compose(None, &[proposal, hook::render(&exe)], &folders(), &exe).document;
        std::fs::write(&settings, serde_json::to_vec(&document).unwrap()).unwrap();
        std::fs::write(&registration, b"{}").unwrap();
        let stub = stubs::manifest(&stubs::front_doors())
            .unwrap()
            .into_iter()
            .find(|entry| entry.identity == "bal-help")
            .unwrap();
        std::fs::write(&help, &stub.bytes).unwrap();
        let empty_path = root.join("bin");
        std::fs::create_dir_all(&empty_path).unwrap();

        let (settings_text, help_text, registration_text) = (
            settings.to_str().unwrap(),
            help.to_str().unwrap(),
            registration.to_str().unwrap(),
        );
        let placement = map(
            None,
            Some(help_text),
            Some(registration_text),
            Some(settings_text),
            Some(settings_text),
        );

        let files_before = listing(&place);
        let projects_before = Admin::projects(&store).unwrap();
        let checks: BTreeMap<ProjectId, baley_store::AnchorCheck> = projects_before
            .iter()
            .map(|(id, _)| (id.clone(), baley_store::AnchorCheck::LocalOnly))
            .collect();
        let health = Admin::doctor(&store, T0, &checks).unwrap();
        let heads_before = heads(&store);
        let ids: Vec<ProjectId> = projects_before.iter().map(|(id, _)| id.clone()).collect();
        let search: OsString = empty_path.into();
        let observation = gather(
            Ok(placement),
            folders(),
            None,
            Some(search.as_os_str()),
            "linux",
            Stored {
                store: &store,
                projects: &ids,
                health: &health,
            },
        );
        let _ = Report::new(&judge(&observation));

        assert_eq!(listing(&place), files_before);
        assert_eq!(Admin::projects(&store).unwrap(), projects_before);
        assert_eq!(heads(&store), heads_before);
        assert_eq!(heads_before.len(), 2);
    }
}
