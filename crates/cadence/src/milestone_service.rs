use crate::{config::reload::ConfigIo, import::SessionFactory};
use cadence::envelope::Refusal;
use cadence::{milestone::{model::{self, Apply, Close, Receipt, Selection, State}, preflight}, store::{Error, Result}};
use serde_json::{Value, json};
use std::path::Path;

pub enum Command {
    Read { occurrence: String, selection: Selection },
    Apply(Apply),
}

pub async fn execute<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    match execute_inner(factory, root, command).await {
        Ok(answer) => Ok(answer),
        Err(error) => Ok(model::refuse("milestone-unavailable", error.to_string())),
    }
}

async fn audits<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, selection: &Selection) -> Result<Vec<Value>> {
    let mut answers = Vec::new();
    for phase in &selection.phases {
        let audit = super::verification_service::execute(factory, root,
            super::verification_service::Command::Query(cadence::verification::model::Query::Audit {
                phase: phase.get(), command: Some("cad-audit".into()),
            })).await?;
        answers.push(json!({"phase":phase.get(),"audit":audit}));
    }
    Ok(answers)
}

async fn execute_inner<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    let session = factory.first_touch(root).await?;
    let store = session.review_store();
    let view = session.derivation_view().await?;
    let binding = cadence::verification::inputs::root_binding(root)?;
    let mut records = model::records::<Close>(&view.snapshot.data, "milestones")?;
    for (id, close) in &records.records {
        if close.id != *id || close.root_binding != binding || close.id != model::identity("milestone", &binding, &close.occurrence) {
            return Err(Error::Invalid("milestone record root or identity mismatch".into()));
        }
        close.selection.validate()?;
    }
    let (occurrence, selection, request) = match command {
        Command::Read { occurrence, selection } => (occurrence, selection, None),
        Command::Apply(Apply::Close { request }) => (request.occurrence.clone(), request.selection.clone(), Some(request)),
    };
    let id = model::identity("milestone", &binding, &occurrence);
    let prior = records.records.get(&id).cloned();
    let raw = request.as_ref().map(serde_json::to_value).transpose()?;
    if let (Some(request), Some(raw)) = (&request, &raw)
        && let Some(answer) = model::replay(&records, &binding, &request.request_id, raw)? { return Ok(answer); }
    let validation = model::name(&occurrence).and_then(|_| selection.validate());
    let answer = if let Err(error) = validation {
        model::refuse("invalid-arguments", error.to_string())
    } else if request.as_ref().is_some_and(|r| model::reused(&records, &binding, &r.request_id)) {
        model::refuse("request-reused", "request_id already binds different milestone inputs")
    } else {
        let preflight = preflight::collect(store, &view, &selection).await;
        match preflight {
            Err(error) => model::refuse("milestone-unavailable", error.to_string()),
            Ok(unsettled) => {
                let generation = prior.as_ref().map_or(0, |c| c.generation);
                if let Some(request) = &request {
                    if model::name(&request.request_id).is_err() {
                        model::refuse("invalid-arguments", "request_id must be nonblank bounded text")
                    } else if !unsettled.is_empty() {
                        let mut refusal = Refusal::new("milestone-unsettled", "selected phases retain unsettled records")
                            .details(json!({"unsettled":unsettled})).value();
                        // Keep the plan 1 caller's top-level unsettled list available.
                        refusal["unsettled"] = refusal["details"]["unsettled"].clone();
                        refusal
                    } else if request.expected_generation != generation {
                        Refusal::new("milestone-generation", "expected generation does not match the current milestone generation")
                            .details(json!({"expected_generation":request.expected_generation,"generation":generation})).value()
                    } else if let Some(close) = &prior {
                        if close.selection != selection { model::refuse("milestone-selection", "a ready close has an immutable phase selection and label") }
                        else { json!({"status":"ok","close":close}) }
                    } else {
                        let mut incomplete = Vec::new();
                        for phase in &selection.phases {
                            if !cadence::verification::completion::applicable(&view.snapshot.data, phase.get())?
                                .is_some_and(|(_, applies, _)| applies) { incomplete.push(phase.get()); }
                        }
                        if !incomplete.is_empty() {
                            Refusal::new("milestone-incomplete", "selected phases need current native completion")
                                .details(json!({"phases":incomplete})).value()
                        } else {
                            // Reading existing audits is not a new audit verdict. Prune is
                            // a later operation; this record changes no authored document.
                            let audit = audits(factory, root, &selection).await?;
                            if audit.iter().any(|a| a["audit"]["status"] != "ok") {
                                Refusal::new("milestone-audit-unavailable", "selected phases need available audit reports")
                                    .details(json!({"audits":audit})).value()
                            } else {
                                let close = Close { id: id.clone(), root_binding: binding.clone(), occurrence: occurrence.clone(), generation: 1,
                                    selection: selection.clone(), state: State::Ready };
                                records.records.insert(id.clone(), close.clone());
                                json!({"status":"ok","close":close})
                            }
                        }
                    }
                } else {
                    let audit = audits(factory, root, &selection).await?;
                    let action = json!({"operation":"milestone-close","request":{
                        "request_id":model::identity("close", &binding, &format!("{occurrence}:{}:{generation}:{}", cadence::store::model::digest(&serde_json::to_vec(&selection)?), view.snapshot.generation)),
                        "occurrence":occurrence,"expected_generation":generation,"selection":selection}});
                    json!({"status":"ok","read_only":true,"close":prior,"identity":id,"generation":generation,
                        "occurrence":occurrence,"selection":selection,"audits":audit,"unsettled":unsettled,
                        "actions":{"close":action}})
                }
            }
        }
    };
    if let (Some(request), Some(raw)) = (request, raw) {
        model::persist(store, &view, "milestones", &mut records, Receipt { root_binding: binding, request_id: request.request_id, request: raw, answer }).await
    } else { Ok(answer) }
}
