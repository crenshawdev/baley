//! Gathering and wiring for the owner commands.
use super::{
    LedgerCommand,
    answer::answer_value,
    clock::SystemClock,
    display::{self, Render},
    forge::GitForge,
    remotes::anchor_plan,
    ticker::ThreadTicker,
    trace::StoreTrace,
};
use baley_core::*;
use baley_store::*;
use baley_store_sqlite::SqliteStore;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

type Forge = GitForge<crate::process::System>;
fn new_request_id() -> RequestId {
    RequestId(uuid::Uuid::new_v4().to_string())
}
fn require_remote(forge: &mut Forge, name: &str) -> Result<(), Render> {
    forge.require_remote(name).map_err(|e| Render::refusal(e.0))
}

/// Wires one owner command to the core and storage port.
pub(super) fn dispatch(
    command: LedgerCommand,
    store: Arc<SqliteStore>,
    cwd: PathBuf,
    started_at: String,
) -> Render {
    let mut forge = GitForge::new(crate::process::System, cwd, SystemClock::seconds);
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
        } => purge(&store, &ProjectId(project), hashes, &reason),
        LedgerCommand::Scrub => scrub(&store),
        LedgerCommand::Rebuild { project } => rebuild(&store, &ProjectId(project)),
        LedgerCommand::Anchor { project, remote } => {
            anchor(store, &mut forge, ProjectId(project), remote, started_at)
        }
        LedgerCommand::AcknowledgeRestore { project, remote } => {
            acknowledge(&store, &mut forge, ProjectId(project), remote)
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
    project: &ProjectId,
    mut hashes: Vec<Hash>,
    reason: &str,
) -> Result<Render, Render> {
    hashes.sort();
    let request_id = new_request_id();
    println!("request {}", request_id.0);
    let digest = request_digest(&json!({"kind":"payload.purge","project":project.0,"actor":"owner","policy_version":0,"hashes":hashes.iter().map(|h| h.to_hex()).collect::<Vec<_>>(),"reason":reason,"scope":[]})).map_err(|e|Render::refusal(e.to_string()))?;
    let command = Command {
        project: project.clone(),
        kind: CommandKind("payload.purge".into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: SystemClock::now(),
        actor: Actor::Owner,
    };
    store
        .purge(&command, &hashes, reason)
        .map(|r| display::purge(&r))
        .map_err(|e| display::store_error(&e, Some(&project.0)))
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
fn anchor(
    store: Arc<SqliteStore>,
    forge: &mut Forge,
    project: ProjectId,
    remote: String,
    started_at: String,
) -> Result<Render, Render> {
    require_remote(forge, &remote)?;
    let request_id = new_request_id();
    println!("request {}", request_id.0);
    let request = AnchorRequest {
        project: project.clone(),
        request_id,
        reconcile_request_id: new_request_id(),
        actor: Actor::Owner,
        owner: ClaimOwner {
            process: std::process::id().to_string(),
            host_session: "cli".into(),
            started_at,
        },
        remote: Some(remote.clone()),
        policy_version: 0,
    };
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
    .map_err(|e| display::anchor_error(&e, &request.request_id.0, &project.0, &remote))?;
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
fn acknowledge(
    store: &SqliteStore,
    forge: &mut Forge,
    project: ProjectId,
    remote: String,
) -> Result<Render, Render> {
    require_remote(forge, &remote)?;
    let request = AcknowledgeRestore {
        project: project.clone(),
        request_id: new_request_id(),
        actor: Actor::Owner,
        policy_version: 0,
        remote: remote.clone(),
    };
    println!("request {}", request.request_id.0);
    let result = acknowledge_restore(&request, store, forge, &mut SystemClock::now)
        .map_err(|e| display::acknowledgement_error(&e, &project.0, &remote))?;
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
