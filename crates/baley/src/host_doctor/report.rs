//! The report: findings as owner lines and an exit code.

use super::{ArtifactState, Findings, MapFault};

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
            });
        }
        if let Some((path, gap_found)) = &findings.executable {
            gap = true;
            lines.push(format!(
                "executable {path} {gap_found}, and the hook and the registration run it"
            ));
        }
        Report {
            lines,
            code: u8::from(gap),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fixtures::*;
    use super::super::placed::{Fault, FileState};
    use super::super::{MapFault, Observation, all_unknown, judge};
    use super::*;
    use crate::host_artifacts::executable::{MissingPrerequisite, PathFault};

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
    }

    #[test]
    fn an_unusable_binary_path_dropped_from_the_report_is_caught() {
        let relative = Observation {
            host: Err(MapFault::Executable {
                path: PathBuf::from("baley"),
                refusal: MissingPrerequisite {
                    fault: PathFault::Relative,
                },
            }),
        };
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
        let unreadable = Observation {
            host: Err(MapFault::PathUnreadable(cause.into())),
        };
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
}
