//! What a server open found when it ran `quick_check`, and the pure rules
//! for judging the result and for deciding whether to run it (design 0001,
//! Opening the store).

/// What an open found out about the file's health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupHealth {
    /// The open did not run `quick_check`: it was not a server open, or the
    /// schema was created by this open.
    NotChecked,
    /// `quick_check` answered `ok`.
    Healthy,
    /// `quick_check` found a fault, or could not run. Every write is
    /// refused and reads go on.
    Unhealthy {
        /// The rows `quick_check` returned, one per line, or the text of the
        /// error that stopped it.
        report: String,
    },
}

/// Judges what running `PRAGMA quick_check` produced: its text rows, or the
/// error text when the pragma itself failed. Only a single `ok` row is
/// healthy. A check that could not run cannot show the store healthy.
pub(crate) fn judge_quick_check(outcome: Result<Vec<String>, String>) -> StartupHealth {
    match outcome {
        Ok(rows) if matches!(rows.as_slice(), [only] if only == "ok") => StartupHealth::Healthy,
        Ok(rows) => StartupHealth::Unhealthy {
            report: rows.join("\n"),
        },
        Err(error) => StartupHealth::Unhealthy { report: error },
    }
}

/// Whether an open runs `quick_check`: only a server open of a schema that
/// already existed. A schema this open just created has nothing to check.
pub(crate) fn runs_quick_check(server_open: bool, schema_existed: bool) -> bool {
    server_open && schema_existed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&str]) -> Result<Vec<String>, String> {
        Ok(lines.iter().map(|line| (*line).to_string()).collect())
    }

    // Catches a verdict that does not recognise the one healthy answer.
    #[test]
    fn a_single_ok_row_is_judged_unhealthy() {
        assert_eq!(judge_quick_check(rows(&["ok"])), StartupHealth::Healthy);
    }

    // Catches a verdict that looks for `ok` anywhere in the rows.
    #[test]
    fn ok_beside_another_row_is_judged_healthy() {
        let verdict = judge_quick_check(rows(&["ok", "row 3 missing from index"]));
        assert!(
            matches!(verdict, StartupHealth::Unhealthy { .. }),
            "{verdict:?}"
        );
    }

    // Catches an empty answer read as nothing wrong.
    #[test]
    fn an_empty_row_list_is_judged_healthy() {
        let verdict = judge_quick_check(rows(&[]));
        assert!(
            matches!(verdict, StartupHealth::Unhealthy { .. }),
            "{verdict:?}"
        );
    }

    // Catches a check that failed to run being taken for a clean store.
    #[test]
    fn a_pragma_error_is_judged_healthy() {
        let verdict = judge_quick_check(Err("database disk image is malformed".into()));
        assert_eq!(
            verdict,
            StartupHealth::Unhealthy {
                report: "database disk image is malformed".into()
            }
        );
    }

    // Catches a report that keeps only the first row, or none of them.
    #[test]
    fn the_report_drops_a_row_the_pragma_returned() {
        let verdict = judge_quick_check(rows(&[
            "CHECK constraint failed in project",
            "second fault",
        ]));
        assert_eq!(
            verdict,
            StartupHealth::Unhealthy {
                report: "CHECK constraint failed in project\nsecond fault".into()
            }
        );
    }

    // Catches a freshly created schema being checked.
    #[test]
    fn the_check_runs_on_a_freshly_created_schema() {
        assert!(!runs_quick_check(true, false));
    }

    // Catches a command-line or guard open running the check.
    #[test]
    fn the_check_runs_without_the_server_option() {
        assert!(!runs_quick_check(false, true));
    }

    // Catches a server open of an existing schema that skips the check.
    #[test]
    fn a_server_open_of_an_existing_schema_skips_the_check() {
        assert!(runs_quick_check(true, true));
    }
}
