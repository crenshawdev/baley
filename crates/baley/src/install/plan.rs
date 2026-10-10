//! Installation writes decided from supplied placements, file observations
//! and the latest ownership record. Any stub conflict refuses the whole plan.
//! Applied outcomes retain older ownership evidence when a write cannot finish.
//! Registration and settings decisions join this judge in phase 15 plan 2.

use std::collections::BTreeMap;
use std::path::PathBuf;

use baley_core::policy::Host;
use baley_store::StoreError;
use serde_json::Value;

use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::host_doctor::placed::{Fault, FileState};
use crate::ledger::display::Render;
use crate::replace::Failure;

use super::event;
use super::receipt::ArtifactOutcome;
use super::stubs::{self, Decision};

/// One read of a placed file and the path's own link status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The bytes or fault found by the read, following links.
    pub state: FileState,
    /// Whether the placed path itself is a symbolic link.
    pub symbolic_link: bool,
}

/// A file write permitted by the ownership decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// The artifact whose bytes will be placed.
    pub artifact: Artifact,
    /// The absolute target path from the placement map.
    pub path: PathBuf,
    /// The manifest bytes to write.
    pub bytes: Vec<u8>,
    /// SHA-256 of the observed bytes, or none when the path was absent.
    pub read_digest: Option<String>,
}

/// Ordered writes and each artifact's state before those writes are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Writes in placement order, excluding unchanged artifacts.
    pub writes: Vec<Write>,
    /// Placed artifacts in map order. Pending writes remain `NotWritten` until
    /// the applier confirms them. Undecided JSON artifacts are `NotWritten` too.
    pub outcomes: Vec<(Artifact, ArtifactOutcome)>,
}

/// Plans stub writes, or refuses every write with one line per conflict.
/// Observations are keyed by path so artifacts sharing a document use one read.
/// A missing stub observation refuses rather than inventing an absent file.
pub(crate) fn judge(
    placements: &PlacementMap,
    latest: Option<&Value>,
    observations: &BTreeMap<PathBuf, Observation>,
) -> Result<Plan, Render> {
    let mut plan = Plan {
        writes: Vec::new(),
        outcomes: Vec::new(),
    };
    let mut conflicts = Vec::new();
    for file in placements.expected_files() {
        let mut outcome = ArtifactOutcome::NotWritten;
        if let Some(entry) = file.stub {
            let missing = Observation {
                state: FileState::Fault(Fault::Unreadable(
                    "no file observation was supplied".into(),
                )),
                symbolic_link: false,
            };
            let seen = observations.get(file.path).unwrap_or(&missing);
            match stubs::judge(entry, file.path, &seen.state, seen.symbolic_link, latest) {
                Ok(Decision::Unchanged) => outcome = ArtifactOutcome::Unchanged,
                Ok(decision) => {
                    let read_digest = match decision {
                        Decision::Replace { read_digest } => Some(read_digest),
                        _ => None,
                    };
                    plan.writes.push(Write {
                        artifact: file.artifact.clone(),
                        path: file.path.into(),
                        bytes: entry.bytes.clone(),
                        read_digest,
                    });
                }
                Err(conflict) => conflicts.push(conflict),
            }
        }
        plan.outcomes.push((file.artifact, outcome));
    }
    if conflicts.is_empty() {
        Ok(plan)
    } else {
        let mut refusal = Render::refusal("");
        refusal.lines = conflicts;
        Err(refusal)
    }
}

/// Each artifact's outcome after applying a plan, with the first write failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Placed artifacts in map order, including writes that were not reached.
    pub outcomes: Vec<(Artifact, ArtifactOutcome)>,
    /// The first write failure's text, including a failed sync after a rename.
    pub failure: Option<String>,
}

/// Interprets write observations in plan order. A failed folder sync still
/// placed the bytes, but every failure stops subsequent writes.
pub(crate) fn applied(plan: &Plan, results: &[Result<(), Failure>]) -> Applied {
    let mut applied = Applied {
        outcomes: plan.outcomes.clone(),
        failure: None,
    };
    for (index, write) in plan.writes.iter().enumerate() {
        let result = results.get(index);
        let written = matches!(result, Some(Ok(()) | Err(Failure::Unsynced { .. })));
        if let Some(Err(failure)) = result {
            applied.failure = Some(match failure {
                Failure::Refused(_) | Failure::Unsynced { .. } => failure.to_string(),
                Failure::Unchanged { path, cause } => {
                    format!("not-writable: {}: {cause}", path.display())
                }
            });
        }
        let outcome = if written {
            if write.read_digest.is_some() {
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
        if let Some((_, state)) = applied
            .outcomes
            .iter_mut()
            .find(|(artifact, _)| *artifact == write.artifact)
        {
            *state = outcome;
        }
    }
    applied
}

/// Completion requires every artifact's current bytes and a recorded catalog seed.
pub(crate) fn complete(
    placements: &PlacementMap,
    outcomes: &[(Artifact, ArtifactOutcome)],
    seed: &Result<bool, StoreError>,
) -> bool {
    placements.not_installed().is_empty()
        && seed.is_ok()
        && placements.expected_files().iter().all(|file| {
            outcomes
                .iter()
                .any(|(artifact, outcome)| *artifact == file.artifact && outcome.is_owned())
        })
}

/// Builds ownership facts from applied outcomes without reading any path.
/// Failed or unreached writes keep that identity's previous entry, so the
/// next install can still recognize an older stub. JSON artifacts join in
/// phase 15 plan 2 at this same seam.
pub(crate) fn facts(
    version: &str,
    placements: &PlacementMap,
    latest: Option<&Value>,
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
    event::Facts {
        host: Host::ClaudeCode,
        binary_version: version.into(),
        binary_path: placements.executable().as_str().into(),
        complete: applied.failure.is_none() && complete(placements, &applied.outcomes, seed),
        registration: None,
        hook: None,
        stubs,
        sandbox: None,
        updates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::host_artifacts::{installed, stubs};

    #[test]
    fn an_unwritten_stub_recorded_as_baleys_is_caught() {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let installed = installed::resolve(
            &Environment {
                home: Some("/home/o".into()),
                ..Environment::default()
            },
            None,
            &manifest,
        )
        .unwrap();
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
            };
            let facts = facts(
                "0.2.0",
                &installed.placements,
                latest,
                &applied,
                &Ok(true),
                event::Updates {
                    auto: Some(false),
                    staged_version: None,
                },
            );
            assert_eq!(facts.stubs, expected);
            assert!(!facts.complete);
        }
    }

    #[test]
    fn stubs_written_around_an_ownership_conflict_is_caught() {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let installed = installed::resolve(
            &Environment {
                home: Some("/home/o".into()),
                ..Environment::default()
            },
            None,
            &manifest,
        )
        .unwrap();
        let capture = PathBuf::from("/home/o/.claude/skills/bal-capture/SKILL.md");
        let help = PathBuf::from("/home/o/.claude/skills/bal-help/SKILL.md");
        let seen = |state| Observation {
            state,
            symbolic_link: false,
        };
        let mut observations = BTreeMap::from([
            (capture.clone(), seen(FileState::Absent)),
            (help.clone(), seen(FileState::Bytes(b"mine".to_vec()))),
        ]);
        let help_line = "install-ownership-conflict: stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";
        let capture_line = "install-ownership-conflict: stub `bal-capture` at /home/o/.claude/skills/bal-capture/SKILL.md holds bytes that match neither this binary's stub nor a recorded hash for this identity; move it aside and run `baley install` again";

        let refusal = judge(&installed.placements, None, &observations)
            .expect_err("a conflict must return no write plan, including the absent stub");
        assert_eq!(refusal.lines, [help_line]);
        assert_eq!(refusal.code, 2);
        assert!(refusal.error);

        observations.insert(capture, seen(FileState::Bytes(b"mine".to_vec())));
        let refusal = judge(&installed.placements, None, &observations)
            .expect_err("both conflicts must return no write plan");
        assert_eq!(refusal.lines, [capture_line, help_line]);
        assert_eq!(refusal.code, 2);
        assert!(refusal.error);
    }
}
