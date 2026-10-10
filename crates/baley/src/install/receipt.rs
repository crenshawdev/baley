//! The install receipt from supplied placements, artifact outcomes, catalog
//! seed and record results. Write and recording failures remain visible.

use std::path::Path;

use baley_core::catalog::{HINT_VERSION, USER_PROJECT};
use baley_store::StoreError;

use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::ledger::display::{self, Render};

/// What the run left at an artifact's placed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactOutcome {
    /// The artifact has not been placed or confirmed as Baley's.
    NotWritten,
    /// The manifest bytes were placed at an absent path.
    Written,
    /// An older owned artifact was replaced with the manifest bytes.
    Replaced,
    /// The artifact is already Baley's and needed no write.
    Unchanged,
    /// The write failed or was not reached, with the cause.
    Failed(String),
}

impl ArtifactOutcome {
    /// Whether this run placed or confirmed the current artifact's bytes.
    pub fn is_owned(&self) -> bool {
        matches!(self, Self::Written | Self::Replaced | Self::Unchanged)
    }
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
    let complete = super::plan::complete(placements, outcomes, seed);
    let mut artifacts = Vec::new();
    for file in placements.expected_files() {
        let outcome = outcomes
            .iter()
            .find(|(artifact, _)| *artifact == file.artifact)
            .map_or(&ArtifactOutcome::NotWritten, |(_, outcome)| outcome);
        let text = match outcome {
            ArtifactOutcome::NotWritten => "not written".into(),
            ArtifactOutcome::Written => "written".into(),
            ArtifactOutcome::Replaced => "replaced".into(),
            ArtifactOutcome::Unchanged => "unchanged".into(),
            ArtifactOutcome::Failed(cause) => format!("not written: {cause}"),
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

/// Adds write and recording failures to the artifact receipt. Either failure
/// leaves a partial outcome, including a rename whose folder was not synced.
pub(crate) fn render_applied(
    version: &str,
    stable_path: &Path,
    placements: &PlacementMap,
    applied: &super::plan::Applied,
    seed: &Result<bool, StoreError>,
    recorded: &Result<bool, StoreError>,
) -> Render {
    let mut receipt = render(version, stable_path, placements, &applied.outcomes, seed);
    let mut failures = Vec::new();
    if let Some(failure) = &applied.failure {
        failures.push(failure.clone());
    }
    if let Err(error) = recorded {
        failures.push(format!(
            "the install record was not written: {}",
            display::store_error(error, Some(USER_PROJECT))
                .lines
                .join("; ")
        ));
    }
    if !failures.is_empty() {
        receipt.lines[0] = "install outcome: partial".into();
        receipt.code = 1;
        let after_seed = 3 + placements.expected_files().len();
        receipt.lines.splice(after_seed..after_seed, failures);
    }
    receipt
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
            (Artifact::Stub("bal-capture".into()), outcome.clone()),
            (Artifact::Stub("bal-help".into()), outcome.clone()),
            (Artifact::Registration, outcome.clone()),
            (Artifact::Hook, outcome.clone()),
            (Artifact::Settings, outcome),
        ]
    }

    #[test]
    fn a_replaced_or_unchanged_stub_reported_as_written_is_caught() {
        let installed = installed();
        let mut applied = super::super::plan::Applied {
            outcomes: vec![
                (
                    Artifact::Stub("bal-capture".into()),
                    ArtifactOutcome::Replaced,
                ),
                (
                    Artifact::Stub("bal-help".into()),
                    ArtifactOutcome::Unchanged,
                ),
            ],
            failure: None,
        };
        let receipt = render_applied(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &applied,
            &Ok(true),
            &Ok(true),
        );
        assert_eq!(
            receipt.lines[2],
            "stub `bal-capture` at /home/o/.claude/skills/bal-capture/SKILL.md: replaced"
        );
        assert_eq!(
            receipt.lines[3],
            "stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md: unchanged"
        );

        applied.outcomes[1].1 = ArtifactOutcome::Failed("Permission denied".into());
        applied.failure = Some(
            "not-writable: /home/o/.claude/skills/bal-help/SKILL.md: Permission denied".into(),
        );
        let receipt = render_applied(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &applied,
            &Ok(true),
            &Err(StoreError::Busy),
        );
        assert_eq!(
            receipt.lines[3],
            "stub `bal-help` at /home/o/.claude/skills/bal-help/SKILL.md: not written: Permission denied"
        );
        assert!(receipt.lines.contains(applied.failure.as_ref().unwrap()));
        assert!(receipt.lines.iter().any(|line| line
            == "the install record was not written: the ledger is busy; run the command again"));
        assert_eq!(receipt.lines[0], "install outcome: partial");
        assert_eq!(receipt.code, 1);
        assert!(!receipt.error);
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
