//! The owner's scope commands (design 0004 sections 5 and 6, D-20): `baley
//! scope approve <digest>` approves the newest draft of a start or a change
//! as the owner, the only way scope is approved (D-24); `baley phase
//! reorder|withdraw`, `baley story reassign|drop` and `baley backlog reorder`
//! submit the id-only changes as drafts the owner then approves; and `baley
//! scope preview|show`, `baley phase show`, `baley backlog show` and `baley
//! story show` print what the ledger holds through the same reads as
//! `document`, recording nothing. Declarations, edits and `project describe`
//! come only from a session, since an agent drafts their text. `baley story
//! truths approve <digest>` and `baley story refinement <id>` are
//! `refine::command`'s: the first approves a story's truth draft as `scope
//! approve` approves a scope draft, the second prints a story's refinement.
//! Approving a commitment, the `phase edit` a session's `phase-plan` drafted
//! for a changed selection, also issues the phase's planner order, routed for
//! Claude Code, and prints its id (D-01).
//!
//! Gathering finds the project and opens the ledger; judging, the arguments,
//! the commands an approval or a submission is recorded under and what is
//! printed, is done by the pure functions below. Every text from the ledger
//! is printed as `escape_debug` writes it, quotes left plain, so a recorded
//! line break cannot forge a line.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;

use baley_core::capture::NO_SUCH_PHASE;
use baley_core::policy::recorded::recorded_policy;
use baley_core::policy::{CONFIG_UNAVAILABLE, EffectivePolicy, Host, SettingsFile};
use baley_core::scope::{
    ChangeKind, ChangeSubmission, DRAFT_APPROVED, Destination, START_KIND, ScopeChange,
    UNKNOWN_DRAFT, judge_change, judge_submission,
};
use baley_core::truth::{STORY_TRUTHS, TruthSubmission, Versions, judge_truths, versions};
use baley_store::{
    Actor, Admin, CanonicalError, Command, CommandKind, Hash, ProjectId, RequestId, request_digest,
};
use baley_store_sqlite::SqliteStore;
use serde_json::{Value, json};

use crate::checkout::{Site, gather_and_admit};
use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, NOT_A_PROJECT, PROJECT_NOT_IN_LEDGER};
use crate::mcp::scope::change_kind;
use crate::providers::identity;
use crate::refine::command::{sentence, truth_detail};
use crate::refine::submit::{current_truths, mark_str};
use crate::scope::approve::{self, APPROVE_COMMAND, OrderRoute};
use crate::scope::read::{
    self, BacklogStory, Detail, Draft, NO_SUCH_STORY, Phase, Preview, PreviewError, ReadError,
    Roadmap, Story,
};
use crate::scope::submit::{self, SUBMIT_COMMAND};
use crate::work_order::route::host_names;
use crate::{init, policy_step, settings};

/// `baley scope`: the owner's approval of scope, and its reads.
#[derive(clap::Args, Debug)]
pub struct ScopeArgs {
    #[command(subcommand)]
    command: ScopeCommand,
}

#[derive(clap::Subcommand, Debug)]
enum ScopeCommand {
    /// Approve the newest draft of a start or a change by its digest, as the
    /// owner. Phase numbers and story ids a start or a declaration gets are
    /// assigned now, and the phases and stories it changed are printed.
    Approve {
        /// The draft's digest from its submission's receipt: 64 lowercase
        /// hexadecimal characters.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
    /// Print a draft by its digest, exactly as an approval would record it.
    Preview {
        /// The draft's digest from its submission's receipt.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
    /// Print the accepted roadmap: the description, phases and stories.
    Show,
}

/// `baley phase`: the owner's reads of accepted phases, and the phase
/// changes the owner submits.
#[derive(clap::Args, Debug)]
pub struct PhaseArgs {
    #[command(subcommand)]
    command: PhaseCommand,
}

#[derive(clap::Subcommand, Debug)]
enum PhaseCommand {
    /// Print one accepted phase with its detail text.
    Show {
        /// The phase number Baley assigned at approval.
        #[arg(value_name = "NUMBER")]
        number: String,
    },
    /// Submit a new phase order as a draft, which the owner approves with
    /// baley scope approve.
    Reorder {
        /// Every phase in the roadmap's order, each once, in the new order.
        #[arg(value_name = "NUMBER", required = true)]
        numbers: Vec<String>,
    },
    /// Submit a phase's withdrawal as a draft, which the owner approves with
    /// baley scope approve.
    Withdraw {
        /// The phase to withdraw.
        #[arg(value_name = "NUMBER")]
        number: String,
        /// One decision for each story the phase serves: STORY=PHASE moves
        /// the story to that phase, STORY=drop drops it.
        #[arg(value_name = "DECISION")]
        decisions: Vec<String>,
    },
}

/// `baley backlog`: the owner's read of the backlog, and its reorder.
#[derive(clap::Args, Debug)]
pub struct BacklogArgs {
    #[command(subcommand)]
    command: BacklogCommand,
}

#[derive(clap::Subcommand, Debug)]
enum BacklogCommand {
    /// Print every accepted story in priority order.
    Show,
    /// Submit a new priority order as a draft, which the owner approves with
    /// baley scope approve.
    Reorder {
        /// Every active and deferred story's id, each once, in the new
        /// priority order.
        #[arg(value_name = "ID", required = true)]
        ids: Vec<String>,
    },
}

/// `baley story`: the owner's reads of accepted stories, and the story
/// changes the owner submits.
#[derive(clap::Args, Debug)]
pub struct StoryArgs {
    #[command(subcommand)]
    pub(crate) command: StoryCommand,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum StoryCommand {
    /// Print one accepted story.
    Show {
        /// The story id Baley assigned at approval.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Submit a story's move to another phase or the backlog as a draft,
    /// which the owner approves with baley scope approve.
    Reassign {
        /// The story to move.
        #[arg(value_name = "ID")]
        id: String,
        /// The phase number that serves it now, or backlog.
        #[arg(value_name = "PHASE")]
        to: String,
    },
    /// Submit a story's drop as a draft, which the owner approves with baley
    /// scope approve.
    Drop {
        /// The story to drop.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Print a story's refinement: the approved description, the story and
    /// the phase it is committed to, and its truths with every version.
    Refinement {
        /// The story id Baley assigned at approval.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Approve a story's drafted truths.
    Truths(crate::refine::command::TruthsArgs),
}

/// Runs one `baley scope` command and prints its result.
pub fn run(args: ScopeArgs) -> ExitCode {
    emit(match args.command {
        ScopeCommand::Approve { digest } => approve(&digest),
        ScopeCommand::Preview { digest } => judge_digest(&digest)
            .and_then(|digest| reading(|store, project| preview(store, project, digest))),
        ScopeCommand::Show => {
            reading(|store, project| roadmap_render(read::roadmap(store, project)))
        }
    })
}

/// Runs one `baley phase` command and prints its result.
pub fn run_phase(args: PhaseArgs) -> ExitCode {
    emit(match args.command {
        PhaseCommand::Show { number } => judge_number(&number).and_then(|number| {
            reading(|store, project| phase_render(read::phase(store, project, number)))
        }),
        PhaseCommand::Reorder { numbers } => submit_change(&ChangeWords::PhaseReorder(numbers)),
        PhaseCommand::Withdraw { number, decisions } => {
            submit_change(&ChangeWords::PhaseWithdraw {
                phase: number,
                decisions,
            })
        }
    })
}

/// Runs one `baley backlog` command and prints its result.
pub fn run_backlog(args: BacklogArgs) -> ExitCode {
    emit(match args.command {
        BacklogCommand::Show => {
            reading(|store, project| backlog_render(read::backlog(store, project)))
        }
        BacklogCommand::Reorder { ids } => submit_change(&ChangeWords::BacklogReorder(ids)),
    })
}

/// Runs one `baley story` command and prints its result.
pub fn run_story(args: StoryArgs) -> ExitCode {
    emit(match args.command {
        StoryCommand::Show { id } => judge_number(&id)
            .and_then(|id| reading(|store, project| story_render(read::story(store, project, id)))),
        StoryCommand::Reassign { id, to } => {
            submit_change(&ChangeWords::StoryReassign { story: id, to })
        }
        StoryCommand::Drop { id } => submit_change(&ChangeWords::StoryDrop(id)),
        StoryCommand::Refinement { id } => crate::refine::command::refinement(&id),
        StoryCommand::Truths(args) => crate::refine::command::run(args),
    })
}

pub(crate) fn emit(result: Result<Render, Render>) -> ExitCode {
    ExitCode::from(display::emit(
        &result.unwrap_or_else(|e| e),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}

/// Finds the project and opens the ledger only to read: no checkout
/// admission, no policy step and nothing recorded, as `document` records
/// nothing.
pub(crate) fn reading(
    render: impl FnOnce(&SqliteStore, &ProjectId) -> Render,
) -> Result<Render, Render> {
    let checkout = checkout()?;
    let store = open(&checkout)?;
    Ok(render(&store, &checkout.project))
}

/// The checkout a command runs in: its project, repository root and
/// working-tree `baley.toml`, with Baley's folders.
pub(crate) struct Checkout {
    folders: Folders,
    pub(crate) project: ProjectId,
    root: PathBuf,
    working: Option<SettingsFile>,
}

/// Finds the project from the working directory through its `baley.toml`.
/// Outside a project the command is refused, naming `baley init`.
pub(crate) fn checkout() -> Result<Checkout, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read())
        .map_err(|e| Render::refusal(e.to_string()))?;
    let unavailable = |e: std::io::Error| {
        display::store_error(&baley_store::StoreError::Unavailable(e.to_string()), None)
    };
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let Discovery::Managed { folder, root } = discovery::discover(&ancestors) else {
        return Err(Render::refusal(format!(
            "{NOT_A_PROJECT}: project commands run in a checkout of a project, and no {PROJECT_FILE} was found in this repository; baley init creates one"
        )));
    };
    let path = folder.join(PROJECT_FILE);
    let read = settings::read(&path);
    let project = match init::observe_file(read.clone()) {
        Ok(Some(identity)) => ProjectId(identity.id),
        Ok(None) => {
            return Err(Render::refusal(format!(
                "{CONFIG_UNAVAILABLE}: {} was not found",
                path.display()
            )));
        }
        Err(unavailable) => return Err(Render::refusal(unavailable.to_string())),
    };
    Ok(Checkout {
        folders,
        project,
        root,
        working: read.ok().flatten(),
    })
}

/// Opens the ledger and checks it lists the checkout's project.
pub(crate) fn open(checkout: &Checkout) -> Result<SqliteStore, Render> {
    let store = crate::ledger::open::store(
        &checkout.folders.home,
        &SystemClock::now(),
        crate::ledger::open::options(),
    )
    .map_err(|e| display::store_error(&e, None))?;
    let listed = store
        .projects()
        .map_err(|e| display::store_error(&e, Some(&checkout.project.0)))?;
    if !listed.iter().any(|(id, _)| *id == checkout.project) {
        return Err(Render::refusal(format!(
            "{PROJECT_NOT_IN_LEDGER}: project {} is not in this machine's ledger; run baley init in this checkout",
            checkout.project.0
        )));
    }
    Ok(store)
}

/// `scope approve`: the digest is judged before anything is read, and the
/// approval's first step runs before checkout admission and the policy step,
/// so a refused approval records nothing at all. The owner is read over the
/// policy the policy step recorded, and the second step records the
/// approval under the version in force.
///
/// A commitment's approval issues its planner order (D-01), so its policy
/// step records Claude Code's effective policy, as a session's write records
/// its host's, and the order is routed for Claude Code with its accepted
/// model names. Claude Code is the only host (`Host::ALL`); a second host
/// needs a host choice for an order the terminal issues.
fn approve(digest: &str) -> Result<Render, Render> {
    let digest = judge_digest(digest)?;
    let checkout = checkout()?;
    let store = open(&checkout)?;
    let project = &checkout.project;
    let judged =
        approve::judge(&store, project, digest).map_err(|answer| answer_render(&answer))?;
    let [host] = Host::ALL;
    let (policy, version) = match judged.planner_order() {
        Some(_) => host_admitted(&store, &checkout, host)?,
        None => admitted(&store, &checkout)?,
    };

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
    if judged.planner_order().is_none() {
        let recorded = approve::record(&store, &command, Some(&judged), &owner);
        return Ok(answer_render(&submit::answer(recorded, &store)));
    }
    let names = host_names(&store, host).map_err(|e| display::store_error(&e, Some(&project.0)))?;
    let route = OrderRoute {
        policy: &policy,
        host,
        names: &names,
    };
    Ok(answer_render(&approve::record_commitment(
        &store, &command, &judged, &owner, route,
    )))
}

/// One of the owner's change commands: the words are judged before anything
/// is read, and the change is checked against accepted scope before checkout
/// admission and the policy step, so a refused change records nothing at all.
/// It is then recorded as the owner under the version in force, in a
/// transaction that confirms the scope the check read has not moved (D-09).
fn submit_change(words: &ChangeWords) -> Result<Render, Render> {
    let change = command_change(words)?;
    let checkout = checkout()?;
    let store = open(&checkout)?;
    let project = &checkout.project;
    let read = submit::check_change(&store, project, &change)
        .map_err(|answer| submission_render(&answer))?;
    let (_, version) = admitted(&store, &checkout)?;

    let command = submit_command(
        project,
        version,
        change.digest(),
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = submit::record_change(&store, &command, &change, Some(&read));
    Ok(submission_render(&submit::answer(recorded, &store)))
}

/// Checkout admission and then the policy step, in the order every owner
/// command that records scope runs them, so approvals and submissions cannot
/// drift apart. It gives the policy built with no host, which names the
/// owner, and the version the step returned.
pub(crate) fn admitted(
    store: &SqliteStore,
    checkout: &Checkout,
) -> Result<(EffectivePolicy, u64), Render> {
    stepped(store, checkout, effective(checkout, None)?)
}

/// [`admitted`] for an owner command that issues an order for `host`: the
/// policy step records `host`'s effective policy, so the order's route names
/// a recorded policy version.
pub(crate) fn host_admitted(
    store: &SqliteStore,
    checkout: &Checkout,
    host: Host,
) -> Result<(EffectivePolicy, u64), Render> {
    stepped(store, checkout, effective(checkout, Some(host))?)
}

/// The checkout's effective policy for `host`, or with none, read from both
/// settings files and judged, recording nothing, so a command can judge
/// against it before it records.
pub(crate) fn effective(
    checkout: &Checkout,
    host: Option<Host>,
) -> Result<EffectivePolicy, Render> {
    let reads = policy_step::gather(
        &checkout.folders.config,
        &checkout.root,
        checkout.working.as_ref(),
        &mut crate::process::System,
    );
    policy_step::build(&reads, host).map_err(|e| Render::refusal(e.to_string()))
}

/// Checkout admission, then the policy step recording `policy`. Gives the
/// policy and the version the step returned.
fn stepped(
    store: &SqliteStore,
    checkout: &Checkout,
    policy: EffectivePolicy,
) -> Result<(EffectivePolicy, u64), Render> {
    let project = &checkout.project;
    let recorded =
        recorded_policy(&checkout.root, &policy).map_err(|e| Render::refusal(e.to_string()))?;
    let site = Site {
        root: &checkout.root,
        policy: &policy,
        path: &recorded.checkout,
    };
    gather_and_admit(
        store,
        project,
        &site,
        &mut crate::process::System,
        new_request_id(),
        &SystemClock::now(),
        None,
    )
    .map_err(|e| display::entry_error(&e, &project.0))?;
    let version = policy_step::step(
        store,
        project,
        &recorded,
        new_request_id(),
        &SystemClock::now(),
        None,
    )
    .map_err(|e| display::store_error(&e, Some(&project.0)))?;
    Ok((policy, version))
}

/// The digest argument: exactly 64 lowercase hexadecimal characters, as
/// a submission's receipt writes it. Anything else is refused before the
/// ledger is read, and the refusal echoes none of it.
pub(crate) fn judge_digest(text: &str) -> Result<Hash, Render> {
    let lowercase_hex =
        text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    lowercase_hex.then(|| Hash::from_hex(text)).flatten().ok_or_else(|| {
        Render::refusal(
            "a draft digest is 64 lowercase hexadecimal characters, as a submission's receipt gives it",
        )
    })
}

/// The command an approval is recorded under: the owner as actor, no caller,
/// Baley's clock and `policy_version` in both its digest and itself, as
/// `purge` builds its own.
pub(crate) fn approve_command(
    project: &ProjectId,
    policy_version: u64,
    digest: Hash,
    request_id: RequestId,
    at: &str,
) -> Result<Command, CanonicalError> {
    owner_command(
        APPROVE_COMMAND,
        project,
        policy_version,
        digest,
        request_id,
        at,
    )
}

/// The command a command-line submission is recorded under, built as an
/// approval's is: the owner as actor, no caller, Baley's clock and
/// `policy_version` in both its digest and itself. The version is safe in the
/// digest because a command-line request id is always fresh.
pub(crate) fn submit_command(
    project: &ProjectId,
    policy_version: u64,
    digest: Hash,
    request_id: RequestId,
    at: &str,
) -> Result<Command, CanonicalError> {
    owner_command(
        SUBMIT_COMMAND,
        project,
        policy_version,
        digest,
        request_id,
        at,
    )
}

/// The owner's command of `kind` over `digest`, a draft's or an answer
/// call's.
pub(crate) fn owner_command(
    kind: &str,
    project: &ProjectId,
    policy_version: u64,
    digest: Hash,
    request_id: RequestId,
    at: &str,
) -> Result<Command, CanonicalError> {
    let request = request_digest(&json!({
        "kind": kind,
        "project": project.0,
        "actor": "owner",
        "policy_version": policy_version,
        "digest": digest.to_hex(),
        "scope": [],
    }))?;
    Ok(Command {
        project: project.clone(),
        kind: CommandKind(kind.into()),
        request_id,
        digest: request,
        scope: Vec::new(),
        policy_version,
        recorded_at: at.into(),
        actor: Actor::Owner,
        caller: None,
    })
}

/// What the approval's answer prints, a start's or a change's: the receipt's
/// three lines, an empty list as `none`, exit 0; a refusal as `<code>:
/// <reason>`, exit 2, a stale draft's naming the first part that differs, a
/// malformed submission's naming its section and field; a failure the same
/// way, exit 1. A commitment's receipt, or a `draft-approved` refusal of one,
/// names the planner order its approval issued, and a further line says so
/// with the order's id (D-01). Any other refusal naming an order, such as
/// `planner-order-pending`, names one this approval did not issue, so it
/// gets no such line.
pub(crate) fn answer_render(answer: &Value) -> Render {
    let mut render = approval_lines(answer);
    let order = match answer["status"].as_str() {
        Some("ok") => answer["work_order"].as_str(),
        Some("refused") if answer["code"] == DRAFT_APPROVED => {
            answer["details"]["work_order"].as_str()
        }
        _ => None,
    };
    if let Some(order) = order {
        render.lines.push(format!(
            "planner work order: {}, which a session's phase-plan with the same stories launches",
            escaped(order)
        ));
    }
    render
}

/// [`answer_render`]'s lines before any planner order's.
fn approval_lines(answer: &Value) -> Render {
    let text = |name: &str| escaped(answer[name].as_str().unwrap_or_default());
    let joined = |name: &str| {
        let numbers: Vec<String> = answer[name]
            .as_array()
            .map(|items| items.iter().map(Value::to_string).collect())
            .unwrap_or_default();
        or_none(numbers)
    };
    let mut line = format!("{}: {}", text("code"), text("reason"));
    let details = &answer["details"];
    if let Some(path) = details["path"].as_str() {
        line = format!("{line}: {}", escaped(path));
    }
    if let (Some(section), Some(field)) = (details["section"].as_str(), details["field"].as_str()) {
        line = format!(
            "{line}: section {}, field {}",
            escaped(section),
            escaped(field)
        );
    }
    match answer["status"].as_str() {
        Some("ok") => Render {
            lines: vec![
                format!(
                    "approved the {} submission {} as {} at {}",
                    text("kind"),
                    text("digest"),
                    text("owner"),
                    text("at")
                ),
                format!("phases: {}", joined("phases")),
                format!("stories: {}", joined("stories")),
            ],
            code: 0,
            error: false,
        },
        Some("refused") => Render::refusal(line),
        _ => Render {
            lines: vec![line],
            code: 1,
            error: true,
        },
    }
}

/// What a command-line submission's answer prints: the receipt as two lines,
/// the draft and the command that approves it, exit 0, since the owner
/// approves at the terminal what they just submitted; a refusal or a failure
/// as `answer_render` prints them.
pub(crate) fn submission_render(answer: &Value) -> Render {
    if answer["status"] != "ok" {
        return answer_render(answer);
    }
    let text = |name: &str| escaped(answer[name].as_str().unwrap_or_default());
    printed(vec![
        format!(
            "submitted the {} draft {} at {}",
            text("kind"),
            text("digest"),
            text("recorded_at")
        ),
        format!("approve it with: baley scope approve {}", text("digest")),
    ])
}

/// Text from the ledger as `escape_debug` writes it, quotes left plain, so a
/// recorded line break or terminal escape cannot forge or rewrite a line of
/// output.
pub(crate) fn escaped(text: &str) -> String {
    // The output is not a literal, so a quote needs no backslash. Every
    // backslash `escape_debug` writes starts an escape, so `\'` and `\"`
    // match only an escaped quote.
    text.escape_debug()
        .to_string()
        .replace("\\'", "'")
        .replace("\\\"", "\"")
}

/// A phase number or story id argument: a whole number above 0, since
/// numbering starts at 1. Anything else names nothing and is refused before
/// the ledger is read.
pub(crate) fn judge_number(text: &str) -> Result<u32, Render> {
    text.parse::<u32>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| Render::refusal("phase numbers and story ids are whole numbers from 1"))
}

/// The words of one of the owner's change commands, as clap gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeWords {
    /// `phase reorder <NUMBER>...`.
    PhaseReorder(Vec<String>),
    /// `phase withdraw <NUMBER> [<STORY>=<PHASE>|<STORY>=drop]...`.
    PhaseWithdraw {
        phase: String,
        decisions: Vec<String>,
    },
    /// `story reassign <ID> <PHASE>|backlog`.
    StoryReassign { story: String, to: String },
    /// `story drop <ID>`.
    StoryDrop(String),
    /// `backlog reorder <ID>...`.
    BacklogReorder(Vec<String>),
}

/// What a withdrawal decision's target is written as for a story to drop,
/// and a reassignment's for the backlog.
const DROP_WORD: &str = "drop";
const BACKLOG_WORD: &str = "backlog";

/// The kind a command's words name and the members a session sends for the
/// same change, decisions in the order given. Every number is judged by
/// `judge_number`, and a word in another form is refused before the ledger
/// is read, echoing none of it.
pub(crate) fn change_members(words: &ChangeWords) -> Result<(ChangeKind, Value), Render> {
    let numbers = |items: &[String]| {
        items
            .iter()
            .map(|item| judge_number(item))
            .collect::<Result<Vec<u32>, Render>>()
    };
    Ok(match words {
        ChangeWords::PhaseReorder(order) => {
            (ChangeKind::PhaseReorder, json!({"order": numbers(order)?}))
        }
        ChangeWords::BacklogReorder(order) => (
            ChangeKind::BacklogReorder,
            json!({"order": numbers(order)?}),
        ),
        ChangeWords::PhaseWithdraw { phase, decisions } => {
            let decisions = decisions
                .iter()
                .map(|decision| decision_members(decision))
                .collect::<Result<Vec<Value>, Render>>()?;
            (
                ChangeKind::PhaseWithdraw,
                json!({"phase": judge_number(phase)?, "decisions": decisions}),
            )
        }
        ChangeWords::StoryReassign { story, to } => {
            let story = judge_number(story)?;
            let phase = if to == BACKLOG_WORD {
                Value::Null
            } else {
                to.parse::<u32>()
                    .ok()
                    .filter(|number| *number > 0)
                    .map(Value::from)
                    .ok_or_else(|| {
                        Render::refusal(
                            "a story is reassigned to a phase number from 1 or to backlog",
                        )
                    })?
            };
            (
                ChangeKind::StoryReassign,
                json!({"story": story, "phase": phase}),
            )
        }
        ChangeWords::StoryDrop(story) => (
            ChangeKind::StoryDrop,
            json!({"story": judge_number(story)?}),
        ),
    })
}

/// One withdrawal decision, `<STORY>=<PHASE>` or `<STORY>=drop`, as the
/// member a session sends.
fn decision_members(decision: &str) -> Result<Value, Render> {
    let form = || {
        Render::refusal(
            "a withdrawal decision is a story id, =, and the phase number that serves it instead or drop",
        )
    };
    let (story, to) = decision.split_once('=').ok_or_else(form)?;
    let story = judge_number(story)?;
    let to = if to == DROP_WORD {
        Value::from(DROP_WORD)
    } else {
        Value::from(judge_number(to).map_err(|_| form())?)
    };
    Ok(json!({"story": story, "to": to}))
}

/// The change a command's words make, judged as a session's members are, so
/// the same change has the same draft digest from either surface. A judgement
/// refusal prints as the session would get it.
pub(crate) fn command_change(words: &ChangeWords) -> Result<ChangeSubmission, Render> {
    let (kind, members) = change_members(words)?;
    judge_change(kind, &members)
        .map_err(|refusal| answer_render(&submit::submission_refused(&refusal)))
}

/// What `scope preview` prints for draft `digest`: a held truth draft
/// through [`truth_preview`], read beside the story's `truths` document, and
/// every other draft, a purged truth draft among them, through
/// [`preview_render`]. Nothing is recorded.
pub(crate) fn preview(store: &SqliteStore, project: &ProjectId, digest: Hash) -> Render {
    match read::draft(store, project, digest) {
        Ok(Draft::Held(preview)) if preview.kind == STORY_TRUTHS => {
            truth_preview(store, project, &preview)
        }
        read => preview_render(read),
    }
}

/// What `scope preview` prints for a held truth draft (design 0005 `story
/// truths submit` preview, D-05): its body judged again with the truth
/// judge, the story's current truths read now, and each truth marked against
/// them. A body that no longer judges prints the unavailable line a change
/// prints; a read fault prints as an answer's failure.
fn truth_preview(store: &SqliteStore, project: &ProjectId, preview: &Preview) -> Render {
    let body = &preview.submission;
    let story = body
        .get("story")
        .and_then(Value::as_u64)
        .and_then(|story| u32::try_from(story).ok())
        .and_then(NonZeroU32::new);
    let set = body.get("set").and_then(Value::as_str);
    let judged = match (story, set, body.get("truths")) {
        (Some(story), Some(set), Some(truths)) => judge_truths(story, set, truths).ok(),
        _ => None,
    };
    let Some(submission) = judged else {
        return unavailable(
            "the draft is recorded, but its body no longer judges as a truth submission",
        );
    };
    let current = match current_truths(store, project, submission.story()) {
        Ok(current) => current,
        Err(answer) => return answer_render(&answer),
    };
    match versions(submission.truths(), &current.truths, &current.kept) {
        Ok(versions) => truth_preview_render(preview, &submission, &versions),
        Err(_) => unavailable("a truth of this story has no version number left"),
    }
}

/// A truth draft's preview lines: the header every draft prints, `story:
/// <id>`, `question set: <set>`, each truth as `truth <id> (<mark>):` and
/// its sentence with its detail line, `removed: <ids>` or `removed: none`,
/// and the command that approves it. Versions are given only at approval
/// (D-02), so none is printed.
fn truth_preview_render(
    preview: &Preview,
    submission: &TruthSubmission,
    versions: &Versions,
) -> Render {
    let mut lines = vec![
        format!(
            "draft {} ({}), submitted {}",
            preview.digest.to_hex(),
            escaped(&preview.kind),
            escaped(&preview.recorded_at)
        ),
        format!("story: {}", submission.story()),
        format!("question set: {}", escaped(submission.set())),
    ];
    for (truth, versioned) in submission.truths().iter().zip(&versions.truths) {
        lines.push(format!(
            "truth {} ({}): {}",
            escaped(&truth.id),
            mark_str(versioned.mark),
            sentence(
                &truth.form,
                &truth.trigger,
                &truth.observer,
                &truth.verb,
                &truth.outcome
            )
        ));
        // A judged truth has both attestations true.
        lines.push(truth_detail(
            &truth.kind,
            true,
            true,
            truth.human_verify.as_deref(),
        ));
    }
    lines.push(format!(
        "removed: {}",
        or_none(versions.removed.iter().map(|id| escaped(id)).collect())
    ));
    lines.push(format!(
        "approve it with: baley story truths approve {}",
        preview.digest.to_hex()
    ));
    printed(lines)
}

/// What `scope preview` prints for a draft read: the header, the
/// description, the stories by label and the phases by position, each with
/// its detail and dependencies, since that is what the owner approves. A
/// draft this project purged prints its header and the purge's reason.
pub(crate) fn preview_render(read: Result<Draft, PreviewError>) -> Render {
    let preview = match read {
        Ok(Draft::Held(preview)) => preview,
        Ok(Draft::Purged(tombstone)) => {
            return printed(vec![
                format!(
                    "draft {} ({}), submitted {}",
                    tombstone.digest.to_hex(),
                    escaped(&tombstone.kind),
                    escaped(&tombstone.recorded_at)
                ),
                format!("purged: {}", escaped(&tombstone.reason)),
            ]);
        }
        Err(PreviewError::UnknownDraft) => {
            return Render::refusal(format!(
                "{UNKNOWN_DRAFT}: no submission in this project has that digest"
            ));
        }
        Err(PreviewError::Unreadable(reason)) => return unavailable(&reason),
        Err(PreviewError::Store(error)) => return display::store_error(&error, None),
    };
    let mut lines = vec![format!(
        "draft {} ({}), submitted {}",
        preview.digest.to_hex(),
        escaped(&preview.kind),
        escaped(&preview.recorded_at)
    )];
    if preview.kind != START_KIND {
        return match change_lines(&preview.kind, &preview.submission) {
            Some(members) => {
                lines.extend(members);
                printed(lines)
            }
            None => unavailable("the draft is recorded, but its body no longer judges as a change"),
        };
    }
    let Ok(submission) = judge_submission(&preview.submission) else {
        return unavailable("the draft is recorded, but its body no longer judges as a submission");
    };
    let description = &submission.description;
    describe(
        &mut lines,
        &description.what,
        &description.core_value,
        &description.constraints,
    );
    for story in &submission.stories {
        lines.push(format!(
            "story {} ({}): {}",
            escaped(&story.label),
            story.status.as_str(),
            escaped(&story.sentence)
        ));
    }
    for (position, phase) in submission.phases.iter().enumerate() {
        let labels: Vec<String> = phase
            .stories
            .iter()
            .filter_map(|index| submission.stories.get(*index))
            .map(|story| escaped(&story.label))
            .collect();
        lines.extend([
            format!("phase {}: {}", position + 1, escaped(&phase.name)),
            format!("  goal: {}", escaped(&phase.goal)),
            format!("  detail: {}", escaped(&phase.detail)),
            format!("  depends on: {}", list(&phase.depends_on)),
            format!("  stories: {}", or_none(labels)),
        ]);
    }
    printed(lines)
}

/// A change draft's members as its preview prints them, in its shape's
/// member order: the change as submitted, which is what the owner approves.
/// The body's `kind` must be the draft's and the rest must judge under it;
/// otherwise there is nothing to print.
fn change_lines(kind: &str, body: &Value) -> Option<Vec<String>> {
    let kind = change_kind(kind)?;
    let mut members = body.as_object()?.clone();
    if members.remove("kind")? != kind.as_str() {
        return None;
    }
    let change = judge_change(kind, &Value::Object(members)).ok()?.change;
    let mut lines = Vec::new();
    match change {
        ScopeChange::ProjectDescribe(description) => describe(
            &mut lines,
            &description.what,
            &description.core_value,
            &description.constraints,
        ),
        ScopeChange::PhaseDeclare(phase) => lines.extend([
            format!("name: {}", escaped(&phase.name)),
            format!("goal: {}", escaped(&phase.goal)),
            format!("detail: {}", escaped(&phase.detail)),
            format!("depends on: {}", list(&phase.depends_on)),
            format!("stories: {}", list(&phase.stories)),
            format!(
                "position: {}",
                phase
                    .position
                    .map_or_else(|| "last".to_owned(), |at| at.to_string())
            ),
        ]),
        ScopeChange::PhaseEdit { phase, fields } => {
            lines.push(format!("phase: {phase}"));
            let texts = [
                ("name", &fields.name),
                ("goal", &fields.goal),
                ("detail", &fields.detail),
            ];
            for (name, text) in texts {
                if let Some(text) = text {
                    lines.push(format!("{name}: {}", escaped(text)));
                }
            }
            let numbers = [
                ("depends on", &fields.depends_on),
                ("stories", &fields.stories),
            ];
            for (name, numbers) in numbers {
                if let Some(numbers) = numbers {
                    lines.push(format!("{name}: {}", list(numbers)));
                }
            }
        }
        ScopeChange::PhaseReorder { order } | ScopeChange::BacklogReorder { order } => {
            lines.push(format!("order: {}", list(&order)));
        }
        ScopeChange::PhaseWithdraw { phase, decisions } => {
            lines.push(format!("phase: {phase}"));
            if decisions.is_empty() {
                lines.push("decisions: none".into());
            }
            for decision in decisions {
                lines.push(match decision.to {
                    Destination::Phase(to) => format!("story {}: to phase {to}", decision.story),
                    Destination::Drop => format!("story {}: dropped", decision.story),
                });
            }
        }
        ScopeChange::StoryDeclare {
            sentence,
            status,
            phase,
        } => lines.extend([
            format!("sentence: {}", escaped(&sentence)),
            format!("status: {}", status.as_str()),
            format!("phase: {}", served_by(phase)),
        ]),
        ScopeChange::StoryEdit { story, sentence } => lines.extend([
            format!("story: {story}"),
            format!("sentence: {}", escaped(&sentence)),
        ]),
        ScopeChange::StoryReassign { story, phase } => lines.extend([
            format!("story: {story}"),
            format!("phase: {}", served_by(phase)),
        ]),
        ScopeChange::StoryDrop { story } => lines.push(format!("story: {story}")),
    }
    Some(lines)
}

/// The phase a change serves a story by, or `backlog`.
fn served_by(phase: Option<u32>) -> String {
    phase.map_or_else(|| "backlog".to_owned(), |phase| phase.to_string())
}

/// What `scope show` prints for a roadmap read: the description, the phases
/// in roadmap order and the stories in priority order, or one line when
/// nothing is approved yet, which is not an error.
pub(crate) fn roadmap_render(read: Result<Roadmap, ReadError>) -> Render {
    let roadmap = match read {
        Ok(roadmap) => roadmap,
        Err(error) => return read_error(error),
    };
    if roadmap.description.is_none() && roadmap.phases.is_empty() && roadmap.stories.is_empty() {
        return Render::line("no scope has been approved for this project", 0);
    }
    let mut lines = Vec::new();
    if let Some(description) = &roadmap.description {
        describe(
            &mut lines,
            &description.what,
            &description.core_value,
            &description.constraints,
        );
    }
    for phase in &roadmap.phases {
        lines.extend([
            format!(
                "phase {} ({}): {}",
                phase.number,
                escaped(&phase.status),
                escaped(&phase.name)
            ),
            format!("  goal: {}", escaped(&phase.goal)),
            format!("  depends on: {}", list(&phase.depends_on)),
            format!("  stories: {}", list(&phase.stories)),
        ]);
    }
    for story in &roadmap.stories {
        lines.push(format!(
            "story {} ({}, {}): {}",
            story.id,
            escaped(&story.status),
            committed(story.phase),
            escaped(&story.sentence)
        ));
    }
    printed(lines)
}

/// What `phase show` prints for a phase read. A detail this project purged
/// prints as `detail purged:` and the reason, a prefix no detail text can
/// take.
pub(crate) fn phase_render(read: Result<Phase, ReadError>) -> Render {
    match read {
        Ok(phase) => printed(vec![
            format!(
                "phase {} ({}): {}",
                phase.number,
                escaped(&phase.status),
                escaped(&phase.name)
            ),
            format!("goal: {}", escaped(&phase.goal)),
            match &phase.detail {
                Detail::Text(detail) => format!("detail: {}", escaped(detail)),
                Detail::Purged(reason) => format!("detail purged: {}", escaped(reason)),
            },
            format!("depends on: {}", list(&phase.depends_on)),
            format!("stories: {}", list(&phase.stories)),
        ]),
        Err(error) => read_error(error),
    }
}

/// What `backlog show` prints for a backlog read: one line per story in
/// priority order, its refinement state shown as `unrefined`, `questions
/// open in round <n>`, `drafting` or `refined`, or one line when it is empty.
pub(crate) fn backlog_render(read: Result<Vec<BacklogStory>, ReadError>) -> Render {
    match read {
        Ok(stories) if stories.is_empty() => Render::line("the backlog is empty", 0),
        Ok(stories) => printed(
            stories
                .iter()
                .map(|story| {
                    let refinement = match story.round {
                        Some(round) => format!("questions open in round {round}"),
                        None => escaped(&story.refinement),
                    };
                    format!(
                        "story {} ({}, {}, {}, {} truths): {}",
                        story.id,
                        escaped(&story.status),
                        committed(story.phase),
                        refinement,
                        story.truths,
                        escaped(&story.sentence)
                    )
                })
                .collect(),
        ),
        Err(error) => read_error(error),
    }
}

/// What `story show` prints for a story read.
pub(crate) fn story_render(read: Result<Story, ReadError>) -> Render {
    match read {
        Ok(story) => printed(vec![
            format!(
                "story {} ({}): {}",
                story.id,
                escaped(&story.status),
                escaped(&story.sentence)
            ),
            format!(
                "phase: {}",
                story
                    .phase
                    .map_or_else(|| "backlog".to_owned(), |phase| phase.to_string())
            ),
            format!("truths: {}", story.truths),
        ]),
        Err(error) => read_error(error),
    }
}

/// The description's lines, as both the preview and the roadmap print them.
pub(crate) fn describe(
    lines: &mut Vec<String>,
    what: &str,
    core_value: &str,
    constraints: &[String],
) {
    lines.push(format!("what: {}", escaped(what)));
    lines.push(format!("core value: {}", escaped(core_value)));
    for constraint in constraints {
        lines.push(format!("constraint: {}", escaped(constraint)));
    }
}

/// Where a story is committed: `phase <n>`, or `backlog`.
fn committed(phase: Option<u32>) -> String {
    phase.map_or_else(|| "backlog".to_owned(), |phase| format!("phase {phase}"))
}

/// Numbers joined by `, `, or `none`.
fn list<T: ToString>(numbers: &[T]) -> String {
    or_none(numbers.iter().map(ToString::to_string).collect())
}

fn or_none(items: Vec<String>) -> String {
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}

pub(crate) fn printed(lines: Vec<String>) -> Render {
    Render {
        lines,
        code: 0,
        error: false,
    }
}

/// A ledger that holds the record but cannot give it whole, exit 1.
fn unavailable(reason: &str) -> Render {
    Render {
        lines: vec![format!("{LEDGER_UNAVAILABLE}: {reason}")],
        code: 1,
        error: true,
    }
}

/// Why accepted scope could not be read: a missing phase or story is a
/// refusal, exit 2; a body no longer held or a broken record is exit 1.
pub(crate) fn read_error(error: ReadError) -> Render {
    match error {
        ReadError::NoSuchPhase => {
            Render::refusal(format!("{NO_SUCH_PHASE}: this project has no such phase"))
        }
        ReadError::NoSuchStory => {
            Render::refusal(format!("{NO_SUCH_STORY}: this project has no such story"))
        }
        ReadError::BodyGone => {
            unavailable("the phase is recorded, but its detail body is no longer held")
        }
        ReadError::Unreadable(reason) => unavailable(&reason),
        ReadError::Store(error) => display::store_error(&error, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::read::{Description, Preview, RoadmapPhase};

    const PROJECT: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const D: &str = "1b11196b8f9f4ffd5c08a4d97643f80867194c5e1a6d8cc51d2929f8db38d68e";
    const R: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    const T: &str = "2026-10-13T09:00:02Z";

    #[test]
    fn a_malformed_digest_argument_passed_on_to_the_ledger_is_caught() {
        let upper = format!("A{}", &D[1..]);
        let g = format!("g{}", &D[1..]);
        let long = format!("{D}a");
        for text in [&D[1..], long.as_str(), upper.as_str(), g.as_str()] {
            let refusal = judge_digest(text).expect_err(text);
            assert_eq!(refusal.code, 2, "{text}");
            assert!(refusal.error, "{text}");
        }
        assert_eq!(judge_digest(D).ok(), Hash::from_hex(D));
    }

    #[test]
    fn a_command_line_approval_built_without_the_owner_or_its_policy_version_is_caught() {
        let project = ProjectId(PROJECT.into());
        let digest = Hash::from_hex(D).unwrap();
        let build =
            |version| approve_command(&project, version, digest, RequestId(R.into()), T).unwrap();
        let command = build(4);
        assert_eq!(command.project, project);
        assert_eq!(command.kind, CommandKind("scope.approve".into()));
        assert_eq!(command.actor, Actor::Owner);
        assert_eq!(command.caller, None);
        assert_eq!(command.policy_version, 4);
        assert_eq!(command.recorded_at, T);
        assert_eq!(command.request_id, RequestId(R.into()));
        assert_ne!(build(5).digest, command.digest);
        assert_eq!(build(4).digest, command.digest);
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn a_command_line_change_drafted_under_another_digest_than_the_sessions_is_caught() {
        use crate::mcp::operations::{Lookup, Tool, lookup};
        use crate::mcp::scope::judge_change_arguments;

        let pairs = [
            (
                "phase-reorder",
                json!({"order": [2, 1]}),
                ChangeWords::PhaseReorder(words(&["2", "1"])),
                "5c4b7650561455ec8d35bfe5be8bdca5b426260fb4ada5934e9a2b0ea0de6619",
            ),
            (
                "phase-withdraw",
                json!({"phase": 2, "decisions": [{"story": 2, "to": 1}]}),
                ChangeWords::PhaseWithdraw {
                    phase: "2".into(),
                    decisions: words(&["2=1"]),
                },
                "830b6dfddb927b4f83c616d5fc67832b672c1af4c686d8ea578f17e01fcdf0b6",
            ),
            (
                "story-reassign",
                json!({"story": 3, "phase": 2}),
                ChangeWords::StoryReassign {
                    story: "3".into(),
                    to: "2".into(),
                },
                "2bcc41abfda87adb37ecabf2bc999506930b94ae01523a53aeeba53fd4f39937",
            ),
            (
                "story-drop",
                json!({"story": 3}),
                ChangeWords::StoryDrop("3".into()),
                "9094033334d2919f56c4bee9f9245d34a8901f297e882a519a7ce167e82b1099",
            ),
            (
                "backlog-reorder",
                json!({"order": [2, 1, 3]}),
                ChangeWords::BacklogReorder(words(&["2", "1", "3"])),
                "078d2bae8750974615966455e5cad318e124b4496836c02f7cec12b2a65f8e0b",
            ),
        ];
        for (operation, submission, said, digest) in pairs {
            let session = judge_change_arguments(&json!({"operation": operation,
                "request_id": R, "submission": submission}))
            .unwrap();
            let typed = command_change(&said).unwrap_or_else(|e| panic!("{operation}: {e:?}"));
            assert_eq!(session.change.digest().to_hex(), digest, "{operation}");
            assert_eq!(typed.digest().to_hex(), digest, "{operation}");
        }
        assert_eq!(lookup(Tool::Apply, Some("scope-approve")), Lookup::Unknown);
    }

    #[test]
    fn a_command_line_change_argument_passed_on_unjudged_is_caught() {
        let refused = [
            (ChangeWords::PhaseReorder(words(&["0"])), "0"),
            (ChangeWords::PhaseReorder(words(&["2", "x"])), "x"),
            (
                ChangeWords::PhaseWithdraw {
                    phase: "2".into(),
                    decisions: words(&["2-1"]),
                },
                "2-1",
            ),
            (
                ChangeWords::PhaseWithdraw {
                    phase: "2".into(),
                    decisions: words(&["2=later"]),
                },
                "later",
            ),
            (
                ChangeWords::StoryReassign {
                    story: "3".into(),
                    to: "later".into(),
                },
                "later",
            ),
        ];
        for (said, argument) in refused {
            let refusal = change_members(&said).expect_err(argument);
            assert_eq!((refusal.code, refusal.error), (2, true), "{argument}");
            assert!(
                refusal.lines.iter().all(|line| !line.contains(argument)),
                "{argument}: {:?}",
                refusal.lines
            );
        }

        let backlog = ChangeWords::StoryReassign {
            story: "3".into(),
            to: "backlog".into(),
        };
        assert_eq!(
            change_members(&backlog).unwrap(),
            (
                ChangeKind::StoryReassign,
                json!({"story": 3, "phase": null})
            )
        );
        let drop = ChangeWords::PhaseWithdraw {
            phase: "2".into(),
            decisions: words(&["2=drop"]),
        };
        assert_eq!(
            change_members(&drop).unwrap(),
            (
                ChangeKind::PhaseWithdraw,
                json!({"phase": 2, "decisions": [{"story": 2, "to": "drop"}]})
            )
        );
    }

    #[test]
    fn a_command_line_submission_built_without_the_owner_or_its_policy_version_is_caught() {
        let project = ProjectId(PROJECT.into());
        let digest =
            Hash::from_hex("5c4b7650561455ec8d35bfe5be8bdca5b426260fb4ada5934e9a2b0ea0de6619")
                .unwrap();
        let build =
            |version| submit_command(&project, version, digest, RequestId(R.into()), T).unwrap();
        let command = build(4);
        assert_eq!(command.project, project);
        assert_eq!(command.kind, CommandKind("scope.submit".into()));
        assert_eq!(command.actor, Actor::Owner);
        assert_eq!(command.caller, None);
        assert_eq!(command.policy_version, 4);
        assert_eq!(command.recorded_at, T);
        assert_eq!(command.request_id, RequestId(R.into()));
        assert_ne!(build(5).digest, command.digest);
        assert_eq!(build(4).digest, command.digest);
    }

    #[test]
    fn an_approval_receipt_or_refusal_printed_with_other_lines_or_exit_is_caught() {
        let receipt = json!({"status": "ok", "kind": "start", "digest": D, "owner": "Ada Owner",
            "at": T, "phases": [1, 2], "stories": [1, 2, 3]});
        let done = answer_render(&receipt);
        assert_eq!(
            done.lines,
            [
                format!("approved the start submission {D} as Ada Owner at {T}"),
                "phases: 1, 2".into(),
                "stories: 1, 2, 3".into(),
            ]
        );
        assert_eq!((done.code, done.error), (0, false));

        let stale = answer_render(&json!({"status": "refused", "code": "stale-draft",
            "reason": "a newer draft was submitted after this one, so it was not approved. The details name the first part that differs",
            "slot": "digest", "details": {"path": "roadmap[1].goal"}}));
        assert_eq!(stale.lines.len(), 1, "{:?}", stale.lines);
        assert!(
            stale.lines[0].starts_with("stale-draft:"),
            "{:?}",
            stale.lines
        );
        assert!(
            stale.lines[0].contains("roadmap[1].goal"),
            "{:?}",
            stale.lines
        );
        assert_eq!((stale.code, stale.error), (2, true));

        let busy = answer_render(&json!({"status": "failed", "code": "ledger-busy",
            "reason": "the ledger is busy, so nothing was recorded. Try it again",
            "place": "ledger", "retryable": true}));
        assert_eq!(busy.lines.len(), 1, "{:?}", busy.lines);
        assert!(
            busy.lines[0].starts_with("ledger-busy:"),
            "{:?}",
            busy.lines
        );
        assert_eq!((busy.code, busy.error), (1, true));
    }

    #[test]
    fn a_commitment_approval_printed_without_its_order_or_a_plain_one_naming_one_is_caught() {
        const O: &str = "1f2e3d4c-5b6a-4978-8695-a4b3c2d1e0f1";
        let names_order = |lines: &[String]| {
            lines
                .iter()
                .any(|line| line.contains(O) && line.contains("phase-plan"))
        };
        let plain = json!({"status": "ok", "kind": "phase-edit", "digest": D,
            "owner": "Ada Owner", "at": T, "phases": [], "stories": []});
        let mut committed = plain.clone();
        committed["work_order"] = json!(O);
        committed["prompt"] = json!(format!("Read Baley work order {O} and follow it."));

        let done = answer_render(&committed);
        assert!(names_order(&done.lines), "{:?}", done.lines);
        assert_eq!((done.code, done.error), (0, false));

        let done = answer_render(&plain);
        assert_eq!(
            done.lines,
            [
                format!("approved the phase-edit submission {D} as Ada Owner at {T}"),
                "phases: none".into(),
                "stories: none".into(),
            ]
        );

        let rerun = answer_render(&json!({"status": "refused", "code": "draft-approved",
            "reason": "this draft was already approved after its latest submission, so it was not approved again. The details name that approval",
            "slot": "digest",
            "details": {"owner": "Ada Owner", "at": T, "work_order": O}}));
        assert!(
            rerun.lines[0].starts_with("draft-approved:"),
            "{:?}",
            rerun.lines
        );
        assert!(names_order(&rerun.lines), "{:?}", rerun.lines);
        assert_eq!((rerun.code, rerun.error), (2, true));
    }

    #[test]
    fn a_pending_planner_order_refusal_printed_as_this_approvals_order_is_caught() {
        // Q is another selection's order, still pending, which this
        // approval did not issue and a phase-plan cannot launch.
        const Q: &str = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";
        let reason = "the phase's planner work order Q is still pending, and a phase has one live planner order, so nothing was approved";
        let pending = answer_render(&json!({"status": "refused",
            "code": "planner-order-pending", "reason": reason, "slot": "digest",
            "details": {"work_order": Q}}));
        assert_eq!(pending.lines, [format!("planner-order-pending: {reason}")]);
        assert_eq!((pending.code, pending.error), (2, true));
    }

    #[test]
    fn a_malformed_submission_refusal_printed_without_its_section_and_field_is_caught() {
        let reason = "a field of the submission is missing, blank, of the wrong type, unknown or out of range, so nothing was recorded. The details name it";
        let malformed = answer_render(&json!({"status": "refused",
            "code": "malformed-submission", "reason": reason, "slot": "submission",
            "details": {"section": "decisions", "field": "decisions[1].story"}}));
        assert_eq!(
            malformed.lines,
            [format!(
                "malformed-submission: {reason}: section decisions, field decisions[1].story"
            )]
        );
        assert_eq!((malformed.code, malformed.error), (2, true));
    }

    /// W's digest: phase 2 withdrawn with its story moved to phase 1.
    const W: &str = "830b6dfddb927b4f83c616d5fc67832b672c1af4c686d8ea578f17e01fcdf0b6";

    #[test]
    fn a_change_approval_receipt_printed_without_its_phases_and_stories_is_caught() {
        let at = "2026-10-13T09:00:07Z";
        let withdrawn = answer_render(&json!({"status": "ok", "kind": "phase-withdraw",
            "digest": W, "owner": "Ada Owner", "at": at, "phases": [2], "stories": [2]}));
        assert_eq!(
            withdrawn.lines,
            [
                format!("approved the phase-withdraw submission {W} as Ada Owner at {at}"),
                "phases: 2".into(),
                "stories: 2".into(),
            ]
        );
        assert_eq!((withdrawn.code, withdrawn.error), (0, false));

        let reordered = answer_render(&json!({"status": "ok", "kind": "phase-reorder",
            "digest": W, "owner": "Ada Owner", "at": at, "phases": [], "stories": []}));
        assert_eq!(reordered.lines[1..], ["phases: none", "stories: none"]);
        assert_eq!((reordered.code, reordered.error), (0, false));
    }

    #[test]
    fn a_submission_receipt_printed_without_its_digest_or_the_approve_command_is_caught() {
        let at = "2026-10-13T09:00:06Z";
        let submitted = submission_render(&json!({"status": "ok", "digest": W,
            "kind": "phase-withdraw", "recorded_at": at}));
        assert_eq!(
            submitted.lines,
            [
                format!("submitted the phase-withdraw draft {W} at {at}"),
                format!("approve it with: baley scope approve {W}"),
            ]
        );
        assert_eq!((submitted.code, submitted.error), (0, false));

        let reason = "phase 2 serves story 2 and the withdrawal gives it no decision, so nothing was recorded";
        let refused = submission_render(&json!({"status": "refused", "code": "story-undecided",
            "reason": reason, "slot": "submission", "details": {"story": 2}}));
        assert_eq!(refused.lines, [format!("story-undecided: {reason}")]);
        assert_eq!((refused.code, refused.error), (2, true));
    }

    fn text(value: &str) -> String {
        value.into()
    }

    fn story(id: u32, sentence: &str, status: &str, phase: Option<u32>) -> Story {
        Story {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            truths: 0,
        }
    }

    /// The approved S1's roadmap, as the plan's Notes give it.
    fn s1_roadmap() -> Roadmap {
        let phase = |number, name: &str, goal: &str, depends_on, stories| RoadmapPhase {
            number,
            name: text(name),
            goal: text(goal),
            status: text("declared"),
            depends_on,
            stories,
        };
        Roadmap {
            description: Some(Description {
                what: text("A ledger for agent work"),
                core_value: text("Nothing moves on an unproven claim"),
                constraints: vec![text("One binary")],
            }),
            phases: vec![
                phase(1, "Records", "Steps are recorded", vec![], vec![1]),
                phase(2, "Approval", "The owner approves", vec![1], vec![2]),
            ],
            stories: vec![
                story(1, "Every step is recorded", "active", Some(1)),
                story(2, "The owner approves scope", "active", Some(2)),
                story(3, "Teammates see the backlog", "deferred", None),
            ],
        }
    }

    fn shown(render: &Render) -> Vec<&str> {
        assert_eq!(
            (render.code, render.error),
            (0, false),
            "{:?}",
            render.lines
        );
        render.lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_preview_that_leaves_out_what_the_owner_approves_is_caught() {
        let preview = Preview {
            kind: text("start"),
            digest: Hash::from_hex(D).unwrap(),
            seq: 2,
            recorded_at: text(T),
            submission: crate::scope::fixtures::s1(),
        };
        let render = preview_render(Ok(Draft::Held(preview)));
        assert_eq!(
            shown(&render),
            [
                format!("draft {D} (start), submitted {T}").as_str(),
                "what: A ledger for agent work",
                "core value: Nothing moves on an unproven claim",
                "constraint: One binary",
                "story record (active): Every step is recorded",
                "story approve (active): The owner approves scope",
                "story later (deferred): Teammates see the backlog",
                "phase 1: Records",
                "  goal: Steps are recorded",
                "  detail: The ledger and its views",
                "  depends on: none",
                "  stories: record",
                "phase 2: Approval",
                "  goal: The owner approves",
                "  detail: Approval by digest",
                "  depends on: 1",
                "  stories: approve",
            ]
        );
    }

    #[test]
    fn a_change_preview_that_leaves_out_what_the_owner_approves_is_caught() {
        use crate::scope::fixtures::{change, d1, e_b, p, w};

        let previewed = |kind: ChangeKind, value: &Value| {
            let held = change(kind, value);
            let digest = held.digest();
            let render = preview_render(Ok(Draft::Held(Preview {
                kind: text(kind.as_str()),
                digest,
                seq: 7,
                recorded_at: text(T),
                submission: held.value().clone(),
            })));
            let header = format!(
                "draft {} ({}), submitted {T}",
                digest.to_hex(),
                kind.as_str()
            );
            (render, header)
        };
        let cases = [
            (
                ChangeKind::PhaseWithdraw,
                w(),
                vec!["phase: 2", "story 2: to phase 1"],
            ),
            (
                ChangeKind::PhaseDeclare,
                p(),
                vec![
                    "name: Reports",
                    "goal: The owner reads progress",
                    "detail: Progress from the ledger",
                    "depends on: 1",
                    "stories: none",
                    "position: last",
                ],
            ),
            (
                ChangeKind::PhaseEdit,
                e_b(),
                vec!["phase: 2", "goal: The owner approves every change"],
            ),
            (
                ChangeKind::ProjectDescribe,
                d1(),
                vec![
                    "what: A ledger for agent work",
                    "core value: Nothing moves on an unproven claim",
                    "constraint: One binary",
                    "constraint: No server",
                ],
            ),
            (
                ChangeKind::StoryReassign,
                json!({"story": 3, "phase": null}),
                vec!["story: 3", "phase: backlog"],
            ),
        ];
        for (kind, value, members) in cases {
            let (render, header) = previewed(kind, &value);
            let mut expected = vec![header.as_str()];
            expected.extend(members);
            assert_eq!(shown(&render), expected, "{}", kind.as_str());
        }
    }

    #[test]
    fn a_roadmap_shown_out_of_order_or_unstarted_shown_as_an_error_is_caught() {
        let render = roadmap_render(Ok(s1_roadmap()));
        assert_eq!(
            shown(&render),
            [
                "what: A ledger for agent work",
                "core value: Nothing moves on an unproven claim",
                "constraint: One binary",
                "phase 1 (declared): Records",
                "  goal: Steps are recorded",
                "  depends on: none",
                "  stories: 1",
                "phase 2 (declared): Approval",
                "  goal: The owner approves",
                "  depends on: 1",
                "  stories: 2",
                "story 1 (active, phase 1): Every step is recorded",
                "story 2 (active, phase 2): The owner approves scope",
                "story 3 (deferred, backlog): Teammates see the backlog",
            ]
        );
        let empty = roadmap_render(Ok(Roadmap {
            description: None,
            phases: vec![],
            stories: vec![],
        }));
        assert_eq!(
            shown(&empty),
            ["no scope has been approved for this project"]
        );
    }

    #[test]
    fn a_phase_shown_without_its_detail_or_dependencies_is_caught() {
        let render = phase_render(Ok(Phase {
            number: 2,
            name: text("Approval"),
            goal: text("The owner approves"),
            status: text("declared"),
            detail: Detail::Text(text("Approval by digest")),
            depends_on: vec![1],
            stories: vec![2],
        }));
        assert_eq!(
            shown(&render),
            [
                "phase 2 (declared): Approval",
                "goal: The owner approves",
                "detail: Approval by digest",
                "depends on: 1",
                "stories: 2",
            ]
        );
    }

    #[test]
    fn a_backlog_or_story_shown_without_its_phase_or_refinement_is_caught() {
        let listed = |id, sentence: &str, status: &str, phase| BacklogStory {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            refinement: text("unrefined"),
            round: None,
            truths: 0,
            size: None,
        };
        let backlog = backlog_render(Ok(vec![
            listed(1, "Every step is recorded", "active", Some(1)),
            listed(2, "The owner approves scope", "active", Some(2)),
            listed(3, "Teammates see the backlog", "deferred", None),
        ]));
        assert_eq!(
            shown(&backlog),
            [
                "story 1 (active, phase 1, unrefined, 0 truths): Every step is recorded",
                "story 2 (active, phase 2, unrefined, 0 truths): The owner approves scope",
                "story 3 (deferred, backlog, unrefined, 0 truths): Teammates see the backlog",
            ]
        );
        assert_eq!(shown(&backlog_render(Ok(vec![]))), ["the backlog is empty"]);
        let one = story_render(Ok(story(1, "Every step is recorded", "active", Some(1))));
        assert_eq!(
            shown(&one),
            [
                "story 1 (active): Every step is recorded",
                "phase: 1",
                "truths: 0"
            ]
        );
    }

    #[test]
    fn a_backlog_line_without_its_state_or_round_is_caught() {
        let listed = |id, refinement: &str, round, truths| BacklogStory {
            id,
            sentence: text("Every step is recorded"),
            status: text("active"),
            phase: Some(1),
            refinement: text(refinement),
            round,
            truths,
            size: None,
        };
        let backlog = backlog_render(Ok(vec![
            listed(1, "questions-open", Some(2), 0),
            listed(2, "drafting", None, 0),
            listed(3, "refined", None, 2),
            listed(4, "unrefined", None, 0),
        ]));
        assert_eq!(
            shown(&backlog),
            [
                "story 1 (active, phase 1, questions open in round 2, 0 truths): Every step is recorded",
                "story 2 (active, phase 1, drafting, 0 truths): Every step is recorded",
                "story 3 (active, phase 1, refined, 2 truths): Every step is recorded",
                "story 4 (active, phase 1, unrefined, 0 truths): Every step is recorded",
            ]
        );
    }

    #[test]
    fn a_recorded_control_character_printed_raw_is_caught() {
        for (sentence, expected) in [
            (
                "one\nstory 9 (active): forged",
                "story 1 (active): one\\nstory 9 (active): forged",
            ),
            ("one\rtwo\u{1b}[2J", "story 1 (active): one\\rtwo\\u{1b}[2J"),
        ] {
            let render = story_render(Ok(story(1, sentence, "active", Some(1))));
            let first = &shown(&render)[0];
            assert_eq!(*first, expected);
            assert!(!first.chars().any(char::is_control), "{first:?}");
        }
    }

    #[test]
    fn a_recorded_quote_printed_with_a_backslash_or_a_recorded_backslash_printed_bare_is_caught() {
        // A bare backslash would make a recorded `\n` read as an escaped
        // line break.
        for (sentence, expected) in [
            (
                "the file's \"name\"",
                "story 1 (active): the file's \"name\"",
            ),
            ("one\\ntwo\\'", "story 1 (active): one\\\\ntwo\\\\'"),
        ] {
            let render = story_render(Ok(story(1, sentence, "active", Some(1))));
            assert_eq!(shown(&render)[0], expected);
        }
    }

    #[test]
    fn a_phase_number_or_story_id_of_zero_or_text_accepted_is_caught() {
        // `phase show` takes `0` and `x`, `story show` takes `0`, through the
        // one judgement both use.
        for text in ["0", "x", "0"] {
            let refusal = judge_number(text).expect_err(text);
            assert_eq!((refusal.code, refusal.error), (2, true), "{text}");
        }
        assert_eq!(judge_number("1").ok(), Some(1));
    }

    /// The digest of S1 submitted in `project` of `store`.
    fn s1_in(store: &SqliteStore, project: &baley_store::ProjectId) -> Hash {
        use crate::scope::fixtures::{R1, T1, receipt, s1, submit_in};
        let recorded = submit_in(store, project, &s1(), R1, 1, T1).unwrap();
        Hash::from_hex(receipt(&recorded)["digest"].as_str().unwrap()).unwrap()
    }

    // Both projects hold S1, so the same bytes: the store would open them
    // for either, and only the purging project's view says it released them.
    #[test]
    fn a_phase_detail_this_project_purged_shown_from_another_projects_copy_is_caught() {
        use baley_core::scope::{PHASE_VIEW, phase_key};
        use baley_store::Views;

        use crate::scope::fixtures::{
            approve_draft_in, project, project_b, purge_in, two_projects,
        };

        let (_dir, store) = two_projects();
        for project in [project(), project_b()] {
            let digest = s1_in(&store, &project);
            approve_draft_in(&store, &project, digest);
        }
        let held = store
            .get(&project(), PHASE_VIEW, &phase_key(2))
            .unwrap()
            .unwrap();
        let detail = Hash::from_hex(held.body["detail"].as_str().unwrap()).unwrap();
        purge_in(&store, &project(), detail, "pasted a secret");

        let purged = phase_render(read::phase(&store, &project(), 2));
        assert_eq!(
            shown(&purged),
            [
                "phase 2 (declared): Approval",
                "goal: The owner approves",
                "detail purged: pasted a secret",
                "depends on: 1",
                "stories: 2",
            ]
        );
        let other = phase_render(read::phase(&store, &project_b(), 2));
        assert_eq!(shown(&other)[2], "detail: Approval by digest");
    }

    #[test]
    fn a_draft_this_project_purged_previewed_from_another_projects_copy_is_caught() {
        use crate::scope::fixtures::{T1, project, project_b, purge_in, two_projects};

        let (_dir, store) = two_projects();
        let digest = s1_in(&store, &project());
        s1_in(&store, &project_b());
        // The digest is the body's hash.
        purge_in(&store, &project(), digest, "pasted a secret");

        let purged = preview_render(read::draft(&store, &project(), digest));
        assert_eq!(
            shown(&purged),
            [
                format!("draft {} (start), submitted {T1}", digest.to_hex()).as_str(),
                "purged: pasted a secret",
            ]
        );
        let other = preview_render(read::draft(&store, &project_b(), digest));
        assert!(
            shown(&other).contains(&"  detail: Approval by digest"),
            "{:?}",
            other.lines
        );
    }

    #[test]
    fn a_truth_draft_previewed_without_its_marks_or_as_unavailable_is_caught() {
        use crate::refine::questions::tests::D as DRAFTING;
        use crate::refine::read::tests::{drafting_from_empty, revise, t3};
        use crate::refine::submit::tests::{t1, t2, truths_submitted};
        use crate::scope::fixtures::{R5, T3, T4, T6, project};

        let (dir, store, set) = drafting_from_empty();
        revise(&store, &set, json!([t1(), t2()]), 10, T3);
        let draft = truths_submitted(&store, 1, &set, &json!([t1(), t3()]), DRAFTING, R5, T4);
        assert_eq!(draft["status"], "ok", "{draft}");
        let digest = draft["digest"].as_str().unwrap().to_owned();
        drop(store);
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            T6,
            crate::ledger::open::options(),
        )
        .unwrap();

        let render = preview(&store, &project(), Hash::from_hex(&digest).unwrap());
        let lines = shown(&render);
        for line in [
            "story: 1",
            format!("question set: {set}").as_str(),
            "truth t1 (unchanged): When the owner approves a draft by a stale digest, the owner is refused stale-draft naming the first differing part.",
            "truth t3 (added): When the owner approves a draft by a stale digest, the owner is refused stale-draft naming the first differing member.",
            "removed: t2",
            format!("approve it with: baley story truths approve {digest}").as_str(),
        ] {
            assert!(lines.contains(&line), "{line} in {lines:?}");
        }
        assert!(
            !lines.iter().any(|line| line.contains(LEDGER_UNAVAILABLE)),
            "{lines:?}"
        );
    }
}
