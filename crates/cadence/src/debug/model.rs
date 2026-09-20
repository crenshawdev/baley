use crate::{envelope::Refusal, store::{Error, Result}};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HypothesisState { Untested, Testing, Refuted, Confirmed }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis { pub id: String, pub description: String, pub rank_reason: String, pub state: HypothesisState }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation { pub test: String, pub result: String, pub rules_in: Vec<String>, pub rules_out: Vec<String> }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attempt { pub description: String, pub result: String }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reproduction { pub test: String, pub result: String, pub passed: bool }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution { pub description: String, pub reproduction: Reproduction }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status { Open, Resolved }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallSnapshot {
    pub backend: String,
    pub results: Vec<RecallHit>,
    pub total: usize,
    pub incomplete: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallHit {
    pub score: f64,
    pub snippet: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<u32>,
    pub provenance: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub slug: String, pub root_binding: String, pub version: u64,
    pub symptom: String, pub hypotheses: Vec<Hypothesis>, pub observations: Vec<Observation>,
    pub attempts: Vec<Attempt>, pub attempt_count: u64, pub status: Status, pub resolution: Option<Resolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<RecallSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub occurrence: String,
    pub material: crate::rail::risk::MaterialIdentity,
    pub observation: String,
    pub admission_request_id: String,
    pub fire: Option<String>,
    #[serde(default)]
    pub history: Vec<super::review::FireReview>,
    #[serde(default)]
    pub pending_fires: Vec<String>,
    #[serde(default)]
    pub settled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    pub request_id: String,
    pub slug: String,
    pub expected_version: u64,
    pub symptom: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HypothesisRequest {
    pub request_id: String,
    pub slug: String,
    pub expected_version: u64,
    pub hypothesis: Hypothesis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationRequest {
    pub request_id: String,
    pub slug: String,
    pub expected_version: u64,
    pub observation: Observation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptRequest {
    pub request_id: String,
    pub slug: String,
    pub expected_version: u64,
    pub attempt: Attempt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resolve {
    pub request_id: String,
    pub slug: String,
    pub expected_version: u64,
    pub resolution: String,
    pub reproduction: Reproduction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum Apply {
    #[serde(rename = "debug-open")]
    Open { request: Open },
    #[serde(rename = "debug-hypothesis")]
    Hypothesis { request: HypothesisRequest },
    #[serde(rename = "debug-observation")]
    Observation { request: ObservationRequest },
    #[serde(rename = "debug-attempt")]
    Attempt { request: AttemptRequest },
    #[serde(rename = "debug-resolve")]
    Resolve { request: Resolve },
}
impl Apply {
    pub fn identity(&self) -> (&str, &str, u64) {
        match self {
            Self::Open { request } => (&request.request_id, &request.slug, request.expected_version),
            Self::Hypothesis { request } => (&request.request_id, &request.slug, request.expected_version),
            Self::Observation { request } => (&request.request_id, &request.slug, request.expected_version),
            Self::Attempt { request } => (&request.request_id, &request.slug, request.expected_version),
            Self::Resolve { request } => (&request.request_id, &request.slug, request.expected_version),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub root_binding: String,
    pub apply: Apply,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<RecallSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Persist coordination before admission without completing the caller request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub coordinating: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt { pub root_binding: String, pub apply: Apply, pub answer: Value }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Namespace {
    pub schema: String,
    pub records: BTreeMap<String, Record>,
    pub requests: BTreeMap<String, Receipt>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_requests: BTreeMap<String, Apply>,
}
pub fn namespace(data: &Value) -> Result<Namespace> {
    let Some(raw) = data.get("debug") else {
        return Ok(Namespace { schema: "debug-1".into(), records: BTreeMap::new(), requests: BTreeMap::new(), pending_requests: BTreeMap::new() });
    };
    let saved: Namespace = serde_json::from_value(raw.clone())?;
    if saved.schema != "debug-1" { return Err(Error::Invalid("unsupported debug namespace".into())); }
    for (slug, record) in &saved.records {
        validate_slug(slug)?;
        if slug != &record.slug || record.version == 0 || record.root_binding.is_empty()
            || record.attempt_count != record.attempts.len() as u64
            || (record.status == Status::Resolved) != record.resolution.is_some() {
            return Err(Error::Invalid("invalid debug record".into()));
        }
    }
    Ok(saved)
}
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 80 || !slug.as_bytes()[0].is_ascii_lowercase()
        || slug.ends_with('-') || !slug.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
        return Err(Error::Invalid("slug must be 1..80 lowercase letters, digits or hyphens, starting with a letter and ending without a hyphen".into()));
    }
    Ok(())
}
fn text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 16384 { return Err(Error::Invalid("debug evidence must be nonblank and at most 16384 bytes".into())); }
    Ok(())
}
pub fn answer(record: &Record) -> Value {
    json!({"status":"ok","record":record,"projection":super::render::render(record)})
}
pub fn unknown(slug: &str) -> Value {
    Refusal::new("debug-unknown", format!("no debug session is named {slug}")).slot("slug").details(json!({"slug":slug})).value()
}
pub fn replay(data: &Value, write: &Write) -> Result<Option<Value>> {
    let (id, _, _) = write.apply.identity();
    if namespace(data)?.pending_requests.get(id).is_some_and(|pending| *pending != write.apply) {
        return Ok(Some(Refusal::new("request-reused", "debug request identity binds different inputs").slot("request.request_id").value()));
    }
    if let Some(saved) = namespace(data)?.requests.get(id) {
        return Ok(Some(if saved.root_binding == write.root_binding && saved.apply == write.apply { saved.answer.clone() }
            else { Refusal::new("request-reused", "debug request identity binds different inputs").slot("request.request_id").value() }));
    }
    Ok(None)
}

/// The Store and recovery rebuild this same transition; no caller supplies record bytes.
pub fn transition(data: &Value, write: &Write) -> Result<std::result::Result<Record, Value>> {
    let (_, slug, expected) = write.apply.identity();
    let saved = namespace(data)?;
    if let Err(error) = validate_slug(slug) {
        return Ok(Err(Refusal::new("debug-slug", error.to_string()).slot("request.slug").details(json!({"slug":slug})).value()));
    }
    let mut record = match &write.apply {
        Apply::Open { request } => {
            if saved.records.contains_key(slug) { return Ok(Err(Refusal::new("debug-exists", "debug session already exists").slot("request.slug").details(json!({"slug":slug})).value())); }
            text(&request.symptom)?;
            Record { slug: slug.into(), root_binding: write.root_binding.clone(), version: 0,
                symptom: request.symptom.clone(), hypotheses: vec![], observations: vec![], attempts: vec![],
                attempt_count: 0, status: Status::Open, resolution: None, recall: write.recall.clone(), review: None }
        }
        _ => match saved.records.get(slug) { Some(record) => record.clone(), None => return Ok(Err(unknown(slug))) },
    };
    if record.root_binding != write.root_binding { return Ok(Err(Refusal::new("debug-root", "debug mutation belongs to another project root").slot("request.slug").value())); }
    if expected != record.version {
        return Ok(Err(Refusal::new("debug-version", "debug version changed").slot("request.expected_version")
            .details(json!({"slug":slug,"expected":record.version,"supplied":expected})).value()));
    }
    if record.status != Status::Open { return Ok(Err(Refusal::new("debug-resolved", "debug session is already resolved").slot("request.slug").value())); }
    if let Some(review) = &write.review {
        review.material.validate()?;
        if review.occurrence != slug || !matches!(write.apply, Apply::Resolve { .. }) {
            return Err(Error::Invalid("debug review requires its resolve occurrence".into()));
        }
        record.review = Some(review.clone());
    }
    // Resolve re-derives its gate from this transaction's preimage. A fire or
    // consequence arriving after service coordination cannot be overwritten by
    // an earlier projection supplied in the internal write.
    if matches!(write.apply, Apply::Resolve { .. }) && record.review.as_ref().is_some_and(|r| r.fire.is_some()) {
        let mut current = saved;
        current.records.insert(slug.into(), record);
        let mut projected = data.clone();
        projected["debug"] = serde_json::to_value(current)?;
        let refreshed = super::review::contribute(&projected, &write.root_binding)?;
        record = namespace(&refreshed)?.records.remove(slug).ok_or_else(|| Error::Invalid("debug record disappeared".into()))?;
    }
    if write.coordinating { return Ok(Ok(record)); }
    match &write.apply {
        Apply::Open { .. } => {},
        Apply::Hypothesis { request } => {
            let hypothesis = &request.hypothesis;
            validate_slug(&hypothesis.id)?; text(&hypothesis.description)?; text(&hypothesis.rank_reason)?;
            if let Some(prior) = record.hypotheses.iter_mut().find(|h| h.id == hypothesis.id) {
                *prior = hypothesis.clone();
            } else { record.hypotheses.push(hypothesis.clone()); }
        }
        Apply::Observation { request } => {
            let observation = &request.observation;
            text(&observation.test)?; text(&observation.result)?;
            let mut selected = std::collections::BTreeSet::new();
            for id in observation.rules_in.iter().chain(&observation.rules_out) {
                if !selected.insert(id) || !record.hypotheses.iter().any(|h| &h.id == id) {
                    return Ok(Err(Refusal::new("debug-hypothesis", "observation must name distinct existing hypotheses").slot("request.observation").details(json!({"hypothesis":id})).value()));
                }
            }
            for hypothesis in &mut record.hypotheses {
                if observation.rules_in.contains(&hypothesis.id) { hypothesis.state = HypothesisState::Confirmed; }
                if observation.rules_out.contains(&hypothesis.id) { hypothesis.state = HypothesisState::Refuted; }
            }
            record.observations.push(observation.clone());
        }
        Apply::Attempt { request } => {
            text(&request.attempt.description)?; text(&request.attempt.result)?;
            record.attempts.push(request.attempt.clone());
        }
        Apply::Resolve { request } => {
            text(&request.resolution)?; text(&request.reproduction.test)?; text(&request.reproduction.result)?;
            if record.review.as_ref().is_some_and(|review| review.fire.is_some() && !review.settled) {
                record.version = record.version.checked_add(1).ok_or_else(|| Error::Invalid("debug version exhausted".into()))?;
                return Ok(Ok(record));
            }
            if request.reproduction.passed {
                record.status = Status::Resolved;
                record.resolution = Some(Resolution { description: request.resolution.clone(), reproduction: request.reproduction.clone() });
            } else {
                record.attempts.push(Attempt { description: request.resolution.clone(),
                    result: format!("{}: {}", request.reproduction.test, request.reproduction.result) });
            }
        }
    }
    record.attempt_count = record.attempts.len() as u64;
    record.version = record.version.checked_add(1).ok_or_else(|| Error::Invalid("debug version exhausted".into()))?;
    Ok(Ok(record))
}
pub fn contribute(data: &Value, write: &Write) -> Result<Value> {
    let (id, _, _) = write.apply.identity();
    crate::milestone::model::name(id)?;
    if write.root_binding.is_empty() || replay(data, write)?.is_some() { return Err(Error::Invalid("debug write requires a new root-bound request".into())); }
    let mut saved = namespace(data)?;
    let response = match outcome(data, write) {
        Ok(record) => {
            let response = response(&record, write);
            saved.records.insert(record.slug.clone(), record);
            response
        }
        Err(refusal) => refusal,
    };
    if write.coordinating {
        saved.pending_requests.insert(id.into(), write.apply.clone());
    } else {
        saved.pending_requests.remove(id);
        saved.requests.insert(id.into(), Receipt { root_binding: write.root_binding.clone(), apply: write.apply.clone(), answer: response });
    }
    let mut next = data.clone();
    next["debug"] = serde_json::to_value(saved)?;
    Ok(next)
}

pub fn response(record: &Record, write: &Write) -> Value {
    if !write.coordinating && matches!(write.apply, Apply::Resolve { .. })
        && let Some(review) = record.review.as_ref().filter(|review| !review.settled)
        && let Some(fire) = review.fire.as_ref() {
        return Refusal::new("debug-review-pending", format!("debug resolve waits for risk fire {fire}"))
            .slot("fire").details(json!({"slug":record.slug,"fire":fire,"pending_fires":review.pending_fires,"record":record})).value();
    }
    answer(record)
}

pub fn outcome(data: &Value, write: &Write) -> std::result::Result<Record, Value> {
    transition(data, write).unwrap_or_else(|error| Err(Refusal::new("debug-invalid", error.to_string())
        .slot("request").details(json!({"slug":write.apply.identity().1})).value()))
}
