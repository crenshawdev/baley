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
//!
//! `plan-submit` and `plan-replace` on `baley_apply` (design 0005 `plan
//! submit` and `plan replace`, D-04, D-08, D-13): a session submits the
//! plan a planner order returned, as a new plan or as the replacement of
//! one in force. The arguments are judged first; the settings are read for
//! the capacity, recording nothing; then the write is prepared under the
//! caller carrying the order, with the submit's or the replacement's check
//! as its own check, and the draft is recorded through
//! [`crate::planning::submit::record`]. The owner approves the draft at the
//! terminal with `baley plan approve`.

use std::cell::RefCell;
use std::num::NonZeroU32;
use std::path::Path;

use baley_core::plan::Replaces;
use baley_store::{CommandKind, Hash, InstructionEvidence, ProjectId, RequestId, request_digest};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, WriteRequest, prepare_settings_read};
use crate::mcp::scope::{bind_instructions, order_caller};
use crate::mcp::work_order::{
    evidenced, judge_instruction, judge_order_id, judge_request_id, prepared_write, shape_fault,
};
use crate::planning::phase_plan::{self, PHASE_PLAN_COMMAND};
use crate::planning::{facts, replace, submit};
use crate::scope::submit::{NOT_RECORDED_BUSY, SUBMIT_COMMAND};
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

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `plan-submit` takes beside its `operation`. Unknown fields
/// are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSubmitShape {
    /// A fresh UUID for this plan submission in lowercase hyphenated form. A
    /// retry of the same plan submission sends the same id.
    pub request_id: String,
    /// The id of the planner work order this plan answers, as its receipt
    /// gave it, a UUID in lowercase hyphenated form.
    pub work_order: String,
    /// The plan exactly as the planner returned it.
    // Typed as an object in the schema, or a client may send it as a string.
    #[schemars(with = "Map<String, Value>")]
    pub plan: Value,
}

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `plan-replace` takes beside its `operation`. Unknown fields
/// are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanReplaceShape {
    /// A fresh UUID for this plan submission in lowercase hyphenated form. A
    /// retry of the same plan submission sends the same id.
    pub request_id: String,
    /// The id of the planner work order this plan answers, as its receipt
    /// gave it, a UUID in lowercase hyphenated form.
    pub work_order: String,
    /// The new plan exactly as the planner returned it.
    // Typed as an object in the schema, or a client may send it as a string.
    #[schemars(with = "Map<String, Value>")]
    pub plan: Value,
    /// The approved plan in force this plan replaces.
    pub replaces: ReplacesShape,
}

// `schema` serves this same declaration, so its doc comments are wire text.
/// The approved plan a replacement names. Unknown fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplacesShape {
    /// The number of the plan to replace, as its approval gave it.
    pub plan: u32,
    /// The digest that plan was approved at: 64 lowercase hexadecimal
    /// characters.
    pub digest: String,
}

/// The tagged forms the two submissions' arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum SubmitCall {
    #[serde(rename = "plan-submit")]
    Submit(PlanSubmitShape),
    #[serde(rename = "plan-replace")]
    Replace(PlanReplaceShape),
}

/// A `plan-submit` or `plan-replace` call whose arguments passed every check
/// made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedSubmit {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The planner order the plan answers.
    pub work_order: String,
    /// The plan as given, for the judge to judge.
    pub plan: Value,
    /// The plan a replacement replaces, or none for `plan-submit`.
    pub replaces: Option<Replaces>,
}

/// Judges `plan-submit`'s or `plan-replace`'s arguments before anything is
/// prepared or recorded: the shape, then the request id, the order id's
/// form, and a `replaces` digest that is not 64 lowercase hexadecimal
/// characters, `invalid-arguments` at slot `replaces`. A fault is the
/// `refused` answer, which records nothing. The request digest covers the
/// command kind, the plan as given, the order and, for a replacement, the
/// binding, so a retry replays and other content under the same id is a
/// reused id; a plan holding a number with no canonical form is
/// `invalid-arguments` at slot `plan`.
pub fn judge_submit_arguments(arguments: &Value) -> Result<JudgedSubmit, Value> {
    let (request_id, work_order, plan, replaces) =
        match serde_json::from_value::<SubmitCall>(arguments.clone()) {
            Ok(SubmitCall::Submit(shape)) => (shape.request_id, shape.work_order, shape.plan, None),
            Ok(SubmitCall::Replace(shape)) => (
                shape.request_id,
                shape.work_order,
                shape.plan,
                Some(shape.replaces),
            ),
            Err(error) => return Err(shape_fault(&error)),
        };
    judge_request_id(&request_id, "submission")?;
    judge_order_id(&work_order)?;
    let replaces = match replaces {
        None => None,
        Some(binding) => Some(Replaces {
            plan: binding.plan,
            digest: lowercase_digest(&binding.digest).ok_or_else(|| {
                Refusal::new(
                    "invalid-arguments",
                    "replaces.digest must be the digest the plan was approved at, 64 lowercase hexadecimal characters",
                )
                .slot("replaces")
                .value()
            })?,
        }),
    };
    let mut covered = json!({"command": SUBMIT_COMMAND, "plan": plan, "work_order": work_order});
    if let Some(Replaces { plan, digest }) = replaces {
        covered["replaces"] = json!({"plan": plan, "digest": digest.to_hex()});
    }
    let digest = request_digest(&covered).map_err(|error| {
        Refusal::new(
            "invalid-arguments",
            format!("the plan holds a number with no canonical form: {error}"),
        )
        .slot("plan")
        .value()
    })?;
    Ok(JudgedSubmit {
        request: WriteRequest {
            kind: CommandKind(SUBMIT_COMMAND.into()),
            request_id: RequestId(request_id),
            digest,
        },
        work_order,
        plan,
        replaces,
    })
}

/// `text` as a digest when it is 64 lowercase hexadecimal characters.
fn lowercase_digest(text: &str) -> Option<Hash> {
    if !text
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Hash::from_hex(text)
}

/// Serves one `plan-submit` or `plan-replace` call for the session's
/// `preparation`. The arguments are judged first; then the settings are read
/// for `planning.phase_capacity`, recording nothing; then the write is
/// prepared under the caller carrying the order, with the submit's or the
/// replacement's check as its own check, so every refusal comes before
/// checkout admission and the policy step and records nothing. A failed
/// settings read is answered by that check, so a retry of a recorded request
/// still replays as preparation keeps it. The command's caller then takes
/// the order's instruction evidence and the draft is recorded. A replay goes
/// to the recording with no read, and the store answers it from its record.
pub fn serve_submit(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_submit_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let caller = match order_caller(&preparation.caller, &judged.work_order) {
        Ok(caller) => caller,
        Err(refusal) => return refusal,
    };
    let capacity = capacity(preparation);
    // Kept for the binding and the recording. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        let capacity = capacity.clone()?;
        let checked = match judged.replaces {
            None => submit::check(store, project, &judged.plan, &judged.work_order, capacity),
            Some(replaces) => replace::check(
                store,
                project,
                &judged.plan,
                &judged.work_order,
                replaces,
                capacity,
            ),
        }?;
        *kept.borrow_mut() = Some(checked);
        Ok(())
    };
    let (store, command, _) = match prepared_write(preparation, &caller, &judged.request, &check) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let kept = kept.borrow();
    let evidence = kept.as_ref().map(|checked| checked.evidence.as_slice());
    match bind_instructions(command, evidence) {
        Ok(command) => {
            crate::scope::submit::answer(submit::record(store, &command, kept.as_ref()), store)
        }
        Err(answer) => answer,
    }
}

/// The capacity the session's settings set, read and judged with nothing
/// recorded, or the answer that read ended with.
fn capacity(preparation: &Preparation) -> Result<Option<NonZeroU32>, Value> {
    let ledger = preparation.ledger.as_deref();
    // With no ledger, the read fails at the project check, before any
    // settings are read from the config folder.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let prepared = prepare_settings_read(
        ledger.map(|ledger| &ledger.store),
        &preparation.caller,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
    )?;
    prepared
        .policy
        .as_ref()
        .map(facts::capacity)
        .ok_or_else(|| {
            failed(
                LEDGER_UNAVAILABLE,
                "the settings were read without a policy",
                PLACE_LEDGER,
            )
        })
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
        let at = names
            .iter()
            .position(|name| *name == "phase-plan")
            .expect("phase-plan is listed");
        // Every spelling up to `phase-plan` stays where it was when
        // `phase-plan` was appended, written out by hand in `expected`.
        assert_eq!(&names[..=at], &APPLY[..=at]);
        assert_eq!(names[at - 1], "story-questions-submit");
    }

    /// O and O2, two planner orders' ids.
    const O: &str = "1f2e3d4c-5b6a-4978-8695-a4b3c2d1e0f1";
    const O2: &str = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";

    fn submit(order: &str) -> Value {
        json!({"operation": "plan-submit", "request_id": R1, "work_order": order,
            "plan": crate::planning::fixtures::pl1()})
    }

    fn replace(plan: u32, digest: &str) -> Value {
        json!({"operation": "plan-replace", "request_id": R1, "work_order": O,
            "plan": crate::planning::fixtures::pl1(),
            "replaces": {"plan": plan, "digest": digest}})
    }

    fn submit_refused(arguments: &Value) -> Value {
        let refusal = judge_submit_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_plan_submission_shape_or_digest_judged_wrongly_is_caught() {
        let mut instruction = submit(O);
        instruction["instruction"] = json!("bal-phase-planner");
        let refusal = submit_refused(&instruction);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = submit_refused(&submit("ORDER"));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "work_order");

        // A retry sends the identical call, which must replay, while the
        // same plan answering another order is another request.
        let first = judge_submit_arguments(&submit(O)).expect("accepted");
        assert_eq!(first.request.kind, CommandKind("scope.submit".into()));
        assert_eq!(first.replaces, None);
        let again = judge_submit_arguments(&submit(O)).expect("accepted");
        assert_eq!(again.request.digest, first.request.digest);
        let other = judge_submit_arguments(&submit(O2)).expect("accepted");
        assert_ne!(other.request.digest, first.request.digest);

        let refusal = submit_refused(&replace(1, "ABC"));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "replaces");

        let digest = "5c".repeat(32);
        let one = judge_submit_arguments(&replace(1, &digest)).expect("accepted");
        let two = judge_submit_arguments(&replace(2, &digest)).expect("accepted");
        assert_eq!(
            one.replaces,
            Some(baley_core::plan::Replaces {
                plan: 1,
                digest: Hash([0x5c; 32])
            })
        );
        assert_ne!(one.request.digest, two.request.digest);
        assert_ne!(one.request.digest, first.request.digest);
    }

    #[test]
    fn plan_submit_left_retired_or_plan_replace_out_of_baseline_order_is_caught() {
        for name in ["plan-submit", "plan-replace"] {
            assert_eq!(
                lookup(Tool::Apply, Some(name)),
                Lookup::Available {
                    needs_project: true
                },
                "{name}"
            );
        }
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        // `plan-submit` keeps the place its retired spelling held, written
        // out by hand in `expected`, right after `execution-authorize`.
        let at = names.iter().position(|name| *name == "plan-submit");
        assert_eq!(at, APPLY.iter().position(|name| *name == "plan-submit"));
        assert_eq!(
            names[at.expect("plan-submit is listed") - 1],
            "execution-authorize"
        );
        let (last, earlier) = names.split_last().expect("a baseline");
        assert_eq!(*last, "plan-replace");
        assert_eq!(earlier.last(), Some(&"phase-plan"));
    }
}
