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
                MapFault::Refused(reason) => format!(
                    "running binary: the placement map was refused: {reason}"
                ),
            });
        }
        for (artifact, state) in &findings.artifacts {
            match state {
                ArtifactState::NotInstalled => {
                    lines.push(format!("{artifact}: not installed, no placement is known"));
                }
            }
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

    use super::super::{MapFault, Observation, all_unknown, judge};
    use super::*;
    use crate::host_artifacts::executable::{MissingPrerequisite, PathFault};

    fn unknown_observation() -> Observation {
        Observation {
            placement: all_unknown(Ok(PathBuf::from("/usr/local/bin/baley"))),
        }
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
            let lines: Vec<&String> = report
                .lines
                .iter()
                .filter(|line| line.starts_with(name))
                .collect();
            assert_eq!(lines.len(), 1, "{name}: {:?}", report.lines);
            assert!(lines[0].contains("not installed"), "{}", lines[0]);
            assert!(!says_installed(lines[0]), "{}", lines[0]);
        }
        assert_eq!(report.code, 0);
    }

    #[test]
    fn an_unusable_binary_path_dropped_from_the_report_is_caught() {
        let relative = Observation {
            placement: Err(MapFault::Executable {
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
            placement: Err(MapFault::PathUnreadable(cause.into())),
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
}
