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
//! The placement map is decided by [`installed_placements`] from the paths
//! the guard and `baley install` share and from the latest install record:
//! only an artifact the record shows `baley install` wrote has a place.
//! Nothing here writes a file, the ledger or Claude Code's settings, and
//! nothing guesses where an artifact belongs: an artifact whose placement is
//! unknown is reported as not installed, and that alone never raises the exit
//! status.

pub mod guard_records;
pub mod placed;
pub mod prerequisites;
pub mod protection;
pub mod report;
pub mod server_context;
pub mod store_health;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use baley_store::{Health, Ledger, ProjectId, StoreError};
use serde_json::Value;

pub use report::Report;

use placed::{ExecutableGap, FileState, RegistrationJudgement, StubJudgement};

use crate::folders::Folders;
use crate::host_artifacts::installed::{self, Installed};
use crate::host_artifacts::placement::{Artifact, Placement, PlacementMap};

/// Why no placement map could be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapFault {
    /// `HOME` or `CLAUDE_CONFIG_DIR` cannot place the artifacts, or the paths
    /// derived from them cannot form a map; the reason as worded for the owner.
    Placements(String),
    /// The latest install record could not be read; the store's text. An
    /// unreadable record is never taken for a machine where install never ran.
    Record(String),
}

/// Decides the placement map from the derivation the guard and install share
/// and from what reading the latest install record for `claude-code` found.
///
/// The executable is always the derived stable path, never the running
/// binary. With no record every artifact and the versions folder are unknown,
/// so the report says `not installed` for each and raises nothing for it. With
/// a record, a stub takes its derived path when the record lists its identity,
/// the registration when the record holds a registration, the hook when it
/// holds a hook and the settings when it holds a sandbox entry, and the
/// versions folder takes its derived path. A derivation refusal or a failed
/// read of the record is the fault.
pub fn installed_placements(
    derived: Result<Installed, installed::Refusal>,
    record: Result<Option<Value>, StoreError>,
) -> Result<PlacementMap, MapFault> {
    let installed = derived.map_err(|refusal| MapFault::Placements(refusal.to_string()))?;
    let record = record.map_err(|error| MapFault::Record(error.to_string()))?;
    let versions = match record {
        Some(_) => Placement::At(installed.layout.versions_folder().to_path_buf()),
        None => Placement::Unknown,
    };
    let record = record.unwrap_or(Value::Null);
    let lists = |identity: &str| {
        record["stubs"]
            .as_array()
            .is_some_and(|stubs| stubs.iter().any(|stub| stub["identity"] == identity))
    };
    let holds = |value: &Value| !value.is_null();
    let place = |held: bool, path: &Path| {
        if held {
            Placement::At(path.to_path_buf())
        } else {
            Placement::Unknown
        }
    };
    let stubs = installed
        .placements
        .expected_files()
        .into_iter()
        .filter_map(|file| {
            file.stub
                .map(|entry| (entry.clone(), place(lists(&entry.identity), file.path)))
        })
        .collect();
    PlacementMap::new(
        installed.placements.executable().clone(),
        stubs,
        place(
            holds(&record["registered"]["registration"]),
            &installed.registration_file,
        ),
        place(
            holds(&record["registered"]["hook"]),
            &installed.settings_file,
        ),
        place(holds(&record["sandbox"]), &installed.settings_file),
        versions,
    )
    .map_err(|refusal| MapFault::Placements(refusal.to_string()))
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
    /// The registration document's entry runs another command or arguments,
    /// or sets environment entries.
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
    /// Whether the guard's per-user records were behind at the start.
    pub guard_records: guard_records::Judged,
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
                guard_records: guard_records::judge(&observation.health),
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
        guard_records: guard_records::judge(&observation.health),
        documents,
    }
}

/// Plain-value builders the tests of this module's files share.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::host_artifacts::executable::Executable;
    use crate::host_artifacts::stubs;

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
            Placement::Unknown,
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

    /// The raw view state of a project: the live view set against the
    /// binary's, and each view's live version against its binary version.
    pub(crate) fn raw_views(
        live_set: Option<u32>,
        binary_set: u32,
        views: &[(Option<u32>, u32)],
    ) -> baley_store::RawViewHealth {
        baley_store::RawViewHealth {
            views: views
                .iter()
                .enumerate()
                .map(|(index, (live, binary))| baley_store::ViewHealth {
                    view: format!("view{index}"),
                    live_version: *live,
                    binary_version: *binary,
                })
                .collect(),
            view_set: (live_set, binary_set),
            building: None,
        }
    }

    /// A project whose chain verified, with `head` as its last sequence.
    pub(crate) fn project_health(
        id: &str,
        raw_views: Result<baley_store::RawViewHealth, baley_store::StoreError>,
        head: Option<u64>,
    ) -> baley_store::ProjectHealth {
        use baley_store::{
            AnchorCheck, AnchorVerdict, ChainReport, ClaimCounts, Hash, Head,
            StoredAnchorComparison, UnanchoredAge, VerifyReport, ViewsReport,
        };
        baley_store::ProjectHealth {
            project: ProjectId(id.into()),
            check: AnchorCheck::LocalOnly,
            verify: Ok(VerifyReport {
                chain: ChainReport {
                    head: head.map(|seq| Head {
                        seq,
                        hash: Hash([5; 32]),
                    }),
                    first_break: None,
                    anchor: AnchorVerdict::NoAnchor,
                    unanchored: None,
                    acknowledged_restores: vec![],
                    age_unanchored_since: None,
                },
                payloads: vec![],
                bodies_checked: 0,
                tombstones_checked: 0,
                stored_anchor: None,
                stored_anchor_comparison: StoredAnchorComparison::NotCompared,
            }),
            remote_absent_local_row: None,
            unanchored: UnanchoredAge::None,
            raw_views,
            views_check: Ok(ViewsReport {
                checked_seq: head.unwrap_or(0),
                differing: vec![],
            }),
            claims: Ok(ClaimCounts {
                active: 0,
                interrupted: 0,
                awaiting_owner: 0,
            }),
        }
    }

    /// The clean health holding exactly these projects.
    pub(crate) fn health_with(projects: Vec<baley_store::ProjectHealth>) -> Health {
        Health {
            projects,
            ..clean_health()
        }
    }

    /// A health whose `user` project has views behind this binary's, or at
    /// them.
    pub(crate) fn user_health(behind: bool) -> Health {
        let set = if behind { 6 } else { 7 };
        health_with(vec![project_health(
            baley_core::catalog::USER_PROJECT,
            Ok(raw_views(Some(set), 7, &[(Some(1), 1), (Some(3), 3)])),
            Some(4),
        )])
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
        let proposal = security::propose(&folders(), &exe, &[SETTINGS.into()], &[]).settings;
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

    fn record_for(
        stubs: &[&str],
        registration: bool,
        hook: bool,
        sandbox: bool,
    ) -> serde_json::Value {
        let settings = "/home/o/.claude/settings.json";
        serde_json::json!({
            "host": "claude-code",
            "binary_version": "0.1.0",
            "binary_path": "/home/o/.local/bin/baley",
            "complete": stubs.len() == 2 && registration && hook && sandbox,
            "registered": {
                "registration": registration.then(|| serde_json::json!({
                    "path": "/home/o/.claude.json",
                    "entry": {"command": "/home/o/.local/bin/baley", "args": ["serve"]},
                })),
                "hook": hook.then(|| serde_json::json!({
                    "path": settings,
                    "item": {"matcher": "Bash", "hooks": [{"type": "command", "command": "baley guard"}]},
                })),
            },
            "stubs": stubs.iter().map(|identity| serde_json::json!({
                "identity": identity,
                "path": format!("/home/o/.claude/skills/{identity}/SKILL.md"),
                "sha256": "a".repeat(64),
            })).collect::<Vec<_>>(),
            "sandbox": sandbox.then(|| serde_json::json!({"settings_path": settings})),
            "defaults": {},
            "updates": {"auto": false, "staged_version": null},
        })
    }

    fn derived() -> Result<
        crate::host_artifacts::installed::Installed,
        crate::host_artifacts::installed::Refusal,
    > {
        Ok(crate::install::fixtures::installed())
    }

    fn files(map: &PlacementMap) -> Vec<(Artifact, String)> {
        map.expected_files()
            .into_iter()
            .map(|file| (file.artifact, file.path.to_str().unwrap().to_owned()))
            .collect()
    }

    #[test]
    fn an_installed_artifact_reported_not_installed_by_the_doctor_is_caught() {
        let record = record_for(&["bal-capture", "bal-help"], true, true, true);
        let map = installed_placements(derived(), Ok(Some(record))).unwrap();
        assert_eq!(map.not_installed(), []);
        assert_eq!(
            files(&map),
            [
                (
                    Artifact::Stub("bal-capture".into()),
                    "/home/o/.claude/skills/bal-capture/SKILL.md".to_owned()
                ),
                (
                    Artifact::Stub("bal-help".into()),
                    "/home/o/.claude/skills/bal-help/SKILL.md".to_owned()
                ),
                (Artifact::Registration, "/home/o/.claude.json".to_owned()),
                (Artifact::Hook, "/home/o/.claude/settings.json".to_owned()),
                (
                    Artifact::Settings,
                    "/home/o/.claude/settings.json".to_owned()
                ),
            ]
        );
        assert_eq!(map.executable().as_str(), "/home/o/.local/bin/baley");
        assert_eq!(
            map.write_only_folders(),
            [PathBuf::from(
                "/home/o/.local/lib/crenshawdev/baley/versions"
            )]
        );
    }

    #[test]
    fn a_never_installed_machine_reported_missing_or_judged_by_the_running_binary_is_caught() {
        let map = installed_placements(derived(), Ok(None)).unwrap();
        assert_eq!(
            map.not_installed(),
            [
                Artifact::Stub("bal-capture".into()),
                Artifact::Stub("bal-help".into()),
                Artifact::Registration,
                Artifact::Hook,
                Artifact::Settings,
            ]
        );
        assert_eq!(files(&map), []);
        assert_eq!(map.executable().as_str(), "/home/o/.local/bin/baley");

        let fault =
            installed_placements(derived(), Err(baley_store::StoreError::Busy)).unwrap_err();
        assert_eq!(
            fault,
            MapFault::Record(baley_store::StoreError::Busy.to_string())
        );
    }

    #[test]
    fn an_artifact_the_record_does_not_list_reported_installed_is_caught() {
        let capture_only = record_for(&["bal-capture"], false, true, true);
        let map = installed_placements(derived(), Ok(Some(capture_only))).unwrap();
        assert_eq!(
            files(&map),
            [
                (
                    Artifact::Stub("bal-capture".into()),
                    "/home/o/.claude/skills/bal-capture/SKILL.md".to_owned()
                ),
                (Artifact::Hook, "/home/o/.claude/settings.json".to_owned()),
                (
                    Artifact::Settings,
                    "/home/o/.claude/settings.json".to_owned()
                ),
            ]
        );
        assert_eq!(
            map.not_installed(),
            [Artifact::Stub("bal-help".into()), Artifact::Registration]
        );

        let help_only = record_for(&["bal-help"], true, false, false);
        let map = installed_placements(derived(), Ok(Some(help_only))).unwrap();
        assert_eq!(
            files(&map),
            [
                (
                    Artifact::Stub("bal-help".into()),
                    "/home/o/.claude/skills/bal-help/SKILL.md".to_owned()
                ),
                (Artifact::Registration, "/home/o/.claude.json".to_owned()),
            ]
        );
        assert_eq!(
            map.not_installed(),
            [
                Artifact::Stub("bal-capture".into()),
                Artifact::Hook,
                Artifact::Settings,
            ]
        );
    }
}

#[cfg(test)]
mod integration {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use baley_core::capture::CaptureKind;
    use baley_store::{
        Admin, ClaimDecision, ClaimOwner, Claimed, Hash, Ledger, Observed, ProjectId, RequestId,
        ServerCaller,
    };
    use serde_json::json;

    use super::fixtures::{executable, folders, map};
    use super::*;
    use crate::host_artifacts::compose::compose;
    use crate::host_artifacts::stubs;
    use crate::host_artifacts::{hook, security};
    use crate::mcp::capture::{CAPTURE_COMMAND, record};
    use crate::mcp::prepare::{WriteRequest, prepared_command};

    const T0: &str = "2026-10-08T09:00:00Z";
    const A: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const B: &str = "7a2d3b5f-9c0e-4f1a-8b2c-3d4e5f6a7b8c";
    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    const CLAIM_REQUEST: &str = "1e2f3a4b-5c6d-4e7f-8a9b-0c1d2e3f4a5b";

    /// Every file under `folder` with its bytes, sorted by path. Over the ledger
    /// home it covers the database and its write-ahead log, which hold every
    /// durable row, so a changed row, new or old, shows as changed bytes. The
    /// `-shm` file is left out: it is SQLite's WAL index, which a plain read may
    /// update and which holds no durable row.
    fn listing(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        let mut pending = vec![folder.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if !path.to_string_lossy().ends_with("-shm") {
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

        let home = root.join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
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
        // An open claim gives the ledger a row a host check could change in place,
        // with no event and no new project.
        let claim_request = WriteRequest {
            kind: baley_store::CommandKind("anchor.push".into()),
            request_id: RequestId(CLAIM_REQUEST.into()),
            digest: Hash([2; 32]),
        };
        let claim_command = prepared_command(&ProjectId(B.into()), &claim_request, 0, T0, &caller);
        let claimed = store
            .claim(&claim_command, &mut |_| {
                Ok(ClaimDecision::Claim {
                    intent: json!({"action": "push"}),
                    owner: ClaimOwner {
                        process: "test".into(),
                        host_session: SESSION.into(),
                        started_at: T0.into(),
                    },
                    git: None,
                    observed: Observed::default(),
                })
            })
            .unwrap();
        assert!(matches!(claimed, Claimed::New { .. }));

        let place = root.join("place");
        let help = place.join("skills/bal-help/SKILL.md");
        let settings = place.join("settings.json");
        let registration = place.join("claude.json");
        std::fs::create_dir_all(help.parent().unwrap()).unwrap();
        let exe = executable();
        let proposal = security::propose(&folders(), &exe, &[], &[]).settings;
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
        let ledger_before = listing(&home);
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
        assert_eq!(listing(&home), ledger_before);
        assert_eq!(heads_before.len(), 2);
    }
}
