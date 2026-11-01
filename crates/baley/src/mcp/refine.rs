//! `story-refine`, `story-questions-submit` and `context-submit` on
//! `baley_apply`: a session asks for the analyzer's next order for one
//! story, submits the questions the analyzer returned as the story's set,
//! and submits the truths it drafted as a draft the owner approves at the
//! terminal (design 0005 `story refine`, `story questions submit` and
//! `story truths submit`, PLN-R2 to PLN-R6, D-06, D-07, D-08, D-09, D-19).
//! No operation here approves anything (ADR 0044). `context-intake` on
//! `baley_query` reads a story's refinement by its id, as the `document`
//! kind `refinement` does, recording nothing and issuing no order (D-10).
//!
//! Each call's arguments are judged before anything is prepared. The write
//! is then prepared with the call's check as its own check, so every refusal
//! comes before checkout admission and the policy step and records nothing,
//! and the order or the set is recorded through the `refine` library
//! functions.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::num::NonZeroU32;

use baley_core::policy::AcceptedNames;
use baley_core::questions::{QuestionSet, SetRefusal, judge_set};
use baley_core::truth::{TruthRefusal, TruthSubmission, judge_truths};
use baley_store::{CommandKind, InstructionEvidence, ProjectId, RequestId, Views, request_digest};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::store_failed;
use crate::mcp::document::{prepared_read, read_failed, refinement_answer};
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::scope::{bind_instructions, order_caller};
use crate::mcp::work_order::{
    evidenced, judge_instruction, judge_order_id, judge_request_id, prepared_write, shape_fault,
};
use crate::refine::order;
use crate::refine::questions::{self, QUESTIONS_SUBMIT_COMMAND};
use crate::refine::read::read as read_refinement;
use crate::refine::submit as truths;
use crate::scope::read::{NO_SUCH_STORY, ReadError};
use crate::scope::submit::{NOT_RECORDED_BUSY, SUBMIT_COMMAND};
use crate::work_order::issue as recording;
use crate::work_order::route::host_names;

/// The command kind a story refine is recorded under.
pub const REFINE_COMMAND: &str = "story.refine";

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `story-refine` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefineShape {
    /// A fresh UUID for this call in lowercase hyphenated form. A retry of
    /// the same call sends the same id.
    pub request_id: String,
    /// The id of the story to refine, as the roadmap or the backlog gives
    /// it.
    pub story: u32,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form `story-refine`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum RefineCall {
    #[serde(rename = "story-refine")]
    Refine(RefineShape),
}

/// A story refine whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedRefine {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The story to refine.
    pub story: u32,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `story-refine`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the instruction. A fault
/// is the `refused` answer, which records nothing. The request digest covers
/// the command kind and the story, so a retry replays and the same request
/// id for another story is a reused id.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedRefine, Value> {
    let shape = match serde_json::from_value::<RefineCall>(arguments.clone()) {
        Ok(RefineCall::Refine(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "call")?;
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    let digest = request_digest(&json!({"command": REFINE_COMMAND, "story": shape.story}))
        .expect("a story id is canonical");
    Ok(JudgedRefine {
        request: WriteRequest {
            kind: CommandKind(REFINE_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        story: shape.story,
        instruction,
    })
}

/// Serves one `story-refine` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the caller
/// with the instruction evidence attached and the refine check as its own
/// check; then, for a write that ran its steps, the host catalog is read and
/// the order the check chose is issued under a fresh lowercase UUID with the
/// prepared policy. A replay goes to the recording with no order, and the
/// store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the issue to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(order::check(store, project, judged.story)?);
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
    order::issue(
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
/// The arguments `story-questions-submit` takes beside its `operation`.
/// Unknown fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionsShape {
    /// A fresh UUID for this submission in lowercase hyphenated form. A
    /// retry of the same submission sends the same id.
    pub request_id: String,
    /// The id of the story the questions are for.
    pub story: u32,
    /// The id of the analyzer's questions work order this set answers, as
    /// its receipt gave it, a UUID in lowercase hyphenated form.
    pub work_order: String,
    /// The analyzer's questions exactly as it returned them, each
    /// `{"id":"<id>","question":"<text>","recommended":"<answer>","depends_on":["<id>"]}`.
    /// An empty list closes the set at once.
    // Typed as a list of objects in the schema, but taken as a value so the
    // set judge judges it and a fault is the judge's own code.
    #[schemars(with = "Vec<Map<String, Value>>")]
    pub questions: Value,
}

/// The tagged form `story-questions-submit`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum QuestionsCall {
    #[serde(rename = "story-questions-submit")]
    Submit(QuestionsShape),
}

/// A question set submission whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedQuestions {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The story the set is for.
    pub story: u32,
    /// The questions order it answers.
    pub work_order: String,
    /// The set, judged.
    pub set: QuestionSet,
}

/// Judges `story-questions-submit`'s arguments before anything is prepared
/// or recorded: the shape, then the request id, the order id's form and the
/// set whole. A fault is the `refused` answer, which records nothing; a set
/// the judge refuses is answered with its code at slot `questions` (PLN-R23).
/// The request digest covers the command kind, the story, the order and the
/// questions, so a retry replays and other questions under the same id are a
/// reused id.
pub fn judge_questions_arguments(arguments: &Value) -> Result<JudgedQuestions, Value> {
    let shape = match serde_json::from_value::<QuestionsCall>(arguments.clone()) {
        Ok(QuestionsCall::Submit(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "submission")?;
    judge_order_id(&shape.work_order)?;
    let set = judge_set(&shape.questions).map_err(|refusal| set_refused(&refusal))?;
    let digest = request_digest(&json!({"command": QUESTIONS_SUBMIT_COMMAND,
        "story": shape.story, "work_order": shape.work_order, "questions": shape.questions}))
    .expect("an accepted set is canonical");
    Ok(JudgedQuestions {
        request: WriteRequest {
            kind: CommandKind(QUESTIONS_SUBMIT_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        story: shape.story,
        work_order: shape.work_order,
        set,
    })
}

/// The `refused` answer for a set the judge refuses, at slot `questions`,
/// naming the question's position, counted from 0, and its id, or for a
/// cycle every id on it in its details.
fn set_refused(refusal: &SetRefusal) -> Value {
    let code = refusal.code();
    match refusal {
        SetRefusal::TypedContent { position } => {
            let answer = Refusal::new(
                code,
                "questions must be a list of objects, each with exactly `id`, `question` and `recommended` as text of at most 4096 bytes and `depends_on` as a list of ids, so nothing was recorded",
            )
            .slot("questions");
            match position {
                Some(position) => answer.entry(*position),
                None => answer,
            }
            .value()
        }
        SetRefusal::QuestionId { position, id }
        | SetRefusal::QuestionBlank { position, id }
        | SetRefusal::QuestionDependency { position, id } => {
            let why = match refusal {
                SetRefusal::QuestionId { .. } => "has a blank id or one an earlier question has",
                SetRefusal::QuestionBlank { .. } => "has a blank question or recommended answer",
                _ => "depends on an id the set lacks or on itself",
            };
            Refusal::new(
                code,
                format!("question `{id}` {why}, so nothing was recorded"),
            )
            .slot("questions")
            .entry(*position)
            .id(id)
            .value()
        }
        SetRefusal::QuestionCycle { questions } => {
            let ids: Vec<&str> = questions.iter().map(|(_, id)| id.as_str()).collect();
            Refusal::new(
                code,
                "questions depend on each other in a cycle, so no round can ask them and nothing was recorded. The details name them",
            )
            .slot("questions")
            .details(json!({"questions": ids}))
            .value()
        }
    }
}

/// Serves one `story-questions-submit` call for the session's
/// `preparation`, as `scope-submit` serves a start: the arguments are judged
/// first; then the write is prepared under the caller carrying the questions
/// order, with the set's check as its own check, so every refusal comes
/// before checkout admission and the policy step and records nothing; then
/// the command's caller takes the order's instruction evidence (D-19). For a
/// write that ran its steps the set is recorded, with the drafting order
/// prepared under the write's policy and the host catalog when the set is
/// empty and no other analyzer order of the story is pending (D-08). A
/// replay goes to the recording with no read, and the store answers it from
/// its record.
pub fn serve_questions(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_questions_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let caller = match order_caller(&preparation.caller, &judged.work_order) {
        Ok(caller) => caller,
        Err(refusal) => return refusal,
    };
    // Kept for the binding and the recording. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(questions::check(
            store,
            project,
            judged.story,
            &judged.work_order,
            &judged.set,
        )?);
        Ok(())
    };
    let (store, command, stepped) =
        match prepared_write(preparation, &caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    let kept = kept.borrow();
    let evidence = kept.as_ref().map(|checked| checked.evidence.as_slice());
    let command = match bind_instructions(command, evidence) {
        Ok(command) => command,
        Err(answer) => return answer,
    };
    let (Some(stepped), Some(checked)) = (stepped, kept.as_ref()) else {
        return questions::answer(
            questions::record(store, &command, &judged.set, None, None),
            store,
        );
    };
    // The catalog is read only for a drafting order, and unused otherwise.
    let names = if checked.drafts(&judged.set) {
        match host_names(store, preparation.host) {
            Ok(names) => names,
            Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
        }
    } else {
        AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        }
    };
    questions::submit(
        store,
        &command,
        &judged.set,
        checked,
        &stepped.policy,
        preparation.host,
        &names,
        &uuid::Uuid::new_v4().to_string(),
    )
}

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `context-submit` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextShape {
    /// A fresh UUID for this submission in lowercase hyphenated form. A
    /// retry of the same submission sends the same id.
    pub request_id: String,
    /// The id of the analyzer's drafting work order these truths answer, as
    /// its receipt gave it, a UUID in lowercase hyphenated form.
    pub work_order: String,
    /// The id of the story the truths are for.
    pub story: NonZeroU32,
    /// The id of the question set the truths were drafted from.
    pub set: String,
    /// The drafted truths, each
    /// `{"id":"<id>","form":"when"|"for-any","trigger":"<one trigger>","observer":"<one observer>","verb":"sees"|"gets"|"is refused","outcome":"<one outcome>","kind":"literal"|"property","observable":true,"fixed_oracle":true}`,
    /// with `human_verify` naming the tool or live service a person needs to
    /// prove it, when one is needed. Every text at most 4096 bytes of UTF-8.
    // Typed as a list of objects in the schema, but taken as a value so the
    // truth judge judges it and a fault is the judge's own code.
    #[schemars(with = "Vec<Map<String, Value>>")]
    pub truths: Value,
}

/// The tagged form `context-submit`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum ContextCall {
    #[serde(rename = "context-submit")]
    Submit(ContextShape),
}

/// A truth submission whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedContext {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The drafting order it answers.
    pub work_order: String,
    /// The accepted submission.
    pub submission: TruthSubmission,
}

/// Judges `context-submit`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, the order id's form and the
/// truths whole. A fault is the `refused` answer, which records nothing; a
/// truth the judge refuses is answered with its code at slot `truths` (D-01).
/// The request digest covers the command kind, the submission's digest and
/// the order, as `scope-submit`'s does, so the same content and order under
/// the same id replay and other content is a reused id.
pub fn judge_context_arguments(arguments: &Value) -> Result<JudgedContext, Value> {
    let shape = match serde_json::from_value::<ContextCall>(arguments.clone()) {
        Ok(ContextCall::Submit(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "submission")?;
    judge_order_id(&shape.work_order)?;
    let submission = judge_truths(shape.story, &shape.set, &shape.truths)
        .map_err(|refusal| truth_refused(&refusal))?;
    let digest = request_digest(&json!({"command": SUBMIT_COMMAND,
        "submission": submission.digest().to_hex(), "work_order": shape.work_order}))
    .expect("text is canonical");
    Ok(JudgedContext {
        request: WriteRequest {
            kind: CommandKind(SUBMIT_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        work_order: shape.work_order,
        submission,
    })
}

/// The `refused` answer for a truth the judge refuses, at slot `truths`,
/// naming the truth's position, counted from 0, and its id once it was read.
fn truth_refused(refusal: &TruthRefusal) -> Value {
    let (position, id, why) = match refusal {
        TruthRefusal::Form { position, id } => (
            *position,
            id.as_deref(),
            "is out of form: one trigger, one observer, a verb of `sees`, `gets` or `is refused`, a kind of `literal` or `property`, no blank part, a `for-any` truth only as `gets` with kind `property`, and only the members a truth takes",
        ),
        TruthRefusal::Id { position, id } => (
            Some(*position),
            Some(id.as_str()),
            "has a blank id or one an earlier truth has",
        ),
        TruthRefusal::Attestation { position, id } => (
            Some(*position),
            Some(id.as_str()),
            "lacks `observable` or `fixed_oracle`, or one is not true or false",
        ),
        TruthRefusal::Internal { position, id } => (
            Some(*position),
            Some(id.as_str()),
            "is attested as not observable from outside",
        ),
        TruthRefusal::ProseOracle { position, id } => (
            Some(*position),
            Some(id.as_str()),
            "is attested as having no fixed oracle",
        ),
    };
    let reason = match position {
        Some(position) => format!("the truth at {position} {why}, so nothing was recorded"),
        None => "truths must be a list of truth objects, so nothing was recorded".to_owned(),
    };
    let mut answer = Refusal::new(refusal.code(), reason).slot("truths");
    if let Some(position) = position {
        answer = answer.entry(position);
    }
    if let Some(id) = id {
        answer = answer.id(id);
    }
    answer.value()
}

/// Serves one `context-submit` call for the session's `preparation`, as
/// `scope-submit` serves a start: the arguments are judged first; then the
/// write is prepared under the caller carrying the drafting order, with the
/// truth submission's check as its own check, so every refusal comes before
/// checkout admission and the policy step and records nothing; then the
/// command's caller takes the order's instruction evidence (D-19) and the
/// draft is recorded through the truth-submission library function (D-06).
/// A replay goes to the recording with no read, and the store answers it
/// from its record.
pub fn serve_context(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_context_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let caller = match order_caller(&preparation.caller, &judged.work_order) {
        Ok(caller) => caller,
        Err(refusal) => return refusal,
    };
    // Kept for the binding and the recording. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(truths::check(
            store,
            project,
            &judged.submission,
            &judged.work_order,
        )?);
        Ok(())
    };
    let (store, command, _) = match prepared_write(preparation, &caller, &judged.request, &check) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let kept = kept.borrow();
    let evidence = kept.as_ref().map(|checked| checked.evidence.as_slice());
    match bind_instructions(command, evidence) {
        Ok(command) => crate::scope::submit::answer(
            truths::record(store, &command, &judged.submission, kept.as_ref()),
            store,
        ),
        Err(answer) => answer,
    }
}

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `context-intake` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntakeShape {
    /// The id of the story whose refinement to read, as the roadmap or the
    /// backlog gives it.
    pub story: u32,
    /// One-based part for answers over 24,576 bytes; defaults to 1.
    /// Concatenate the returned bodies in order.
    pub part: Option<usize>,
}

/// The tagged form `context-intake`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum IntakeCall {
    #[serde(rename = "context-intake")]
    Intake(IntakeShape),
}

/// Judges `context-intake`'s arguments. A shape fault, the parked `phase`
/// key among them, is `invalid-arguments`, answered before anything is
/// prepared.
pub fn judge_intake_arguments(arguments: &Value) -> Result<IntakeShape, Value> {
    match serde_json::from_value::<IntakeCall>(arguments.clone()) {
        Ok(IntakeCall::Intake(shape)) => Ok(shape),
        Err(error) => Err(Refusal::new("invalid-arguments", error.to_string())
            .slot("arguments")
            .value()),
    }
}

/// Serves one `context-intake` call for the session's `preparation`, as
/// `document` serves a read: the arguments are judged first, then the read
/// is prepared, which records nothing, then [`intake_answer`] reads the
/// story's refinement from the project. A `failed` preparation answer is
/// returned as it is. No order is issued and nothing is appended (D-10).
pub fn serve_intake(arguments: &Value, preparation: &Preparation) -> Value {
    let shape = match judge_intake_arguments(arguments) {
        Ok(shape) => shape,
        Err(refusal) => return refusal,
    };
    match prepared_read(preparation) {
        Ok((store, project)) => intake_answer(store, &project, &shape),
        Err(answer) => answer,
    }
}

/// The answer for `context-intake` over `store`: the story's refinement in
/// the members the `document` read of kind `refinement` answers, without an
/// `identity`, in parts when large. A story the roadmap does not hold is
/// `no-such-story` at slot `story`, echoing none of its id.
pub fn intake_answer<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    shape: &IntakeShape,
) -> Value {
    match read_refinement(store, project, shape.story) {
        Ok(refinement) => refinement_answer(None, refinement, shape.part),
        Err(ReadError::NoSuchStory) => {
            Refusal::new(NO_SUCH_STORY, "this project has no such story")
                .slot("story")
                .value()
        }
        Err(error) => read_failed(error),
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, D-19 and
    //! HST-R5, never from running this code.

    use baley_core::policy::Host;
    use baley_store::ServerCaller;

    use super::*;
    use crate::mcp::gate::Called;
    use crate::mcp::operations::{APPLY_OPERATIONS, Lookup, Tool, lookup, schema_answer};

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn refine(request_id: &str, story: u32) -> Value {
        json!({"operation": "story-refine", "request_id": request_id, "story": story})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_story_refine_call_judged_loosely_or_replayed_as_another_is_caught() {
        let first = judge_arguments(&refine(R1, 1)).expect("accepted");
        assert_eq!(first.request.kind, CommandKind("story.refine".into()));
        assert_eq!(first.story, 1);

        let mut unknown = refine(R1, 1);
        unknown["phase"] = json!(1);
        let refusal = refused(&unknown);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = refused(&refine(&R1.to_uppercase(), 1));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let mut nope = refine(R1, 1);
        nope["instruction"] = json!("bal-nope");
        let refusal = refused(&nope);
        assert_eq!(refusal["code"], "unknown-instruction");
        assert_eq!(refusal["slot"], "instruction");

        // A retry sends the identical call, which must replay, while the
        // same request id for another story is a reused id.
        let again = judge_arguments(&refine(R1, 1)).expect("accepted");
        assert_eq!(again.request.digest, first.request.digest);
        let other = judge_arguments(&refine(R1, 2)).expect("accepted");
        assert_ne!(other.request.digest, first.request.digest);
    }

    #[test]
    fn story_refine_left_out_of_the_baseline_or_unserved_is_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names
            .iter()
            .position(|name| *name == "story-refine")
            .expect("listed");
        assert_eq!(names[at - 1], "questions-answer");
        assert_eq!(names[at + 1], "story-questions-submit");
        assert_eq!(
            lookup(Tool::Apply, Some("story-refine")),
            Lookup::Available {
                needs_project: true
            }
        );
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "story-refine"}));
        assert_eq!(answer["status"], "ok", "{answer}");
        let required = answer["schema"]["required"].as_array().unwrap();
        for field in ["request_id", "story"] {
            assert!(required.contains(&json!(field)), "{field} in {answer}");
        }
        assert!(!required.contains(&json!("instruction")), "{answer}");
        assert!(
            answer["schema"]["properties"].get("instruction").is_some(),
            "{answer}"
        );

        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger and no checkout to discover, so a served call reaches
        // preparation and answers `failed` at its project check, never
        // `unknown-operation`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        let answer = crate::mcp::handler::operate(
            &Called::Operation(Tool::Apply, "story-refine".into()),
            Some(&refine(R1, 1)),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }

    const Q: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";

    fn submission(questions: Value) -> Value {
        json!({"operation": "story-questions-submit", "request_id": R1, "story": 1,
            "work_order": Q, "questions": questions})
    }

    fn question(id: &str, depends_on: &[&str]) -> Value {
        json!({"id": id, "question": "Which store backs the ledger?", "recommended": "SQLite",
            "depends_on": depends_on})
    }

    #[test]
    fn a_questions_submission_judged_loosely_is_caught() {
        let judged = |arguments: &Value| {
            let refusal = judge_questions_arguments(arguments).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{refusal}");
            refusal
        };
        let itself = judged(&submission(json!([
            question("q1", &[]),
            question("q2", &["q2"])
        ])));
        assert_eq!(itself["code"], "question-dependency");
        assert_eq!(itself["slot"], "questions");
        assert_eq!(itself["entry"], 1);
        assert_eq!(itself["id"], "q2");

        let cycle = judged(&submission(json!([
            question("q1", &["q2"]),
            question("q2", &["q1"])
        ])));
        assert_eq!(cycle["code"], "question-cycle");
        assert_eq!(cycle["slot"], "questions");
        assert_eq!(cycle["details"], json!({"questions": ["q1", "q2"]}));

        let mut instructed = submission(json!([question("q1", &[])]));
        instructed["instruction"] = json!("bal-rounds");
        let refusal = judged(&instructed);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = judged(&submission(json!("q1")));
        assert_eq!(refusal["code"], "typed-content");
        assert_eq!(refusal["slot"], "questions");
    }

    #[test]
    fn story_questions_submit_out_of_the_baseline_or_unserved_is_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names
            .iter()
            .position(|name| *name == "story-questions-submit")
            .expect("listed");
        assert_eq!(names[at - 2..at], ["questions-answer", "story-refine"]);
        assert_eq!(names[at + 1], "phase-plan");
        let answer = schema_answer(
            &json!({"operation": "schema", "tool": "apply", "for": "story-questions-submit"}),
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let required = answer["schema"]["required"].as_array().unwrap();
        for field in ["request_id", "story", "work_order", "questions"] {
            assert!(required.contains(&json!(field)), "{field} in {answer}");
        }
        assert!(
            answer["schema"]["properties"].get("instruction").is_none(),
            "{answer}"
        );

        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        let answer = crate::mcp::handler::operate(
            &Called::Operation(Tool::Apply, "story-questions-submit".into()),
            Some(&submission(json!([question("q1", &[])]))),
            Some(&preparation),
        );
        // No ledger and no checkout to discover, so a served call reaches
        // preparation and answers `failed` at its project check, never
        // `unknown-operation`.
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    fn context(request_id: &str, truths: Value) -> Value {
        json!({"operation": "context-submit", "request_id": request_id, "work_order": Q,
            "story": 1, "set": S, "truths": truths})
    }

    #[test]
    fn a_context_submit_judged_loosely_or_under_another_digest_is_caught() {
        use crate::refine::submit::tests::{t1, t2, with};

        let u1 = judge_context_arguments(&context(R1, json!([t1(), t2()]))).expect("accepted");
        assert_eq!(
            u1.submission.digest().to_hex(),
            "c85deb14471b95d360f3d94b276751e67770ea1ce447597bce8cec83436e2bab"
        );
        assert_eq!(u1.work_order, Q);
        assert_eq!(u1.request.kind, CommandKind("scope.submit".into()));

        let refusal =
            judge_context_arguments(&context(R1, json!([t1(), with(t2(), "verb", "sees")])))
                .expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "truth-form");
        assert_eq!(refusal["slot"], "truths");
        assert_eq!(refusal["entry"], 1);
        assert_eq!(refusal["id"], "t2");

        let mut instructed = context(R1, json!([t1(), t2()]));
        instructed["instruction"] = json!("bal-analyzer-drafting");
        let refusal = judge_context_arguments(&instructed).expect_err("refused");
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let again = judge_context_arguments(&context(R1, json!([t1(), t2()]))).expect("accepted");
        assert_eq!(again.request.digest, u1.request.digest);
    }

    #[test]
    fn context_submit_left_retired_or_moved_is_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names
            .iter()
            .position(|name| *name == "context-submit")
            .expect("listed");
        assert_eq!(names[at - 1], "plan-submit");
        assert_eq!(names[at + 1], "review-admit");
        assert_eq!(
            lookup(Tool::Apply, Some("context-submit")),
            Lookup::Available {
                needs_project: true
            }
        );
        let answer = schema_answer(
            &json!({"operation": "schema", "tool": "apply", "for": "context-submit"}),
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let required = answer["schema"]["required"].as_array().unwrap();
        for field in ["request_id", "work_order", "story", "set", "truths"] {
            assert!(required.contains(&json!(field)), "{field} in {answer}");
        }

        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        let answer = crate::mcp::handler::operate(
            &Called::Operation(Tool::Apply, "context-submit".into()),
            Some(&context(R1, json!([]))),
            Some(&preparation),
        );
        // No ledger and no checkout to discover, so a served call reaches
        // preparation and answers `failed` at its project check, never
        // `operation-unavailable`.
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }

    #[test]
    fn context_intake_left_retired_keyed_by_phase_or_moved_is_caught() {
        use crate::mcp::operations::QUERY_OPERATIONS;

        let names: Vec<&str> = QUERY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names
            .iter()
            .position(|name| *name == "context-intake")
            .expect("listed");
        assert_eq!(names[at - 1], "plan-read");
        assert_eq!(names[at + 1], "route");
        assert_eq!(
            lookup(Tool::Query, Some("context-intake")),
            Lookup::Available {
                needs_project: true
            }
        );
        let answer = schema_answer(
            &json!({"operation": "schema", "tool": "query", "for": "context-intake"}),
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let required = answer["schema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("story")), "{answer}");
        assert!(!required.contains(&json!("part")), "{answer}");
        assert!(!required.contains(&json!("phase")), "{answer}");
        assert!(
            answer["schema"]["properties"].get("part").is_some(),
            "{answer}"
        );

        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        // The arguments are judged before anything is prepared, so the parked
        // phase key is refused even with no ledger.
        let answer = crate::mcp::handler::operate(
            &Called::Operation(Tool::Query, "context-intake".into()),
            Some(&json!({"operation": "context-intake", "phase": 1})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "invalid-arguments", "{answer}");
        assert_eq!(answer["slot"], "arguments", "{answer}");
    }
}
