//! Resident adapter; all writes use the existing session's single store queue.
use cadence::{store::{Error, Result, writer::Operation}, verification::{inputs, model::Query, persistence}};
use serde_json::{Value, json};
use std::path::Path;

pub async fn execute<I: crate::config::reload::ConfigIo + Clone + Sync>(
    factory: &crate::import::SessionFactory<I>, root: &Path, query: Query,
) -> Result<Value> {
    match execute_inner(factory, root, query).await {
        Ok(answer) => Ok(answer),
        Err(error) => Ok(super::execution_service::native_error(error)),
    }
}

async fn execute_inner<I: crate::config::reload::ConfigIo + Clone + Sync>(
    factory: &crate::import::SessionFactory<I>, root: &Path, query: Query,
) -> Result<Value> {
    let snapshot = cadence::plan::persistence::read_snapshot(root)?;
    let data = snapshot.as_ref().map(|s| s.data.clone()).unwrap_or_else(|| json!({}));
    match query {
        Query::Audit { phase } => Err(inputs::refuse(phase, "verification-unavailable", "operation", "verification-audit is not implemented")),
        Query::Read { phase, attempt } => {
            let saved = persistence::attempts(&data)?.into_iter().rev()
                .find(|a| a.inputs.basis.phase == phase && attempt.as_ref().is_none_or(|id| a.id == *id))
                .ok_or_else(|| inputs::refuse(phase, "verification-attempt", "attempt", "retained attempt absent"))?;
            Ok(json!({"status":"ok","attempt":saved}))
        }
        Query::Next { phase, request_id } => {
            if phase == 0 { return Err(inputs::refuse(phase, "verification-phase", "phase", "positive integer phase required")); }
            if cadence::context::persistence::saved(&data, phase)?.is_none() {
                // Request identity reuse is checked first whenever a store exists.
                if let Some(id) = &request_id { persistence::replay(&data, &inputs::root_binding(root)?, phase, id)?; }
                return Err(inputs::refuse(phase, "native-approved-truths", "context", "native approved truths required"));
            }
            let binding = inputs::root_binding(root)?;
            if let Some(id) = &request_id
                && let Some(saved) = persistence::replay(&data, &binding, phase, id)? {
                return Ok(json!({"status":"ok","attempt":saved}));
            }
            let request_id = match request_id {
                Some(id) => id,
                None => {
                    let observed = inputs::observe(root, &data, phase)?;
                    format!("verify-{}", cadence::store::model::digest(&serde_json::to_vec(&observed.basis)?))
                }
            };
            if let Some(saved) = persistence::replay(&data, &binding, phase, &request_id)? {
                return Ok(json!({"status":"ok","attempt":saved}));
            }
            let request = persistence::prepare(root.into(), &data, phase, request_id)?;
            let session = factory.first_touch(root).await?;
            session.config()?;
            let view = session.derivation_view().await?;
            let written = session.review_store().request(Operation::VerificationV1 {
                expected_generation: view.snapshot.generation, expected_integrity: view.snapshot.integrity,
                request: Box::new(request.clone()),
            }).await?;
            let saved = persistence::replay(&written.snapshot.data, &binding, phase, &request.attempt.request_id)?
                .ok_or_else(|| Error::Invalid("confirmed verification attempt absent".into()))?;
            inputs::reobserve_external(root, &saved.inputs, &request.documents)?;
            Ok(json!({"status":"ok","attempt":saved}))
        }
    }
}
