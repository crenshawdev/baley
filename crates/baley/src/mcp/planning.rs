//! `phase-plan` on `baley_apply`: a session asks for one phase's planner with
//! an ordered selection of stories (design 0005 `phase plan`, PLN-R7, D-08 to
//! D-11, D-15). A changed selection is answered with the `phase edit` draft
//! the owner approves at the terminal; an unchanged one with a planning or
//! revision order; an approved commitment with the order its approval issued.
//! Nothing here approves anything (ADR 0044), and `phase-plan` has no
//! terminal command.
//!
//! The arguments are judged before anything is prepared. The write is then
//! prepared with the `phase plan` check as its own check, so every refusal
//! comes before checkout admission and the policy step and records nothing,
//! and the judged path is run through [`crate::planning::phase_plan`].

use std::cell::RefCell;

use baley_store::{CommandKind, InstructionEvidence, ProjectId, RequestId, request_digest};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp::capture::store_failed;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::work_order::{
    evidenced, judge_instruction, judge_request_id, prepared_write, shape_fault,
};
use crate::planning::phase_plan::{self, PHASE_PLAN_COMMAND};
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::issue as recording;
use crate::work_order::route::host_names;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `phase-plan` takes beside its `operation`. Unknown fields
/// are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhasePlanShape {
    /// A fresh UUID for this call in lowercase hyphenated form. A retry of
    /// the same call sends the same id.
    pub request_id: String,
    /// The number of the phase to plan, as the roadmap gives it.
    pub phase: u32,
    /// The ids of the stories to commit to the phase, in the order they are
    /// planned. A list that differs from the phase's stories, a reorder
    /// included, is answered with a draft the owner approves first.
    pub stories: Vec<u32>,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form `phase-plan`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum PhasePlanCall {
    #[serde(rename = "phase-plan")]
    Plan(PhasePlanShape),
}

/// A `phase-plan` call whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedPhasePlan {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The phase to plan.
    pub phase: u32,
    /// The selection, in order.
    pub stories: Vec<u32>,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `phase-plan`'s arguments before anything is prepared or recorded:
/// the shape, then the request id, then the instruction. A fault is the
/// `refused` answer, which records nothing. The request digest covers the
/// command kind, the phase and the stories in order, so a retry replays and
/// a reorder under the same request id is a reused id.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedPhasePlan, Value> {
    let shape = match serde_json::from_value::<PhasePlanCall>(arguments.clone()) {
        Ok(PhasePlanCall::Plan(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "call")?;
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    let digest = request_digest(&json!({"command": PHASE_PLAN_COMMAND,
        "phase": shape.phase, "stories": shape.stories}))
    .expect("numbers are canonical");
    Ok(JudgedPhasePlan {
        request: WriteRequest {
            kind: CommandKind(PHASE_PLAN_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        phase: shape.phase,
        stories: shape.stories,
        instruction,
    })
}

/// Serves one `phase-plan` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the caller
/// with the instruction evidence attached and the `phase plan` check as its
/// own check; then, for a write that ran its steps, the host catalog is read
/// and the judged path is run under a fresh lowercase UUID with the prepared
/// policy and host. A replay goes to the recording with nothing to record,
/// and the store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the run to record and observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(phase_plan::check(
            store,
            project,
            judged.phase,
            &judged.stories,
        )?);
        Ok(())
    };
    let caller = evidenced(&preparation.caller, judged.instruction.as_ref());
    let (store, command, stepped) =
        match prepared_write(preparation, &caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    let kept = kept.borrow();
    let (Some(stepped), Some(checked)) = (stepped, kept.as_ref()) else {
        return recording::answer(recording::record(store, &command, None), store);
    };
    let names = match host_names(store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
    };
    phase_plan::run(
        store,
        &command,
        checked,
        &stepped.policy,
        preparation.host,
        &names,
        &uuid::Uuid::new_v4().to_string(),
    )
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005 `phase plan` and
    //! D-08, never from running this code.

    use super::*;
    use crate::mcp::operations::expected::APPLY;
    use crate::mcp::operations::{APPLY_OPERATIONS, Lookup, Tool, lookup};

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn plan(stories: Value) -> Value {
        json!({"operation": "phase-plan", "request_id": R1, "phase": 1, "stories": stories})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_phase_plan_shape_or_digest_judged_wrongly_is_caught() {
        let mut unknown = plan(json!([1, 4]));
        unknown["plan"] = json!(1);
        let refusal = refused(&unknown);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let mut bad_id = plan(json!([1, 4]));
        bad_id["request_id"] = json!("X");
        let refusal = refused(&bad_id);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let refusal = refused(&plan(json!("1")));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let mut nothing = plan(json!([1, 4]));
        nothing["instruction"] = json!("bal-nothing");
        let refusal = refused(&nothing);
        assert_eq!(refusal["code"], "unknown-instruction");
        assert_eq!(refusal["slot"], "instruction");

        // A retry sends the identical call, which must replay, while a
        // reorder under the same request id is another request.
        let first = judge_arguments(&plan(json!([1, 4]))).expect("accepted");
        assert_eq!(first.request.kind, CommandKind("phase.plan".into()));
        assert_eq!((first.phase, first.stories.as_slice()), (1, &[1, 4][..]));
        let again = judge_arguments(&plan(json!([1, 4]))).expect("accepted");
        assert_eq!(again.request.digest, first.request.digest);
        let reordered = judge_arguments(&plan(json!([4, 1]))).expect("accepted");
        assert_ne!(reordered.request.digest, first.request.digest);
    }

    #[test]
    fn a_phase_plan_left_unserved_or_out_of_baseline_order_is_caught() {
        assert_eq!(
            lookup(Tool::Apply, Some("phase-plan")),
            Lookup::Available {
                needs_project: true
            }
        );
        assert_eq!(lookup(Tool::Query, Some("phase-plan")), Lookup::Unknown);
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let (last, earlier) = names.split_last().expect("a baseline");
        assert_eq!(*last, "phase-plan");
        // Every earlier spelling stays where it was before `phase-plan` was
        // appended, written out by hand in `expected`.
        assert_eq!(earlier, &APPLY[..APPLY.len() - 1]);
        assert_eq!(earlier.last(), Some(&"story-questions-submit"));
    }
}
