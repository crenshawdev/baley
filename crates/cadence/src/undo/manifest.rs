use crate::process::Process;
use super::model::{Manifest, records};
use crate::{execution::{admission, history, model::ExecutionOccurrence}, rail::git, store::{Error, Result}};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, path::Path};

pub fn identity(manifest: &Manifest) -> Result<String> {
    Ok(format!("undo-manifest-{}", crate::store::model::hex(&Sha256::digest(serde_json::to_vec(&(
        manifest.phase, &manifest.root_binding, &manifest.source, &manifest.occurrence,
        &manifest.hashes, &manifest.provenance))?))))
}

/// Accepted closes are already chronological; neither task names nor Git dates order them.
pub fn native(data: &Value, phase: u32) -> Result<Option<(String, Vec<String>, Value)>> {
    let key = phase.to_string();
    let admissions = admission::records(data, phase)?;
    let events = history::records(data, phase)?;
    let raw = data["execution"]["occurrences"].get(&key);
    if raw.is_none() && admissions.is_empty() && events.is_empty() { return Ok(None); }
    let occurrence: ExecutionOccurrence = serde_json::from_value(raw.cloned().unwrap_or(Value::Null))?;
    if occurrence.phase != phase || occurrence.active.is_some() { return Err(Error::Invalid(format!("phase {phase} has an active or malformed execution occurrence"))); }
    let current = admissions.last().ok_or_else(|| Error::Invalid(format!("phase {phase} has no admitted native manifest")))?;
    let publication = crate::plan::persistence::saved(data, phase)?.ok_or_else(|| Error::Invalid(format!("phase {phase} native publication is absent")))?;
    if current.request.contract.plans.iter().any(|p| publication.publications.get(&p.plan).is_none_or(|saved|
        saved.revision != p.content_revision || saved.map_revision.as_ref() != Some(&p.map_revision))) {
        return Err(Error::Conflict(format!("phase {phase} current publication differs from its admitted execution")));
    }
    let admitted = history::admitted_plans(data, phase)?;
    let mut hashes = Vec::new();
    let mut provenance = Vec::new();
    for record in events {
        let task = &record.request.task;
        if task.occurrence != current.request.contract.occurrence { continue; }
        if !admitted.iter().any(|(p, _)| p.plan == task.plan && p.admission_digest == task.admission_digest)
            || !current.request.contract.allocation.iter().any(|a| a.plan == task.plan && a.task == task.task) { continue; }
        if let history::Event::Close(proof) = &record.request.event {
            if proof.source.completion != proof.submission.completion {
                return Err(Error::Invalid(format!("task close {} has inconsistent completion identity", record.request.request_id)));
            }
            hashes.push(proof.source.completion.clone());
            provenance.push(json!({"task":task,"close":record.request.request_id,"completion":proof.source.completion}));
        }
    }
    Ok(Some((current.request.contract.occurrence.clone(), hashes, json!(provenance))))
}

pub fn read(root: &Path, data: &Value, binding: &str, phase: u32, process: &mut dyn Process) -> Result<Manifest> {
    let native = native(data, phase)?;
    if let Some(prior) = records(data)?.values().find(|r| r.manifest.phase == phase) {
        if prior.manifest.root_binding != binding { return Err(Error::Conflict("undo root binding changed".into())); }
        return Ok(prior.manifest.clone());
    }
    let source = "execution";
    let (occurrence, supplied, provenance) = native
        .ok_or_else(|| Error::Invalid(format!("phase {phase} has no native execution to undo")))?;
    if supplied.is_empty() { return Err(Error::Invalid(format!("phase {phase} {source} commit manifest is empty"))); }
    let project = root.parent().ok_or_else(|| Error::Invalid("planning root lacks project".into()))?;
    let mut resolutions: BTreeMap<String, String> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut hashes = Vec::new();
    for input in supplied {
        if !(7..=64).contains(&input.len()) || !input.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!("phase {phase} {source} malformed commit identity {input:?}")));
        }
        let hash = if let Some(hash) = resolutions.get(&input) { hash.clone() } else {
            // Resolve object prefixes, never a same-spelled ref or a peeled tag.
            let candidates = git::run(project, ["rev-parse".to_owned(), format!("--disambiguate={input}")], process)?;
            let candidates: Vec<_> = candidates.split(|b| *b == b'\n').filter(|line| !line.is_empty()).collect();
            let [candidate] = candidates.as_slice() else {
                return Err(Error::Invalid(format!("phase {phase} {source} commit {input} is absent or ambiguous")));
            };
            let hash = git::object_id(candidate.to_vec())?;
            if git::run(project, ["cat-file", "-t", &hash], process)? != b"commit\n" {
                return Err(Error::Invalid(format!("phase {phase} {source} object {input} is not a commit")));
            }
            let object = git::run(project, ["cat-file", "commit", &hash], process)?;
            let headers = object.split(|b| *b == b'\n').take_while(|line| !line.is_empty());
            if headers.filter(|line| line.starts_with(b"parent ")).count() > 1 {
                return Err(Error::Invalid(format!("phase {phase} {source} merge commit {hash} is unsupported")));
            }
            resolutions.insert(input, hash.clone());
            hash
        };
        if seen.insert(hash.clone()) { hashes.push(hash); }
    }
    let mut manifest = Manifest { id: String::new(), phase, root_binding: binding.into(), source: source.into(), occurrence, hashes, provenance };
    manifest.id = identity(&manifest)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_without_native_execution_has_no_manifest() {
        let mut process = crate::process::Recorded::new();
        assert_eq!(
            read(Path::new("/project/.planning"), &json!({}), "binding", 3, &mut process),
            Err(Error::Invalid("phase 3 has no native execution to undo".into()))
        );
        assert!(process.launches().is_empty(), "no git before a manifest exists");
    }
}
