//! The owner's scope commands (design 0004 sections 5 and 6, D-20): `baley
//! scope approve <digest>` approves the newest start draft as the owner, the
//! only way scope is approved (D-24), and `baley scope preview|show`, `baley
//! phase show`, `baley backlog show` and `baley story show` print what the
//! ledger holds through the same reads as `document`, recording nothing. A
//! submission comes only from a session, so no command here submits scope.
//!
//! Gathering finds the project and opens the ledger; judging, the arguments,
//! the command an approval is recorded under and what is printed, is done by
//! the pure functions below. Every text from the ledger is printed as
//! `escape_debug` writes it, so a recorded line break cannot forge a line.

use std::path::PathBuf;
use std::process::ExitCode;

use baley_core::capture::NO_SUCH_PHASE;
use baley_core::policy::recorded::recorded_policy;
use baley_core::policy::{CONFIG_UNAVAILABLE, SettingsFile};
use baley_core::scope::{UNKNOWN_DRAFT, judge_submission};
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
use crate::providers::identity;
use crate::scope::approve::{self, APPROVE_COMMAND};
use crate::scope::read::{
    self, BacklogStory, Detail, Draft, NO_SUCH_STORY, Phase, PreviewError, ReadError, Roadmap,
    Story,
};
use crate::scope::submit;
use crate::{init, policy_step, settings};

/// `baley scope`: the owner's approval of scope, and its reads.
#[derive(clap::Args, Debug)]
pub struct ScopeArgs {
    #[command(subcommand)]
    command: ScopeCommand,
}

#[derive(clap::Subcommand, Debug)]
enum ScopeCommand {
    /// Approve the newest start draft by its digest, as the owner. Phase
    /// numbers and story ids are assigned now and printed.
    Approve {
        /// The draft's digest from the scope-submit receipt: 64 lowercase
        /// hexadecimal characters.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
    /// Print a draft by its digest, exactly as an approval would record it.
    Preview {
        /// The draft's digest from the scope-submit receipt.
        #[arg(value_name = "DIGEST")]
        digest: String,
    },
    /// Print the accepted roadmap: the description, phases and stories.
    Show,
}

/// `baley phase`: the owner's reads of accepted phases.
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
}

/// `baley backlog`: the owner's read of the backlog.
#[derive(clap::Args, Debug)]
pub struct BacklogArgs {
    #[command(subcommand)]
    command: BacklogCommand,
}

#[derive(clap::Subcommand, Debug)]
enum BacklogCommand {
    /// Print every accepted story in priority order.
    Show,
}

/// `baley story`: the owner's reads of accepted stories.
#[derive(clap::Args, Debug)]
pub struct StoryArgs {
    #[command(subcommand)]
    command: StoryCommand,
}

#[derive(clap::Subcommand, Debug)]
enum StoryCommand {
    /// Print one accepted story.
    Show {
        /// The story id Baley assigned at approval.
        #[arg(value_name = "ID")]
        id: String,
    },
}

/// Runs one `baley scope` command and prints its result.
pub fn run(args: ScopeArgs) -> ExitCode {
    emit(match args.command {
        ScopeCommand::Approve { digest } => approve(&digest),
        ScopeCommand::Preview { digest } => judge_digest(&digest).and_then(|digest| {
            reading(|store, project| preview_render(read::draft(store, project, digest)))
        }),
        ScopeCommand::Show => {
            reading(|store, project| roadmap_render(read::roadmap(store, project)))
        }
    })
}

/// Runs one `baley phase` command and prints its result.
pub fn run_phase(args: PhaseArgs) -> ExitCode {
    let PhaseCommand::Show { number } = args.command;
    emit(judge_number(&number).and_then(|number| {
        reading(|store, project| phase_render(read::phase(store, project, number)))
    }))
}

/// Runs one `baley backlog` command and prints its result.
pub fn run_backlog(args: BacklogArgs) -> ExitCode {
    let BacklogCommand::Show = args.command;
    emit(reading(|store, project| {
        backlog_render(read::backlog(store, project))
    }))
}

/// Runs one `baley story` command and prints its result.
pub fn run_story(args: StoryArgs) -> ExitCode {
    let StoryCommand::Show { id } = args.command;
    emit(
        judge_number(&id)
            .and_then(|id| reading(|store, project| story_render(read::story(store, project, id)))),
    )
}

fn emit(result: Result<Render, Render>) -> ExitCode {
    ExitCode::from(display::emit(
        &result.unwrap_or_else(|e| e),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}

/// Finds the project and opens the ledger only to read: no checkout
/// admission, no policy step and nothing recorded, as `document` records
/// nothing.
fn reading(render: impl FnOnce(&SqliteStore, &ProjectId) -> Render) -> Result<Render, Render> {
    let checkout = checkout()?;
    let store = open(&checkout)?;
    Ok(render(&store, &checkout.project))
}

/// The checkout a command runs in: its project, repository root and
/// working-tree `baley.toml`, with Baley's folders.
struct Checkout {
    folders: Folders,
    project: ProjectId,
    root: PathBuf,
    working: Option<SettingsFile>,
}

/// Finds the project from the working directory through its `baley.toml`.
/// Outside a project the command is refused, naming `baley init`.
fn checkout() -> Result<Checkout, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read())
        .map_err(|e| Render::refusal(e.to_string()))?;
    let unavailable = |e: std::io::Error| {
        display::store_error(&baley_store::StoreError::Unavailable(e.to_string()), None)
    };
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let Discovery::Managed { folder, root } = discovery::discover(&ancestors) else {
        return Err(Render::refusal(format!(
            "{NOT_A_PROJECT}: scope commands run in a checkout of a project, and no {PROJECT_FILE} was found in this repository; baley init creates one"
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
fn open(checkout: &Checkout) -> Result<SqliteStore, Render> {
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
fn approve(digest: &str) -> Result<Render, Render> {
    let digest = judge_digest(digest)?;
    let checkout = checkout()?;
    let store = open(&checkout)?;
    let project = &checkout.project;
    let judged =
        approve::judge(&store, project, digest).map_err(|answer| answer_render(&answer))?;

    let reads = policy_step::gather(
        &checkout.folders.config,
        &checkout.root,
        checkout.working.as_ref(),
        &mut crate::process::System,
    );
    let policy = policy_step::build(&reads, None).map_err(|e| Render::refusal(e.to_string()))?;
    let recorded =
        recorded_policy(&checkout.root, &policy).map_err(|e| Render::refusal(e.to_string()))?;
    let site = Site {
        root: &checkout.root,
        policy: &policy,
        path: &recorded.checkout,
    };
    gather_and_admit(
        &store,
        project,
        &site,
        &mut crate::process::System,
        new_request_id(),
        &SystemClock::now(),
        None,
    )
    .map_err(|e| display::entry_error(&e, &project.0))?;
    let version = policy_step::step(
        &store,
        project,
        &recorded,
        new_request_id(),
        &SystemClock::now(),
        None,
    )
    .map_err(|e| display::store_error(&e, Some(&project.0)))?;

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
    Ok(answer_render(&submit::answer(recorded, &store)))
}

/// The digest argument: exactly 64 lowercase hexadecimal characters, as
/// the scope-submit receipt writes it. Anything else is refused before the
/// ledger is read, and the refusal echoes none of it.
pub(crate) fn judge_digest(text: &str) -> Result<Hash, Render> {
    let lowercase_hex =
        text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    lowercase_hex.then(|| Hash::from_hex(text)).flatten().ok_or_else(|| {
        Render::refusal(
            "a draft digest is 64 lowercase hexadecimal characters, as the scope-submit receipt gives it",
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
    let request = request_digest(&json!({
        "kind": APPROVE_COMMAND,
        "project": project.0,
        "actor": "owner",
        "policy_version": policy_version,
        "digest": digest.to_hex(),
        "scope": [],
    }))?;
    Ok(Command {
        project: project.clone(),
        kind: CommandKind(APPROVE_COMMAND.into()),
        request_id,
        digest: request,
        scope: Vec::new(),
        policy_version,
        recorded_at: at.into(),
        actor: Actor::Owner,
        caller: None,
    })
}

/// What the approval's answer prints: the receipt's three lines, exit 0; a
/// refusal as `<code>: <reason>`, exit 2, a stale draft's naming the first
/// part that differs; a failure the same way, exit 1.
pub(crate) fn answer_render(answer: &Value) -> Render {
    let text = |name: &str| escaped(answer[name].as_str().unwrap_or_default());
    let joined = |name: &str| {
        let numbers: Vec<String> = answer[name]
            .as_array()
            .map(|items| items.iter().map(Value::to_string).collect())
            .unwrap_or_default();
        numbers.join(", ")
    };
    let mut line = format!("{}: {}", text("code"), text("reason"));
    if let Some(path) = answer["details"]["path"].as_str() {
        line = format!("{line}: {}", escaped(path));
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

/// Text from the ledger as `escape_debug` writes it, so a recorded line
/// break or terminal escape cannot forge or rewrite a line of output.
fn escaped(text: &str) -> String {
    text.escape_debug().to_string()
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
    let Ok(submission) = judge_submission(&preview.submission) else {
        return unavailable("the draft is recorded, but its body no longer judges as a submission");
    };
    let mut lines = vec![format!(
        "draft {} ({}), submitted {}",
        preview.digest.to_hex(),
        escaped(&preview.kind),
        escaped(&preview.recorded_at)
    )];
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
/// priority order, or one line when it is empty.
pub(crate) fn backlog_render(read: Result<Vec<BacklogStory>, ReadError>) -> Render {
    match read {
        Ok(stories) if stories.is_empty() => Render::line("the backlog is empty", 0),
        Ok(stories) => printed(
            stories
                .iter()
                .map(|story| {
                    format!(
                        "story {} ({}, {}, {}, {} truths): {}",
                        story.id,
                        escaped(&story.status),
                        committed(story.phase),
                        escaped(&story.refinement),
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
fn describe(lines: &mut Vec<String>, what: &str, core_value: &str, constraints: &[String]) {
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

fn printed(lines: Vec<String>) -> Render {
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
fn read_error(error: ReadError) -> Render {
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
}
