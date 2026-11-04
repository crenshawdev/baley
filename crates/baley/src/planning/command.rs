//! `baley plan approve <digest>`: the owner approves a plan draft by its
//! digest at the terminal, the only way a plan is approved (design 0005
//! section 5 `plan approve`, D-16, ADR 0044). `baley plan show <phase>`:
//! the owner reads a phase's approved plans with their evidence maps,
//! readiness and checker rounds, through the read `plan-read` serves (D-04,
//! D-17), recording nothing.
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
use crate::planning::read::{self, CheckerRound, PhasePlans};
use crate::providers::identity;
use crate::scope::command::{
    admitted, approve_command, checkout, effective, emit, escaped, judge_digest, judge_number,
    open as open_ledger, read_error, reading,
};
use crate::scope::read::ReadError;
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
    /// Show a phase's approved plans in plan-number order, replaced ones
    /// included, each with its evidence map, whether its items still name
    /// their truths' current versions, and its checker rounds' findings.
    Show {
        /// The phase's number, as the roadmap gives it.
        #[arg(value_name = "PHASE")]
        phase: String,
    },
}

/// Runs one `baley plan` command and prints its result.
pub fn run(args: PlanArgs) -> ExitCode {
    emit(match args.command {
        PlanCommand::Approve { digest } => approve(&digest),
        PlanCommand::Show { phase } => show(&phase),
    })
}

/// `plan approve`: the digest is judged before anything is read; then the
/// checkout and the ledger; the settings read and judged into the effective
/// policy, recording nothing, since the first step needs the capacity and
/// the suite while a refusal must leave no policy step behind; the
/// approval's first step under that policy, whose refusal prints and records
/// nothing; checkout admission and the policy step, which reads the settings
/// again; the judgement held to the policy the step recorded, judged again
/// when its capacity or suite differs from the first read's, so the
/// approval never names one read's version over the other read's judgement
/// (a refusal there leaves only the policy step behind); the owner over the
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
    let judged = approve::rejudged(&store, project, judged, &policy)
        .map_err(|answer| approval_render(&answer))?;

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

/// `plan show`: the phase number is judged before anything is read; then
/// the phase's approved plans are read, recording nothing (ADR 0044).
fn show(phase: &str) -> Result<Render, Render> {
    let phase = judge_number(phase)?;
    reading(|store, project| show_render(read::read(store, project, phase)))
}

/// What `baley plan show` prints for a phase's plans read: each plan as
/// `plan <n> at <digest>: <state>`, the state `replaced by <digest>` for
/// a replaced one, then `ready` or `not ready:` and the items bound to a
/// version that is not current; then its stories, its size and its suite or
/// `suite: none`; then one line per item as `item <id>: story <s>, truth
/// <t> version <v>, <kind>`, with `level <level>` for a check; then its
/// checker rounds by [`round_lines`]. A phase with no approved plan prints one
/// line saying so. A refusal prints `<code>: <reason>`, exit 2, and a failure
/// exit 1. Every text from the ledger is printed through `escaped`.
pub(crate) fn show_render(read: Result<PhasePlans, ReadError>) -> Render {
    let plans = match read {
        Ok(plans) => plans,
        Err(error) => return read_error(error),
    };
    let text = |value: &Value| escaped(value.as_str().unwrap_or_default());
    let mut lines = Vec::new();
    if plans.plans.is_empty() {
        lines.push(format!("phase {} has no approved plan", plans.phase));
    }
    for plan in &plans.plans {
        let state = match &plan.replaced_by {
            Some(by) => format!("{} by {}", escaped(&plan.state), escaped(by)),
            None => escaped(&plan.state),
        };
        let readiness = if plan.readiness.ready {
            "ready".to_owned()
        } else {
            let named: Vec<String> = plan
                .readiness
                .not_ready
                .iter()
                .map(|id| escaped(id))
                .collect();
            format!("not ready: {}", named.join(", "))
        };
        lines.push(format!(
            "plan {} at {}: {state}, {readiness}",
            plan.plan,
            escaped(&plan.digest)
        ));
        let stories: Vec<String> = plan.stories.iter().map(u32::to_string).collect();
        lines.push(format!("stories: {}", stories.join(", ")));
        lines.push(format!("size: {} tasks", plan.size));
        lines.push(match &plan.suite {
            Some(suite) => format!("suite: {}", escaped(suite)),
            None => "suite: none".to_owned(),
        });
        for item in &plan.items {
            let mut line = format!(
                "item {}: story {}, truth {} version {}, {}",
                text(&item["id"]),
                item["story"],
                text(&item["truth"]),
                item["truth_version"],
                text(&item["kind"])
            );
            if item["kind"] == "check" {
                line = format!("{line}, level {}", text(&item["spec"]["level"]));
            }
            lines.push(line);
        }
        lines.extend(round_lines(&plan.checker_rounds));
    }
    Render {
        lines,
        code: 0,
        error: false,
    }
}

/// A plan's checker rounds as the owner reads them under it, none printing
/// nothing: each round as `checker round <r>, scope <scope>`, each of its
/// findings as `<severity> at <location>: <claim>` and `fix: <fix>`. Findings
/// refuse nothing here (D-04). A checker's words reach the terminal only
/// through `escaped`, so a line break in a finding cannot forge a line.
pub(crate) fn round_lines(rounds: &[CheckerRound]) -> Vec<String> {
    let text = |value: &Value| escaped(value.as_str().unwrap_or_default());
    let mut lines = Vec::new();
    for round in rounds {
        lines.push(format!(
            "checker round {}, scope {}",
            round.round,
            escaped(&round.scope)
        ));
        for finding in &round.findings {
            lines.push(format!(
                "  {} at {}: {}",
                text(&finding["severity"]),
                text(&finding["location"]),
                text(&finding["claim"])
            ));
            lines.push(format!("    fix: {}", text(&finding["fix"])));
        }
    }
    lines
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

    #[test]
    fn a_plan_shown_without_its_levels_versions_or_readiness_is_caught() {
        use crate::planning::fixtures::{e1, e2};
        use crate::planning::read::{ApprovedPlan, Readiness};

        const Y: &str = "6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d";
        let plan = |number: u32, digest: &str, suite: Option<&str>| ApprovedPlan {
            plan: number,
            digest: digest.into(),
            stories: vec![1],
            size: 1,
            suite: suite.map(str::to_owned),
            set: "s".into(),
            owner: "Ada Owner".into(),
            at: AT.into(),
            state: "approved".into(),
            replaced_by: None,
            readiness: Readiness {
                ready: true,
                not_ready: Vec::new(),
            },
            items: vec![e1(), e2()],
            checker_rounds: Vec::new(),
        };
        let mut one = plan(1, X, None);
        one.state = "replaced".into();
        one.replaced_by = Some(Y.into());
        let mut two = plan(2, OLD, Some("cargo test\nplan 9 at x: approved, ready"));
        two.readiness = Readiness {
            ready: false,
            not_ready: vec!["e1".into()],
        };
        let shown = show_render(Ok(PhasePlans {
            phase: 1,
            plans: vec![one, two],
        }));
        assert_eq!(shown.code, 0);
        assert!(!shown.error);
        assert_eq!(
            shown.lines[..6],
            [
                format!("plan 1 at {X}: replaced by {Y}, ready"),
                "stories: 1".to_owned(),
                "size: 1 tasks".to_owned(),
                "suite: none".to_owned(),
                "item e1: story 1, truth t1 version 1, check, level unit".to_owned(),
                "item e2: story 1, truth t2 version 1, check, level integration".to_owned(),
            ]
        );
        assert_eq!(
            shown.lines[6],
            format!("plan 2 at {OLD}: approved, not ready: e1")
        );
        assert_eq!(
            shown.lines[9],
            "suite: cargo test\\nplan 9 at x: approved, ready"
        );
        assert_eq!(shown.lines.len(), 12, "{:?}", shown.lines);
    }

    #[test]
    fn a_plan_shown_without_its_checker_findings_is_caught() {
        use crate::planning::read::tests::approved_through_two_rounds;
        use crate::scope::fixtures::project;

        let (_dir, store) = approved_through_two_rounds();
        let shown = show_render(read::read(&store, &project(), 1));
        assert_eq!(shown.code, 0);
        let rounds = shown
            .lines
            .iter()
            .position(|line| line.starts_with("checker round"))
            .expect("a checker round is printed");
        assert_eq!(
            shown.lines[rounds..],
            [
                "checker round 1, scope full",
                "  blocker at tasks[0]: No task makes truth t2 of story 1 true",
                "    fix: Add a task that records the round",
                "  warning at tasks[0].verify[0]: The verify runs every ledger test",
                "    fix: Name the round test",
                "checker round 2, scope full",
                "  warning at tasks[0].verify[0]: The verify runs every ledger test",
                "    fix: Name the round test",
            ]
        );

        let forged = round_lines(&[CheckerRound {
            round: 1,
            scope: "full".into(),
            findings: vec![
                serde_json::json!({"severity": "warning", "location": "tasks[0]",
                "claim": "fine\nchecker round 9, scope full", "fix": "none\nplan 9"}),
            ],
        }]);
        assert_eq!(
            forged,
            [
                "checker round 1, scope full",
                "  warning at tasks[0]: fine\\nchecker round 9, scope full",
                "    fix: none\\nplan 9",
            ]
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
