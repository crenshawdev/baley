//! Gathering and wiring for the owner commands.
use super::{
    LedgerCommand,
    anchor_plan::{self, CheckAgainst},
    answer::answer_value,
    clock::SystemClock,
    command_plan::{self, Facts, Form, Op, ProjectFile, Settings, Verb},
    display::{self, Render},
    forge::GitForge,
    ticker::ThreadTicker,
    trace::StoreTrace,
};
use crate::checkout::{Site, gather_and_admit};
use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::{host_doctor, init, policy_step, settings};
use baley_core::policy::recorded::{PurgePolicy, RecordedPolicy, purge_policy, recorded_policy};
use baley_core::policy::{CONFIG_UNAVAILABLE, EffectivePolicy, ProjectIdentity, Unavailable};
use baley_core::*;
use baley_store::*;
use baley_store_sqlite::SqliteStore;
use serde_json::json;
use std::{
    collections::BTreeMap,
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
            local_only,
            views,
        } => command_plan::verify_form(project.as_deref(), local_only, views)
            .map_err(Render::refusal)
            .and_then(|form| verify(&store, &mut forge, config, &cwd, form)),
        LedgerCommand::Doctor => doctor(&store, &mut forge, config, &cwd),
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
/// Anchored `verify` reads the settings and writes nothing. The flagged
/// forms read no settings file.
fn verify(
    store: &SqliteStore,
    forge: &mut Forge,
    config: &Path,
    cwd: &Path,
    form: Form<'_>,
) -> Result<Render, Render> {
    let (ops, _) = settle(Verb::Verify(form), forge, config, cwd)?;
    match ops.as_slice() {
        [Op::Views(project)] => {
            let project = ProjectId(project.clone());
            store
                .verify_views(&project)
                .map(|r| display::views(&r))
                .map_err(|e| display::store_error(&e, Some(&project.0)))
        }
        [Op::Verify { project, remote }] => {
            let project = ProjectId(project.clone());
            verify_project(
                store,
                forge,
                &project,
                remote.as_deref(),
                &mut SystemClock::now,
            )
            .map(|r| display::verification(&r, remote.as_deref()))
            .map_err(|e| display::store_error(&e, Some(&project.0)))
        }
        _ => Err(unexpected_plan()),
    }
}
/// Checks the checkout's project against its `git.remote` and every other
/// project locally.
fn doctor(
    store: &SqliteStore,
    forge: &mut Forge,
    config: &Path,
    cwd: &Path,
) -> Result<Render, Render> {
    let projects = store
        .projects()
        .map_err(|e| display::store_error(&e, None))?;
    let gathered = gather_settings(cwd, config)?;
    let judged = command_plan::doctor_settings(&gathered.settings);
    let plan = anchor_plan::doctor_checks(judged.discovered.as_deref(), &judged.remote, &projects);
    if let Some(name) = &plan.validate {
        require_remote(forge, name)?;
    }
    let mut reasons = BTreeMap::new();
    let mut checks = BTreeMap::new();
    for (project, against) in plan.checks {
        let check = match &against {
            CheckAgainst::Remote(name) => anchor_check(forge, &project, Some(name)),
            CheckAgainst::Local(reason) => {
                reasons.insert(project.clone(), *reason);
                anchor_check(forge, &project, None)
            }
        };
        checks.insert(project, check);
    }
    let health = store
        .doctor(&SystemClock::now(), &checks)
        .map_err(|e| display::store_error(&e, None))?;
    let rendered = display::doctor(&health, &projects, &reasons, &judged);
    Ok(display::with_host(rendered, &host_report()))
}

/// The host section: the running binary against a placement map with every
/// artifact unknown, so each one is reported as not installed.
fn host_report() -> host_doctor::Report {
    let observation = host_doctor::gather(host_doctor::all_unknown(std::env::current_exe()));
    host_doctor::Report::new(&host_doctor::judge(&observation))
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
/// the project it names, after checkout admission, 0 anywhere else, where no
/// settings file is read and nothing is admitted.
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
    let reads = policy_step::gather(config, &root, working.as_ref(), &mut crate::process::System);
    let policy = policy_step::build(&reads, None).map_err(|e| Render::refusal(e.to_string()))?;
    let recorded = recorded_policy(&root, &policy).map_err(|e| Render::refusal(e.to_string()))?;
    let site = Site {
        root: &root,
        policy: &policy,
        path: &recorded.checkout,
    };
    admit_checkout(store, project, &site)?;
    run_step(store, &recorded, project)
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
        caller: None,
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
            let reads = policy_step::gather(
                config,
                &root,
                read.as_ref().ok().and_then(Option::as_ref),
                &mut crate::process::System,
            );
            Gathered {
                settings: Settings {
                    project_file: Some(ProjectFile { path, id }),
                    policy: policy_step::build(&reads, None),
                    pending: command_plan::pending_note(&reads),
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
                    policy: policy_step::build(&reads, None),
                    pending: command_plan::pending_note(&reads),
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

/// The checkout's root, built policy and recorded policy, built once for
/// checkout admission and the policy step.
fn checkout_policy(
    gathered: Option<&Gathered>,
) -> Result<(&Path, &EffectivePolicy, RecordedPolicy), Render> {
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
    Ok((root, policy, recorded))
}

/// Gathers the checkout's facts and admits it, before the policy step. It
/// prints nothing.
fn admit_checkout(store: &SqliteStore, project: &ProjectId, site: &Site<'_>) -> Result<(), Render> {
    gather_and_admit(
        store,
        project,
        site,
        &mut crate::process::System,
        new_request_id(),
        &SystemClock::now(),
        None,
    )
    .map_err(|e| display::entry_error(&e, &project.0))
}

/// Runs the policy step for the checkout and returns the version in force.
fn run_step(
    store: &SqliteStore,
    recorded: &RecordedPolicy,
    project: &ProjectId,
) -> Result<u64, Render> {
    policy_step::step(
        store,
        project,
        recorded,
        new_request_id(),
        &SystemClock::now(),
        None,
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
    let [Op::AdmitCheckout, Op::Step, Op::Anchor { project, remote }] = ops.as_slice() else {
        return Err(unexpected_plan());
    };
    let project = ProjectId(project.clone());
    let (root, policy, recorded) = checkout_policy(gathered.as_ref())?;
    let site = Site {
        root,
        policy,
        path: &recorded.checkout,
    };
    admit_checkout(&store, &project, &site)?;
    let policy_version = run_step(&store, &recorded, &project)?;
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
    let [
        Op::AdmitCheckout,
        Op::Step,
        Op::Acknowledge { project, remote },
    ] = ops.as_slice()
    else {
        return Err(unexpected_plan());
    };
    let project = ProjectId(project.clone());
    let (root, policy, recorded) = checkout_policy(gathered.as_ref())?;
    let site = Site {
        root,
        policy,
        path: &recorded.checkout,
    };
    admit_checkout(store, &project, &site)?;
    let policy_version = run_step(store, &recorded, &project)?;
    let request = acknowledge_request(&project, remote, policy_version, new_request_id());
    display::request_line(&mut std::io::stdout(), &request.request_id.0);
    let result = acknowledge_restore(&request, store, forge, &mut SystemClock::now)
        .map_err(|e| display::acknowledgement_error(&e, &project.0))?;
    Ok(display::acknowledgement(&result, &project, store))
}
