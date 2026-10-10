//! The install receipt from supplied placements, artifact outcomes, catalog
//! seed and record results. Write and recording failures remain visible.

use std::path::Path;

use baley_core::catalog::{HINT_VERSION, USER_PROJECT};
use baley_store::StoreError;

use crate::host_artifacts::placement::{Artifact, PlacementMap};
use crate::ledger::display::{self, Render};

use super::registration;
use super::settings::{self, Replaced};

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

/// What the settings step adds to the receipt beyond the artifact lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notes {
    /// Each gap with its fix. Any gap makes the install partial.
    pub gaps: Vec<String>,
    /// Owner values Baley's secure values replace when the settings write
    /// places them. The receipt reports them only then.
    pub replaced: Vec<Replaced>,
    /// Why the sandbox block was left out of the settings file, if it was.
    pub sandbox_held_back: Option<String>,
    /// Whether a `claude` command failed, so the owner is given the command
    /// to run by hand.
    pub registration_by_hand: bool,
}

impl Notes {
    /// The notes a settings decision gives the receipt.
    pub fn from_settings(decision: &settings::Decision, wiring_gap: Option<&str>) -> Self {
        Self {
            gaps: decision
                .gaps
                .iter()
                .cloned()
                .chain(wiring_gap.map(str::to_owned))
                .collect(),
            replaced: decision.replaced.clone(),
            sandbox_held_back: decision.held_back.clone(),
            registration_by_hand: false,
        }
    }
}

/// Renders every placed artifact in map order. A missing outcome counts as
/// not written. Completion requires every placement to be known, every
/// artifact to be Baley's, no gap and the catalog seed to be recorded.
pub(crate) fn render(
    version: &str,
    stable_path: &Path,
    placements: &PlacementMap,
    outcomes: &[(Artifact, ArtifactOutcome)],
    notes: &Notes,
    seed: &Result<bool, StoreError>,
) -> Render {
    let complete = super::plan::complete(placements, outcomes, notes, seed);
    let mut artifacts = Vec::new();
    let mut settings_path = None;
    for file in placements.expected_files() {
        if file.artifact == Artifact::Settings {
            settings_path = Some(file.path.display().to_string());
        }
        let outcome = outcomes
            .iter()
            .find(|(artifact, _)| *artifact == file.artifact)
            .map_or(&ArtifactOutcome::NotWritten, |(_, outcome)| outcome);
        let text = match outcome {
            ArtifactOutcome::NotWritten => "not written".into(),
            ArtifactOutcome::Written
                if file.artifact == Artifact::Settings && notes.sandbox_held_back.is_some() =>
            {
                "written without the sandbox block".into()
            }
            ArtifactOutcome::Written if file.artifact == Artifact::Registration => {
                "registered".into()
            }
            ArtifactOutcome::Written => "written".into(),
            ArtifactOutcome::Replaced => "replaced".into(),
            ArtifactOutcome::Failed(cause)
                if file.artifact == Artifact::Registration && notes.registration_by_hand =>
            {
                format!(
                    "not written: {cause}; run `{}` by hand to finish it",
                    registration::hand_command(&registration::entry(placements.executable()))
                )
            }
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
    let settings_path = settings_path.unwrap_or_default();
    // A replacement is only planned until the settings write places it.
    let settings_placed = outcomes
        .iter()
        .any(|(artifact, outcome)| *artifact == Artifact::Settings && outcome.is_owned());
    for replaced in notes.replaced.iter().filter(|_| settings_placed) {
        lines.push(format!(
            "replaced {} in {settings_path}: it held {}, and Baley's secure value {} is now there",
            replaced.key,
            replaced.was,
            settings::secure_value(&replaced.key)
        ));
    }
    lines.extend(notes.gaps.iter().map(|gap| format!("gap: {gap}")));
    let sandbox_written = outcomes
        .iter()
        .any(|(artifact, outcome)| *artifact == Artifact::Settings && outcome.is_owned())
        && notes.sandbox_held_back.is_none();
    if sandbox_written {
        lines.push(format!(
            "the sandbox settings in {settings_path} apply to every Claude Code session on this machine, including projects Baley does not manage"
        ));
        lines.push(format!(
            "managed settings, command-line settings and a project's own .claude/settings.json and .claude/settings.local.json take precedence over {settings_path} for single values such as sandbox.enabled"
        ));
    }
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
    let mut receipt = render(
        version,
        stable_path,
        placements,
        &applied.outcomes,
        &applied.notes,
        seed,
    );
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
            notes: Notes::default(),
            registration_seen: None,
            registration_removed: false,
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
            &Notes::default(),
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
            &Notes::default(),
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
            &Notes::default(),
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
                &Notes::default(),
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
    #[test]
    fn a_gap_left_unnamed_or_an_install_with_a_gap_called_complete_is_caught() {
        use std::collections::BTreeMap;

        use serde_json::json;

        use crate::host_doctor::placed::FileState;
        use crate::install::fixtures::{folders, prerequisites};
        use crate::install::plan::{self, Gathered, Observation};
        use crate::install::registration::{Attempt, Registered};

        let installed = installed();
        let owner = json!({"sandbox": {
            "excludedCommands": ["docker"],
            "filesystem": {
                "disabled": true,
                "allowRead": ["/home/o/.local/share/crenshawdev/baley"],
                "allowWrite": ["~/scratch"],
            },
        }});
        let seen = |state| Observation {
            state,
            symbolic_link: false,
        };
        let observations = BTreeMap::from([
            (
                installed.settings_file.clone(),
                seen(FileState::Bytes(serde_json::to_vec(&owner).unwrap())),
            ),
            (installed.registration_file.clone(), seen(FileState::Absent)),
            (
                "/home/o/.claude/skills/bal-capture/SKILL.md".into(),
                seen(FileState::Absent),
            ),
            (
                "/home/o/.claude/skills/bal-help/SKILL.md".into(),
                seen(FileState::Absent),
            ),
        ]);
        let no_socat = prerequisites("linux", &["bwrap"]);
        let plan = plan::judge(&Gathered {
            placements: &installed.placements,
            folders: &folders(),
            latest: None,
            observations: &observations,
            prerequisites: &no_socat,
            executable: None,
        })
        .expect("a gap is not a refusal");
        let written: Vec<Result<(), crate::replace::Failure>> =
            plan.writes.iter().map(|_| Ok(())).collect();
        let registered = Attempt {
            seen: None,
            removed: false,
            result: Registered::Registered,
        };
        let applied = plan::applied(&plan, &written, &registered);

        let receipt = render_applied(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &applied,
            &Ok(true),
            &Ok(true),
        );

        assert_eq!(receipt.lines[0], "install outcome: partial");
        assert!(
            !receipt
                .lines
                .iter()
                .any(|line| line == "install outcome: complete")
        );
        let gap = |parts: &[&str]| {
            receipt
                .lines
                .iter()
                .find(|line| line.starts_with("gap: ") && parts.iter().all(|p| line.contains(p)))
                .unwrap_or_else(|| panic!("no gap line holds {parts:?}: {:#?}", receipt.lines))
        };
        gap(&["docker", "sandbox.excludedCommands", "remove it"]);
        gap(&[
            "sandbox.filesystem.disabled",
            "remove it or set it to false",
        ]);
        gap(&[
            "/home/o/.local/share/crenshawdev/baley",
            "sandbox.filesystem.allowRead",
            "remove it",
        ]);
        gap(&[
            "~/scratch",
            "sandbox.filesystem.allowWrite",
            "write it as an absolute path outside Baley's folders or remove it",
        ]);
        gap(&["socat", "apt-get install bubblewrap socat"]);
        assert_eq!(
            receipt
                .lines
                .iter()
                .filter(|line| line.starts_with("gap: "))
                .count(),
            5
        );
        assert_eq!(receipt.code, 1);
        assert!(!receipt.error);
    }
    #[test]
    fn a_settings_change_or_its_reach_hidden_from_the_receipt_is_caught() {
        let installed = installed();
        let render_with = |outcome: ArtifactOutcome, notes: Notes| {
            render(
                "0.2.0",
                installed.layout.stable_path(),
                &installed.placements,
                &outcomes(outcome),
                &notes,
                &Ok(true),
            )
        };
        let line_with =
            |receipt: &Render, text: &str| receipt.lines.iter().any(|line| line.contains(text));
        let every_session = "apply to every Claude Code session on this machine, including projects Baley does not manage";
        let precedence = [
            "managed settings",
            "command-line settings",
            ".claude/settings.json",
            ".claude/settings.local.json",
            "take precedence",
        ];
        let settings_line = |receipt: &Render| {
            receipt
                .lines
                .iter()
                .find(|line| line.starts_with("settings at "))
                .cloned()
        };

        let replaced = Notes {
            replaced: vec![Replaced {
                key: "sandbox.enabled".into(),
                was: serde_json::json!(false),
            }],
            ..Notes::default()
        };
        let written = render_with(ArtifactOutcome::Written, replaced);
        assert_eq!(
            settings_line(&written).unwrap(),
            "settings at /home/o/.claude/settings.json: written"
        );
        assert!(written.lines.iter().any(|line| {
            line.contains("sandbox.enabled")
                && line.contains("false")
                && line.contains("/home/o/.claude/settings.json")
        }));
        assert!(line_with(&written, every_session));
        assert!(
            written
                .lines
                .iter()
                .any(|line| precedence.iter().all(|part| line.contains(part)))
        );

        let held_back = Notes {
            sandbox_held_back: Some("socat missing from PATH".into()),
            ..Notes::default()
        };
        let held = render_with(ArtifactOutcome::Written, held_back);
        assert_eq!(
            settings_line(&held).unwrap(),
            "settings at /home/o/.claude/settings.json: written without the sandbox block"
        );
        assert!(!line_with(&held, "every Claude Code session"));
        assert!(!line_with(&held, "take precedence"));

        let unchanged = render_with(ArtifactOutcome::Unchanged, Notes::default());
        assert_eq!(
            settings_line(&unchanged).unwrap(),
            "settings at /home/o/.claude/settings.json: unchanged"
        );
        assert!(line_with(&unchanged, every_session));
        assert!(
            unchanged
                .lines
                .iter()
                .any(|line| precedence.iter().all(|part| line.contains(part)))
        );
    }

    #[test]
    fn a_secure_value_reported_as_placed_after_a_failed_settings_write_is_caught() {
        let installed = installed();
        let notes = || Notes {
            replaced: vec![Replaced {
                key: "sandbox.enabled".into(),
                was: serde_json::json!(false),
            }],
            ..Notes::default()
        };
        let says_placed = |receipt: &Render| {
            receipt
                .lines
                .iter()
                .any(|line| line.starts_with("replaced ") || line.contains("is now there"))
        };

        for outcome in [
            ArtifactOutcome::Failed("Permission denied".into()),
            ArtifactOutcome::NotWritten,
        ] {
            let receipt = render(
                "0.2.0",
                installed.layout.stable_path(),
                &installed.placements,
                &outcomes(outcome),
                &notes(),
                &Ok(true),
            );
            assert!(!says_placed(&receipt), "{:#?}", receipt.lines);
            assert_eq!(receipt.lines[0], "install outcome: partial");
        }

        let receipt = render(
            "0.2.0",
            installed.layout.stable_path(),
            &installed.placements,
            &outcomes(ArtifactOutcome::Written),
            &notes(),
            &Ok(true),
        );
        assert!(says_placed(&receipt), "{:#?}", receipt.lines);
    }
    #[test]
    fn a_registration_failure_without_the_command_to_run_is_caught() {
        let cause =
            "not-writable: /home/o/.claude.json: could not start claude: No such file or directory";
        let failed = |home: &str| {
            let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
            let env = Environment {
                home: Some(home.into()),
                ..Environment::default()
            };
            let installed = installed::resolve(&env, None, &manifest).unwrap();
            let mut outcomes = outcomes(ArtifactOutcome::Written);
            outcomes[2].1 = ArtifactOutcome::Failed(cause.into());
            let applied = super::super::plan::Applied {
                outcomes,
                failure: None,
                notes: Notes {
                    registration_by_hand: true,
                    ..Notes::default()
                },
                registration_seen: None,
                registration_removed: false,
            };
            render_applied(
                "0.2.0",
                installed.layout.stable_path(),
                &installed.placements,
                &applied,
                &Ok(true),
                &Ok(true),
            )
        };

        let receipt = failed("/home/o");
        assert_eq!(receipt.lines[0], "install outcome: partial");
        assert_eq!(receipt.code, 1);
        let line = receipt
            .lines
            .iter()
            .find(|line| line.starts_with("registration at "))
            .expect("a registration line");
        assert!(line.contains(": not written: "), "{line}");
        assert!(line.contains("could not start claude"), "{line}");
        assert!(
            line.contains(
                r#"claude mcp add-json --scope user baley '{"command":"/home/o/.local/bin/baley","args":["serve"]}'"#
            ),
            "{line}"
        );

        let receipt = failed("/home/o'connor");
        let line = receipt
            .lines
            .iter()
            .find(|line| line.starts_with("registration at "))
            .expect("a registration line");
        assert!(
            line.contains(
                r#"claude mcp add-json --scope user baley '{"command":"/home/o'\''connor/.local/bin/baley","args":["serve"]}'"#
            ),
            "{line}"
        );
    }
}
