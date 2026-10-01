//! Gathering and wiring for the owner commands.
use super::{
    LedgerCommand,
    answer::answer_value,
    clock::SystemClock,
    command_plan::{self, Facts, Op, ProjectFile, Settings, Verb},
    display::{self, Render},
    forge::GitForge,
    remotes::anchor_plan,
    ticker::ThreadTicker,
    trace::StoreTrace,
};
use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::{init, policy_step, settings};
use baley_core::policy::recorded::{PurgePolicy, purge_policy, recorded_policy};
use baley_core::policy::{CONFIG_UNAVAILABLE, ProjectIdentity, Unavailable};
use baley_core::*;
use baley_store::*;
use baley_store_sqlite::SqliteStore;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

type Forge = GitForge<crate::process::System>;
pub(crate) fn new_request_id() -> RequestId {
    RequestId(uuid::Uuid::new_v4().to_string())
}
fn require_remote(forge: &mut Forge, name: &str) -> Result<(), Render> {
    forge.require_remote(name).map_err(|e| Render::refusal(e.0))
}

/// Wires one owner command to the core and storage port. `config` is the
/// config folder, where `purge`'s policy step finds the global file.
pub(super) fn dispatch(
    command: LedgerCommand,
    store: Arc<SqliteStore>,
    cwd: PathBuf,
    config: &Path,
    started_at: String,
) -> Render {
    let mut forge = GitForge::new(crate::process::System, cwd.clone(), SystemClock::seconds);
    let result = match command {
        LedgerCommand::Verify {
            project,
            remote,
            views,
            ..
        } => verify(
            &store,
            &mut forge,
            &ProjectId(project),
            remote.as_deref(),
            views,
        ),
        LedgerCommand::Doctor { remote, local_only } => {
            doctor(&store, &mut forge, &remote, &local_only)
        }
        LedgerCommand::Export { project, to } => export(&store, &ProjectId(project), &to),
        LedgerCommand::Purge {
            project,
            hashes,
            reason,
        } => purge(&store, config, &cwd, &ProjectId(project), hashes, &reason),
        LedgerCommand::Scrub => scrub(&store),
        LedgerCommand::Rebuild { project } => rebuild(&store, &ProjectId(project)),
        LedgerCommand::Anchor { project } => anchor(
            store,
            &mut forge,
            config,
            &cwd,
            project.as_deref(),
            started_at,
        ),
        LedgerCommand::AcknowledgeRestore { project } => {
            acknowledge(&store, &mut forge, config, &cwd, project.as_deref())
        }
    };
    let mut rendered = result.unwrap_or_else(|e| e);
    if let Some(message) = forge.last_error.filter(|s| !s.is_empty()) {
        rendered.lines.push(format!("git: {message}"));
    }
    rendered
}
fn verify(
    store: &SqliteStore,
    forge: &mut Forge,
    project: &ProjectId,
    remote: Option<&str>,
    views: bool,
) -> Result<Render, Render> {
    if views {
        return store
            .verify_views(project)
            .map(|r| display::views(&r))
            .map_err(|e| display::store_error(&e, Some(&project.0)));
    }
    if let Some(remote) = remote {
        require_remote(forge, remote)?;
    }
    verify_project(store, forge, project, remote, &mut SystemClock::now)
        .map(|r| display::verification(&r, remote))
        .map_err(|e| display::store_error(&e, Some(&project.0)))
}
fn doctor(
    store: &SqliteStore,
    forge: &mut Forge,
    remotes: &[(String, String)],
    local: &[String],
) -> Result<Render, Render> {
    let projects = store
        .projects()
        .map_err(|e| display::store_error(&e, None))?;
    let plan = anchor_plan(&projects, remotes, local).map_err(|e| Render::refusal(e.0))?;
    for remote in plan.values().flatten() {
        require_remote(forge, remote)?;
    }
    let checks = plan
        .into_iter()
        .map(|(project, remote)| {
            let check = anchor_check(forge, &project, remote.as_deref());
            (project, check)
        })
        .collect();
    store
        .doctor(&SystemClock::now(), &checks)
        .map(|h| display::doctor(&h, &projects))
        .map_err(|e| display::store_error(&e, None))
}
fn export(store: &SqliteStore, project: &ProjectId, to: &Path) -> Result<Render, Render> {
    store
        .export(project, to, &SystemClock::now())
        .map(|r| display::export(&project.0, &r))
        .map_err(|e| display::store_error(&e, Some(&project.0)))
}
fn purge(
    store: &SqliteStore,
    config: &Path,
    cwd: &Path,
    project: &ProjectId,
    mut hashes: Vec<Hash>,
    reason: &str,
) -> Result<Render, Render> {
    hashes.sort();
    let policy_version = purge_policy_version(store, config, cwd, project)?;
    let request_id = new_request_id();
    display::request_line(&mut std::io::stdout(), &request_id.0);
    let at = SystemClock::now();
    let command = purge_command(project, policy_version, &hashes, reason, request_id, &at)
        .map_err(|e| Render::refusal(e.to_string()))?;
    store
        .purge(&command, &hashes, reason)
        .map(|r| display::purge(&r))
        .map_err(|e| display::store_error(&e, Some(&project.0)))
}

/// The policy version purge records: the policy step's from a checkout of
/// the project it names, 0 anywhere else, where no settings file is read.
fn purge_policy_version(
    store: &SqliteStore,
    config: &Path,
    cwd: &Path,
    project: &ProjectId,
) -> Result<u64, Render> {
    let ancestors = discovery::ancestors(cwd)
        .map_err(|e| display::store_error(&StoreError::Unavailable(e.to_string()), None))?;
    let checkout = match discovery::discover(&ancestors) {
        Discovery::Managed { folder, root } => {
            let path = folder.join(PROJECT_FILE);
            let read = settings::read(&path);
            let id =
                purge_project(&path, init::observe_file(read.clone())).map_err(Render::refusal)?;
            Some((id, root, read.ok().flatten()))
        }
        Discovery::Unmanaged { .. } | Discovery::Outside => None,
    };
    let discovered = checkout.as_ref().map(|(id, ..)| id.as_str());
    let choice = purge_policy(discovered, &project.0);
    let (PurgePolicy::RunStep, Some((_, root, working))) = (choice, checkout) else {
        return Ok(0);
    };
    let reads = policy_step::gather(config, &root, working.as_ref());
    let policy = policy_step::build(&reads).map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = recorded_policy(&root, &policy).map_err(|e| Render::refusal(e.to_string()))?;
    policy_step::step(
        store,
        project,
        &recorded,
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| display::store_error(&e, Some(&project.0)))
}

/// The project id of the checkout's `baley.toml` at `path`, from
/// `init::observe_file`. A managed checkout whose file yields no id is
/// refused, never taken as no project, since that would record version 0
/// for a purge run in the project's own checkout.
pub(super) fn purge_project(
    path: &Path,
    observed: Result<Option<ProjectIdentity>, Unavailable>,
) -> Result<String, String> {
    const OUTSIDE: &str =
        "(a purge run outside a checkout of the project records policy version 0)";
    match observed {
        Ok(Some(identity)) => Ok(identity.id),
        Ok(None) => Err(format!(
            "{CONFIG_UNAVAILABLE}: {} was not found {OUTSIDE}",
            path.display()
        )),
        Err(unavailable) => Err(format!("{unavailable} {OUTSIDE}")),
    }
}

/// Purge's own command, carrying `policy_version` in both its digest and
/// itself. `hashes` are sorted.
pub(super) fn purge_command(
    project: &ProjectId,
    policy_version: u64,
    hashes: &[Hash],
    reason: &str,
    request_id: RequestId,
    at: &str,
) -> Result<Command, CanonicalError> {
    let hashes: Vec<String> = hashes.iter().map(|hash| hash.to_hex()).collect();
    let digest = request_digest(&json!({
        "kind": "payload.purge",
        "project": project.0,
        "actor": "owner",
        "policy_version": policy_version,
        "hashes": hashes,
        "reason": reason,
        "scope": [],
    }))?;
    Ok(Command {
        project: project.clone(),
        kind: CommandKind("payload.purge".into()),
        request_id,
        digest,
        scope: vec![],
        policy_version,
        recorded_at: at.into(),
        actor: Actor::Owner,
    })
}
fn scrub(store: &SqliteStore) -> Result<Render, Render> {
    store
        .scrub()
        .map(|r| {
            Render::line(
                if r.scrubbed {
                    "scrub complete"
                } else {
                    "scrub incomplete: close any reader of the ledger and run baley scrub again"
                },
                u8::from(!r.scrubbed),
            )
        })
        .map_err(|e| display::store_error(&e, None))
}
fn rebuild(store: &SqliteStore, project: &ProjectId) -> Result<Render, Render> {
    store
        .rebuild(project)
        .map(|r| {
            Render::line(
                format!(
                    "generation {} is live; {} events replayed",
                    r.generation, r.events
                ),
                0,
            )
        })
        .map_err(|e| display::store_error(&e, Some(&project.0)))
}
/// What the checkout's settings reads found, with the repository root the
/// policy step records.
struct Gathered {
    settings: Settings,
    root: Option<PathBuf>,
}

/// Finds the project from `cwd`, as `purge` does, and reads both settings
/// files once. The `--project-root` flag is not read here.
fn gather_settings(cwd: &Path, config: &Path) -> Result<Gathered, Render> {
    let ancestors = discovery::ancestors(cwd)
        .map_err(|e| display::store_error(&StoreError::Unavailable(e.to_string()), None))?;
    Ok(match discovery::discover(&ancestors) {
        Discovery::Managed { folder, root } => {
            let path = folder.join(PROJECT_FILE);
            let read = settings::read(&path);
            let id = init::observe_file(read.clone());
            let reads =
                policy_step::gather(config, &root, read.as_ref().ok().and_then(Option::as_ref));
            Gathered {
                settings: Settings {
                    project_file: Some(ProjectFile { path, id }),
                    policy: policy_step::build(&reads),
                },
                root: Some(root),
            }
        }
        Discovery::Unmanaged { .. } | Discovery::Outside => {
            let reads = policy_step::Reads {
                global: settings::read(&config.join(settings::GLOBAL_FILE)),
                head: None,
            };
            Gathered {
                settings: Settings {
                    project_file: None,
                    policy: policy_step::build(&reads),
                },
                root: None,
            }
        }
    })
}

/// Asks the plan and performs the reads and the check it requests, until it
/// gives the operations that follow them or a refusal.
fn settle(
    command: Verb<'_>,
    forge: &mut Forge,
    config: &Path,
    cwd: &Path,
) -> Result<(Vec<Op>, Option<Gathered>), Render> {
    let plan = |settings: Option<&Settings>, remote_check: Option<&Result<(), String>>| {
        command_plan::next(&Facts {
            command,
            settings,
            remote_check,
        })
        .map_err(Render::refusal)
    };
    let mut ops = plan(None, None)?;
    let mut gathered = None;
    if ops == [Op::ReadSettings] {
        let read = gather_settings(cwd, config)?;
        ops = plan(Some(&read.settings), None)?;
        gathered = Some(read);
    }
    if let [Op::CheckRemote(name)] = ops.as_slice() {
        let verdict = forge.require_remote(name).map_err(|refusal| refusal.0);
        ops = plan(gathered.as_ref().map(|g| &g.settings), Some(&verdict))?;
    }
    Ok((ops, gathered))
}

/// A plan that asked for an operation its facts cannot carry out.
fn unexpected_plan() -> Render {
    Render::refusal("internal error: the command plan asked for an operation it has no facts for")
}

/// Runs the policy step for the checkout and returns the version in force.
fn run_step(
    store: &SqliteStore,
    gathered: Option<&Gathered>,
    project: &ProjectId,
) -> Result<u64, Render> {
    let Some(Gathered {
        settings: Settings {
            policy: Ok(policy), ..
        },
        root: Some(root),
    }) = gathered
    else {
        return Err(unexpected_plan());
    };
    let recorded = recorded_policy(root, policy).map_err(|e| Render::refusal(e.to_string()))?;
    policy_step::step(
        store,
        project,
        &recorded,
        new_request_id(),
        &SystemClock::now(),
    )
    .map_err(|e| display::store_error(&e, Some(&project.0)))
}

/// The anchor request, carrying the version the policy step returned in both
/// its digest and itself.
pub(super) fn anchor_request(
    project: &ProjectId,
    remote: Option<&str>,
    policy_version: u64,
    request_id: RequestId,
    reconcile_request_id: RequestId,
    owner: ClaimOwner,
) -> AnchorRequest {
    AnchorRequest {
        project: project.clone(),
        request_id,
        reconcile_request_id,
        actor: Actor::Owner,
        owner,
        remote: remote.map(Into::into),
        policy_version,
    }
}

fn anchor(
    store: Arc<SqliteStore>,
    forge: &mut Forge,
    config: &Path,
    cwd: &Path,
    named: Option<&str>,
    started_at: String,
) -> Result<Render, Render> {
    let (ops, gathered) = settle(Verb::Anchor { named }, forge, config, cwd)?;
    let [Op::Step, Op::Anchor { project, remote }] = ops.as_slice() else {
        return Err(unexpected_plan());
    };
    let project = ProjectId(project.clone());
    // Owner: T13 (phase 9) adds the checkout's admission here, between the
    // settings read above and the policy step below.
    let policy_version = run_step(&store, gathered.as_ref(), &project)?;
    let request_id = new_request_id();
    display::request_line(&mut std::io::stdout(), &request_id.0);
    let owner = ClaimOwner {
        process: std::process::id().to_string(),
        host_session: "cli".into(),
        started_at,
    };
    let request = anchor_request(
        &project,
        remote.as_deref(),
        policy_version,
        request_id,
        new_request_id(),
        owner,
    );
    let mut ticker = ThreadTicker::new(Arc::new(SystemClock::now));
    let trace = StoreTrace(store.clone());
    let report = anchor_command(
        &request,
        AnchorSeams {
            ledger: store.clone(),
            forge,
            ticker: &mut ticker,
            trace: &trace,
            now: &mut SystemClock::now,
        },
    )
    .map_err(|e| display::anchor_error(&e, &request.request_id.0, remote.as_deref()))?;
    let outcome = match &report.outcome {
        AnchorOutcome::Recorded { outcome, .. }
        | AnchorOutcome::LateReplay { outcome, .. }
        | AnchorOutcome::Refused { outcome, .. }
        | AnchorOutcome::Replayed(outcome) => Some(outcome),
        _ => None,
    };
    let answer = outcome.map(|o| answer_value(&o.answer, store.as_ref()));
    Ok(display::anchor(
        &report,
        answer.as_ref(),
        &project,
        &request.request_id.0,
    ))
}
/// The acknowledgement, carrying the version the policy step returned.
pub(super) fn acknowledge_request(
    project: &ProjectId,
    remote: &str,
    policy_version: u64,
    request_id: RequestId,
) -> AcknowledgeRestore {
    AcknowledgeRestore {
        project: project.clone(),
        request_id,
        actor: Actor::Owner,
        policy_version,
        remote: remote.into(),
    }
}

fn acknowledge(
    store: &SqliteStore,
    forge: &mut Forge,
    config: &Path,
    cwd: &Path,
    named: Option<&str>,
) -> Result<Render, Render> {
    let (ops, gathered) = settle(Verb::AcknowledgeRestore { named }, forge, config, cwd)?;
    let [Op::Step, Op::Acknowledge { project, remote }] = ops.as_slice() else {
        return Err(unexpected_plan());
    };
    let project = ProjectId(project.clone());
    // Owner: T13 (phase 9) adds the checkout's admission here, between the
    // settings read above and the policy step below.
    let policy_version = run_step(store, gathered.as_ref(), &project)?;
    let request = acknowledge_request(&project, remote, policy_version, new_request_id());
    display::request_line(&mut std::io::stdout(), &request.request_id.0);
    let result = acknowledge_restore(&request, store, forge, &mut SystemClock::now)
        .map_err(|e| display::acknowledgement_error(&e, &project.0))?;
    let outcome = match result {
        Recorded::New { outcome, .. } | Recorded::Replayed { outcome } => outcome,
    };
    Ok(display::recorded(
        outcome.kind,
        &answer_value(&outcome.answer, store),
        &project,
        true,
    ))
}
