use crate::{config::reload::ConfigIo, import::SessionFactory};
use cadence::envelope::Refusal;
use cadence::{landing::model::{Apply, Landing}, milestone::model::{self, Receipt}, store::{Error, Result}};
use serde_json::{Value, json};
use std::path::Path;

pub enum Command { Read { landing: String }, Apply(Apply) }

pub async fn execute<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    match execute_inner(factory, root, command).await {
        Ok(answer) => Ok(answer),
        Err(error) => Ok(model::refuse("landing-unavailable", error.to_string())),
    }
}

async fn execute_inner<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    let session = factory.first_touch(root).await?;
    let store = session.review_store();
    let view = session.derivation_view().await?;
    let binding = cadence::verification::inputs::root_binding(root)?;
    let mut records = model::records::<Landing>(&view.snapshot.data, "landings")?;
    for (id, landing) in &records.records {
        if landing.id != *id || landing.root_binding != binding || landing.id != model::identity("landing", &binding, &landing.occurrence) {
            return Err(Error::Invalid("landing record root or identity mismatch".into()));
        }
    }
    let apply = match command {
        Command::Read { landing } => return Ok(match records.records.get(&landing) {
            None => model::refuse("landing-unknown", landing),
            Some(record) => json!({"status":"ok","landing":record,"refusals":records.receipts.values()
                .filter(|r| r.answer["status"] == "refused" && r.request["request"]["landing"] == landing).collect::<Vec<_>>()}),
        }),
        Command::Apply(apply) => apply,
    };
    let raw = serde_json::to_value(&apply)?;
    let request_id = match &apply { Apply::Start { request } => &request.request_id, Apply::Publish { request } => &request.request_id };
    if let Some(answer) = model::replay(&records, &binding, request_id, &raw)? { return Ok(answer); }
    let answer = if model::reused(&records, &binding, request_id) {
        model::refuse("request-reused", "request_id already binds different landing inputs")
    } else if model::name(request_id).is_err() {
        model::refuse("invalid-arguments", "request_id must be nonblank bounded text")
    } else { match &apply {
        Apply::Start { request } => {
            let valid = [&request.occurrence, &request.source.branch, &request.base.branch, &request.remote.name, &request.remote.url]
                .iter().all(|s| model::name(s).is_ok() && !s.starts_with('-'))
                && [&request.source.head, &request.base.head].iter().all(|h| matches!(h.len(), 40 | 64) && h.bytes().all(|b| b.is_ascii_hexdigit()));
            let candidate = Landing::new(binding.clone(), request);
            let prior = records.records.get(&candidate.id);
            let generation = prior.map_or(0, |p| p.generation);
            if !valid { model::refuse("invalid-arguments", "landing requires named branches and remote, and exact source/base commit hashes") }
            else if request.expected_generation != generation {
                Refusal::new("landing-generation", "expected generation does not match the current landing generation")
                    .details(json!({"generation":generation,"expected_generation":request.expected_generation})).value()
            } else if let Some(prior) = prior {
                if prior.source != request.source || prior.base != request.base || prior.remote != request.remote {
                    model::refuse("landing-inputs", "landing source, base and remote are immutable")
                } else { json!({"status":"ok","landing":prior}) }
            } else {
                records.records.insert(candidate.id.clone(), candidate.clone());
                json!({"status":"ok","landing":candidate})
            }
        }
        Apply::Publish { request } => {
            // This is deliberately the first preflight, before even inspecting
            // repository refs. An unruled member in ANY home forbids effects.
            match cadence::review::consumers::unruled_members(store).await {
                Err(error) => model::refuse("landing-unavailable", error.to_string()),
                Ok(unsettled) if !unsettled.is_empty() => {
                    let mut refusal = Refusal::new("landing-unsettled", "unruled deferred members forbid publishing")
                        .details(json!({"landing":request.landing,"step":"publish","unsettled":unsettled})).value();
                    // Keep the plan 1 caller's top-level unsettled list available.
                    refusal["unsettled"] = refusal["details"]["unsettled"].clone();
                    refusal
                },
                Ok(_) => match records.records.get(&request.landing) {
                    None => model::refuse("landing-unknown", &request.landing),
                    Some(landing) if landing.generation != request.expected_generation =>
                        Refusal::new("landing-generation", "expected generation does not match the current landing generation")
                            .details(json!({"landing":landing.id,"generation":landing.generation,"expected_generation":request.expected_generation})).value(),
                    Some(landing) => Refusal::new("landing-authorization-required",
                        "an explicit landing step authorization is required; publication is not available in this plan")
                        .details(json!({"landing":landing.id,"step":"publish"})).value(),
                },
            }
        }
    }};
    let receipt = Receipt { root_binding: binding, request_id: request_id.clone(), request: raw, answer };
    model::persist(store, &view, "landings", &mut records, receipt).await
}
