//! The owner's scope commands (design 0004 section 5, D-20): `baley scope
//! approve <digest>` approves the newest start draft as the owner, the only
//! way scope is approved (D-24). A submission comes only from a session,
//! so no command here submits scope.
//!
//! Gathering finds the project and opens the ledger; judging, the digest
//! argument, the command an approval is recorded under and what is printed,
//! is done by the pure functions below.

use std::path::PathBuf;
use std::process::ExitCode;

use baley_core::policy::recorded::recorded_policy;
use baley_core::policy::{CONFIG_UNAVAILABLE, SettingsFile};
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
use crate::mcp::prepare::{NOT_A_PROJECT, PROJECT_NOT_IN_LEDGER};
use crate::providers::identity;
use crate::scope::approve::{self, APPROVE_COMMAND};
use crate::scope::submit;
use crate::{init, policy_step, settings};

/// `baley scope`: the owner's approval of scope.
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
}

/// Runs one `baley scope` command and prints its result.
pub fn run(args: ScopeArgs) -> ExitCode {
    let result = match args.command {
        ScopeCommand::Approve { digest } => approve(&digest),
    }
    .unwrap_or_else(|e| e);
    ExitCode::from(display::emit(
        &result,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
