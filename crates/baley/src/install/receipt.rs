//! The install receipt from supplied placements, artifact outcomes and seed result.

use std::path::Path;

use baley_core::catalog::{HINT_VERSION, USER_PROJECT};
use baley_store::StoreError;

use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::ledger::display::{self, Render};

/// What the run left at an artifact's placed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactOutcome {
    /// The artifact has not been placed or confirmed as Baley's.
    NotWritten,
    /// The artifact is already Baley's and needed no write.
    Unchanged,
}

/// Renders every placed artifact in map order. A missing outcome counts as
/// not written. Completion requires every placement to be known, every
/// artifact to be Baley's and the catalog seed to be recorded.
pub(crate) fn render(
    version: &str,
    stable_path: &Path,
    placements: &PlacementMap,
    outcomes: &[(Artifact, ArtifactOutcome)],
    seed: &Result<bool, StoreError>,
) -> Render {
    let mut complete = placements.not_installed().is_empty() && seed.is_ok();
    let mut artifacts = Vec::new();
    for file in placements.expected_files() {
        let outcome = outcomes
            .iter()
            .find(|(artifact, _)| *artifact == file.artifact)
            .map_or(ArtifactOutcome::NotWritten, |(_, outcome)| *outcome);
        let text = match outcome {
            ArtifactOutcome::NotWritten => {
                complete = false;
                "not written"
            }
            ArtifactOutcome::Unchanged => "unchanged",
        };
        artifacts.push(format!(
            "{} at {}: {text}",
            file.artifact,
            file.path.display()
        ));
    }
    let mut lines = vec![
        format!(
            "install outcome: {}",
            if complete { "complete" } else { "partial" }
        ),
        format!("baley {version} at {}", stable_path.display()),
    ];
    lines.extend(artifacts);
    lines.push(match seed {
        Ok(true) => format!("seeded the model catalog with hint table version {HINT_VERSION}"),
        Ok(false) => format!(
            "the model catalog seed with hint table version {HINT_VERSION} is already recorded"
        ),
        Err(error) => format!(
            "the model catalog seed was not recorded: {}",
            display::store_error(error, Some(USER_PROJECT))
                .lines
                .join("; ")
        ),
    });
    lines.extend([
        "start a new Claude Code session to load the wiring".into(),
        "run baley init in each checkout Baley should manage".into(),
        "automatic update checks follow the updates.auto setting; baley config set --global updates.auto=true turns them on".into(),
        "baley update checks for a newer version by hand".into(),
        "baley config interview chooses settings later".into(),
        "baley doctor checks the installed result".into(),
    ]);
    Render {
        lines,
        code: u8::from(!complete),
        error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::host_artifacts::{installed, stubs};

    fn installed() -> installed::Installed {
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        installed::resolve(&env, None, &manifest).unwrap()
    }

    fn outcomes(outcome: ArtifactOutcome) -> Vec<(Artifact, ArtifactOutcome)> {
        vec![
            (Artifact::Stub("bal-capture".into()), outcome),
            (Artifact::Stub("bal-help".into()), outcome),
            (Artifact::Registration, outcome),
            (Artifact::Hook, outcome),
            (Artifact::Settings, outcome),
        ]
    }

    #[test]
    fn an_install_with_an_unwritten_artifact_reported_complete_is_caught() {
        let installed = installed();
        let receipt = render(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &outcomes(ArtifactOutcome::NotWritten),
            &Ok(true),
        );
        assert_eq!(receipt.lines[0], "install outcome: partial");
        assert!(
            !receipt
                .lines
                .iter()
                .any(|line| line == "install outcome: complete")
        );
        assert_eq!(
            receipt.lines[2..7],
            [
                "stub `bal-capture` at /home/o/.claude/skills/bal-capture/SKILL.md: not written",
                "stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md: not written",
                "registration at /home/o/.claude.json: not written",
                "hook at /home/o/.claude/settings.json: not written",
                "settings at /home/o/.claude/settings.json: not written",
            ]
        );
        assert_eq!(receipt.code, 1);
        assert!(!receipt.error);
    }

    #[test]
    fn a_receipt_missing_a_next_step_is_caught() {
        let installed = installed();
        let receipt = render(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &outcomes(ArtifactOutcome::NotWritten),
            &Ok(true),
        );
        assert_eq!(receipt.lines[1], "baley 0.2.0 at /home/o/.local/bin/baley");
        assert_eq!(
            receipt.lines[8..],
            [
                "start a new Claude Code session to load the wiring",
                "run baley init in each checkout Baley should manage",
                "automatic update checks follow the updates.auto setting; baley config set --global updates.auto=true turns them on",
                "baley update checks for a newer version by hand",
                "baley config interview chooses settings later",
                "baley doctor checks the installed result",
            ]
        );
    }

    #[test]
    fn a_catalog_seed_failure_hidden_by_the_receipt_is_caught() {
        let installed = installed();
        let outcomes = outcomes(ArtifactOutcome::Unchanged);
        let receipt = render(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &outcomes,
            &Err(StoreError::Busy),
        );
        assert_eq!(
            receipt.lines[7],
            "the model catalog seed was not recorded: the ledger is busy; run the command again"
        );
        assert_eq!(receipt.lines[0], "install outcome: partial");
        assert_eq!(receipt.code, 1);
        assert!(!receipt.error);

        for recorded in [true, false] {
            let receipt = render(
                "0.2.0",
                installed.layout.stable_path(),
                &installed.placements,
                &outcomes,
                &Ok(recorded),
            );
            assert_eq!(receipt.lines[0], "install outcome: complete");
            assert_eq!(receipt.code, 0);
            assert!(!receipt.error);
            let expected = if recorded {
                format!("seeded the model catalog with hint table version {HINT_VERSION}")
            } else {
                format!(
                    "the model catalog seed with hint table version {HINT_VERSION} is already recorded"
                )
            };
            assert_eq!(receipt.lines[7], expected);
        }
    }
}
