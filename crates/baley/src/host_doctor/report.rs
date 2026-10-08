//! The report: findings as owner lines and an exit code.

use std::path::Path;

use super::prerequisites::{Judged as Prerequisites, Kind};
use super::protection::{Judged, NotJudged};
use super::{ArtifactState, Findings, MapFault, server_context};
use crate::host_artifacts::coverage::{Access, Cause, Mechanism, Tool, Verdict};
use crate::mcp::context::ProjectContext;

/// The heading that opens the host section.
const HEADING: &str = "Claude Code host checks";

/// The host section: its lines, in print order, and the exit status they
/// call for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The lines to print.
    pub lines: Vec<String>,
    /// 0, or 1 when a host gap was found. A host finding never gives 2 or 3.
    pub code: u8,
}

impl Report {
    /// Words the findings as lines and sets the code.
    pub fn new(findings: &Findings) -> Self {
        let mut lines = vec![HEADING.to_owned()];
        let mut gap = false;
        if let Some(fault) = &findings.no_map {
            gap = true;
            lines.push(match fault {
                MapFault::PathUnreadable(cause) => format!(
                    "running binary: its path could not be read: {cause}, so no hook or registration can be rendered for it"
                ),
                MapFault::Executable { path, refusal } => format!(
                    "running binary {}: {refusal}, so no hook or registration can be rendered for it",
                    path.display()
                ),
                MapFault::Refused(reason) => {
                    format!("running binary: the placement map was refused: {reason}")
                }
            });
        }
        for (artifact, state) in &findings.artifacts {
            gap |= state.is_gap();
            lines.push(match state {
                ArtifactState::NotInstalled => {
                    format!("{artifact}: not installed, no placement is known")
                }
                ArtifactState::NotRead { path } => {
                    format!("{artifact}: {} was not read", path.display())
                }
                ArtifactState::Missing { path } => {
                    format!("{artifact}: missing, nothing is at {}", path.display())
                }
                ArtifactState::Fault { path, fault } => {
                    format!("{artifact}: {} {fault}", path.display())
                }
                ArtifactState::Read { path } => format!("{artifact}: read {}", path.display()),
                ArtifactState::StubMatches { path, digest } => format!(
                    "{artifact}: {} matches the stub Baley renders, sha256 {digest}",
                    path.display()
                ),
                ArtifactState::StubDiffers {
                    path,
                    expected,
                    found,
                } => format!(
                    "{artifact}: {} differs from the stub Baley renders: expected sha256 {expected}, found sha256 {found}",
                    path.display()
                ),
                ArtifactState::RegistrationMatches { path } => format!(
                    "{artifact}: {} holds the entry Baley renders for this binary",
                    path.display()
                ),
                ArtifactState::RegistrationMissing { path } => format!(
                    "{artifact}: {} has no mcpServers entry under the key {}",
                    path.display(),
                    crate::host_artifacts::registration::KEY
                ),
                ArtifactState::RegistrationDiffers { path, found } => format!(
                    "{artifact}: {} runs another command or arguments than Baley renders: {found}",
                    path.display()
                ),
            });
        }
        if let Some((path, gap_found)) = &findings.executable {
            gap = true;
            lines.push(format!(
                "executable {path} {gap_found}, and the hook and the registration run it"
            ));
        }
        gap |= prerequisite_lines(&findings.prerequisites, &mut lines);
        match &findings.coverage {
            None => {}
            Some(Err(NotJudged::Unknown(missing))) => {
                let names: Vec<String> = missing.iter().map(ToString::to_string).collect();
                lines.push(if names.len() == 1 {
                    format!(
                        "coverage: not judged because the {} placement is unknown",
                        names[0]
                    )
                } else {
                    "coverage: not judged because the settings and hook placements are unknown"
                        .to_owned()
                });
            }
            Some(Err(NotJudged::Unusable)) => lines.push(
                "coverage: not judged because a settings or hook document above could not be used"
                    .to_owned(),
            ),
            Some(Ok(judged)) => gap |= coverage_lines(judged, &mut lines),
        }
        if let Some(nine) = &findings.nine_tools {
            let hook = nine.hook.display();
            if let Some(disabled) = &nine.disabled_in {
                gap = true;
                lines.push(format!(
                    "guard hook: disableAllHooks is true in {}, so no hook runs the guard for any tool",
                    disabled.display()
                ));
            }
            if nine.missing.is_empty() {
                if nine.disabled_in.is_none() {
                    lines.push(format!(
                        "guard hook: a PreToolUse item in {hook} runs the guard for all nine tools in the matcher"
                    ));
                }
            } else {
                gap = true;
                let names: Vec<&str> = nine.missing.iter().map(|tool| tool.name()).collect();
                lines.push(format!(
                    "guard hook: no PreToolUse item in {hook} runs the guard for {}",
                    names.join(", ")
                ));
            }
        }
        server_lines(&findings.server, &mut lines);
        Report {
            lines,
            code: u8::from(gap),
        }
    }
}

/// The server's context as the last recorded server call shows it. None of
/// these lines is a finding that raises the code: a working directory that
/// differs from the project is valid, and the lines only tell the owner what
/// to set when the project variable is not usable.
fn server_lines(judged: &server_context::Judged, lines: &mut Vec<String>) {
    match judged {
        server_context::Judged::NoCall => {
            lines.push("server context: no server call is recorded in the ledger yet".to_owned());
        }
        server_context::Judged::ReadError(text) => lines.push(format!(
            "server context: the last server call could not be read: {text}"
        )),
        server_context::Judged::Call {
            project,
            recorded,
            working_directory,
            recorded_at,
        } => {
            lines.push(match project {
                ProjectContext::Valid(text) => format!("server CLAUDE_PROJECT_DIR: {text}"),
                ProjectContext::Missing => "server CLAUDE_PROJECT_DIR is not set (project-context-missing): set it to the project's absolute folder".to_owned(),
                ProjectContext::Invalid(fault) => format!(
                    "server CLAUDE_PROJECT_DIR {} cannot be the project (project-context-invalid): {fault}; restore the folder or start the session in an existing project so Claude Code sets it",
                    recorded.as_deref().unwrap_or_default()
                ),
            });
            lines.push(format!(
                "server working directory: {working_directory} (from the last server call, recorded at {recorded_at})"
            ));
        }
    }
}

/// The sandbox prerequisite lines for the platform. Returns whether the
/// sandbox cannot run here: a platform it does not run on, or a program it
/// needs that no `PATH` entry holds.
fn prerequisite_lines(judged: &Prerequisites, lines: &mut Vec<String>) -> bool {
    match judged.kind {
        Kind::MacOs => {
            lines.push("sandbox: macOS uses the built-in Seatbelt and needs no program".to_owned())
        }
        Kind::Other => lines.push(format!(
            "sandbox: {} is a platform where Claude Code's sandbox does not run",
            judged.os
        )),
        Kind::Linux => {
            for program in &judged.required {
                lines.push(match &program.found {
                    Some(path) => format!("sandbox: {} found at {}", program.name, path.display()),
                    None => format!(
                        "sandbox: {} is missing from PATH, so the sandbox cannot run; install {} with `apt-get install bubblewrap socat` or `dnf install bubblewrap socat` (this lookup used this command's PATH, which may differ from the one Claude Code runs with)",
                        program.name, program.package
                    ),
                });
            }
        }
    }
    !judged.unsupported().is_empty()
}

/// The coverage section: one line per tool access and per write-only file,
/// worded as configuration of the named documents, then two lines on what
/// the verdicts do not say. Returns whether any verdict is a gap.
fn coverage_lines(judged: &Judged, lines: &mut Vec<String>) -> bool {
    let (settings, hook) = (judged.settings.as_path(), judged.hook.as_path());
    let mut gap = false;
    lines.push(if settings == hook {
        format!(
            "coverage judged over the settings and hook in {}",
            settings.display()
        )
    } else {
        format!(
            "coverage judged over the settings in {} and the hook in {}",
            settings.display(),
            hook.display()
        )
    });
    for judged_tool in &judged.coverage.tools {
        let tool = judged_tool.tool;
        let rules = match tool {
            Tool::Read | Tool::Grep | Tool::Glob => "Read",
            _ => "Edit",
        };
        let access = match judged_tool.access {
            Access::Read => "read",
            Access::Write => "write",
        };
        let best_effort = if matches!(tool, Tool::Grep | Tool::Glob) {
            " (best-effort)"
        } else {
            ""
        };
        gap |= matches!(judged_tool.verdict, Verdict::Gap(_));
        lines.push(format!(
            "tool {}, {access} access: {}{best_effort}",
            tool.name(),
            verdict_text(&judged_tool.verdict, rules, settings, hook)
        ));
    }
    for file in &judged.coverage.files {
        gap |= matches!(file.write, Verdict::Gap(_));
        lines.push(format!(
            "write-only file {}: write access {}",
            file.path.display(),
            verdict_text(&file.write, "Edit", settings, hook)
        ));
    }
    let documents = if settings == hook {
        settings.display().to_string()
    } else {
        format!("{} and {}", settings.display(), hook.display())
    };
    lines.push(format!(
        "These verdicts describe configuration in {documents} and are not proof that Claude Code enforces it."
    ));
    lines.push(
        "Other Claude Code settings files (managed, command line, local, project and user) and the sandbox.enabledPlatforms policy setting can change what applies: lists combine across files and a single value comes from the highest file.".to_owned(),
    );
    gap
}

/// One verdict as configuration: the mechanisms that carry it, or each gap
/// with the setting to change.
fn verdict_text(verdict: &Verdict, rules: &str, settings: &Path, hook: &Path) -> String {
    match verdict {
        Verdict::Covered { by, .. } => {
            let by: Vec<String> = by
                .iter()
                .map(|mechanism| match mechanism {
                    Mechanism::Sandbox => format!("the sandbox in {}", settings.display()),
                    Mechanism::PermissionRules => {
                        format!("the {rules} deny rules in {}", settings.display())
                    }
                    Mechanism::Hook => format!("the guard hook in {}", hook.display()),
                })
                .collect();
            format!("configured as covered by {}", by.join(" and "))
        }
        Verdict::Gap(causes) => {
            let causes: Vec<String> = causes
                .iter()
                .map(|cause| cause_text(cause, settings, hook))
                .collect();
            format!("configured with a gap: {}", causes.join("; "))
        }
    }
}

/// What one cause of a gap asks the owner to change, naming the document
/// the coverage judge read it from.
fn cause_text(cause: &Cause, settings: &Path, hook: &Path) -> String {
    let (s, h) = (settings.display(), hook.display());
    match cause {
        Cause::Unsupported(mechanism) => format!(
            "{} cannot run on this machine",
            match mechanism {
                Mechanism::Sandbox => "the sandbox",
                Mechanism::PermissionRules => "the permission rules",
                Mechanism::Hook => "the guard hook",
            }
        ),
        Cause::SandboxSetting(key) => {
            let secure = if *key == "allowUnsandboxedCommands" || *key == "filesystem.disabled" {
                "false"
            } else {
                "true"
            };
            format!("set sandbox.{key} to {secure} in {s}")
        }
        Cause::Excluded(entry) => format!(
            "remove \"{entry}\" from sandbox.excludedCommands in {s}, since a listed command runs outside the sandbox"
        ),
        Cause::NotDenied { list, path } => {
            format!("add {path} to sandbox.filesystem.{list} in {s}")
        }
        Cause::Reopened { list, entry } => format!(
            "remove {entry} from sandbox.filesystem.{list} in {s}, since it re-opens a protected path"
        ),
        Cause::Unjudged { list, entry } => format!(
            "{entry} in sandbox.filesystem.{list} in {s} does not start with / and cannot be judged"
        ),
        Cause::NoRule(rule) => format!("add {rule} to permissions.deny in {s}"),
        Cause::NoGuard => format!("no PreToolUse item in {h} runs the guard for it"),
        Cause::HooksDisabled if settings == hook => format!("disableAllHooks is true in {h}"),
        Cause::HooksDisabled => format!("disableAllHooks is true in {h} or {s}"),
        Cause::Unrendered(report) => report.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fixtures::*;
    use super::super::placed::{Fault, FileState};
    use super::super::{MapFault, Observation, all_unknown, judge};
    use super::*;
    use crate::host_artifacts::compose::compose;
    use crate::host_artifacts::executable::{MissingPrerequisite, PathFault};
    use crate::host_artifacts::{hook, security};

    fn unknown_observation() -> Observation {
        let map = all_unknown(Ok(PathBuf::from(EXECUTABLE))).unwrap();
        observed(map, vec![])
    }

    fn report_of(observation: &Observation) -> Report {
        Report::new(&judge(observation))
    }

    /// Whether a line says the artifact is installed, once the words
    /// "not installed" are set aside.
    fn says_installed(line: &str) -> bool {
        let rest = line.replace("not installed", "");
        ["installed", "present", "matches"]
            .iter()
            .any(|word| rest.contains(word))
    }

    /// The one line that starts with `name`.
    fn line_for<'r>(report: &'r Report, name: &str) -> &'r String {
        let lines: Vec<&String> = report
            .lines
            .iter()
            .filter(|line| line.starts_with(name))
            .collect();
        assert_eq!(lines.len(), 1, "{name}: {:?}", report.lines);
        lines[0]
    }

    #[test]
    fn an_unknown_placement_shown_as_installed_or_left_unlisted_is_caught() {
        let report = report_of(&unknown_observation());
        let names = [
            "stub `bal-capture`",
            "stub `bal-help`",
            "registration",
            "hook",
            "settings",
        ];
        for name in names {
            let line = line_for(&report, name);
            assert!(line.contains("not installed"), "{line}");
            assert!(!says_installed(line), "{line}");
        }
        assert_eq!(report.code, 0);

        for (os, code) in [("macos", 0), ("freebsd", 1)] {
            let mut other = unknown_observation();
            other.prerequisites = found_on(os, &[]);
            let report = report_of(&other);
            for name in names {
                let line = line_for(&report, name);
                assert!(line.contains("not installed"), "{os}: {line}");
                assert!(!says_installed(line), "{os}: {line}");
            }
            assert_eq!(report.code, code, "{os}: {:?}", report.lines);
        }
    }

    #[test]
    fn an_unusable_binary_path_dropped_from_the_report_is_caught() {
        let relative = unmapped(MapFault::Executable {
            path: PathBuf::from("baley"),
            refusal: MissingPrerequisite {
                fault: PathFault::Relative,
            },
        });
        let report = report_of(&relative);
        let named: Vec<&String> = report
            .lines
            .iter()
            .filter(|line| line.contains("baley") && line.contains("not absolute"))
            .collect();
        assert_eq!(named.len(), 1, "{:?}", report.lines);
        assert_eq!(report.code, 1);
        assert!(!report.lines.iter().any(|line| says_installed(line)));

        let cause = "No such file or directory (os error 2)";
        let unreadable = unmapped(MapFault::PathUnreadable(cause.into()));
        let report = report_of(&unreadable);
        let named: Vec<&String> = report
            .lines
            .iter()
            .filter(|line| line.contains("could not be read") && line.contains(cause))
            .collect();
        assert_eq!(named.len(), 1, "{:?}", report.lines);
        assert_eq!(report.code, 1);
        assert!(!report.lines.iter().any(|line| says_installed(line)));
    }

    #[test]
    fn a_placed_artifact_found_missing_reported_as_not_installed_is_caught() {
        let map = map(None, Some(HELP), Some(REGISTRATION), None, None);
        let report = report_of(&observed(
            map,
            vec![(HELP, FileState::Absent), (REGISTRATION, FileState::Absent)],
        ));
        for (name, path) in [("stub `bal-help`", HELP), ("registration", REGISTRATION)] {
            let line = line_for(&report, name);
            assert!(line.contains(path) && line.contains("missing"), "{line}");
            assert!(!line.contains("not installed"), "{line}");
        }
        for name in ["stub `bal-capture`", "hook", "settings"] {
            assert!(line_for(&report, name).contains("not installed"));
        }
        assert_eq!(report.code, 1);
    }

    #[test]
    fn a_placed_document_that_cannot_be_read_left_out_of_the_code_is_caught() {
        let cases = [
            (FileState::Bytes(b"{".to_vec()), "is not JSON"),
            (
                FileState::Fault(Fault::Unreadable("Permission denied (os error 13)".into())),
                "Permission denied (os error 13)",
            ),
        ];
        for (state, words) in cases {
            let map = map(None, None, None, None, Some(SETTINGS));
            let report = report_of(&observed(map, vec![(SETTINGS, state)]));
            let line = line_for(&report, "settings");
            assert!(line.contains(SETTINGS) && line.contains(words), "{line}");
            assert_eq!(report.code, 1, "{line}");
        }
    }
    #[test]
    fn a_differing_stub_reported_without_both_digests_is_caught() {
        use sha2::{Digest, Sha256};

        let manifest =
            crate::host_artifacts::stubs::manifest(&crate::host_artifacts::stubs::front_doors())
                .unwrap();
        let help = manifest
            .iter()
            .find(|entry| entry.identity == "bal-help")
            .unwrap();
        let mut changed = help.bytes.clone();
        changed[0] ^= 1;
        let found: String = Sha256::digest(&changed)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();

        let differing = report_of(&observed(
            map(None, Some(HELP), None, None, None),
            vec![(HELP, FileState::Bytes(changed))],
        ));
        let line = line_for(&differing, "stub `bal-help`");
        for part in [HELP, help.digest.as_str(), found.as_str()] {
            assert!(line.contains(part), "{part} missing from {line}");
        }
        assert_eq!(differing.code, 1);

        let equal = report_of(&observed(
            map(None, Some(HELP), None, None, None),
            vec![(HELP, FileState::Bytes(help.bytes.clone()))],
        ));
        let line = line_for(&equal, "stub `bal-help`");
        assert!(
            line.contains("matches") && !line.contains("differs"),
            "{line}"
        );
        assert!(!equal.lines.iter().any(|line| line.contains("differs")));
        assert_eq!(equal.code, 0);
    }
    /// Baley's proposal and hook composed into one document.
    fn composed_value(write_only: &[&str]) -> serde_json::Value {
        let exe = executable();
        let write_only: Vec<PathBuf> = write_only.iter().map(PathBuf::from).collect();
        let proposal = security::propose(&folders(), &exe, &write_only).settings;
        compose(None, &[proposal, hook::render(&exe)], &folders(), &exe).document
    }

    /// The same document, as the bytes a read finds.
    fn composed(write_only: &[&str]) -> Vec<u8> {
        serde_json::to_vec(&composed_value(write_only)).unwrap()
    }

    /// Settings and hook placed in one document, the document composed from
    /// Baley's own values.
    fn one_document() -> Report {
        report_of(&observed(
            map(None, None, None, Some(SETTINGS), Some(SETTINGS)),
            vec![(SETTINGS, FileState::Bytes(composed(&[SETTINGS])))],
        ))
    }

    /// The tool and write-only file verdict lines.
    fn verdict_lines(report: &Report) -> Vec<&String> {
        report
            .lines
            .iter()
            .filter(|line| line.starts_with("tool ") || line.starts_with("write-only file "))
            .collect()
    }

    /// What a verdict line claims, after the tool or file it is about.
    fn claim(line: &str) -> &str {
        line.split_once(": ").unwrap().1
    }

    fn tool_lines<'r>(report: &'r Report, tool: &str) -> Vec<&'r String> {
        let prefix = format!("tool {tool}, ");
        let lines: Vec<&String> = report
            .lines
            .iter()
            .filter(|line| line.starts_with(&prefix))
            .collect();
        assert!(!lines.is_empty(), "{tool}: {:?}", report.lines);
        lines
    }

    #[test]
    fn a_coverage_verdict_shown_as_proof_or_grep_and_glob_unlabelled_is_caught() {
        let report = one_document();
        let verdicts = verdict_lines(&report);
        assert_eq!(verdicts.len(), 14, "{:?}", report.lines);
        for line in &verdicts {
            assert!(
                line.contains(SETTINGS) && line.contains("configured"),
                "{line}"
            );
            assert!(
                !line.contains("enforced") && !line.contains("proven"),
                "{line}"
            );
            let labelled = line.starts_with("tool Grep,") || line.starts_with("tool Glob,");
            assert_eq!(line.contains("best-effort"), labelled, "{line}");
        }
        let notes: Vec<&String> = report
            .lines
            .iter()
            .filter(|line| line.contains("configuration") && line.contains("not proof"))
            .collect();
        assert_eq!(notes.len(), 1, "{:?}", report.lines);
        assert_eq!(report.code, 0, "{:?}", report.lines);

        let split = report_of(&observed(
            map(None, None, None, Some(HOOKS), Some(SETTINGS)),
            vec![
                (
                    SETTINGS,
                    FileState::Bytes(
                        serde_json::to_vec(
                            &security::propose(
                                &folders(),
                                &executable(),
                                &[SETTINGS.into(), HOOKS.into()],
                            )
                            .settings,
                        )
                        .unwrap(),
                    ),
                ),
                (
                    HOOKS,
                    FileState::Bytes(serde_json::to_vec(&hook::render(&executable())).unwrap()),
                ),
            ],
        ));
        for tool in ["Read", "Grep", "Glob", "Write", "Edit", "NotebookEdit"] {
            for line in tool_lines(&split, tool) {
                assert!(line.contains(SETTINGS) && line.contains(HOOKS), "{line}");
            }
        }
        let shell = ["Bash", "Monitor", "PowerShell"]
            .iter()
            .flat_map(|tool| tool_lines(&split, tool))
            .collect::<Vec<_>>();
        let files: Vec<&String> = split
            .lines
            .iter()
            .filter(|line| line.starts_with("write-only file "))
            .collect();
        assert_eq!(files.len(), 3, "{:?}", split.lines);
        for line in shell.into_iter().chain(files) {
            let claim = claim(line);
            assert!(claim.contains(SETTINGS) && !claim.contains(HOOKS), "{line}");
        }
    }

    #[test]
    fn shell_sandboxing_and_file_tool_rules_reported_as_one_mechanism_is_caught() {
        let report = one_document();
        for tool in ["Bash", "Monitor", "PowerShell"] {
            for line in tool_lines(&report, tool) {
                assert!(line.contains("the sandbox"), "{line}");
                assert!(!line.contains("deny rules"), "{line}");
            }
        }
        for tool in ["Read", "Grep", "Glob"] {
            for line in tool_lines(&report, tool) {
                assert!(
                    line.contains("Read deny rules") && line.contains("guard hook"),
                    "{line}"
                );
                assert!(
                    !line.contains("Edit deny rules") && !line.contains("sandbox"),
                    "{line}"
                );
            }
        }
        for tool in ["Write", "Edit", "NotebookEdit"] {
            for line in tool_lines(&report, tool) {
                assert!(
                    line.contains("Edit deny rules") && line.contains("guard hook"),
                    "{line}"
                );
                assert!(
                    !line.contains("Read deny rules") && !line.contains("sandbox"),
                    "{line}"
                );
            }
        }
    }

    #[test]
    fn other_settings_scopes_left_unmentioned_beside_coverage_is_caught() {
        let mentions = |report: &Report| -> Vec<String> {
            report
                .lines
                .iter()
                .filter(|line| line.contains("sandbox.enabledPlatforms"))
                .cloned()
                .collect()
        };
        let judged = mentions(&one_document());
        assert_eq!(judged.len(), 1, "{judged:?}");
        assert!(judged[0].contains("settings files"), "{}", judged[0]);
        assert!(
            judged[0].contains("can change what applies"),
            "{}",
            judged[0]
        );
        assert!(!judged[0].contains("cannot"), "{}", judged[0]);
        assert!(mentions(&report_of(&unknown_observation())).is_empty());
    }
    /// The report for settings and hook in one document, edited by `edit`.
    fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> Report {
        let mut document = composed_value(&[SETTINGS]);
        edit(&mut document);
        report_of(&observed(
            map(None, None, None, Some(SETTINGS), Some(SETTINGS)),
            vec![(
                SETTINGS,
                FileState::Bytes(serde_json::to_vec(&document).unwrap()),
            )],
        ))
    }

    /// Removes the entries of the arrays at `pointers` that `drop` picks.
    fn without(document: &mut serde_json::Value, pointers: &[&str], drop: impl Fn(&str) -> bool) {
        for pointer in pointers {
            document
                .pointer_mut(pointer)
                .and_then(serde_json::Value::as_array_mut)
                .unwrap()
                .retain(|item| !drop(item.as_str().unwrap()));
        }
    }

    fn names(report: &Report, words: &[&str]) -> bool {
        report
            .lines
            .iter()
            .any(|line| words.iter().all(|word| line.contains(word)))
    }

    #[test]
    fn a_host_gap_left_unnamed_in_the_report_is_caught() {
        use serde_json::json;

        let clean = edited(|_| {});
        assert_eq!(clean.code, 0, "{:?}", clean.lines);

        let off = edited(|document| document["sandbox"]["enabled"] = json!(false));
        assert!(names(&off, &["sandbox.enabled"]), "{:?}", off.lines);
        assert_eq!(off.code, 1);

        let mut no_bwrap = observed(
            map(None, None, None, Some(SETTINGS), Some(SETTINGS)),
            vec![(SETTINGS, FileState::Bytes(composed(&[SETTINGS])))],
        );
        no_bwrap.prerequisites = found_on("linux", &["socat"]);
        let no_bwrap = report_of(&no_bwrap);
        assert!(
            names(&no_bwrap, &["bwrap", "bubblewrap"]),
            "{:?}",
            no_bwrap.lines
        );
        assert_eq!(no_bwrap.code, 1);

        let no_config = edited(|document| {
            without(
                document,
                &[
                    "/sandbox/filesystem/denyRead",
                    "/sandbox/filesystem/denyWrite",
                    "/permissions/deny",
                ],
                |item| item.contains("/.config/"),
            );
        });
        assert!(names(&no_config, &[CONFIG]), "{:?}", no_config.lines);
        assert_eq!(no_config.code, 1);

        let no_deny_read =
            edited(|document| document["sandbox"]["filesystem"]["denyRead"] = json!([]));
        assert!(
            names(&no_deny_read, &["denyRead"]),
            "{:?}",
            no_deny_read.lines
        );
        assert_eq!(no_deny_read.code, 1);

        let no_edit_rules = edited(|document| {
            without(document, &["/permissions/deny"], |rule| {
                rule.starts_with("Edit(")
            });
        });
        for tool in ["Write", "Edit", "NotebookEdit"] {
            for line in tool_lines(&no_edit_rules, tool) {
                assert!(
                    line.contains("Edit(//home/o/.local/share/crenshawdev/baley/**)"),
                    "{line}"
                );
            }
        }
        assert_eq!(no_edit_rules.code, 1);

        let six_tools = edited(|document| {
            document["hooks"]["PreToolUse"][0]["matcher"] =
                json!("Read|Grep|Glob|Write|Edit|NotebookEdit");
        });
        let line = six_tools
            .lines
            .iter()
            .find(|line| line.starts_with("guard hook:"))
            .unwrap_or_else(|| panic!("{:?}", six_tools.lines));
        for tool in ["Bash", "Monitor", "PowerShell"] {
            assert!(line.contains(tool), "{line}");
        }
        assert!(!line.contains("Read"), "{line}");
        assert_eq!(six_tools.code, 1);

        let mut changed =
            crate::host_artifacts::stubs::manifest(&crate::host_artifacts::stubs::front_doors())
                .unwrap()
                .into_iter()
                .find(|entry| entry.identity == "bal-help")
                .unwrap()
                .bytes;
        changed[0] ^= 1;
        let stub = report_of(&observed(
            map(None, Some(HELP), None, None, None),
            vec![(HELP, FileState::Bytes(changed))],
        ));
        assert!(
            names(&stub, &["bal-help", HELP, "differs"]),
            "{:?}",
            stub.lines
        );
        assert_eq!(stub.code, 1);

        let mut freebsd = unknown_observation();
        freebsd.prerequisites = found_on("freebsd", &[]);
        let freebsd = report_of(&freebsd);
        assert!(
            names(&freebsd, &["freebsd", "sandbox does not run"]),
            "{:?}",
            freebsd.lines
        );
        assert_eq!(freebsd.code, 1);
    }

    #[test]
    fn an_executable_with_no_execute_bit_left_out_of_the_report_and_its_code_is_caught() {
        use super::super::placed::{ExecutableSeen, Target, executable_gap};

        let mut observation = unknown_observation();
        let healthy = report_of(&observation);
        assert_eq!(healthy.code, 0, "{:?}", healthy.lines);
        assert!(!healthy.lines.iter().any(|line| line.contains("executable")));

        let gap = executable_gap(ExecutableSeen {
            link: Ok(false),
            target: Ok(Target {
                regular_file: true,
                mode: 0o644,
            }),
        });
        observation.host.as_mut().unwrap().executable = gap;
        let report = report_of(&observation);
        assert!(
            names(
                &report,
                &["executable", EXECUTABLE, "no execute permission"]
            ),
            "{:?}",
            report.lines
        );
        assert_eq!(report.code, 1, "{:?}", report.lines);
    }

    #[test]
    fn a_guard_credited_for_all_nine_tools_while_disable_all_hooks_is_true_is_caught() {
        let mut document = hook::render(&executable());
        document["disableAllHooks"] = serde_json::json!(true);
        let report = report_of(&observed(
            map(None, None, None, Some(HOOKS), None),
            vec![(
                HOOKS,
                FileState::Bytes(serde_json::to_vec(&document).unwrap()),
            )],
        ));
        assert!(
            names(&report, &["guard hook", "disableAllHooks", HOOKS]),
            "{:?}",
            report.lines
        );
        assert!(
            !names(&report, &["runs the guard for all nine tools"]),
            "{:?}",
            report.lines
        );
        assert_eq!(report.code, 1, "{:?}", report.lines);
    }

    use super::super::server_context::{Observed as Server, ServerCall};

    fn server_call(project: Option<&str>, is_directory: bool, working: &str) -> Server {
        Server::Call(ServerCall {
            project: project.map(str::to_owned),
            project_is_directory: is_directory,
            working_directory: working.to_owned(),
            recorded_at: "2026-10-08T10:00:05Z".to_owned(),
        })
    }

    fn with_server(server: Server) -> Report {
        let mut observation = unknown_observation();
        observation.server = server;
        report_of(&observation)
    }

    #[test]
    fn a_missing_or_invalid_project_variable_without_its_fix_is_caught() {
        let missing = with_server(server_call(None, false, "/w/other"));
        assert!(
            names(
                &missing,
                &[
                    "CLAUDE_PROJECT_DIR",
                    "project-context-missing",
                    "not set",
                    "set it to the project's absolute folder"
                ]
            ),
            "{:?}",
            missing.lines
        );

        let invalid = with_server(server_call(Some("/w/gone"), false, "/w/other"));
        assert!(
            names(
                &invalid,
                &[
                    "CLAUDE_PROJECT_DIR",
                    "project-context-invalid",
                    "/w/gone",
                    "it is not an existing directory",
                    "restore the folder or start the session in an existing project so Claude Code sets it"
                ]
            ),
            "{:?}",
            invalid.lines
        );
        for report in [&missing, &invalid] {
            let working = line_for(report, "server working directory");
            assert!(working.contains("/w/other"), "{working}");
            assert!(!working.contains("project-context-"), "{working}");
            assert!(
                !report
                    .lines
                    .iter()
                    .any(|line| line.contains("CLAUDE_PROJECT_DIR") && line.contains("/w/other")),
                "{:?}",
                report.lines
            );
        }
    }

    #[test]
    fn a_project_and_working_directory_difference_called_a_finding_is_caught() {
        for working in ["/w/p/sub", "/tmp"] {
            let report = with_server(server_call(Some("/w/p"), true, working));
            let project = line_for(&report, "server CLAUDE_PROJECT_DIR");
            assert!(project.contains("/w/p"), "{project}");
            assert!(!project.contains(working), "{project}");
            let directory = line_for(&report, "server working directory");
            assert!(directory.contains(working), "{directory}");
            assert!(
                !report
                    .lines
                    .iter()
                    .any(|line| line.contains("project-context-")),
                "{:?}",
                report.lines
            );
            assert_eq!(report.code, 0, "{:?}", report.lines);
        }
    }

    #[test]
    fn no_recorded_server_call_reported_as_a_missing_project_is_caught() {
        let report = with_server(Server::NoCall);
        assert_eq!(
            report
                .lines
                .iter()
                .filter(|line| line.contains("no server call is recorded"))
                .count(),
            1,
            "{:?}",
            report.lines
        );
        for word in [
            "project-context-missing",
            "project-context-invalid",
            "CLAUDE_PROJECT_DIR",
            "working directory",
        ] {
            assert!(
                !report.lines.iter().any(|line| line.contains(word)),
                "{word}: {:?}",
                report.lines
            );
        }
        assert_eq!(report.code, 0);
    }
}
