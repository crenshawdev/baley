//! Installation writes decided from supplied placements, file observations
//! and the latest ownership record. Any stub or settings conflict refuses the
//! whole plan. Applied outcomes retain older ownership evidence when a write
//! cannot finish.
//! The registration decision joins this judge in phase 15 plan 2.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use baley_core::policy::Host;
use baley_store::StoreError;
use serde_json::Value;

use crate::folders::Folders;
use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::host_doctor::placed::{Fault, FileState};
use crate::host_doctor::prerequisites::Judged;
use crate::ledger::display::Render;
use crate::replace::Failure;

use super::event;
use super::receipt::{ArtifactOutcome, Notes};
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
    /// Baley's data and configuration folders.
    pub folders: &'a Folders,
    /// The latest install record's payload, when there is one.
    pub latest: Option<&'a Value>,
    /// One observation per placed file the decision judges, keyed by path.
    pub observations: &'a BTreeMap<PathBuf, Observation>,
    /// The sandbox programs the platform needs, judged by the doctor's check.
    pub prerequisites: &'a Judged,
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
}

fn unobserved() -> Observation {
    Observation {
        state: FileState::Fault(Fault::Unreadable("no file observation was supplied".into())),
        symbolic_link: false,
    }
}

/// Plans the writes, or refuses every write with one line per conflict.
/// Observations are keyed by path so artifacts sharing a document use one read.
/// A missing observation refuses rather than inventing an absent file.
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
        }) {
            Err(lines) => {
                conflicts.extend(lines);
                None
            }
            Ok(decision) => Some(decision),
        },
    };
    let Some(decision) = decision.filter(|_| conflicts.is_empty()) else {
        let mut refusal = Render::refusal("");
        refusal.lines = conflicts;
        return Err(refusal);
    };

    let mut writes = Vec::new();
    match &decision.bytes {
        Some(bytes) => writes.push(Write {
            artifacts: vec![Artifact::Hook, Artifact::Settings],
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
    Ok(Plan {
        writes,
        outcomes,
        settings: decision,
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
}

/// Interprets write observations in plan order. A failed folder sync still
/// placed the bytes, but every failure stops subsequent writes.
pub(crate) fn applied(plan: &Plan, results: &[Result<(), Failure>]) -> Applied {
    let mut applied = Applied {
        outcomes: plan.outcomes.clone(),
        failure: None,
        notes: Notes::from_settings(&plan.settings),
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
/// next install can still recognize an older stub, hook or deny entry.
/// The registration joins in phase 15 plan 2 at this same seam.
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
        .collect();
    let settings_owned = applied
        .outcomes
        .iter()
        .any(|(artifact, outcome)| *artifact == Artifact::Settings && outcome.is_owned());
    let (hook, sandbox) = settings::recorded(settings, settings_owned, latest);
    event::Facts {
        host: Host::ClaudeCode,
        binary_version: version.into(),
        binary_path: placements.executable().as_str().into(),
        complete: applied.failure.is_none()
            && complete(placements, &applied.outcomes, &applied.notes, seed),
        registration: None,
        hook,
        stubs,
        sandbox,
        updates,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
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
            (capture.clone(), seen(FileState::Absent)),
            (help.clone(), seen(FileState::Bytes(b"mine".to_vec()))),
        ]);
        let help_line = "install-ownership-conflict: stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";
        let capture_line = "install-ownership-conflict: stub `bal-capture` at /home/o/.claude/skills/bal-capture/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";
        let judged = |observations: &BTreeMap<PathBuf, Observation>| {
            judge(&Gathered {
                placements: &installed.placements,
                folders: &folders(),
                latest: None,
                observations,
                prerequisites: &linux(),
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
}
