//! Installation writes decided from supplied placements, file observations
//! and the latest ownership record. Any stub conflict refuses the whole plan.
//! Registration and settings decisions join this judge in phase 15 plan 2.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::host_doctor::placed::{Fault, FileState};
use crate::ledger::display::Render;

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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "The install command consumes this in phase 15 plan 1 task 7."
    )
)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::host_artifacts::{installed, stubs};

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
