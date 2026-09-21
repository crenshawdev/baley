//! task-open and task-close. A treeless episode is resident memory for the
//! run: no first_touch, no store, nothing created under the project.
use crate::{config::reload::ConfigIo, import::SessionFactory};
use cadence::{
    envelope::Refusal,
    pause::branch,
    rail::{risk, risk_diff},
    store::{Error, Result},
    task::{self, model::{self, Apply, Mode, Recording, Risk, Root}},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};

pub enum Command { Apply(Apply) }

/// One open treeless task, held by the resident until its close.
#[derive(Clone, Debug)]
pub struct Episode {
    pub slug: String,
    pub mode: Mode,
    pub token: String,
    pub root: Root,
    pub branch: String,
    pub start: String,
    pub description: String,
}

/// The resident's memory of treeless tasks: open episodes by project and
/// token, and each request's answer for replay. Lost on restart by design.
#[derive(Default)]
pub struct Episodes {
    open: BTreeMap<(PathBuf, String), Episode>,
    receipts: BTreeMap<(PathBuf, String), (Apply, Value)>,
}

static TOKEN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub async fn execute<I: ConfigIo + Clone + Sync>(
    factory: &SessionFactory<I>, root: &Path, command: Command, episodes: &mut Episodes,
) -> Result<Value> {
    let Command::Apply(apply) = command;
    let slug = apply.slug().to_owned();
    Ok(match execute_inner(factory, root, apply, episodes).await {
        Ok(answer) => answer,
        Err(error) => Refusal::new("task-unavailable", error.to_string()).slot("request").details(json!({"slug":slug})).value(),
    })
}

async fn execute_inner<I: ConfigIo + Clone + Sync>(
    factory: &SessionFactory<I>, root: &Path, apply: Apply, episodes: &mut Episodes,
) -> Result<Value> {
    let project = root.parent().ok_or_else(|| Error::Invalid("planning root lacks project".into()))?.to_path_buf();
    let key = (project.clone(), apply.request_id().to_owned());
    if let Some((saved, answer)) = episodes.receipts.get(&key) {
        return Ok(if *saved == apply { answer.clone() } else { model::reused() });
    }
    cadence::milestone::model::name(apply.request_id())?;
    // The boundary decides root presence from the root path itself, before
    // any first_touch and never from an unrelated missing file.
    let classified = Root::classify(root)?;
    let answer = match &apply {
        Apply::Open { request } => open(factory, &project, classified, request, episodes).await?,
        Apply::Close { request } => close(factory, &project, classified, request, episodes).await?,
    };
    episodes.receipts.insert(key, (apply, answer.clone()));
    Ok(answer)
}

async fn open<I: ConfigIo + Clone + Sync>(
    factory: &SessionFactory<I>, project: &Path, root: Root, request: &model::Open, episodes: &mut Episodes,
) -> Result<Value> {
    if let Err(error) = model::validate_slug(&request.slug).and_then(|()| model::validate_description(&request.description)) {
        return Ok(model::invalid(error.to_string(), &request.slug));
    }
    let Root::Absent { .. } = &root else {
        return Ok(Refusal::new("task-record-unavailable",
                format!("a planning root is present at {}; this build serves task records only without one", root.path()))
            .rule("rooted-record").slot("root").details(json!({"root":root,"slug":request.slug})).value());
    };
    // The global layer is the only configuration a treeless project has; it
    // is observed, never acquired, and nothing is persisted from it.
    let (generation, _) = factory.observe_config(&project.join(".planning"))?;
    let policy = super::pause_service::policy(&generation)?;
    let observed = {
        let (project, planning, policy) = (project.to_path_buf(), PathBuf::from(root.path()), policy.clone());
        tokio::task::spawn_blocking(move || branch::observe(&project, &planning, &policy)).await.map_err(|_| Error::Closed)??
    };
    match branch::protected(&policy, &observed) {
        Ok(None) => {}
        Ok(Some(gate)) => {
            return Ok(Refusal::new("protected-branch",
                    format!("branch {} is protected and the policy asks; switch to an authorized work branch before task-open", observed.branch))
                .rule("branch-policy").slot("branch")
                .details(json!({"branch":observed.branch,"permission":"ask","gate":gate,
                    "policy":{"protected":policy.protected,"on_protected":policy.on_protected}}))
                .value());
        }
        Err(Error::Policy(_)) => {
            return Ok(Refusal::new("protected-branch",
                    format!("branch {} is protected and the policy refuses work on it", observed.branch))
                .rule("branch-policy").slot("branch")
                .details(json!({"branch":observed.branch,"permission":"refuse",
                    "policy":{"protected":policy.protected,"on_protected":policy.on_protected}}))
                .value());
        }
        Err(error) => return Err(error),
    }
    let sequence = TOKEN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let identity = cadence::store::model::digest(&serde_json::to_vec(&(
        project.to_string_lossy(), &request.slug, &observed.head, &request.request_id, std::process::id(), sequence))?);
    let episode = Episode {
        slug: request.slug.clone(), mode: request.mode, token: format!("task-{}", &identity[..24]), root: root.clone(),
        branch: observed.branch.clone(), start: observed.head.clone(), description: request.description.clone(),
    };
    let answer = json!({"status":"ok","outcome":"open","ephemeral":true,
        "task":{"slug":episode.slug,"mode":episode.mode,"token":episode.token,"branch":episode.branch,"start":episode.start},
        "root":root,"recording":model::unrecorded(&root),
        "policy":{"protected":policy.protected,"on_protected":policy.on_protected,"permission":"pass"}});
    episodes.open.insert((project.to_path_buf(), episode.token.clone()), episode);
    Ok(answer)
}

async fn close<I: ConfigIo + Clone + Sync>(
    factory: &SessionFactory<I>, project: &Path, root: Root, request: &model::Close, episodes: &mut Episodes,
) -> Result<Value> {
    let key = (project.to_path_buf(), request.token.clone());
    let Some(episode) = episodes.open.get(&key).filter(|episode| episode.slug == request.slug).cloned() else {
        return Ok(model::unknown_task(&request.slug, &request.token));
    };
    if episode.root != root {
        return Ok(model::invalid(format!("planning root changed since open: {} is now {}", episode.root.path(),
            match root { Root::Absent { .. } => "absent", Root::Present { .. } => "present" }), &request.slug));
    }
    // A missing report file is that file, named by its own path.
    let report = match &request.report {
        model::Report::Text { text } => text.clone(),
        model::Report::File { path } => match std::fs::read(path) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| Error::Invalid("report file is not UTF-8".into()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(model::missing_file(path, &root)),
            Err(error) => return Err(Error::Io(format!("report file {path}: {error}"))),
        },
    };
    if let Err(error) = model::validate_report(&report) {
        return Ok(model::invalid(error.to_string(), &request.slug));
    }
    let range = {
        let (project, start) = (project.to_path_buf(), episode.start.clone());
        tokio::task::spawn_blocking(move || task::observe_range(&project, &start)).await.map_err(|_| Error::Closed)??
    };
    let (generation, _) = factory.observe_config(&project.join(".planning"))?;
    let gate = crate::config::merge::get(&generation.effective.values, "review.triggers.risk_surface.gate")
        .and_then(Value::as_str).ok_or_else(|| Error::Policy("missing effective risk_surface gate".into()))?.to_owned();
    let mut transient = Value::Null;
    let risk = match &range.material {
        None => Risk::Skipped,
        Some(_) => {
            let Some(surfaces) = request.surfaces.clone() else {
                return Ok(Refusal::new("unanswered-surfaces",
                        format!("{}..{} landed commits and no risk surfaces were answered for this run", episode.start, range.head))
                    .rule("risk-gate").slot("request.surfaces").details(json!({"slug":request.slug})).value());
            };
            let surfaces = match risk::validate_surfaces(surfaces) {
                Ok(surfaces) => surfaces,
                Err(error) => return Ok(model::invalid(error.to_string(), &request.slug)),
            };
            let scan = risk_diff::scan(Some(&range.diff), &range.diff_paths, &surfaces)?;
            if cadence::rail::receipts::scan_requires_review(&scan) {
                // Matched material is held per run and gone before the answer.
                let held = task::Transient::create(&episode.token)?;
                let path = held.hold(&format!("risk-task-{}.diff", episode.slug), &range.diff)?;
                let bytes = std::fs::read(&path)?;
                transient = json!({"location":held.location(),"file":path,"bytes":bytes.len(),
                    "digest":cadence::store::model::digest(&bytes)});
                drop(held);
            }
            model::disposition(scan, surfaces, &gate)
        }
    };
    let record = model::Record {
        schema: model::RECORD_SCHEMA.into(), slug: episode.slug.clone(), mode: episode.mode,
        description: episode.description.clone(), token: episode.token.clone(),
        root: root.clone(), branch: episode.branch.clone(), start: episode.start.clone(), head: range.head.clone(),
        commits: range.commits.clone(), files: range.files.clone(), risk, report, recording: model::unrecorded(&root),
    };
    if matches!(record.risk, Risk::Blocked { .. }) {
        return Ok(model::blocked(&record, transient));
    }
    debug_assert!(matches!(record.recording, Recording::Unrecorded { .. }));
    episodes.open.remove(&key);
    Ok(model::done(&record))
}
