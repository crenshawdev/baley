//! Resident adapter; all writes use the existing session's single store queue.
use cadence::{store::{Error, Result, writer::Operation}, verification::{inputs, model::{Query, Apply}, persistence, runner, verdicts}};
use serde_json::{Value, json};
use std::path::Path;

pub enum Command { Query(Query), Apply(Apply) }

pub async fn execute<I: crate::config::reload::ConfigIo + Clone + Sync>(
    factory: &crate::import::SessionFactory<I>, root: &Path, command: Command,
) -> Result<Value> {
    let result = match command {
        Command::Query(query) => execute_inner(factory, root, query).await,
        Command::Apply(Apply::Run { request }) => {
            let session = factory.first_touch(root).await?;
            session.config()?;
            runner::launch(session.review_store().clone(), root.into(), *request).await
                .map(|receipt| json!({"status":"ok","receipt":receipt}))
        }
        Command::Apply(Apply::Submit { patch }) => {
            let session = factory.first_touch(root).await?;
            session.config()?;
            let view = session.derivation_view().await?;
            if let Some(answer) = verdicts::replay(&view.snapshot.data, &patch)? { return Ok(answer); }
            let claim = verdicts::prepare(root, &view.snapshot.data, *patch)?;
            let transaction = verdicts::transaction(&view.snapshot.data, &claim)?;
            let written = session.review_store().request(Operation::CompareTransact {
                expected_generation: view.snapshot.generation, expected_integrity: view.snapshot.integrity, transaction,
            }).await;
            written.and_then(|view| verdicts::replay(&view.snapshot.data, &claim.patch)?
                .ok_or_else(|| Error::Invalid("confirmed verification claim absent".into())))
        }
        Command::Apply(_) => Err(inputs::refuse(0, "verification-unavailable", "operation", "operation is not implemented")),
    };
    match result {
        Ok(answer) => Ok(answer),
        Err(error) => Ok(super::execution_service::native_error(error)),
    }
}

async fn execute_inner<I: crate::config::reload::ConfigIo + Clone + Sync>(
    factory: &crate::import::SessionFactory<I>, root: &Path, query: Query,
) -> Result<Value> {
    let snapshot = if matches!(query, Query::Read { .. }) && root.join(cadence::store::model::STATE).exists() {
        Some(factory.first_touch(root).await?.derivation_view().await?.snapshot)
    } else {
        cadence::plan::persistence::read_snapshot(root)?
    };
    let data = snapshot.as_ref().map(|s| s.data.clone()).unwrap_or_else(|| json!({}));
    match query {
        Query::Audit { phase } => Err(inputs::refuse(phase, "verification-unavailable", "operation", "verification-audit is not implemented")),
        Query::Read { phase, attempt } => {
            let saved = persistence::attempts(&data)?.into_iter().rev()
                .find(|a| a.inputs.basis.phase == phase && attempt.as_ref().is_none_or(|id| a.id == *id))
                .ok_or_else(|| inputs::refuse(phase, "verification-attempt", "attempt", "retained attempt absent"))?;
            let runs: Vec<_> = runner::records(&data)?.into_iter().filter(|r| r.attempt == saved.id).collect();
            let unknown: Vec<_> = runs.iter().filter(|r| matches!(r.event, runner::Event::Launch { .. }) && runner::result(&runs, &r.id).is_none()).map(|r| r.id.clone()).collect();
            Ok(json!({"status":"ok","attempt":saved,"runs":runs,"unknown_runs":unknown}))
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
