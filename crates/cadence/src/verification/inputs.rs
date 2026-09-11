//! Observe current native authority and committed source as one checked input.
use super::model::{Basis, Source, TruthVersion};
use crate::{execution::{admission, history, receipts, runner}, plan, store::{Error, Result, Storage, model::digest}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    pub basis: Basis,
    pub map: Value,
    pub admissions: Vec<admission::Record>,
    pub execution: Value,
    pub checks: Vec<Value>,
    pub authority_digest: String,
}

pub fn refuse(phase: u32, rule: &str, slot: &str, reason: impl Into<String>) -> Error {
    admission::refuse(phase, rule, slot, "", reason)
}

pub fn authority_digest(data: &Value) -> Result<String> {
    let mut data = data.clone();
    if let Some(object) = data.as_object_mut() { object.remove(super::persistence::NAMESPACE); }
    Ok(digest(&serde_json::to_vec(&data)?))
}

pub fn root_binding(root: &Path) -> Result<String> {
    Ok(crate::store::filesystem::Filesystem::new(root)?.read(crate::store::model::STATE)?.directory_identity)
}

/// Observe actual tracked bytes too: Git's index flags cannot hide modified
/// files. Symlinks contribute their own text, never their destination's bytes.
pub fn source(project: &Path) -> Result<Source> {
    runner::clean(project)?;
    let head = runner::git_text(project, &["rev-parse", "HEAD"])?;
    let tree = runner::git_text(project, &["rev-parse", "HEAD^{tree}"])?;
    let index = runner::git(project, &["ls-files", "--stage", "-z"])?;
    let mut material = Vec::new();
    for row in index.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let row = std::str::from_utf8(row).map_err(|_| Error::Invalid("unrepresentable index path".into()))?;
        let (entry, name) = row.split_once('\t').ok_or_else(|| Error::Invalid("invalid index entry".into()))?;
        if !entry.ends_with(" 0") || !crate::execution::patch::safe_relative_path(name) {
            return Err(Error::Invalid("unmerged or unsafe index path".into()));
        }
        let path = project.join(name);
        let metadata = std::fs::symlink_metadata(&path)?;
        let bytes = if metadata.file_type().is_symlink() {
            std::fs::read_link(path)?.as_os_str().as_encoded_bytes().to_vec()
        } else if metadata.is_file() { std::fs::read(path)? }
        else { return Err(Error::Invalid(format!("ambiguous source material: {name}"))); };
        if runner::git(project, &["show", &format!("{head}:{name}")])? != bytes {
            return Err(Error::Invalid(format!("tracked material differs from HEAD: {name}")));
        }
        material.push((entry.to_owned(), name.to_owned(), digest(&bytes)));
    }
    runner::clean(project)?;
    if runner::git_text(project, &["rev-parse", "HEAD"])? != head
        || runner::git(project, &["ls-files", "--stage", "-z"])? != index {
        return Err(Error::Conflict("source changed during observation".into()));
    }
    Ok(Source { head, tree, index_digest: digest(&index), material_digest: digest(&serde_json::to_vec(&material)?) })
}

pub fn observe(root: &Path, data: &Value, phase: u32) -> Result<Inputs> {
    let project = root.parent().ok_or_else(|| refuse(phase, "verification-root", "project", "project root required"))?;
    let context = crate::context::persistence::saved(data, phase)?
        .ok_or_else(|| refuse(phase, "native-approved-truths", "context", "native approved truths required"))?;
    let admissions = admission::records(data, phase)?;
    let latest = admissions.last().ok_or_else(|| refuse(phase, "admission-required", "admissions", "complete native admission required"))?;
    let inventory = plan::inventory::read(root, &phase.to_string(), data)?;
    admission::validate(data, &inventory.documents, &latest.request.contract)?;
    let binding = root_binding(root)?;
    if admissions.iter().any(|a| a.root_binding != binding || admission::request_digest(&a.request).ok().as_ref() != Some(&a.request_digest)) {
        return Err(refuse(phase, "verification-admission", "admissions", "admission identity differs from this root"));
    }
    let events = history::records(data, phase)?;
    let plan_events = history::plan_records(data, phase)?;
    if data["execution"]["occurrences"][phase.to_string()]["active"].is_object() {
        return Err(refuse(phase, "verification-execution", "execution.active", "execution dispatch remains active"));
    }
    for (identity, _) in history::admitted_plans(data, phase)? {
        if !history::plan_project(&plan_events, &identity).completed {
            return Err(refuse(phase, "verification-execution", "execution.plans", format!("plan {} has unfinished suite or risk settlement", identity.plan)));
        }
        for task in history::plan_task_views(data, &events, phase, identity.plan)? {
            if !task.state.completed || !task.state.unknown_runs.is_empty() {
                return Err(refuse(phase, "verification-execution", "execution.tasks", format!("task {} is unfinished or has unanswered launches", task.task.task)));
            }
            let proof = events.iter().find_map(|r| match &r.request.event {
                history::Event::Close(proof) if r.request.task == task.task => Some(proof), _ => None,
            }).ok_or_else(|| refuse(phase, "verification-execution", "execution.tasks", "close proof absent"))?;
            receipts::validate_pairs(data, &events, &proof.submission, project)?;
            receipts::validate_close_owner(data, &events, &proof.submission)?;
            receipts::reobserve_source(project, &proof.dispatch, &task.task.task, &proof.source)?;
        }
    }
    for record in &events {
        if record.root_binding != binding || record.request_digest != history::request_digest(&record.request)? {
            return Err(refuse(phase, "verification-execution", "execution.events", "task event identity mismatch"));
        }
    }
    for record in &plan_events {
        if record.root_binding != binding || record.request_digest != history::plan_request_digest(&record.request)? {
            return Err(refuse(phase, "verification-execution", "execution.plan_events", "plan event identity mismatch"));
        }
    }
    let source = source(project).map_err(|e| refuse(phase, "verification-source", "source", e.to_string()))?;
    let map = serde_json::to_value(plan::map_view::read(root, phase)?)?;
    if map["coherence"] != "consistent" {
        return Err(refuse(phase, "verification-map", "map", "complete coherent evidence map required"));
    }
    let observed = plan::persistence::read_snapshot(root)?.ok_or_else(|| refuse(phase, "verification-inputs", "snapshot", "native snapshot absent"))?;
    if authority_digest(&observed.data)? != authority_digest(data)? {
        return Err(refuse(phase, "verification-inputs", "snapshot", "authority changed while observing map"));
    }
    let checks = map["items"].as_array().ok_or_else(|| refuse(phase, "verification-map", "items", "canonical items absent"))?
        .iter().filter(|i| i["kind"] == "check").cloned().collect();
    let execution = json!({"events":events,"plan_events":plan_events});
    let basis = Basis { project: project.to_string_lossy().into_owned(), root_binding: binding,
        phase, occurrence: latest.request.contract.occurrence.clone(),
        context_digest: digest(&serde_json::to_vec(&context)?),
        truths: context.truths.iter().map(|t| TruthVersion { id: t.id.clone(), version: t.version }).collect(),
        publications: latest.request.contract.plans.clone(),
        map_digest: map["input_digest"].as_str().ok_or_else(|| refuse(phase, "verification-map", "input_digest", "map digest absent"))?.into(),
        admission_digests: admissions.iter().map(|a| a.request_digest.clone()).collect(),
        execution_digest: digest(&serde_json::to_vec(&execution)?), source };
    Ok(Inputs { basis, map, admissions, execution, checks, authority_digest: authority_digest(data)? })
}

/// Transaction replay checks the captured authority against its preimage, and
/// reobserves external material without asking readback to ignore a live intent.
pub fn reobserve_external(root: &Path, inputs: &Inputs, documents: &BTreeMap<String, String>) -> Result<()> {
    let phase = inputs.basis.phase;
    if root_binding(root)? != inputs.basis.root_binding
        || root.parent().map(|p| p.to_string_lossy().into_owned()).as_ref() != Some(&inputs.basis.project) {
        return Err(refuse(phase, "verification-root", "basis", "bound project or root changed"));
    }
    if source(Path::new(&inputs.basis.project)).map_err(|e| refuse(phase, "verification-source", "source", e.to_string()))? != inputs.basis.source {
        return Err(refuse(phase, "verification-source", "source", "committed source, index or material changed"));
    }
    let observed = plan::inventory::read(root, &phase.to_string(), &json!({}))?;
    if observed.documents != *documents {
        return Err(refuse(phase, "verification-inputs", "publications", "installed plan inventory changed before confirmation"));
    }
    Ok(())
}
