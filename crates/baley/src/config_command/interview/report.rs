//! The ordered writes after the final `yes`: which step runs next, where the
//! run stops and what the owner is told (design 0003 section 5, Confirmed
//! writes). It is pure over the planned steps and the outcome of each step as
//! it arrives, so it is tested without a store or a file.

use std::path::Path;

use baley_core::policy::Value;
use baley_core::policy::config_command::TypedPair;

use super::writes::{Ack, Plan};
use crate::ledger::display::Render;
use crate::providers::warning;

/// One of the four writes, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Record the acknowledgement unless a current one covers it.
    Acknowledgement,
    /// Add the API hosts to Claude Code's settings file.
    Hosts,
    /// Write the global `review.reviewers` list.
    Reviewers,
    /// Write the settings answers through `config set`.
    Settings,
}

/// What a step that succeeded did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Done {
    /// The acknowledgement was recorded, or a current one already covered it.
    Acknowledgement {
        /// Whether an event was appended.
        recorded: bool,
    },
    /// The hosts were added. The note is the sync failure's text when the file
    /// was replaced but its folder could not be synced.
    Hosts {
        /// The sync failure's text, if any.
        note: Option<String>,
    },
    /// The list was written. The version is the policy version the step
    /// recorded in a project that is in the ledger.
    Reviewers {
        /// The policy version, if the step ran.
        version: Option<u64>,
    },
    /// `config set` wrote the settings, and printed these lines.
    Settings(Vec<String>),
}

/// What a step did: its success, or the failure it printed with its code.
pub(super) type Outcome = Result<Done, Render>;

/// The steps the plan runs, in order.
fn steps(plan: &Plan) -> Vec<Step> {
    let mut steps = Vec::new();
    if plan.acknowledgement.is_some() {
        steps.push(Step::Acknowledgement);
    }
    if plan.hosts.is_some() {
        steps.push(Step::Hosts);
    }
    if plan.reviewers.is_some() {
        steps.push(Step::Reviewers);
    }
    if !plan.settings.is_empty() {
        steps.push(Step::Settings);
    }
    steps
}

/// The step to run after the outcomes so far: the next planned one, or none
/// when every step has run or one failed.
pub(super) fn next(plan: &Plan, outcomes: &[Outcome]) -> Option<Step> {
    if outcomes.iter().any(Result::is_err) {
        return None;
    }
    steps(plan).get(outcomes.len()).copied()
}

/// The owner's words for a step that was not started.
fn unwritten(step: Step) -> &'static str {
    match step {
        Step::Acknowledgement => "the acknowledgement",
        Step::Hosts => "the API hosts",
        Step::Reviewers => "review.reviewers",
        Step::Settings => "the settings",
    }
}

/// The lines for one completed write.
fn written(plan: &Plan, global: &Path, done: &Done) -> Vec<String> {
    match done {
        Done::Acknowledgement { recorded } => {
            let Some(Ack::Requested(who) | Ack::Record(who, _) | Ack::Reused(who)) =
                &plan.acknowledgement
            else {
                return Vec::new();
            };
            let names: Vec<&str> = who
                .providers
                .iter()
                .map(|provider| provider.name())
                .collect();
            let was = if *recorded {
                "recorded"
            } else {
                "already recorded"
            };
            vec![format!(
                "The acknowledgement of {} at warning version {} was {was}.",
                names.join(", "),
                warning::WARNING_VERSION
            )]
        }
        Done::Hosts { note } => {
            let Some(hosts) = &plan.hosts else {
                return Vec::new();
            };
            let mut lines = vec![format!(
                "Allowed {} in {} (sandbox.network.allowedDomains).",
                hosts.allowance.added.join(", "),
                hosts.path.display()
            )];
            lines.extend(note.clone());
            lines
        }
        Done::Reviewers { version } => {
            let Some(TypedPair {
                value: Value::ReviewerList(names),
                ..
            }) = &plan.reviewers
            else {
                return Vec::new();
            };
            let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
            let policy = version.map_or_else(String::new, |version| {
                format!(" (policy version {version})")
            });
            vec![format!(
                "Wrote review.reviewers = [{}] to {}{policy}.",
                quoted.join(", "),
                global.display()
            )]
        }
        Done::Settings(lines) => lines.clone(),
    }
}

/// What the owner is told once the run ends, and how it exits. A run that
/// stopped at a failure prints that step's own lines first, with its code,
/// then each write already completed, then the steps never started and how to
/// finish them. A run with no failure prints each step's line and exits 0.
pub(super) fn report(plan: &Plan, global: &Path, outcomes: &[Outcome]) -> Render {
    let mut completed = Vec::new();
    let mut failure = None;
    for outcome in outcomes {
        match outcome {
            Ok(done) => completed.extend(written(plan, global, done)),
            Err(render) => failure = Some(render),
        }
    }
    let Some(failure) = failure else {
        if outcomes.is_empty() {
            return Render::line("Nothing changed: every answer kept the value in force.", 0);
        }
        let nothing_new = outcomes
            .iter()
            .all(|outcome| matches!(outcome, Ok(Done::Acknowledgement { recorded: false })));
        if nothing_new {
            completed.push("No file was changed.".to_owned());
        }
        return Render {
            lines: completed,
            code: 0,
            error: false,
        };
    };
    let mut lines = failure.lines.clone();
    lines.extend(completed);
    let not_started: Vec<&str> = steps(plan)
        .into_iter()
        .skip(outcomes.len())
        .map(unwritten)
        .collect();
    if !not_started.is_empty() {
        lines.push(format!("Not written: {}.", not_started.join(", ")));
    }
    lines.push("Run the interview again: it finishes only what is missing.".to_owned());
    Render {
        lines,
        code: failure.code,
        error: true,
    }
}

#[cfg(test)]
mod tests {
    use baley_core::catalog::Provider;
    use baley_core::policy::Value;
    use baley_core::policy::config_command::TypedPair;
    use serde_json::json;

    use super::*;
    use crate::config_command::interview::providers::Acknowledge;
    use crate::config_command::interview::writes::Hosts;
    use crate::install::hosts::Allowance;

    const SETTINGS: &str = "/home/o/.claude/settings.json";
    const GLOBAL: &str = "/c/config.toml";

    /// The four steps AC7 plans: the acknowledgement of `openai`,
    /// `api.openai.com` into the Claude settings file, `["host","openai"]` into
    /// the global file and one settings answer.
    fn four_steps() -> Plan {
        Plan {
            acknowledgement: Some(Ack::Requested(Acknowledge {
                providers: vec![Provider::OpenAi],
                owner: "Ada Owner".to_owned(),
            })),
            hosts: Some(Hosts {
                path: SETTINGS.into(),
                allowance: Allowance {
                    document: json!({}),
                    bytes: Vec::new(),
                    read_digest: None,
                    added: vec!["api.openai.com"],
                },
            }),
            reviewers: Some(TypedPair {
                name: "review.reviewers".to_owned(),
                host: None,
                value: Value::ReviewerList(vec!["host".to_owned(), "openai".to_owned()]),
            }),
            settings: vec![("roles.planner.effort".to_owned(), "max".to_owned())],
        }
    }

    fn recorded() -> Outcome {
        Ok(Done::Acknowledgement { recorded: true })
    }

    fn failed(lines: &[&str], code: u8) -> Outcome {
        Err(Render {
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            code,
            error: true,
        })
    }

    fn text(render: &Render) -> String {
        render.lines.join("\n")
    }

    #[test]
    fn a_step_run_after_a_failure_or_a_failure_without_the_completed_writes_is_caught() {
        let plan = four_steps();
        let denied =
            "not-writable: /home/o/.claude/.settings.json.7-0.tmp: Permission denied (os error 13)";

        let outcomes = [recorded(), failed(&[denied], 3)];
        assert_eq!(next(&plan, &outcomes[..0]), Some(Step::Acknowledgement));
        assert_eq!(next(&plan, &outcomes[..1]), Some(Step::Hosts));
        assert_eq!(next(&plan, &outcomes), None);
        let report = report(&plan, Path::new(GLOBAL), &outcomes);
        assert!(report.lines[0].contains("not-writable"), "{report:?}");
        assert!(
            report.lines[1..]
                .iter()
                .any(|line| line.contains("acknowledgement")
                    && line.contains("openai")
                    && line.contains("recorded")),
            "{report:?}"
        );
        let after = report.lines[1..].join("\n");
        assert!(after.contains("review.reviewers"), "{report:?}");
        assert!(after.contains("settings"), "{report:?}");
        assert_eq!((report.code, report.error), (3, true));

        let outcomes = [failed(&["ledger-busy: database is locked"], 3)];
        assert_eq!(next(&plan, &outcomes), None);
        let report = super::report(&plan, Path::new(GLOBAL), &outcomes);
        assert!(report.lines[0].contains("ledger-busy"), "{report:?}");
        let rest = report.lines[1..].join("\n");
        assert!(!rest.contains("was recorded"), "{report:?}");
        for unwritten in ["hosts", "review.reviewers", "settings"] {
            assert!(rest.contains(unwritten), "{unwritten}: {report:?}");
        }
        assert_eq!((report.code, report.error), (3, true));

        let conflict =
            "config-conflict: /c/config.toml changed since it was shown; run the interview again";
        let outcomes = [
            recorded(),
            Ok(Done::Hosts { note: None }),
            Ok(Done::Reviewers { version: None }),
            failed(&[conflict], 2),
        ];
        assert_eq!(next(&plan, &outcomes), None);
        let report = super::report(&plan, Path::new(GLOBAL), &outcomes);
        assert_eq!(report.lines[0], conflict);
        let rest = report.lines[1..].join("\n");
        assert!(rest.contains("openai"), "{report:?}");
        assert!(rest.contains("api.openai.com"), "{report:?}");
        assert!(rest.contains("review.reviewers"), "{report:?}");
        assert!(!text(&report).contains("Nothing was written"), "{report:?}");
        assert_eq!((report.code, report.error), (2, true));
    }

    /// The plan with the host step left out.
    fn without_hosts() -> Plan {
        Plan {
            hosts: None,
            ..four_steps()
        }
    }

    #[test]
    fn a_completed_run_reported_as_a_failure_or_missing_a_write_is_caught() {
        let plan = four_steps();
        let mut outcomes: Vec<Outcome> = Vec::new();
        let mut order = Vec::new();

        while let Some(step) = next(&plan, &outcomes) {
            order.push(step);
            outcomes.push(match step {
                Step::Acknowledgement => recorded(),
                Step::Hosts => Ok(Done::Hosts { note: None }),
                Step::Reviewers => Ok(Done::Reviewers { version: None }),
                Step::Settings => Ok(Done::Settings(vec![
                    "Wrote roles.planner.effort to /c/config.toml".to_owned(),
                ])),
            });
        }

        assert_eq!(
            order,
            [
                Step::Acknowledgement,
                Step::Hosts,
                Step::Reviewers,
                Step::Settings
            ]
        );
        let report = report(&plan, Path::new(GLOBAL), &outcomes);
        let all = text(&report);
        assert!(all.contains("acknowledgement of openai"), "{all}");
        assert!(all.contains("api.openai.com"), "{all}");
        assert!(all.contains(SETTINGS), "{all}");
        assert!(all.contains("review.reviewers"), "{all}");
        assert!(all.contains("[\"host\", \"openai\"]"), "{all}");
        assert!(all.contains("Wrote roles.planner.effort"), "{all}");
        assert_eq!((report.code, report.error), (0, false));
    }

    #[test]
    fn a_settings_refusal_after_provider_writes_claiming_nothing_was_written_is_caught() {
        let plan = without_hosts();
        let refusal =
            r#"unknown-model: roles.planner.model is "x", which no host's catalog accepts"#;
        let outcomes = [
            recorded(),
            Ok(Done::Reviewers { version: None }),
            failed(&[refusal], 2),
        ];

        assert_eq!(next(&plan, &outcomes), None);
        let report = report(&plan, Path::new(GLOBAL), &outcomes);

        assert_eq!(report.lines[0], refusal);
        let rest = report.lines[1..].join("\n");
        assert!(rest.contains("acknowledgement of openai"), "{report:?}");
        assert!(rest.contains("review.reviewers"), "{report:?}");
        assert!(rest.contains("[\"host\", \"openai\"]"), "{report:?}");
        assert!(!text(&report).contains("Nothing was written"), "{report:?}");
        assert_eq!((report.code, report.error), (2, true));
    }

    #[test]
    fn a_replaced_global_file_left_out_of_a_policy_step_failure_report_is_caught() {
        let plan = without_hosts();
        let note = "wrote /c/config.toml, and the change stands";
        let outcomes = [
            recorded(),
            failed(&["ledger-busy: database is locked", note], 3),
        ];

        assert_eq!(next(&plan, &outcomes), None);
        let report = report(&plan, Path::new(GLOBAL), &outcomes);

        assert!(report.lines[0].starts_with("ledger-busy:"), "{report:?}");
        assert!(report.lines.contains(&note.to_owned()), "{report:?}");
        let all = text(&report);
        assert!(all.contains("acknowledgement of openai"), "{all}");
        assert!(!all.contains("Not written: review.reviewers"), "{all}");
        assert_eq!((report.code, report.error), (3, true));
    }
}
