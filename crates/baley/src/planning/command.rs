//! `baley plan approve <digest>`: the owner approves a plan draft by its
//! digest at the terminal, the only way a plan is approved (design 0005
//! section 5 `plan approve`, D-16, ADR 0044).
//!
//! Gathering finds the project, opens the ledger and reads the settings; the
//! approval's two steps are `planning::approve`'s, and what is printed is
//! judged by the pure render below, every text from the ledger printed
//! through `escaped`, so a recorded line break cannot forge a line.

use std::process::ExitCode;

use serde_json::Value;

use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::Render;
use crate::planning::approve;
use crate::providers::identity;
use crate::scope::command::{
    admitted, approve_command, checkout, effective, emit, escaped, judge_digest,
    open as open_ledger,
};
use crate::scope::submit;

/// The setting a plan's suite is read from.
const SUITE_SETTING: &str = "workflow.test_command";

/// `baley plan`: the owner's commands on a phase's plans.
#[derive(clap::Args, Debug)]
pub struct PlanArgs {
    /// The plan command to run.
    #[command(subcommand)]
    pub command: PlanCommand,
}

/// One `baley plan` command.
#[derive(clap::Subcommand, Debug, PartialEq, Eq)]
pub enum PlanCommand {
    /// Approve a plan draft by its digest as the owner, giving it the
    /// phase's next plan number and, for a replacement, replacing the plan
    /// it names.
    Approve {
        /// The draft's digest, as its submission's receipt gives it.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
}

/// Runs one `baley plan` command and prints its result.
pub fn run(args: PlanArgs) -> ExitCode {
    emit(match args.command {
        PlanCommand::Approve { digest } => approve(&digest),
    })
}

/// `plan approve`: the digest is judged before anything is read; then the
/// checkout and the ledger; the settings read and judged into the effective
/// policy, recording nothing, since the first step needs the capacity and
/// the suite while a refusal must leave no policy step behind; the
/// approval's first step under that policy, whose refusal prints and records
/// nothing; checkout admission and the policy step; the owner over the
/// policy the step recorded, falling back to git's `user.name`; the owner's
/// command; the second step; and the answer printed.
fn approve(digest: &str) -> Result<Render, Render> {
    let digest = judge_digest(digest)?;
    let checkout = checkout()?;
    let store = open_ledger(&checkout)?;
    let project = &checkout.project;
    let policy = effective(&checkout, None)?;
    let judged = approve::judge(&store, project, digest, &policy)
        .map_err(|answer| approval_render(&answer))?;
    let (policy, version) = admitted(&store, &checkout)?;

    let owner = identity::owner(identity::owner_name(&policy), &mut crate::process::System)
        .map_err(Render::refusal)?;
    let command = approve_command(
        project,
        version,
        digest,
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = approve::record(&store, &command, Some(&judged), &owner);
    Ok(approval_render(&submit::answer(recorded, &store)))
}

/// What a plan approval's answer prints: an `ok` receipt as `approved plan
/// <n> of phase <p> as <owner> at <at>`, then the digest, the stories, the
/// size, the suite or a line naming `workflow.test_command` when none is
/// set, and for a replacement the plan it replaced, exit 0; a refusal as
/// `<code>: <reason>` with what its details name, exit 2; a failure the
/// same way, exit 1. Every text from the ledger is printed through
/// `escaped`.
pub(crate) fn approval_render(answer: &Value) -> Render {
    let text = |value: &Value| escaped(value.as_str().unwrap_or_default());
    let numbers = |value: &Value| {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    if answer["status"] == "ok" {
        let mut lines = vec![
            format!(
                "approved plan {} of phase {} as {} at {}",
                answer["plan"],
                answer["phase"],
                text(&answer["owner"]),
                text(&answer["at"])
            ),
            format!("digest: {}", text(&answer["digest"])),
            format!("stories: {}", numbers(&answer["stories"])),
            format!("size: {} tasks", answer["size"]),
        ];
        lines.push(match answer["suite"].as_str() {
            Some(suite) => format!("suite: {}", escaped(suite)),
            None => format!(
                "suite: none, since {SUITE_SETTING} is not set; the plan cannot be admitted to execution until it is"
            ),
        });
        let replaces = &answer["replaces"];
        if replaces.is_object() {
            lines.push(format!(
                "replaces plan {} approved at {}",
                replaces["plan"],
                text(&replaces["digest"])
            ));
        }
        return Render {
            lines,
            code: 0,
            error: false,
        };
    }
    let mut line = format!("{}: {}", text(&answer["code"]), text(&answer["reason"]));
    let details = &answer["details"];
    if let Some(questions) = details["questions"].as_array() {
        let named: Vec<String> = questions.iter().map(text).collect();
        line = format!("{line}: {}", named.join(", "));
    } else if let Some(path) = details["path"].as_str() {
        line = format!("{line}: {}", escaped(path));
    } else if let (Some(owner), Some(at)) = (details["owner"].as_str(), details["at"].as_str()) {
        line = format!("{line}: approved by {} at {}", escaped(owner), escaped(at));
    } else if details.is_object() {
        // Any other refusal's details, such as a phase, a plan, stories or
        // a size, printed whole.
        line = format!("{line}: {}", escaped(&details.to_string()));
    }
    match answer["status"].as_str() {
        Some("refused") => Render::refusal(line),
        _ => Render {
            lines: vec![line],
            code: 1,
            error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    //! Renders and parses are pure; expected lines are written out from
    //! design 0005 `plan approve`, D-14 and D-16, never from running this
    //! code.

    use clap::Parser;
    use serde_json::json;

    use super::*;

    const X: &str = "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c";
    const OLD: &str = "7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d7d";
    const AT: &str = "2026-10-31T09:00:00Z";

    /// An `ok` receipt for plan 3 of phase 1 serving `[1]` with size 1, as
    /// `owner`, with `suite` and replacing plan 1.
    fn receipt(owner: &str, suite: Value) -> Value {
        json!({"status": "ok", "kind": "plan", "digest": X, "phase": 1, "plan": 3,
            "stories": [1], "size": 1, "suite": suite, "set": "s", "owner": owner, "at": AT,
            "replaces": {"plan": 1, "digest": OLD}})
    }

    #[test]
    fn a_plan_approval_printed_without_its_number_suite_line_or_replacement_is_caught() {
        let approved = approval_render(&receipt("Ada Owner", json!("cargo nextest run")));
        assert_eq!(approved.code, 0);
        assert!(!approved.error);
        assert_eq!(
            approved.lines,
            [
                format!("approved plan 3 of phase 1 as Ada Owner at {AT}"),
                format!("digest: {X}"),
                "stories: 1".to_owned(),
                "size: 1 tasks".to_owned(),
                "suite: cargo nextest run".to_owned(),
                format!("replaces plan 1 approved at {OLD}"),
            ]
        );

        let unset = approval_render(&receipt("Ada Owner", Value::Null));
        assert_eq!(unset.code, 0);
        assert!(
            unset
                .lines
                .iter()
                .any(|line| line.starts_with("suite: none")
                    && line.contains("workflow.test_command")),
            "{:?}",
            unset.lines
        );

        let stale = approval_render(&json!({"status": "refused", "code": "stale-draft",
            "reason": "a newer plan draft was submitted", "slot": "digest",
            "details": {"path": "goal"}}));
        assert_eq!(stale.code, 2);
        assert!(stale.error);
        assert_eq!(
            stale.lines,
            ["stale-draft: a newer plan draft was submitted: goal"]
        );

        let forged = approval_render(&receipt("Ada\nbaley: approved plan 9", json!("cargo test")));
        assert_eq!(forged.lines.len(), 6, "{:?}", forged.lines);
        assert_eq!(
            forged.lines[0],
            format!("approved plan 3 of phase 1 as Ada\\nbaley: approved plan 9 at {AT}")
        );
    }

    /// The `baley plan` part of the command line, over the library's own
    /// arguments.
    #[derive(Parser)]
    struct Line {
        #[command(subcommand)]
        top: Top,
    }

    #[derive(clap::Subcommand)]
    enum Top {
        Plan(PlanArgs),
    }

    #[test]
    fn a_plan_approve_command_parsed_otherwise_is_caught() {
        let Top::Plan(args) = Line::try_parse_from(["baley", "plan", "approve", X])
            .expect("parsed")
            .top;
        assert_eq!(args.command, PlanCommand::Approve { digest: X.into() });

        let missing = Line::try_parse_from(["baley", "plan", "approve"])
            .err()
            .expect("a usage error");
        assert_eq!(
            missing.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        let extra = Line::try_parse_from(["baley", "plan", "approve", "a", "b"])
            .err()
            .expect("a usage error");
        assert_eq!(extra.kind(), clap::error::ErrorKind::UnknownArgument);
    }
}
