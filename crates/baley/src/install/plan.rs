//! Installation writes decided from supplied placements, file observations
//! and the latest ownership record. Any stub, settings or registration
//! conflict refuses the whole plan. Applied outcomes retain older ownership
//! evidence when a write or launch cannot finish.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use baley_core::policy::Host;
use baley_store::StoreError;
use serde_json::Value;

use crate::folders::Folders;
use crate::host_artifacts::compose::same_server;
use crate::host_artifacts::installed::stub_path;
use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::host_doctor::placed::{ExecutableGap, Fault, FileState};
use crate::host_doctor::prerequisites::Judged;
use crate::ledger::display::Render;
use crate::replace::Failure;
use crate::store::model::digest;

use super::event;
use super::receipt::{ArtifactOutcome, Notes, RetiredOutcome};
use super::registration::{self, Attempt, Registered, Step};
use super::settings::{self, Input};
use super::stubs::{self, Decision};

/// One read of a placed file and the path's own link status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The bytes or fault found by the read, following links.
    pub state: FileState,
    /// Whether the placed path itself is a symbolic link.
    pub symbolic_link: bool,
}

/// Everything the install decision reads, gathered before any write.
#[derive(Debug, Clone, Copy)]
pub struct Gathered<'a> {
    /// Where every artifact goes, and the stable executable.
    pub placements: &'a PlacementMap,
    /// Claude Code's configuration folder, which holds the stubs.
    pub claude_folder: &'a Path,
    /// Baley's data and configuration folders.
    pub folders: &'a Folders,
    /// The latest install record's payload, when there is one.
    pub latest: Option<&'a Value>,
    /// One observation per placed file the decision judges, keyed by path.
    pub observations: &'a BTreeMap<PathBuf, Observation>,
    /// The sandbox programs the platform needs, judged by the doctor's check.
    pub prerequisites: &'a Judged,
    /// What is wrong at the stable path, or none when a runnable file sits
    /// there. While something is wrong, the hook and the registration that
    /// run it are not written.
    pub executable: Option<&'a ExecutableGap>,
}

/// A file write permitted by the ownership decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// The artifacts whose bytes will be placed: one stub, or the hook and
    /// the settings together, since they share one document.
    pub artifacts: Vec<Artifact>,
    /// The absolute target path from the placement map.
    pub path: PathBuf,
    /// The bytes to write.
    pub bytes: Vec<u8>,
    /// SHA-256 of the observed bytes, or none when the path was absent.
    pub read_digest: Option<String>,
}

/// Ordered writes and each artifact's state before those writes are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Writes in order, the settings file first, excluding unchanged files.
    pub writes: Vec<Write>,
    /// Placed artifacts in map order. Pending writes remain `NotWritten` until
    /// the applier confirms them. An undecided registration is `NotWritten` too.
    pub outcomes: Vec<(Artifact, ArtifactOutcome)>,
    /// The settings decision, kept for the receipt and the record.
    pub settings: settings::Decision,
    /// The registration decision: the `claude` commands to run after every
    /// file is written, or that the entry is Baley's already.
    pub registration: Step,
    /// The `mcpServers.baley` entry the first read of the registration file
    /// held, which the record keeps if no launch changes it.
    pub registration_seen: Option<Value>,
    /// Why the hook and the registration were held back, when nothing
    /// runnable sits at the stable path.
    pub wiring: Option<Wiring>,
    /// Stubs this binary no longer ships whose files still hold the bytes
    /// the record says Baley placed, removed after the writes.
    pub removals: Vec<Retired>,
    /// Stubs this binary no longer ships whose files could not be read, with
    /// the cause. They stay recorded, since they may still be Baley's.
    pub unreadable: Vec<(Retired, String)>,
    /// Stubs this binary no longer ships that the record places anywhere but
    /// their own stub path here, such as under an earlier Claude folder. They
    /// are never read or touched and stay recorded as they were.
    pub carried: Vec<Retired>,
}

/// A stub the latest record lists that this binary does not place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retired {
    /// The identity the record lists.
    pub identity: String,
    /// The absolute path the record lists.
    pub path: PathBuf,
    /// The SHA-256 the record holds for the bytes Baley placed there.
    pub sha256: String,
}

/// The stubs the latest record lists that this binary does not place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RetiredStubs {
    /// Recorded at exactly the path a stub of its identity takes under the
    /// current Claude folder, which is no placed file's path: judged, and
    /// removed when it is still Baley's.
    pub own: Vec<Retired>,
    /// Recorded anywhere else. Never read or touched, so a record cannot
    /// point a removal outside a stub's own file, and kept in the record.
    pub elsewhere: Vec<Retired>,
}

/// Sorts the stubs the latest record lists that this binary does not place
/// by whether they sit at their own stub path under `claude_folder`. The
/// identity must be one path name spelled exactly, so `bal-capture/` cannot
/// stand for `bal-capture`.
pub(crate) fn retired(
    latest: Option<&Value>,
    placements: &PlacementMap,
    claude_folder: &Path,
) -> RetiredStubs {
    let placed = placements.expected_files();
    let mut found = RetiredStubs::default();
    let Some(stubs) = latest
        .and_then(|record| record.get("stubs"))
        .and_then(Value::as_array)
    else {
        return found;
    };
    for stub in stubs {
        let field = |name| stub.get(name).and_then(Value::as_str);
        let (Some(identity), Some(path), Some(sha256)) =
            (field("identity"), field("path"), field("sha256"))
        else {
            continue;
        };
        let retired = Retired {
            identity: identity.into(),
            path: PathBuf::from(path),
            sha256: sha256.into(),
        };
        if placed.iter().any(|file| {
            file.stub
                .is_some_and(|entry| entry.identity == retired.identity)
        }) {
            continue;
        }
        let mut parts = Path::new(&retired.identity).components();
        let plain = matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(name)), None) if name == OsStr::new(&retired.identity)
        );
        // A second guard: whatever the identity says, a placed file is
        // never a retired one.
        let own = plain
            && retired.path == stub_path(claude_folder, &retired.identity)
            && placed.iter().all(|file| file.path != retired.path);
        if own {
            found.own.push(retired);
        } else {
            found.elsewhere.push(retired);
        }
    }
    found
}

/// What install does with a retired stub's file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Retiring {
    /// The bytes have the recorded hash and the path is no link.
    Remove,
    /// The file could not be read, so it may still be Baley's.
    Keep(String),
    /// Gone, a link or other bytes: nothing of Baley's is there.
    Leave,
}

fn retiring(retired: &Retired, seen: &Observation) -> Retiring {
    match &seen.state {
        _ if seen.symbolic_link => Retiring::Leave,
        FileState::Bytes(bytes) if digest(bytes) == retired.sha256 => Retiring::Remove,
        FileState::Fault(fault @ Fault::Unreadable(_)) => {
            Retiring::Keep(format!("the file {fault}"))
        }
        _ => Retiring::Leave,
    }
}

/// The hook and the registration held back from an empty stable path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wiring {
    /// The stable path and what is wrong there, as the artifact lines say it.
    pub cause: String,
    /// The gap with its fix.
    pub gap: String,
}

fn unobserved() -> Observation {
    Observation {
        state: FileState::Fault(Fault::Unreadable("no file observation was supplied".into())),
        symbolic_link: false,
    }
}

/// Plans the writes and the retired stubs' removals, or refuses every write
/// with one line per conflict. Observations are keyed by path so artifacts
/// sharing a document use one read. A missing observation refuses a placed
/// artifact rather than inventing an absent file, and removes nothing.
pub(crate) fn judge(gathered: &Gathered<'_>) -> Result<Plan, Render> {
    let placements = gathered.placements;
    let observed = |path: &Path| {
        gathered
            .observations
            .get(path)
            .cloned()
            .unwrap_or_else(unobserved)
    };
    let mut stub_writes = Vec::new();
    let mut outcomes = Vec::new();
    let mut conflicts = Vec::new();
    for file in placements.expected_files() {
        let mut outcome = ArtifactOutcome::NotWritten;
        if let Some(entry) = file.stub {
            let seen = observed(file.path);
            match stubs::judge(
                entry,
                file.path,
                &seen.state,
                seen.symbolic_link,
                gathered.latest,
            ) {
                Ok(Decision::Unchanged) => outcome = ArtifactOutcome::Unchanged,
                Ok(decision) => {
                    let read_digest = match decision {
                        Decision::Replace { read_digest } => Some(read_digest),
                        _ => None,
                    };
                    stub_writes.push(Write {
                        artifacts: vec![file.artifact.clone()],
                        path: file.path.into(),
                        bytes: entry.bytes.clone(),
                        read_digest,
                    });
                }
                Err(conflict) => conflicts.push(conflict),
            }
        }
        outcomes.push((file.artifact, outcome));
    }

    let settings_path = placements
        .expected_files()
        .into_iter()
        .find(|file| file.artifact == Artifact::Settings)
        .map(|file| file.path.to_path_buf())
        .expect("install places the settings file");
    let decision = match settings::current(&settings_path, &observed(&settings_path)) {
        Err(line) => {
            conflicts.push(line);
            None
        }
        Ok(current) => match settings::judge(&Input {
            path: &settings_path,
            current: &current,
            latest: gathered.latest,
            placements,
            folders: gathered.folders,
            prerequisites: gathered.prerequisites,
            wire: gathered.executable.is_none(),
        }) {
            Err(lines) => {
                conflicts.extend(lines);
                None
            }
            Ok(decision) => Some(decision),
        },
    };
    let registration_path = placements
        .expected_files()
        .into_iter()
        .find(|file| file.artifact == Artifact::Registration)
        .map(|file| file.path.to_path_buf())
        .expect("install places the registration file");
    let registration_read = observed(&registration_path).state;
    let registration = match registration::judge(
        &registration_path,
        &registration_read,
        gathered.latest,
        placements.executable(),
    ) {
        Ok(step) => Some(step),
        Err(line) => {
            conflicts.push(line);
            None
        }
    };
    let (Some(decision), Some(registration), true) = (decision, registration, conflicts.is_empty())
    else {
        let mut refusal = Render::refusal("");
        refusal.lines = conflicts;
        return Err(refusal);
    };

    let wiring = gathered.executable.map(|gap| {
        let path = placements.executable().as_str();
        Wiring {
            cause: format!("the stable path {path} {gap}"),
            gap: format!(
                "nothing runnable is at {path}: it {gap}, so the hook and the registration that run it were not written; run install.sh, or place the baley binary or a link to it at that path, then run baley install again"
            ),
        }
    });
    let mut writes = Vec::new();
    match &decision.bytes {
        Some(bytes) => writes.push(Write {
            artifacts: if wiring.is_some() {
                vec![Artifact::Settings]
            } else {
                vec![Artifact::Hook, Artifact::Settings]
            },
            path: settings_path,
            bytes: bytes.clone(),
            read_digest: decision.read_digest.clone(),
        }),
        None => {
            for (artifact, outcome) in &mut outcomes {
                if matches!(artifact, Artifact::Hook | Artifact::Settings) {
                    *outcome = ArtifactOutcome::Unchanged;
                }
            }
        }
    }
    writes.extend(stub_writes);
    if registration == Step::Unchanged {
        for (artifact, outcome) in &mut outcomes {
            if *artifact == Artifact::Registration {
                *outcome = ArtifactOutcome::Unchanged;
            }
        }
    }
    if let Some(wiring) = &wiring {
        for (artifact, outcome) in &mut outcomes {
            if matches!(artifact, Artifact::Hook | Artifact::Registration) {
                *outcome = ArtifactOutcome::Failed(wiring.cause.clone());
            }
        }
    }
    // A retired stub that is not provably Baley's is the owner's: it is left
    // where it is and refuses nothing.
    let mut removals = Vec::new();
    let mut unreadable = Vec::new();
    let found = retired(gathered.latest, placements, gathered.claude_folder);
    for stub in found.own {
        match retiring(&stub, &observed(&stub.path)) {
            Retiring::Remove => removals.push(stub),
            Retiring::Keep(cause) => unreadable.push((stub, cause)),
            Retiring::Leave => {}
        }
    }
    Ok(Plan {
        writes,
        outcomes,
        settings: decision,
        registration,
        registration_seen: registration::held(&registration_read),
        wiring,
        removals,
        unreadable,
        carried: found.elsewhere,
    })
}

/// Each artifact's outcome after applying a plan, with the first write failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Placed artifacts in map order, including writes that were not reached.
    pub outcomes: Vec<(Artifact, ArtifactOutcome)>,
    /// The first write failure's text, including a failed sync after a rename.
    pub failure: Option<String>,
    /// The gaps and replaced values the receipt reports.
    pub notes: Notes,
    /// The `mcpServers.baley` entry the latest read of the registration file
    /// held.
    pub registration_seen: Option<Value>,
    /// Whether a remove launch succeeded after that read.
    pub registration_removed: bool,
    /// Each planned removal of a retired stub, in plan order, with what
    /// became of it, then each retired stub that could not be read, then
    /// each one recorded away from its own stub path.
    pub retired: Vec<(Retired, RetiredOutcome)>,
}

impl Applied {
    /// Whether no retired stub this run judged was left behind.
    pub fn no_retired_left(&self) -> bool {
        !self
            .retired
            .iter()
            .any(|(_, outcome)| matches!(outcome, RetiredOutcome::Failed(_)))
    }
}

/// Interprets write observations in plan order, then the removals and the
/// registration step. A failed folder sync still placed the bytes or removed
/// the file, but leaves the run partial, and every failure stops subsequent
/// writes.
pub(crate) fn applied(
    plan: &Plan,
    results: &[Result<(), Failure>],
    removals: &[Result<(), Failure>],
    attempt: &Attempt,
) -> Applied {
    let mut applied = Applied {
        outcomes: plan.outcomes.clone(),
        failure: None,
        notes: Notes::from_settings(
            &plan.settings,
            plan.wiring.as_ref().map(|wiring| wiring.gap.as_str()),
        ),
        registration_seen: attempt.seen.clone(),
        registration_removed: attempt.removed,
        retired: Vec::new(),
    };
    for (index, write) in plan.writes.iter().enumerate() {
        let result = results.get(index);
        let written = matches!(result, Some(Ok(()) | Err(Failure::Unsynced { .. })));
        let settings_file = write.artifacts.contains(&Artifact::Settings);
        if let Some(Err(failure)) = result {
            applied.failure = Some(match failure {
                Failure::Refused(conflict) if settings_file => settings::replace_refusal(conflict),
                Failure::Refused(_) | Failure::Unsynced { .. } => failure.to_string(),
                Failure::Unchanged { path, cause } => {
                    format!("not-writable: {}: {cause}", path.display())
                }
            });
        }
        let outcome = if written {
            if write.read_digest.is_some() && !settings_file {
                ArtifactOutcome::Replaced
            } else {
                ArtifactOutcome::Written
            }
        } else {
            let cause = applied
                .failure
                .as_deref()
                .unwrap_or("the write was not reached");
            ArtifactOutcome::Failed(if result.is_none() {
                format!("an earlier write failed: {cause}")
            } else {
                cause.into()
            })
        };
        for artifact in &write.artifacts {
            if let Some((_, state)) = applied
                .outcomes
                .iter_mut()
                .find(|(placed, _)| placed == artifact)
            {
                *state = outcome.clone();
            }
        }
    }
    for (index, stub) in plan.removals.iter().enumerate() {
        let outcome = match removals.get(index) {
            Some(Ok(())) => RetiredOutcome::Removed,
            // The file is gone, but like an unsynced write the run is partial.
            Some(Err(Failure::Unsynced { path, cause })) => {
                applied.failure.get_or_insert_with(|| {
                    format!(
                        "{} was removed, but its folder could not be synced, so the removal may not survive a crash: {cause}",
                        path.display()
                    )
                });
                RetiredOutcome::Removed
            }
            Some(Err(Failure::Unchanged { path, cause })) => {
                RetiredOutcome::Failed(format!("not-removable: {}: {cause}", path.display()))
            }
            Some(Err(Failure::Refused(conflict))) => RetiredOutcome::Failed(conflict.to_string()),
            None => RetiredOutcome::Failed(format!(
                "an earlier write failed: {}",
                applied
                    .failure
                    .as_deref()
                    .unwrap_or("the removal was not reached")
            )),
        };
        applied.retired.push((stub.clone(), outcome));
    }
    for (stub, cause) in &plan.unreadable {
        applied
            .retired
            .push((stub.clone(), RetiredOutcome::Failed(cause.clone())));
    }
    for stub in &plan.carried {
        applied
            .retired
            .push((stub.clone(), RetiredOutcome::Carried));
    }
    let registration = match &attempt.result {
        Registered::Unchanged => ArtifactOutcome::Unchanged,
        Registered::Registered => ArtifactOutcome::Written,
        Registered::Replaced => ArtifactOutcome::Replaced,
        Registered::Refused(cause) => ArtifactOutcome::Failed(cause.clone()),
        Registered::Failed(cause) => {
            applied.notes.registration_by_hand = true;
            ArtifactOutcome::Failed(cause.clone())
        }
        Registered::Withheld(cause) => ArtifactOutcome::Failed(cause.clone()),
        Registered::NotReached(cause) => ArtifactOutcome::Failed(match &applied.failure {
            Some(failure) => format!("{cause}: {failure}"),
            None => cause.clone(),
        }),
    };
    if let Some((_, state)) = applied
        .outcomes
        .iter_mut()
        .find(|(placed, _)| *placed == Artifact::Registration)
    {
        *state = registration;
    }
    applied
}

/// Completion requires every artifact's current bytes, no gap and a recorded
/// catalog seed.
pub(crate) fn complete(
    placements: &PlacementMap,
    outcomes: &[(Artifact, ArtifactOutcome)],
    notes: &Notes,
    seed: &Result<bool, StoreError>,
) -> bool {
    placements.not_installed().is_empty()
        && notes.gaps.is_empty()
        && seed.is_ok()
        && placements.expected_files().iter().all(|file| {
            outcomes
                .iter()
                .any(|(artifact, outcome)| *artifact == file.artifact && outcome.is_owned())
        })
}

/// Builds ownership facts from applied outcomes without reading any path.
/// Failed or unreached writes keep that identity's previous entry, so the
/// next install can still recognize an older stub, hook, deny entry or
/// registration while the file still holds it. A retired stub whose removal
/// failed keeps its entry too.
pub(crate) fn facts(
    version: &str,
    placements: &PlacementMap,
    latest: Option<&Value>,
    settings: &settings::Decision,
    applied: &Applied,
    seed: &Result<bool, StoreError>,
    updates: event::Updates,
) -> event::Facts {
    let stubs = placements
        .expected_files()
        .into_iter()
        .filter_map(|file| {
            let entry = file.stub?;
            let owned = applied
                .outcomes
                .iter()
                .any(|(artifact, outcome)| *artifact == file.artifact && outcome.is_owned());
            if owned {
                Some(event::Stub {
                    identity: entry.identity.clone(),
                    path: file.path.to_str().expect("placements are UTF-8").into(),
                    sha256: entry.digest.clone(),
                })
            } else {
                let previous = latest?.get("stubs")?.as_array()?.iter().find(|stub| {
                    stub.get("identity").and_then(Value::as_str) == Some(entry.identity.as_str())
                })?;
                Some(event::Stub {
                    identity: entry.identity.clone(),
                    path: previous.get("path")?.as_str()?.into(),
                    sha256: previous.get("sha256")?.as_str()?.into(),
                })
            }
        })
        // A retired stub still on disk, or recorded under another Claude
        // folder, stays recorded, so a later install can still prove it is
        // Baley's and remove it.
        .chain(
            applied
                .retired
                .iter()
                .filter(|(_, outcome)| *outcome != RetiredOutcome::Removed)
                .map(|(stub, _)| event::Stub {
                    identity: stub.identity.clone(),
                    path: stub.path.to_string_lossy().into_owned(),
                    sha256: stub.sha256.clone(),
                }),
        )
        .collect();
    let settings_owned = applied
        .outcomes
        .iter()
        .any(|(artifact, outcome)| *artifact == Artifact::Settings && outcome.is_owned());
    let (hook, sandbox) = settings::recorded(settings, settings_owned, latest);
    let registration = recorded_registration(placements, latest, applied);
    event::Facts {
        host: Host::ClaudeCode,
        binary_version: version.into(),
        binary_path: placements.executable().as_str().into(),
        complete: applied.failure.is_none()
            && applied.no_retired_left()
            && complete(placements, &applied.outcomes, &applied.notes, seed),
        registration,
        hook,
        stubs,
        sandbox,
        updates,
    }
}

/// The registration to record. With the entry Baley's after the run, it is
/// this binary's. Otherwise the file still holds what the latest read saw, so
/// it is this binary's entry when that read held it, or the latest record's
/// entry while that read holds it by the registration rule and no remove
/// launch succeeded since. Dropping it would make the next install refuse
/// Baley's own older registration as foreign.
fn recorded_registration(
    placements: &PlacementMap,
    latest: Option<&Value>,
    applied: &Applied,
) -> Option<event::Registration> {
    let path = placements
        .expected_files()
        .into_iter()
        .find(|file| file.artifact == Artifact::Registration)?
        .path
        .to_str()
        .expect("placements are UTF-8")
        .to_owned();
    let ours = registration::entry(placements.executable());
    let owned = applied
        .outcomes
        .iter()
        .any(|(artifact, outcome)| *artifact == Artifact::Registration && outcome.is_owned());
    if owned {
        return Some(event::Registration { path, entry: ours });
    }
    let seen = applied.registration_seen.as_ref()?;
    if same_server(seen, &ours) {
        return Some(event::Registration { path, entry: ours });
    }
    let older = latest?.pointer("/registered/registration")?;
    let entry = older.get("entry")?;
    (!applied.registration_removed && same_server(seen, entry)).then(|| event::Registration {
        path: older
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(&path)
            .to_owned(),
        entry: entry.clone(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::host_artifacts::installed::Installed;
    use crate::host_artifacts::stubs;
    use crate::install::fixtures::{
        complete_settings, folders, four_spaces, hook_item, installed, linux,
    };
    use crate::store::model::digest;

    const SETTINGS: &str = "/home/o/.claude/settings.json";

    fn updates() -> event::Updates {
        event::Updates {
            auto: Some(false),
            staged_version: None,
        }
    }

    /// The settings decision over a document the owner's file holds.
    fn decision(document: &Value, latest: Option<&Value>) -> settings::Decision {
        let installed = installed();
        let seen = Observation {
            state: FileState::Bytes(four_spaces(document)),
            symbolic_link: false,
        };
        let current = settings::current(Path::new(SETTINGS), &seen).unwrap();
        settings::judge(&Input {
            path: Path::new(SETTINGS),
            current: &current,
            latest,
            placements: &installed.placements,
            folders: &folders(),
            prerequisites: &linux(),
            wire: true,
        })
        .expect("composes")
    }

    #[test]
    fn an_unwritten_stub_recorded_as_baleys_is_caught() {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let installed = installed();
        let capture = event::Stub {
            identity: "bal-capture".into(),
            path: "/home/o/.claude/skills/bal-capture/SKILL.md".into(),
            sha256: manifest[0].digest.clone(),
        };
        let help = event::Stub {
            identity: "bal-help".into(),
            path: "/home/o/.claude/skills/bal-help/SKILL.md".into(),
            sha256: manifest[1].digest.clone(),
        };
        let old_help = event::Stub {
            sha256: "cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4".into(),
            ..help.clone()
        };
        let latest = serde_json::json!({"stubs": [old_help]});
        let failure = "not-writable: /home/o/.claude/skills/bal-help/SKILL.md: Permission denied";
        let settings = decision(&json!({}), None);
        for (latest, capture_outcome, help_outcome, expected) in [
            (
                None,
                ArtifactOutcome::Written,
                ArtifactOutcome::Failed(failure.into()),
                vec![capture.clone()],
            ),
            (
                None,
                ArtifactOutcome::Replaced,
                ArtifactOutcome::Unchanged,
                vec![capture.clone(), help],
            ),
            (
                Some(&latest),
                ArtifactOutcome::Replaced,
                ArtifactOutcome::Failed(failure.into()),
                vec![capture, old_help],
            ),
        ] {
            let applied = Applied {
                outcomes: vec![
                    (Artifact::Stub("bal-capture".into()), capture_outcome),
                    (Artifact::Stub("bal-help".into()), help_outcome),
                    (Artifact::Registration, ArtifactOutcome::NotWritten),
                    (Artifact::Hook, ArtifactOutcome::NotWritten),
                    (Artifact::Settings, ArtifactOutcome::NotWritten),
                ],
                failure: None,
                notes: Notes::default(),
                registration_seen: None,
                registration_removed: false,
                retired: Vec::new(),
            };
            let facts = facts(
                "0.2.0",
                &installed.placements,
                latest,
                &settings,
                &applied,
                &Ok(true),
                updates(),
            );
            assert_eq!(facts.stubs, expected);
            assert!(!facts.complete);
        }
    }

    #[test]
    fn stubs_written_around_an_ownership_conflict_is_caught() {
        let installed = installed();
        let capture = PathBuf::from("/home/o/.claude/skills/bal-capture/SKILL.md");
        let help = PathBuf::from("/home/o/.claude/skills/bal-help/SKILL.md");
        let seen = |state| Observation {
            state,
            symbolic_link: false,
        };
        let mut observations = BTreeMap::from([
            (PathBuf::from(SETTINGS), seen(FileState::Absent)),
            (
                PathBuf::from("/home/o/.claude.json"),
                seen(FileState::Absent),
            ),
            (capture.clone(), seen(FileState::Absent)),
            (help.clone(), seen(FileState::Bytes(b"mine".to_vec()))),
        ]);
        let help_line = "install-ownership-conflict: stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";
        let capture_line = "install-ownership-conflict: stub `bal-capture` at /home/o/.claude/skills/bal-capture/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";
        let judged = |observations: &BTreeMap<PathBuf, Observation>| {
            judge(&Gathered {
                placements: &installed.placements,
                claude_folder: &installed.claude_folder,
                folders: &folders(),
                latest: None,
                observations,
                prerequisites: &linux(),
                executable: None,
            })
        };

        let refusal = judged(&observations)
            .expect_err("a conflict must return no write plan, including the absent stub");
        assert_eq!(refusal.lines, [help_line]);
        assert_eq!(refusal.code, 2);
        assert!(refusal.error);

        observations.insert(capture, seen(FileState::Bytes(b"mine".to_vec())));
        let refusal = judged(&observations).expect_err("both conflicts must return no write plan");
        assert_eq!(refusal.lines, [capture_line, help_line]);
        assert_eq!(refusal.code, 2);
        assert!(refusal.error);
    }

    #[test]
    fn a_settings_entry_recorded_that_the_file_does_not_hold_or_dropped_while_it_does_is_caught() {
        let installed = installed();
        let outcomes = |stubs: ArtifactOutcome, settings: ArtifactOutcome| {
            vec![
                (Artifact::Stub("bal-capture".into()), stubs.clone()),
                (Artifact::Stub("bal-help".into()), stubs),
                (Artifact::Registration, ArtifactOutcome::NotWritten),
                (Artifact::Hook, settings.clone()),
                (Artifact::Settings, settings),
            ]
        };
        let refused = ArtifactOutcome::Failed(
            "not-writable: /home/o/.claude/settings.json: Permission denied".into(),
        );
        let build = |latest: Option<&Value>,
                     decision: &settings::Decision,
                     outcomes: Vec<(Artifact, ArtifactOutcome)>,
                     failure: Option<&str>| {
            let applied = Applied {
                outcomes,
                failure: failure.map(str::to_owned),
                notes: Notes::default(),
                registration_seen: None,
                registration_removed: false,
                retired: Vec::new(),
            };
            facts(
                "0.2.0",
                &installed.placements,
                latest,
                decision,
                &applied,
                &Ok(true),
                updates(),
            )
        };
        let failure = Some("not-writable: /home/o/.claude/settings.json: Permission denied");

        // The write failed and no record says Baley ever wrote anything.
        let owner = json!({"model": "opus"});
        let facts = build(
            None,
            &decision(&owner, None),
            outcomes(ArtifactOutcome::Written, refused.clone()),
            failure,
        );
        assert_eq!(facts.hook, None);
        assert_eq!(facts.sandbox, None);
        assert!(!facts.complete);
        assert_eq!(facts.stubs.len(), 2);

        // A completed installation left unchanged stays in the record.
        let complete = complete_settings();
        let facts = build(
            None,
            &decision(&complete, None),
            outcomes(ArtifactOutcome::Unchanged, ArtifactOutcome::Unchanged),
            None,
        );
        let hook = facts.hook.expect("the unchanged hook stays recorded");
        assert_eq!(hook.path, SETTINGS);
        assert_eq!(hook.item, hook_item());
        let sandbox = facts.sandbox.expect("the unchanged sandbox stays recorded");
        assert_eq!(sandbox.settings_path, SETTINGS);
        assert_eq!(sandbox.sha256, digest(&four_spaces(&complete)));
        assert_eq!(
            sandbox.deny_read,
            [
                "/home/o/.local/share/crenshawdev/baley",
                "/home/o/.config/crenshawdev/baley"
            ]
        );
        assert_eq!(sandbox.deny_write.len(), 8);
        assert_eq!(sandbox.permissions_deny.len(), 10);

        // The write failed over Baley's own older entries.
        let old_hook = json!({
            "matcher": "Bash|Write|Edit",
            "hooks": [{"type": "command", "command": "'/home/o/.local/bin/baley' guard", "timeout": 5}],
        });
        let record = json!({
            "registered": {"hook": {"path": SETTINGS, "item": old_hook}},
            "sandbox": {
                "permissions_deny": ["Edit(//home/o/old/**)", "Edit(//home/o/gone/**)"],
                "deny_read": [],
                "deny_write": ["/home/o/old"],
            },
        });
        let older = json!({
            "hooks": {"PreToolUse": [old_hook]},
            "permissions": {"deny": ["Edit(//home/o/old/**)"]},
            "sandbox": {"filesystem": {"denyWrite": ["/home/o/old"]}},
        });
        let facts = build(
            Some(&record),
            &decision(&older, Some(&record)),
            outcomes(ArtifactOutcome::Unchanged, refused),
            failure,
        );
        assert_eq!(facts.hook.expect("the older hook is kept").item, old_hook);
        let sandbox = facts.sandbox.expect("the older entries are kept");
        assert_eq!(sandbox.settings_path, SETTINGS);
        assert_eq!(sandbox.sha256, digest(&four_spaces(&older)));
        assert_eq!(sandbox.permissions_deny, ["Edit(//home/o/old/**)"]);
        assert_eq!(sandbox.deny_write, ["/home/o/old"]);
        assert!(sandbox.deny_read.is_empty());
        assert!(!facts.complete);
    }
    /// Every judged path absent, as on a machine install never ran on.
    fn all_absent(installed: &Installed) -> BTreeMap<PathBuf, Observation> {
        let absent = Observation {
            state: FileState::Absent,
            symbolic_link: false,
        };
        let stubs = installed
            .placements
            .expected_files()
            .into_iter()
            .filter(|file| file.stub.is_some())
            .map(|file| file.path.to_path_buf());
        stubs
            .chain([
                installed.settings_file.clone(),
                installed.registration_file.clone(),
            ])
            .map(|path| (path, absent.clone()))
            .collect()
    }

    #[test]
    fn an_artifact_under_baleys_name_replaced_without_record_evidence_is_caught() {
        let installed = installed();
        for (path, bytes, artifact) in [
            (
                "/home/o/.claude/skills/bal-help/SKILL.md",
                &b"my own notes"[..],
                "stub `bal-help`",
            ),
            (
                "/home/o/.claude.json",
                &br#"{"mcpServers":{"baley":{"command":"/opt/other/baley","args":["serve"]}}}"#[..],
                "mcpServers.baley",
            ),
            (
                SETTINGS,
                &br#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/opt/other/baley guard"}]}]}}"#[..],
                "guard hook",
            ),
        ] {
            let mut observations = all_absent(&installed);
            observations.insert(
                PathBuf::from(path),
                Observation {
                    state: FileState::Bytes(bytes.to_vec()),
                    symbolic_link: false,
                },
            );

            let refusal = judge(&Gathered {
                placements: &installed.placements,
                claude_folder: &installed.claude_folder,
                folders: &folders(),
                latest: None,
                observations: &observations,
                prerequisites: &linux(),
                executable: None,
            })
            .expect_err("content under Baley's name without evidence is refused");

            assert_eq!(refusal.lines.len(), 1, "{:?}", refusal.lines);
            let line = &refusal.lines[0];
            assert!(line.starts_with("install-ownership-conflict: "), "{line}");
            assert!(line.contains(artifact), "{line}");
            assert!(line.contains(path), "{line}");
            assert_eq!(refusal.code, 2);
            assert!(refusal.error);
        }
    }
    fn gathered_for<'a>(
        installed: &'a Installed,
        latest: Option<&'a Value>,
        observations: &'a BTreeMap<PathBuf, Observation>,
        prerequisites: &'a Judged,
        folders: &'a Folders,
    ) -> Gathered<'a> {
        Gathered {
            placements: &installed.placements,
            claude_folder: &installed.claude_folder,
            folders,
            latest,
            observations,
            prerequisites,
            executable: None,
        }
    }

    #[test]
    fn an_executable_reference_to_anything_but_the_stable_path_is_caught() {
        let installed = installed();
        let observations = all_absent(&installed);
        let (folders, prerequisites) = (folders(), linux());

        let plan = judge(&gathered_for(
            &installed,
            None,
            &observations,
            &prerequisites,
            &folders,
        ))
        .expect("an empty machine plans");

        let document = &plan.settings.document;
        assert_eq!(
            document["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "'/home/o/.local/bin/baley' guard"
        );
        let Step::Run(launches) = &plan.registration else {
            panic!("the registration is requested");
        };
        assert_eq!(launches.len(), 1);
        let arguments: Vec<&str> = launches[0]
            .args
            .iter()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        let entry: Value = serde_json::from_str(arguments[5]).unwrap();
        assert_eq!(entry["command"], "/home/o/.local/bin/baley");
        assert!(
            document["permissions"]["deny"]
                .as_array()
                .unwrap()
                .contains(&json!("Edit(//home/o/.local/bin/baley)"))
        );
        assert!(
            document["sandbox"]["filesystem"]["denyWrite"]
                .as_array()
                .unwrap()
                .contains(&json!("/home/o/.local/bin/baley"))
        );
        let mut named = Vec::new();
        collect_versions(document, &mut named);
        named.extend(
            arguments
                .iter()
                .filter(|argument| argument.contains("/versions"))
                .map(|argument| (*argument).to_owned()),
        );
        named.sort();
        assert_eq!(
            named,
            [
                "/home/o/.local/lib/crenshawdev/baley/versions",
                "Edit(//home/o/.local/lib/crenshawdev/baley/versions/**)",
            ]
        );
    }

    /// Every string in the document that names the versions folder.
    fn collect_versions(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) if text.contains("/crenshawdev/baley/versions") => {
                found.push(text.clone());
            }
            Value::Array(items) => items.iter().for_each(|item| collect_versions(item, found)),
            Value::Object(map) => map.values().for_each(|item| collect_versions(item, found)),
            _ => {}
        }
    }

    #[test]
    fn a_completed_installation_rewritten_by_a_second_install_is_caught() {
        let installed = installed();
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let mut observations = all_absent(&installed);
        let seen = |bytes: Vec<u8>| Observation {
            state: FileState::Bytes(bytes),
            symbolic_link: false,
        };
        observations.insert(
            PathBuf::from(SETTINGS),
            seen(four_spaces(&complete_settings())),
        );
        for entry in &manifest {
            observations.insert(
                PathBuf::from(format!(
                    "/home/o/.claude/skills/{}/SKILL.md",
                    entry.identity
                )),
                seen(entry.bytes.clone()),
            );
        }
        let stored = json!({"mcpServers": {"baley": {
            "type": "stdio",
            "command": "/home/o/.local/bin/baley",
            "args": ["serve"],
            "env": {},
        }}});
        observations.insert(
            PathBuf::from("/home/o/.claude.json"),
            seen(serde_json::to_vec(&stored).unwrap()),
        );
        let record = json!({
            "registered": {
                "registration": {
                    "path": "/home/o/.claude.json",
                    "entry": {"command": "/home/o/.local/bin/baley", "args": ["serve"]},
                },
                "hook": {"path": SETTINGS, "item": hook_item()},
            },
            "stubs": manifest.iter().map(|entry| json!({
                "identity": entry.identity,
                "path": format!("/home/o/.claude/skills/{}/SKILL.md", entry.identity),
                "sha256": entry.digest,
            })).collect::<Vec<_>>(),
        });
        let (folders, prerequisites) = (folders(), linux());

        let plan = judge(&gathered_for(
            &installed,
            Some(&record),
            &observations,
            &prerequisites,
            &folders,
        ))
        .expect("a completed installation plans");

        assert!(plan.writes.is_empty(), "{:?}", plan.writes);
        assert_eq!(plan.registration, Step::Unchanged);
        assert_eq!(plan.outcomes.len(), 5);
        for (artifact, outcome) in &plan.outcomes {
            assert_eq!(*outcome, ArtifactOutcome::Unchanged, "{artifact}");
        }
    }

    #[test]
    fn a_failed_seed_recorded_complete_or_the_registration_misrecorded_after_a_failed_launch_is_caught()
     {
        let installed = installed();
        let settings = decision(&complete_settings(), None);
        let outcomes = |registration: ArtifactOutcome| {
            vec![
                (
                    Artifact::Stub("bal-capture".into()),
                    ArtifactOutcome::Written,
                ),
                (Artifact::Stub("bal-help".into()), ArtifactOutcome::Written),
                (Artifact::Registration, registration),
                (Artifact::Hook, ArtifactOutcome::Written),
                (Artifact::Settings, ArtifactOutcome::Written),
            ]
        };
        let build = |latest: Option<&Value>, applied: Applied, seed: &Result<bool, StoreError>| {
            facts(
                "0.2.0",
                &installed.placements,
                latest,
                &settings,
                &applied,
                seed,
                updates(),
            )
        };
        let applied = |outcomes, seen: Option<Value>, removed| Applied {
            outcomes,
            failure: None,
            notes: Notes::default(),
            registration_seen: seen,
            registration_removed: removed,
            retired: Vec::new(),
        };

        // Everything is Baley's: complete follows the catalog seed.
        let done = || applied(outcomes(ArtifactOutcome::Written), None, false);
        let busy: Result<bool, StoreError> = Err(StoreError::Busy);
        assert!(!build(None, done(), &busy).complete);
        assert!(build(None, done(), &Ok(true)).complete);

        // A failed remove leaves Baley's older entry in the file.
        let older = json!({"command": "/home/o/.local/bin/baley", "args": ["serve", "--old"]});
        let record = json!({"registered": {"registration": {
            "path": "/home/o/.claude.json",
            "entry": older,
        }}});
        let stored = json!({
            "type": "stdio",
            "command": "/home/o/.local/bin/baley",
            "args": ["serve", "--old"],
            "env": {},
        });
        let failed =
            ArtifactOutcome::Failed("not-writable: /home/o/.claude.json: exit status 1".into());
        let facts = build(
            Some(&record),
            applied(outcomes(failed.clone()), Some(stored.clone()), false),
            &Ok(true),
        );
        let kept = facts
            .registration
            .expect("the older entry is still in the file");
        assert_eq!(kept.path, "/home/o/.claude.json");
        assert_eq!(kept.entry, older);
        assert!(!facts.complete);

        // A failed add after a removed entry leaves nothing to record.
        let facts = build(
            Some(&record),
            applied(outcomes(failed), Some(stored), true),
            &Ok(true),
        );
        assert_eq!(facts.registration, None);
        assert!(!facts.complete);
    }
    #[test]
    fn a_hook_or_registration_wired_to_an_empty_stable_path_is_caught() {
        let installed = installed();
        let observations = all_absent(&installed);
        let (folders, prerequisites) = (folders(), linux());
        let plan_for = |gap: Option<&ExecutableGap>| {
            judge(&Gathered {
                placements: &installed.placements,
                claude_folder: &installed.claude_folder,
                folders: &folders,
                latest: None,
                observations: &observations,
                prerequisites: &prerequisites,
                executable: gap,
            })
            .expect("an empty machine plans")
        };
        let stable = "/home/o/.local/bin/baley";

        for (gap, text) in [
            (ExecutableGap::Missing, "does not exist"),
            (ExecutableGap::DanglingLink, "is a link to a missing file"),
        ] {
            let plan = plan_for(Some(&gap));

            assert!(matches!(plan.registration, Step::Run(_)));
            let document = &plan.settings.document;
            assert!(document.get("hooks").is_none(), "{document}");
            assert_eq!(document["sandbox"]["enabled"], true);
            assert_eq!(
                document["permissions"]["deny"].as_array().unwrap().len(),
                10
            );
            let stubs = plan
                .writes
                .iter()
                .filter(|write| matches!(write.artifacts[0], Artifact::Stub(_)))
                .count();
            assert_eq!(stubs, 2);
            assert_eq!(plan.writes[0].artifacts, [Artifact::Settings]);
            let wiring = plan.wiring.as_ref().expect("the wiring is held back");
            assert!(wiring.gap.contains(stable), "{}", wiring.gap);
            assert!(wiring.gap.contains(text), "{}", wiring.gap);
            assert!(wiring.gap.contains("install.sh"), "{}", wiring.gap);
            assert_eq!(plan.settings.entries.hook, None);
            for artifact in [Artifact::Hook, Artifact::Registration] {
                let outcome = plan
                    .outcomes
                    .iter()
                    .find(|(placed, _)| *placed == artifact)
                    .map(|(_, outcome)| outcome)
                    .unwrap();
                let ArtifactOutcome::Failed(cause) = outcome else {
                    panic!("{artifact} must be held back, not {outcome:?}");
                };
                assert!(cause.contains(stable) && cause.contains(text), "{cause}");
            }
        }

        let plan = plan_for(None);
        let Step::Run(launches) = &plan.registration else {
            panic!("the registration is requested");
        };
        assert_eq!(launches.len(), 1);
        assert_eq!(
            plan.settings.document["hooks"]["PreToolUse"],
            json!([hook_item()])
        );
        assert!(plan.wiring.is_none());
        assert!(
            plan.settings.gaps.iter().all(|gap| !gap.contains(stable)),
            "{:?}",
            plan.settings.gaps
        );
    }

    #[test]
    fn owner_provider_hosts_dropped_or_an_acknowledgement_recorded_by_install_is_caught() {
        let installed = installed();
        let mut observations = all_absent(&installed);
        observations.insert(
            PathBuf::from(SETTINGS),
            Observation {
                state: FileState::Bytes(
                    br#"{"model":"opus","sandbox":{"network":{"allowedDomains":["api.openai.com","example.com"]}}}"#
                        .to_vec(),
                ),
                symbolic_link: false,
            },
        );
        let (folders, prerequisites) = (folders(), linux());

        let plan = judge(&gathered_for(
            &installed,
            None,
            &observations,
            &prerequisites,
            &folders,
        ))
        .expect("an owner's settings file with provider hosts plans");

        let write = plan
            .writes
            .iter()
            .find(|write| write.path == Path::new(SETTINGS))
            .expect("the settings file is written");
        let written: Value = serde_json::from_slice(&write.bytes).unwrap();
        assert_eq!(written["model"], "opus");
        assert_eq!(
            written["sandbox"]["network"],
            json!({"allowedDomains": ["api.openai.com", "example.com"]})
        );

        // The record install builds for that plan holds no provider or
        // acknowledgement.
        let attempt = Attempt {
            seen: None,
            removed: false,
            result: Registered::Registered,
        };
        let results: Vec<Result<(), Failure>> = plan.writes.iter().map(|_| Ok(())).collect();
        let applied = applied(&plan, &results, &[], &attempt);
        let facts = facts(
            "0.2.0",
            &installed.placements,
            None,
            &plan.settings,
            &applied,
            &Ok(true),
            updates(),
        );
        let recorded = event::recorded_event(&facts);
        assert_eq!(recorded.stream.0, "install");
        assert_eq!(recorded.type_name, "install.recorded");
        assert_eq!(recorded.payload["defaults"], json!({}));
    }

    const OLD: &str = "/home/o/.claude/skills/bal-old/SKILL.md";
    const CAPTURE: &str = "/home/o/.claude/skills/bal-capture/SKILL.md";
    const HELP: &str = "/home/o/.claude/skills/bal-help/SKILL.md";

    /// A file at a path, read as `bytes` or absent, and whether the path is a
    /// link.
    fn file(bytes: Option<&[u8]>, symbolic_link: bool) -> Observation {
        Observation {
            state: bytes.map_or(FileState::Absent, |bytes| FileState::Bytes(bytes.to_vec())),
            symbolic_link,
        }
    }

    /// The plan for a machine with nothing placed, whose latest record lists
    /// `stubs`, with `extra` observed beside the placed paths.
    fn plan_with(stubs: Value, extra: &[(&str, Observation)]) -> Plan {
        let installed = installed();
        let mut observations = BTreeMap::from([
            (PathBuf::from(SETTINGS), file(None, false)),
            (PathBuf::from("/home/o/.claude.json"), file(None, false)),
            (PathBuf::from(CAPTURE), file(None, false)),
            (PathBuf::from(HELP), file(None, false)),
        ]);
        for (path, seen) in extra {
            observations.insert(PathBuf::from(path), seen.clone());
        }
        let latest = json!({ "stubs": stubs });
        judge(&Gathered {
            placements: &installed.placements,
            claude_folder: &installed.claude_folder,
            folders: &folders(),
            latest: Some(&latest),
            observations: &observations,
            prerequisites: &linux(),
            executable: None,
        })
        .expect("plans")
    }

    /// `bal-old`, a stub this binary does not ship, recorded at `OLD` with
    /// the hash of `old stub`.
    fn old_stub() -> Retired {
        Retired {
            identity: "bal-old".into(),
            path: OLD.into(),
            sha256: digest(b"old stub"),
        }
    }

    fn recorded(stub: &Retired) -> Value {
        json!({"identity": stub.identity, "path": stub.path, "sha256": stub.sha256})
    }

    #[test]
    fn a_retired_stub_baley_placed_not_planned_for_removal_is_caught() {
        let old = old_stub();
        let plan = plan_with(
            json!([recorded(&old)]),
            &[(OLD, file(Some(b"old stub"), false))],
        );
        assert_eq!(plan.removals, [old]);
        assert!(plan.unreadable.is_empty(), "{:?}", plan.unreadable);
    }

    #[test]
    fn an_owner_file_at_a_retired_stub_path_planned_for_removal_is_caught() {
        let old = old_stub();
        for (case, seen) in [
            ("other bytes", file(Some(b"mine"), false)),
            ("a link", file(Some(b"old stub"), true)),
            ("absent", file(None, false)),
        ] {
            let plan = plan_with(json!([recorded(&old)]), &[(OLD, seen)]);
            assert!(plan.removals.is_empty(), "{case}: {:?}", plan.removals);
            assert!(plan.unreadable.is_empty(), "{case}: {:?}", plan.unreadable);
        }
    }

    #[test]
    fn a_retired_stub_recorded_outside_its_own_stub_path_planned_for_removal_is_caught() {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let capture = &manifest[0];
        assert_eq!(capture.identity, "bal-capture");
        let old: &[u8] = b"old stub";
        for (case, identity, path, bytes) in [
            ("another file", "bal-old", "/home/o/notes.md", old),
            (
                "an alias through ..",
                "bal-old",
                "/home/o/.claude/skills/bal-help/../bal-old/SKILL.md",
                old,
            ),
            (
                "an identity naming a parent",
                "../bal-old",
                "/home/o/.claude/skills/../bal-old/SKILL.md",
                old,
            ),
            (
                "an identity with a trailing slash",
                "bal-old/",
                "/home/o/.claude/skills/bal-old/SKILL.md",
                old,
            ),
            (
                "a shipped identity with a trailing slash",
                "bal-capture/",
                CAPTURE,
                &capture.bytes,
            ),
            (
                "another Claude folder",
                "bal-old",
                "/home/o/other/skills/bal-old/SKILL.md",
                old,
            ),
        ] {
            let stub = Retired {
                identity: identity.into(),
                path: path.into(),
                sha256: digest(bytes),
            };
            let plan = plan_with(
                json!([recorded(&stub)]),
                &[(path, file(Some(bytes), false))],
            );
            assert!(plan.removals.is_empty(), "{case}: {:?}", plan.removals);
            assert!(plan.unreadable.is_empty(), "{case}: {:?}", plan.unreadable);
            // Never touched, but kept, so a later install can still act on it.
            assert_eq!(plan.carried, std::slice::from_ref(&stub), "{case}");
            let written: Vec<Result<(), Failure>> = plan.writes.iter().map(|_| Ok(())).collect();
            let attempt = Attempt {
                seen: None,
                removed: false,
                result: Registered::Registered,
            };
            assert_eq!(
                applied(&plan, &written, &[], &attempt).retired,
                [(stub, RetiredOutcome::Carried)],
                "{case}"
            );
        }
    }

    #[test]
    fn an_unreadable_retired_stub_dropped_without_a_word_is_caught() {
        let old = old_stub();
        let cause = "the file cannot be read: Permission denied (os error 13)";
        let plan = plan_with(
            json!([recorded(&old)]),
            &[(
                OLD,
                Observation {
                    state: FileState::Fault(Fault::Unreadable(
                        "Permission denied (os error 13)".into(),
                    )),
                    symbolic_link: false,
                },
            )],
        );
        assert!(plan.removals.is_empty(), "{:?}", plan.removals);
        assert_eq!(plan.unreadable, [(old.clone(), cause.to_owned())]);

        let attempt = Attempt {
            seen: None,
            removed: false,
            result: Registered::Registered,
        };
        let written: Vec<Result<(), Failure>> = plan.writes.iter().map(|_| Ok(())).collect();
        assert_eq!(
            applied(&plan, &written, &[], &attempt).retired,
            [(old, RetiredOutcome::Failed(cause.into()))]
        );
    }

    #[test]
    fn a_shipped_stub_planned_for_removal_is_caught() {
        let older_help = Retired {
            identity: "bal-help".into(),
            path: HELP.into(),
            sha256: digest(b"older help"),
        };
        // A retired entry naming a placed path whose bytes it would match.
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let capture = &manifest[0];
        assert_eq!(capture.identity, "bal-capture");
        let at_capture = Retired {
            path: CAPTURE.into(),
            sha256: capture.digest.clone(),
            ..old_stub()
        };
        let plan = plan_with(
            json!([recorded(&older_help), recorded(&at_capture)]),
            &[
                (HELP, file(Some(b"older help"), false)),
                (CAPTURE, file(Some(&capture.bytes), false)),
            ],
        );
        assert!(plan.removals.is_empty(), "{:?}", plan.removals);
        let replaced: Vec<_> = plan.writes.iter().map(|write| &write.path).collect();
        assert!(replaced.contains(&&PathBuf::from(HELP)), "{replaced:?}");
    }

    #[test]
    fn a_retired_stub_whose_removal_failed_dropped_from_the_record_is_caught() {
        let installed = installed();
        let settings = decision(&complete_settings(), None);
        let old = old_stub();
        let build = |outcome: RetiredOutcome| {
            let applied = Applied {
                outcomes: vec![
                    (
                        Artifact::Stub("bal-capture".into()),
                        ArtifactOutcome::Unchanged,
                    ),
                    (
                        Artifact::Stub("bal-help".into()),
                        ArtifactOutcome::Unchanged,
                    ),
                    (Artifact::Registration, ArtifactOutcome::Unchanged),
                    (Artifact::Hook, ArtifactOutcome::Unchanged),
                    (Artifact::Settings, ArtifactOutcome::Unchanged),
                ],
                failure: None,
                notes: Notes::default(),
                registration_seen: None,
                registration_removed: false,
                retired: vec![(old.clone(), outcome)],
            };
            facts(
                "0.2.0",
                &installed.placements,
                None,
                &settings,
                &applied,
                &Ok(true),
                updates(),
            )
        };
        let identities = |facts: &event::Facts| -> Vec<String> {
            facts
                .stubs
                .iter()
                .map(|stub| stub.identity.clone())
                .collect()
        };

        let removed = build(RetiredOutcome::Removed);
        assert_eq!(identities(&removed), ["bal-capture", "bal-help"]);
        assert!(removed.complete);

        let left = build(RetiredOutcome::Failed("not-removable: denied".into()));
        assert_eq!(identities(&left), ["bal-capture", "bal-help", "bal-old"]);
        assert_eq!(
            left.stubs[2],
            event::Stub {
                identity: "bal-old".into(),
                path: OLD.into(),
                sha256: digest(b"old stub"),
            }
        );
        assert!(!left.complete);

        // Recorded under another Claude folder: kept, and nothing failed.
        let carried = build(RetiredOutcome::Carried);
        assert_eq!(identities(&carried), ["bal-capture", "bal-help", "bal-old"]);
        assert!(carried.complete);
    }

    #[test]
    fn a_removal_result_misread_is_caught() {
        let old = old_stub();
        let plan = plan_with(
            json!([recorded(&old)]),
            &[(OLD, file(Some(b"old stub"), false))],
        );
        let attempt = Attempt {
            seen: None,
            removed: false,
            result: Registered::Registered,
        };
        let written: Vec<Result<(), Failure>> = plan.writes.iter().map(|_| Ok(())).collect();
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let outcome = |writes: &[Result<(), Failure>], removals: &[Result<(), Failure>]| {
            applied(&plan, writes, removals, &attempt).retired
        };

        assert_eq!(applied(&plan, &written, &[Ok(())], &attempt).failure, None);
        // The file is gone, yet the run is partial, like an unsynced write.
        let unsynced = [Err(Failure::Unsynced {
            path: OLD.into(),
            cause: denied(),
        })];
        let synced_not = applied(&plan, &written, &unsynced, &attempt);
        assert_eq!(synced_not.retired, [(old.clone(), RetiredOutcome::Removed)]);
        assert_eq!(
            synced_not.failure.as_deref(),
            Some(
                format!(
                    "{OLD} was removed, but its folder could not be synced, so the removal may not survive a crash: {}",
                    denied()
                )
                .as_str()
            )
        );
        let unchanged = [Err(Failure::Unchanged {
            path: OLD.into(),
            cause: denied(),
        })];
        assert_eq!(
            outcome(&written, &unchanged),
            [(
                old.clone(),
                RetiredOutcome::Failed(format!("not-removable: {OLD}: {}", denied()))
            )]
        );
        let failed_write = [Err(Failure::Unchanged {
            path: SETTINGS.into(),
            cause: denied(),
        })];
        let unreached = outcome(&failed_write, &[]);
        let [(_, RetiredOutcome::Failed(cause))] = unreached.as_slice() else {
            panic!("an unreached removal must be a failure: {unreached:?}");
        };
        assert!(
            cause.starts_with(
                "an earlier write failed: not-writable: /home/o/.claude/settings.json"
            ),
            "{cause}"
        );
    }
}
