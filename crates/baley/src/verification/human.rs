//! Attributed human UAT results the binary retains and renders (D-127).
//!
//! A human result is an owner-approved record bound to one phase, occurrence
//! and item. Replies are immutable: a later result names the record it
//! supersedes and the first-pass outcome is carried forward, never rewritten.
//! The installed UAT.md is the binary's render of these records; a phase
//! UAT.md that is not that render is refused, never overwritten. A blank
//! reply is refused, a skipped outcome resolves nothing, and no verifier
//! patch can create or change a human record.
use super::{inputs, model::{HumanOutcome, HumanResult}, persistence, projections, verdicts};
use crate::{execution::receipts::OwnerApproval, store::{Error, Result, model::{digest, DecisionRecord, Decision, Origin, Evidence}}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "verification-human-1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: String,
    pub submission: HumanResult,
    pub approval: OwnerApproval<HumanResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    pub id: String,
    pub request_id: String,
    pub root_binding: String,
    pub submission: HumanResult,
    pub approval: OwnerApproval<HumanResult>,
    /// The earliest outcome known for this item: the first native result.
    pub first_pass: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub schema: String,
    pub root: PathBuf,
    pub root_binding: String,
    pub request: Request,
    pub payload_digest: String,
    pub authority_digest: String,
    /// The UAT.md text observed when the request arrived, or None when absent.
    pub observed: Option<String>,
    pub unreadable: Option<String>,
    pub answer: Value,
}

pub fn records(data: &Value) -> Result<Vec<Record>> {
    persistence::attempt_values(data)?;
    Ok(data[persistence::NAMESPACE].get("humans").cloned().map(serde_json::from_value).transpose()?.unwrap_or_default())
}

pub fn outcome_name(outcome: &HumanOutcome) -> &'static str {
    match outcome { HumanOutcome::Passed => "pass", HumanOutcome::Failed => "fail", HumanOutcome::Skipped => "skipped" }
}

pub fn payload_digest(request: &Request) -> Result<String> {
    Ok(digest(&serde_json::to_vec(request)?))
}

pub fn replay(data: &Value, request: &Request) -> Result<Option<Value>> {
    let Some(prior) = records(data)?.into_iter().find(|r| r.request_id == request.request_id) else { return Ok(None) };
    let same = prior.submission == request.submission && prior.approval == request.approval;
    Ok(Some(if same { json!({"status":"ok","receipt":{"schema":SCHEMA,"record":prior}}) } else {
        verdicts::refusal("verification-human-reuse", "request_id", &request.request_id,
            "request already names a different human result payload", json!(payload_digest(request)?), json!(prior.id))
    }))
}

pub fn uat_path(root: &Path, phase: u32) -> PathBuf {
    root.join(format!("phases/{phase}/UAT.md"))
}

pub fn prepare(root: &Path, data: &Value, request: Request) -> Result<Claim> {
    let phase = request.submission.phase;
    let mut claim = Claim { schema: SCHEMA.into(), root: root.into(), root_binding: inputs::root_binding(root)?,
        payload_digest: payload_digest(&request)?, authority_digest: inputs::authority_digest(data)?, request,
        observed: None, unreadable: None, answer: Value::Null };
    match std::fs::read(uat_path(root, phase)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => claim.observed = Some(text),
            Err(_) => claim.unreadable = Some("UAT.md is not UTF-8 text".into()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => claim.unreadable = Some(error.to_string()),
    }
    claim.answer = assess(data, &claim)?;
    Ok(claim)
}

/// Whether the observed phase UAT.md is the binary's own: its current render,
/// or no file at all before the phase's first result. Any other document is
/// caller-owned and not the binary's to replace.
pub fn owned_uat(observed: Option<&str>, render: Option<&str>) -> bool {
    observed == render
}

fn blank(text: &str) -> bool { text.trim().is_empty() || text.len() > 16384 }

/// The latest native record for one item of one phase.
pub fn latest<'a>(history: &'a [Record], phase: u32, id: &str) -> Option<&'a Record> {
    history.iter().rev().find(|r| r.submission.phase == phase && r.submission.id == id)
}

fn assess(data: &Value, claim: &Claim) -> Result<Value> {
    let request = &claim.request;
    let submission = &request.submission;
    let approval = &request.approval;
    let phase = submission.phase;
    let denied = |rule: &str, slot: &str, reason: &str, requested: Value, current: Value| {
        Ok(verdicts::refusal(rule, slot, &submission.id, reason, requested, current))
    };
    if request.request_id.trim().is_empty() || request.request_id.len() > 256 {
        return denied("verification-request", "request_id", "bounded nonblank request identity required", json!(request.request_id.len()), json!(256));
    }
    if !approval.approved {
        return denied("verification-approval", "approval.approved", "a human result needs the owner's affirmative approval", json!(false), json!(true));
    }
    for (slot, value) in [("approval.owner", &approval.owner), ("approval.at", &approval.at)] {
        if blank(value) { return denied("verification-approval", slot, "approval needs a nonblank owner name and reported time", json!(value), Value::Null); }
    }
    if approval.submission != *submission {
        return denied("verification-approval", "approval.submission", "approval must echo the exact submission the owner saw",
            json!(digest(&serde_json::to_vec(&approval.submission)?)), json!(digest(&serde_json::to_vec(submission)?)));
    }
    if phase == 0 || crate::context::persistence::saved(data, phase)?.is_none() {
        return denied("native-approved-truths", "submission.phase", "human results belong to a phase with native approved truths", json!(phase), Value::Null);
    }
    let occurrence = crate::plan::persistence::occurrence(data, phase)?;
    if submission.occurrence != occurrence {
        return denied("verification-human", "submission.occurrence", "human result names another phase occurrence", json!(submission.occurrence), json!(occurrence));
    }
    if submission.id.trim().is_empty() || submission.id.len() > 256 {
        return denied("verification-human", "submission.id", "bounded nonblank human item identity required", json!(submission.id.len()), json!(256));
    }
    if blank(&submission.reply) {
        return denied("verification-human", "submission.reply", "a blank reply is not a result and never consent", json!(submission.reply), Value::Null);
    }
    for (slot, value) in [("submission.owner", &submission.owner), ("submission.at", &submission.at)] {
        if blank(value) { return denied("verification-human", slot, "a human result carries a nonblank name and date", json!(value), Value::Null); }
    }
    if let Some(reason) = &claim.unreadable {
        return denied("verification-human", "uat", "the phase UAT.md could not be observed", json!(reason), Value::Null);
    }
    let history = records(data)?;
    let prior = latest(&history, phase, &submission.id);
    if submission.supersedes.as_deref() != prior.map(|r| r.id.as_str()) {
        return denied("verification-human", "submission.supersedes", "a result supersedes the latest retained result for its item, or none when there is none",
            json!(submission.supersedes), json!(prior.map(|r| &r.id)));
    }
    let expected = projections::uat(data, phase)?;
    if !owned_uat(claim.observed.as_deref(), expected.as_deref()) {
        return denied("verification-human", "uat", "UAT.md differs from the native render; edit human results through verification-human-result",
            json!(claim.observed.as_deref().map(|t| digest(t.as_bytes()))), json!(expected.as_deref().map(|t| digest(t.as_bytes()))));
    }
    let first_pass = match prior {
        Some(prior) => prior.first_pass.clone(),
        None => outcome_name(&submission.outcome).to_owned(),
    };
    let record = Record { schema: SCHEMA.into(), id: digest(&serde_json::to_vec(&(&request.request_id, &claim.root_binding, submission))?),
        request_id: request.request_id.clone(), root_binding: claim.root_binding.clone(), submission: submission.clone(),
        approval: approval.clone(), first_pass };
    Ok(json!({"status":"ok","receipt":{"schema":SCHEMA,"record":record}}))
}

pub fn contribute(data: &Value, binding: &str, claim: &Claim) -> Result<Value> {
    if claim.schema != SCHEMA || claim.root_binding != binding
        || claim.payload_digest != payload_digest(&claim.request)?
        || claim.authority_digest != inputs::authority_digest(data)?
        || replay(data, &claim.request)?.is_some() || claim.answer != assess(data, claim)? {
        return Err(Error::Invalid("human result claim differs from committing authority or immutable outcome".into()));
    }
    if claim.answer["status"] != "ok" {
        return Err(Error::Invalid("a refused human result is never retained".into()));
    }
    let record: Record = serde_json::from_value(claim.answer["receipt"]["record"].clone())?;
    let mut next = data.clone();
    if next.get(persistence::NAMESPACE).is_none() {
        next[persistence::NAMESPACE] = json!({"schema":"verification-1","attempts":[]});
    }
    let mut history = records(data)?;
    history.push(record);
    next[persistence::NAMESPACE]["humans"] = json!(history);
    Ok(next)
}

pub fn decision(claim: &Claim) -> Result<DecisionRecord> {
    Ok(DecisionRecord { version: 1, id: format!("verification-human:{}", digest(claim.request.request_id.as_bytes())), revision: 1,
        origin: Origin { source: SCHEMA.into(), original: Evidence::Missing },
        decision: Decision::Gate { outcome: SCHEMA.into(), evidence: Evidence::Text(serde_json::to_string(claim)?) },
        at: crate::store::model::stamped_at() })
}

/// The confirmed transaction installs the record and the rendered UAT.md
/// together, against the exact observed preimage.
pub fn transaction(data: &Value, claim: &Claim, expected: crate::store::Observed) -> Result<crate::store::transaction::Transaction> {
    let phase = claim.request.submission.phase;
    let next = contribute(data, &claim.root_binding, claim)?;
    let rendered = projections::uat(&next, phase)?.ok_or_else(|| Error::Invalid("native UAT render absent after a human result".into()))?;
    if expected.bytes.as_deref().map(|b| String::from_utf8_lossy(b).into_owned()) != claim.observed {
        return Err(Error::Conflict("UAT.md changed after the human result was observed".into()));
    }
    Ok(crate::store::transaction::Transaction { id: format!("verification-human-result:{}", digest(claim.request.request_id.as_bytes())),
        items: vec![], decisions: vec![decision(claim)?], snapshot: Some(next),
        external: vec![crate::store::transaction::ExternalChange { target: format!("phase-uat:{phase}"), expected, bytes: rendered.into_bytes() }] })
}

pub fn reobserve(claim: &Claim) -> Result<()> {
    if inputs::root_binding(&claim.root)? != claim.root_binding {
        return Err(Error::Conflict("human result claim root changed".into()));
    }
    Ok(())
}

/// Every human item of a phase with its resolution: native results whose
/// latest outcome is not a pass are unfinished human work.
pub fn items(data: &Value, phase: u32) -> Result<Vec<Value>> {
    let history = records(data)?;
    let mut rows = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for record in history.iter().filter(|r| r.submission.phase == phase) {
        if seen.insert(record.submission.id.clone()) { rows.push(row(&history, phase, &record.submission.id)); }
    }
    Ok(rows)
}

fn row(history: &[Record], phase: u32, id: &str) -> Value {
    let chain: Vec<&Record> = history.iter().filter(|r| r.submission.phase == phase && r.submission.id == id).collect();
    let latest = chain.last();
    let status = latest.map_or("missing", |record| outcome_name(&record.submission.outcome));
    let resolved = latest.is_some_and(|r| r.submission.outcome == HumanOutcome::Passed);
    json!({"id":id,"status":status,"first_pass":latest.map(|r| r.first_pass.clone()),
        "resolved":resolved,"required":!resolved,
        "history":chain.iter().map(|r| json!({"id":r.id,"request_id":r.request_id,"outcome":outcome_name(&r.submission.outcome),
            "reply":r.submission.reply,"owner":r.submission.owner,"at":r.submission.at,"supersedes":r.submission.supersedes})).collect::<Vec<_>>()})
}

/// Unfinished human work for a phase: every required, unresolved item.
pub fn unfinished(data: &Value, phase: u32) -> Result<Vec<Value>> {
    Ok(items(data, phase)?.into_iter().filter(|r| r["required"] == true).collect())
}

#[cfg(test)]
mod tests {
    use super::owned_uat;

    const RENDER: &str = "## Native human results\n\n### 1. delivery\nstatus: pass\n";

    #[test]
    fn no_uat_before_the_first_result_is_owned() {
        assert!(owned_uat(None, None));
    }

    #[test]
    fn a_caller_owned_uat_before_the_first_result_is_refused() {
        assert!(!owned_uat(Some("# UAT\n\n### 1. Delivery\nstatus: fail\n"), None));
    }

    #[test]
    fn the_exact_native_render_is_owned() {
        assert!(owned_uat(Some(RENDER), Some(RENDER)));
    }

    #[test]
    fn a_changed_render_is_refused() {
        assert!(!owned_uat(Some(&RENDER.replace("pass", "fail")), Some(RENDER)));
    }
}
