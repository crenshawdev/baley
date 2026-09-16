//! Root-bound adapter for the shared native runner; it owns no second writer.
use crate::{config::reload::ConfigIo, import::SessionFactory};
use cadence::{execution::{history, runner}, store::{Error, Result}};
use serde_json::{Value, json};
use std::path::Path;

pub async fn apply<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, raw: Value) -> Result<Value> {
    let input: runner::Apply = match serde_json::from_value(raw) {
        Ok(input) => input,
        Err(error) => return Ok(super::execution_service::native_error(Error::Invalid(error.to_string()))),
    };
    let session = factory.first_touch(root).await?;
    let view = session.derivation_view().await?;
    let task = match &input { runner::Apply::Start { request } => &request.task, runner::Apply::Run { request } => &request.task };
    let active = &view.snapshot.data["execution"]["occurrences"][task.phase.to_string()]["active"];
    if active["plan"] != task.plan || active["phase"] != task.phase {
        return Ok(super::execution_service::native_error(Error::Invalid("native task requires its active dispatch".into())));
    }
    let project = root.parent().ok_or_else(|| Error::Invalid("project root missing".into()))?;
    let result = match input {
        runner::Apply::Start { request } => runner::start(session.review_store(), project, request).await,
        runner::Apply::Run { request } => runner::launch(session.review_store().clone(), project.to_path_buf(), request).await,
    };
    Ok(match result {
        Ok(receipt) => json!({"status":"ok","receipt":receipt}),
        Err(error) => super::execution_service::native_error(error),
    })
}

/// The plan-level operations: the suite launch, the operator's absence
/// attestation for one relaunch, and native completion. Completion needs the
/// plan's one passing suite receipt and its exact risk settlement together.
pub async fn plan_apply<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, raw: Value) -> Result<Value> {
    use cadence::execution::history::{Completion, PlanEvent, PlanRequest};
    let input: runner::PlanApply = match serde_json::from_value(raw) {
        Ok(input) => input,
        Err(error) => return Ok(super::execution_service::native_error(Error::Invalid(error.to_string()))),
    };
    let session = factory.first_touch(root).await?;
    let view = session.derivation_view().await?;
    let project = root.parent().ok_or_else(|| Error::Invalid("project root missing".into()))?;
    let result = match input {
        runner::PlanApply::Suite { request } => runner::suite_launch(session.review_store().clone(), project.to_path_buf(), request).await,
        runner::PlanApply::RepairAnswer { request } => match request.plan_request() {
            Ok(request) => runner::plan_append(session.review_store(), request).await,
            Err(error) => Err(error),
        },
        runner::PlanApply::Repair { request } => runner::suite_repair(session.review_store(), project, request).await,
        runner::PlanApply::Relaunch { request } => runner::plan_append(session.review_store(), PlanRequest { request_id: request.request_id,
            plan: request.plan, expected_version: request.expected_version, event: PlanEvent::SuiteRelaunch(request.statement) }).await,
        runner::PlanApply::Complete { request } => {
            let records = history::plan_records(&view.snapshot.data, request.plan.phase)?;
            if let Some(prior) = records.iter().find(|r| r.request.request_id == request.request_id) {
                if matches!(prior.request.event, PlanEvent::Completion(_)) && prior.request.plan == request.plan && prior.request.expected_version == request.expected_version {
                    Ok(prior.clone())
                } else {
                    Err(cadence::execution::admission::refuse(request.plan.phase, "plan-request-reuse", "request_id", &request.request_id, "request already names another payload"))
                }
            } else {
                // The settlement is computed only for a passing suite; every other
                // state is refused by the store with its own located rule.
                let projection = history::plan_project(&records, &request.plan);
                let suite_run = projection.launches.last().cloned().unwrap_or_default();
                let passed = history::suite_result(&records, &request.plan, &suite_run).is_some_and(history::suite_passed);
                let active = view.snapshot.data["execution"]["occurrences"][request.plan.phase.to_string()]["active"]["id"].as_str().unwrap_or_default().to_owned();
                let settlement = match (passed && !projection.completed && !active.is_empty())
                    .then(|| super::execution_service::native_settlement(&session, &view, root, request.plan.phase, request.plan.plan, &active)) {
                    Some(Ok(settlement)) => Some(settlement),
                    Some(Err(reason)) => return Ok(super::execution_service::native_error(cadence::execution::admission::refuse(request.plan.phase, "risk-pending", "plan",
                        &request.plan.plan.to_string(), format!("the suite receipt is retained; completion waits for the exact risk settlement: {reason}")))),
                    None => None,
                };
                runner::plan_append(session.review_store(), PlanRequest { request_id: request.request_id, plan: request.plan,
                    expected_version: request.expected_version, event: PlanEvent::Completion(Completion { suite_run, settlement }) }).await
            }
        }
    };
    Ok(match result {
        Ok(receipt) => json!({"status":"ok","receipt":receipt}),
        Err(error) => super::execution_service::native_error(error),
    })
}

/// One run by its id (GH-263): the launch and result records that name it,
/// from the task events or the plan events. Each capture is rendered as text
/// beside its digest and byte length; the record keeps its bytes. A run the
/// phase never retained is refused, never answered with the phase.
pub async fn read_run<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, phase: u32, run: &str) -> Result<Value> {
    use cadence::execution::history::{Event, PlanEvent};
    let session = factory.first_touch(root).await?;
    let view = session.derivation_view().await?;
    let mut launch = None;
    let mut result = None;
    for record in history::records(&view.snapshot.data, phase)? {
        match &record.request.event {
            Event::Launch(l) if l.run_id == run => launch = Some(serde_json::to_value(&record)?),
            Event::Result(r) if r.run_id == run => result = Some(serde_json::to_value(&record)?),
            _ => {}
        }
    }
    for record in history::plan_records(&view.snapshot.data, phase)? {
        match &record.request.event {
            PlanEvent::SuiteLaunch(l) if l.run_id == run => launch = Some(serde_json::to_value(&record)?),
            PlanEvent::SuiteResult(r) if r.run_id == run => result = Some(serde_json::to_value(&record)?),
            _ => {}
        }
    }
    let Some(launch) = launch else {
        return Ok(cadence::envelope::Refusal::new("run-not-retained", format!("phase {phase} retains no run {run}"))
            .rule("task-history-shape").slot("run").phase(phase).value());
    };
    let result = result.map(|mut record| { captures_as_text(&mut record["request"]["event"]); record });
    Ok(json!({"status":"ok","schema":"native-run-history-1","phase":phase,"run_id":run,"launch":launch,"result":result}))
}

/// A capture's bytes under `text` with their `byte_length`: a capture retained
/// as an integer array is rendered as lossy UTF-8, one retained as text is
/// already there; digest, completeness and result lines are untouched.
fn captures_as_text(event: &mut Value) {
    for stream in ["stdout", "stderr"] {
        let Some(capture) = event[stream].as_object_mut() else { continue };
        if let Some(bytes) = capture.remove("bytes") {
            let bytes: Vec<u8> = serde_json::from_value(bytes).unwrap_or_default();
            capture.insert("byte_length".into(), json!(bytes.len()));
            capture.insert("text".into(), json!(String::from_utf8_lossy(&bytes)));
        } else if let Some(text) = capture.get("text").and_then(Value::as_str) {
            capture.insert("byte_length".into(), json!(text.len()));
        }
    }
}

pub async fn read<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, phase: u32) -> Result<Value> {
    let session = factory.first_touch(root).await?;
    let view = session.derivation_view().await?;
    let records = history::records(&view.snapshot.data, phase)?;
    let project = root.parent().ok_or_else(|| Error::Invalid("project root missing".into()))?;
    let mut tasks = Vec::new();
    for basis in &cadence::execution::admission::records(&view.snapshot.data, phase)? {
        for binding in &basis.request.contract.plans {
            if tasks.iter().any(|value: &Value| value["task"]["plan"] == binding.plan) { continue }
            for task in history::plan_task_views(&view.snapshot.data, &records, phase, binding.plan)? {
                let uncertainty = runner::uncertainty(project, &records, &task)?;
                tasks.push(json!({"task":task.task,"state":task.state,"uncertainty":uncertainty}));
            }
        }
    }
    let mut checkpoints=Vec::new();
    for decision in &view.decisions {
        if let Some(record)=cadence::evidence::persistence::decode_history(decision)?
            && record.scope.phase==phase.to_string() && record.scope.planning_root==root.to_string_lossy() {
            checkpoints.push(record);
        }
    }
    let plan_events = history::plan_records(&view.snapshot.data, phase)?;
    let outcomes = history::plan_outcomes(&view.snapshot.data, phase)?;
    let plans = history::admitted_plans(&view.snapshot.data, phase)?.into_iter()
        .map(|(plan, _)| json!({"state":history::plan_project(&plan_events, &plan),"plan":plan,
            "outcome":outcomes.iter().find(|outcome| outcome.plan == plan.plan)})).collect::<Vec<_>>();
    let active = view.snapshot.data["execution"]["occurrences"][phase.to_string()]["active"].clone();
    Ok(json!({"status":"ok","schema":"native-task-history-1","events":records,"tasks":tasks,"checkpoint_history":checkpoints,
        "plan_events":plan_events,"plans":plans,"active":active,"repaired":view.snapshot.repaired}))
}
