//! The work-order operations on `baley_apply` (design 0004 section 4,
//! design 0012). `project-start` records one `work_order.issued` for the
//! start planner on the `roadmap` stream, with the brief as a `record`
//! payload, after the arguments, the start check and preparation pass.
//! `work-order-fail` records one `work_order.failed` for a pending order a
//! session reports could not launch or returned without a valid result.
//! `work-order-retry` records the next attempt of a failed order as a new
//! `work_order.issued`, its route resolved again under the current policy. A
//! retried checker order keeps its parent's inputs and so its binding, its
//! result finding that binding through the chain's first order with no second
//! `plan.check_issued`, and it runs only while the submission it checks is the
//! phase's newest plan draft (D-10). A refusal records nothing. The `route`
//! query on `baley_query` explains the route a role or an order runs under, and
//! records nothing at all.

use std::cell::RefCell;
use std::num::NonZeroU32;
use std::path::Path;

use baley_core::capture::is_blank;
use baley_core::plan::{PLAN_KIND, phase_stream};
use baley_core::policy::recorded::{
    POLICY_VIEW, PolicyJudgement, effective_payload, judge_policy, policy_key, recorded_policy,
};
use baley_core::policy::{AcceptedNames, CONFIG_UNAVAILABLE, EffectivePolicy, Host, Layer, Role};
use baley_core::scope::{DRAFT_VIEW, STALE_DRAFT, draft_key};
use baley_core::work_order::{
    ANSWERED, MATERIAL_UNAVAILABLE, NO_SUCH_WORK_ORDER, PENDING, Retry, RetryRefusal,
    START_ORDER_PENDING, START_SUBJECT, WORK_ORDER_NOT_FAILED, WORK_ORDER_NOT_PENDING,
    WORK_ORDER_RETRIED, WORK_ORDER_VIEW, judge_retry, subject_phase, subject_story, work_order_key,
};
use baley_store::{
    Absence, Command, CommandKind, Decision, Document, Hash, InstructionEvidence, Ledger, Observed,
    ObservedDocument, OutcomeKind, Payloads, ProjectId, RETENTION_STREAM, Recorded, RequestId,
    ServerCaller, StaleInput, StoreError, StreamName, Transaction, Views, request_digest,
};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::checkout::{holds_existing_code, tracked_files};
use crate::discovery::PROJECT_FILE;
use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::client::MAX_ECHOED_BYTES;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{
    CHECKOUT_FACTS_UNAVAILABLE, LEDGER_UNAVAILABLE, Prepared, Stepped, WriteRequest,
    prepare_checked, prepare_settings_read,
};
use crate::planning::check_return::order_binding;
use crate::planning::checker::{Refused, checker_absences, checker_order_pending, pending_checker};
use crate::planning::order::{pending_planner, planner_absences, planner_order_pending};
use crate::process::Process;
use crate::refine::order::{
    analyzer_absences, analyzer_order_pending, pending_analyzer, refinement_state,
};
use crate::scope::approve::newest_query;
use crate::scope::submit::{NOT_RECORDED_BUSY, start_once};
use crate::work_order::fail::{self, Pending};
use crate::work_order::issue::{self, Input, Issue, start_absences};
use crate::work_order::read::{order_document, pending_start, releases};
use crate::work_order::route::{compose, host_names};

/// The command kind a project start is recorded under.
pub const START_COMMAND: &str = "project.start";

/// The code for a brief that is empty or only whitespace.
pub const BRIEF_BLANK: &str = "brief-blank";

/// The code for a brief over [`MAX_BRIEF_BYTES`].
pub const BRIEF_TOO_LARGE: &str = "brief-too-large";

/// The most bytes of UTF-8 a brief may hold (D-01).
pub const MAX_BRIEF_BYTES: usize = 65_536;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `project-start` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartShape {
    /// A fresh UUID for this start in lowercase hyphenated form. A retry of
    /// the same start sends the same id.
    pub request_id: String,
    /// The project brief as text, which the session reads and sends whole.
    /// Baley reads no file for it. It must not be blank, and it holds at
    /// most 65536 bytes of UTF-8.
    pub brief: String,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum StartCall {
    #[serde(rename = "project-start")]
    Start(StartShape),
}

/// A project start whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedStart {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The brief, as sent.
    pub brief: String,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `project-start`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, the brief and the instruction,
/// in that order. A fault is the `refused` answer, which records nothing.
pub fn judge_start_arguments(arguments: &Value) -> Result<JudgedStart, Value> {
    let shape = match serde_json::from_value::<StartCall>(arguments.clone()) {
        Ok(StartCall::Start(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "start")?;
    if is_blank(&shape.brief) {
        return Err(Refusal::new(
            BRIEF_BLANK,
            "the brief is empty or only whitespace, so nothing was recorded. Send the brief's text",
        )
        .slot("brief")
        .value());
    }
    let size = shape.brief.len();
    if size > MAX_BRIEF_BYTES {
        return Err(Refusal::new(
            BRIEF_TOO_LARGE,
            format!(
                "the brief is {size} bytes of UTF-8, over the limit of {MAX_BRIEF_BYTES} bytes, so nothing was recorded. Send a shorter brief"
            ),
        )
        .slot("brief")
        .value());
    }
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    // The brief's hash stands for it, so the same brief under the same id
    // replays and another brief is a reused id.
    let brief = Hash(Sha256::digest(shape.brief.as_bytes()).into());
    let digest = request_digest(&json!({"command": START_COMMAND, "brief": brief.to_hex()}))
        .expect("text is canonical");
    Ok(JudgedStart {
        request: WriteRequest {
            kind: CommandKind(START_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        brief: shape.brief,
        instruction,
    })
}

/// Project start's check, run by preparation after the replay lookup and
/// before the settings, recording nothing: an approved start is
/// `project-already-started` (D-19), and a pending start planner order is
/// `start-order-pending`, naming it (D-02). A read fault is the store
/// error's `failed` answer.
pub fn start_check<S: Ledger + Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<(), Value> {
    start_once(store, project)?;
    let pending =
        pending_start(store, project).map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
    let Some(order) = pending else {
        return Ok(());
    };
    let id = order.get("id").and_then(Value::as_str).unwrap_or_default();
    Err(Refusal::new(
        START_ORDER_PENDING,
        format!(
            "the start planner's work order {id} is still pending, so no other start was recorded. Read that order with `document`, or fail it with `work-order-fail` and retry it with `work-order-retry`"
        ),
    )
    .slot("operation")
    .details(json!({"work_order": id}))
    .value())
}

/// The lines a start's answer carries when settings are missing (D-18):
/// one when `policy`, the host's effective policy, has no global file, and
/// one when no setting comes from the project file. Each says the other
/// file's settings and the defaults apply and names `baley config interview`.
pub fn settings_lines(policy: &EffectivePolicy) -> Vec<String> {
    let mut lines = Vec::new();
    if policy.global.is_none() {
        lines.push(
            "There is no global settings file, so the project file's settings and Baley's defaults apply. The owner can run `baley config interview` to choose settings".to_string(),
        );
    }
    let from_project = policy
        .settings
        .values()
        .any(|setting| matches!(setting.source.layer, Layer::Project | Layer::ProjectHost));
    if !from_project {
        lines.push(
            "The project file sets nothing beyond [project], so the global settings file's settings and Baley's defaults apply to this project. The owner can run `baley config interview` to choose settings".to_string(),
        );
    }
    lines
}

/// The instruction a start order carries for its planner.
const START_PLANNER: &str = "bal-start-planner";

/// The instruction a start order adds when the checkout holds code.
const START_SURVEY: &str = "bal-start-survey";

/// Issues the start planner's first order as `id` under `command`. It
/// carries the start planner's instruction, then the survey's when the
/// checkout holds `existing` code (D-20, D-24), and runs under the route for
/// `planner` at attempt 1 under `policy` at the command's policy version
/// (D-11) with the catalog `names`. The id is also its request, it has no
/// parent, and `brief` is its one input. The receipt adds the settings lines
/// `policy` gives and whether code exists.
#[allow(clippy::too_many_arguments)]
pub fn issue_start_order(
    store: &SqliteStore,
    command: &Command,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    existing: bool,
    brief: &str,
    id: &str,
) -> Value {
    let mut instructions = Vec::new();
    for identity in std::iter::once(START_PLANNER).chain(existing.then_some(START_SURVEY)) {
        match crate::instruction::evidence(identity) {
            Ok(evidence) => instructions.push(evidence),
            Err(refusal) => return refusal,
        }
    }
    let route = match compose(
        policy,
        command.policy_version,
        host,
        Role::Planner,
        NonZeroU32::MIN,
        names,
    ) {
        Ok(route) => route,
        Err(refusal) => return Refusal::new(refusal.code(), refusal.to_string()).value(),
    };
    let lines = settings_lines(policy);
    let mut further = Map::new();
    if !lines.is_empty() {
        further.insert("settings".into(), json!(lines));
    }
    further.insert("existing_code".into(), json!(existing));
    let inputs = [Input::Bytes {
        name: "brief",
        bytes: brief.as_bytes(),
    }];
    let absences = start_absences();
    let issue = Issue {
        id,
        request: id,
        parent: None,
        subject: START_SUBJECT,
        route: &route,
        instructions: &instructions,
        inputs: &inputs,
        absences: &absences,
        streams: &[],
        documents: &[],
        further: &further,
    };
    issue::answer(issue::record(store, command, Some(&issue)), store)
}

/// Serves one `project-start` call for the session's `preparation`. The
/// arguments are judged first, and a refusal is answered before anything is
/// prepared. Then the write is prepared under the caller with the
/// instruction evidence attached, with [`start_check`] as its own check, so
/// a start already approved or a start order still pending ends the call
/// before checkout admission and the policy step.
///
/// For a write that ran its steps, the checkout is judged for existing code
/// from the root commit and, when there is one, HEAD's listing, and
/// [`issue_start_order`] issues the order under a fresh UUID with the
/// prepared policy and the host catalog read now. A replay goes to the
/// recording with no order, and the store answers it from its record. No
/// acknowledgement is checked (D-12) and no `forge.checked` is recorded
/// (D-21).
pub fn serve_start(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_start_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder or the check is asked.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => start_check(&ledger.store, project),
        None => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was checked without a ledger",
            PLACE_LEDGER,
        )),
    };
    let caller = evidenced(&preparation.caller, judged.instruction.as_ref());
    let mut process = crate::process::System;
    let prepared = match prepare_checked(
        ledger.map(|ledger| &ledger.store),
        &caller,
        &judged.request,
        config,
        preparation.host,
        &preparation.at,
        &mut process,
        &check,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command.as_ref()) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    let store = &ledger.store;
    let Some(stepped) = &prepared.stepped else {
        return issue::answer(issue::record(store, command, None), store);
    };
    let existing = match existing_code(&prepared, stepped, &mut process) {
        Ok(existing) => existing,
        Err(text) => return failed(CHECKOUT_FACTS_UNAVAILABLE, text, "git"),
    };
    let names = match host_names(store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
    };
    issue_start_order(
        store,
        command,
        &stepped.policy,
        preparation.host,
        &names,
        existing,
        &judged.brief,
        &uuid::Uuid::new_v4().to_string(),
    )
}

/// Whether the prepared checkout holds existing code. HEAD's files are
/// listed only when it has a root commit, so an unborn HEAD runs no git.
fn existing_code(
    prepared: &Prepared,
    stepped: &Stepped,
    process: &mut dyn Process,
) -> Result<bool, String> {
    let Some(root_commit) = stepped.root_commit.as_deref() else {
        return Ok(false);
    };
    let listing = tracked_files(&prepared.root, process)?;
    let file = prepared
        .project_file
        .strip_prefix(&prepared.root)
        .unwrap_or(Path::new(PROJECT_FILE))
        .to_string_lossy();
    Ok(holds_existing_code(Some(root_commit), &listing, &file))
}

/// The caller a start is prepared under: `caller` with `instruction`'s
/// evidence attached, or as it is when the session named none, as `capture`
/// attaches it, so every event of the call carries it.
pub(crate) fn evidenced(
    caller: &ServerCaller,
    instruction: Option<&InstructionEvidence>,
) -> ServerCaller {
    match instruction {
        None => caller.clone(),
        Some(evidence) => caller
            .clone()
            .with_instructions(vec![evidence.clone()])
            .expect("one entry is within the caller's instruction limit"),
    }
}

/// The command kind a work order's failure is recorded under.
pub const FAIL_COMMAND: &str = "work_order.fail";

/// The most bytes of UTF-8 a failure's reason may hold.
pub const MAX_REASON_BYTES: usize = 4_096;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `work-order-fail` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailShape {
    /// A fresh UUID for this report in lowercase hyphenated form. A retry of
    /// the same report sends the same id.
    pub request_id: String,
    /// The id of the work order whose worker could not launch or returned
    /// without a valid result, as the order's receipt gave it.
    pub work_order: String,
    /// Why the order failed, in plain words. It must not be blank, and it
    /// holds at most 4096 bytes of UTF-8.
    pub reason: String,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form `work-order-fail`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum FailCall {
    #[serde(rename = "work-order-fail")]
    Fail(FailShape),
}

/// A failure report whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedFail {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The failed order's id.
    pub work_order: String,
    /// Why it failed, as sent.
    pub reason: String,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `work-order-fail`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, the order id, the reason and
/// the instruction, in that order. A fault is the `refused` answer, which
/// records nothing.
pub fn judge_fail_arguments(arguments: &Value) -> Result<JudgedFail, Value> {
    let shape = match serde_json::from_value::<FailCall>(arguments.clone()) {
        Ok(FailCall::Fail(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "report")?;
    judge_order_id(&shape.work_order)?;
    if is_blank(&shape.reason) {
        return Err(Refusal::new(
            "invalid-arguments",
            "the reason is empty or only whitespace, so nothing was recorded. Say why the worker could not launch or returned without a valid result",
        )
        .slot("reason")
        .value());
    }
    let size = shape.reason.len();
    if size > MAX_REASON_BYTES {
        return Err(Refusal::new(
            "invalid-arguments",
            format!(
                "the reason is {size} bytes of UTF-8, over the limit of {MAX_REASON_BYTES} bytes, so nothing was recorded. Send a shorter reason"
            ),
        )
        .slot("reason")
        .value());
    }
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    let digest = request_digest(
        &json!({"command": FAIL_COMMAND, "work_order": shape.work_order,
        "reason": shape.reason}),
    )
    .expect("text is canonical");
    Ok(JudgedFail {
        request: WriteRequest {
            kind: CommandKind(FAIL_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        work_order: shape.work_order,
        reason: shape.reason,
        instruction,
    })
}

/// The refusal for an order id this project's `work_order` view does not
/// hold. It echoes none of the id.
fn no_such_order() -> Value {
    Refusal::new(
        NO_SUCH_WORK_ORDER,
        "this project holds no work order with that id, so nothing was recorded",
    )
    .slot("work_order")
    .value()
}

/// A failure's check, run by preparation before admission and recording
/// nothing: the order by id, none `no-such-work-order`, and one that is not
/// pending `work-order-not-pending`, naming its state. A pass gives back
/// what the recording observes.
pub fn fail_check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<Pending, Value> {
    let document = order_document(store, project, id)
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
        .ok_or_else(no_such_order)?;
    let text = |name: &str| document.body.get(name).and_then(Value::as_str);
    let (Some(state), Some(subject)) = (text("state"), text("subject")) else {
        return Err(order_unreadable());
    };
    if state != PENDING {
        return Err(Refusal::new(
            WORK_ORDER_NOT_PENDING,
            format!(
                "work order {id} is {state}, so no failure was recorded. A failed order is retried with `work-order-retry`, and an answered one has already returned"
            ),
        )
        .slot("work_order")
        .details(json!({"state": state}))
        .value());
    }
    Ok(Pending {
        id: id.into(),
        subject: subject.into(),
        produced_seq: document.produced_seq,
    })
}

fn order_unreadable() -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        "the work order is recorded, but its view document cannot be read",
        PLACE_LEDGER,
    )
}

/// Serves one `work-order-fail` call for the session's `preparation`
/// (decision 2: a session reports a failure, nothing infers one). The
/// arguments are judged first; then the write is prepared under the caller
/// with the instruction evidence attached and [`fail_check`] as its check,
/// and the failure is recorded over the order the check read. A replay goes
/// to the recording with no read, and the store answers it from its record.
pub fn serve_fail(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_fail_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the recording to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(fail_check(store, project, &judged.work_order)?);
        Ok(())
    };
    let caller = evidenced(&preparation.caller, judged.instruction.as_ref());
    match prepared_write(preparation, &caller, &judged.request, &check) {
        Ok((store, command, _)) => issue::answer(
            fail::record(store, &command, &judged.reason, kept.borrow().as_ref()),
            store,
        ),
        Err(answer) => answer,
    }
}

/// Prepares a work-order write for the session under `caller` with `check`
/// as its own check, asked against the session's ledger. It gives the
/// ledger, the command to record under and what the write's steps found, or
/// the answer preparation ended with.
pub(crate) fn prepared_write<'a>(
    preparation: &'a Preparation,
    caller: &ServerCaller,
    request: &WriteRequest,
    check: &dyn Fn(&SqliteStore, &ProjectId) -> Result<(), Value>,
) -> Result<(&'a SqliteStore, Command, Option<Stepped>), Value> {
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder or the check is asked.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => check(&ledger.store, project),
        None => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was checked without a ledger",
            PLACE_LEDGER,
        )),
    };
    let prepared = prepare_checked(
        ledger.map(|ledger| &ledger.store),
        caller,
        request,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
        &check,
    )?;
    match (ledger, prepared.command) {
        (Some(ledger), Some(command)) => Ok((&ledger.store, command, prepared.stepped)),
        _ => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        )),
    }
}

/// The command kind a work order's retry is recorded under.
pub const RETRY_COMMAND: &str = "work_order.retry";

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `work-order-retry` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryShape {
    /// A fresh UUID for this retry in lowercase hyphenated form. Sending the
    /// same retry again sends the same id, and it issues no second attempt.
    pub request_id: String,
    /// The id of the failed work order to issue the next attempt of, as its
    /// receipt gave it. It must be the newest attempt of its chain.
    pub work_order: String,
    /// The identity of the instruction the session followed, if it followed
    /// one.
    pub instruction: Option<String>,
}

/// The tagged form `work-order-retry`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum RetryCall {
    #[serde(rename = "work-order-retry")]
    Retry(RetryShape),
}

/// A retry whose arguments passed every check made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedRetry {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The failed order's id.
    pub work_order: String,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `work-order-retry`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, the order id and the
/// instruction, in that order. The request digest covers the order alone, so
/// the same retry sent again under its request id is replayed from its
/// record and counts once (D-14). A fault is the `refused` answer, which
/// records nothing.
pub fn judge_retry_arguments(arguments: &Value) -> Result<JudgedRetry, Value> {
    let shape = match serde_json::from_value::<RetryCall>(arguments.clone()) {
        Ok(RetryCall::Retry(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "retry")?;
    judge_order_id(&shape.work_order)?;
    let instruction = judge_instruction(shape.instruction.as_deref())?;
    let digest = request_digest(&json!({"command": RETRY_COMMAND, "work_order": shape.work_order}))
        .expect("text is canonical");
    Ok(JudgedRetry {
        request: WriteRequest {
            kind: CommandKind(RETRY_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        work_order: shape.work_order,
        instruction,
    })
}

/// What a retry's check read, which its issue records and observes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedRetry {
    /// What the retry re-issues, as the retry judge gave it.
    pub retry: Retry,
    /// The identities of the parent's instructions, in order.
    pub instructions: Vec<String>,
    /// The parent's document and each earlier attempt's, for an analyzer
    /// order the documents its story's refinement state was read from, and
    /// for a checker order the `phase_plan` document, at the sequence that
    /// last produced each.
    pub documents: Vec<ObservedDocument>,
    /// The `retention` stream's version the releases were read up to.
    pub retention: u64,
    /// For a checker order, the phase's newest plan draft the check read, by
    /// digest and sequence, which the issue confirms.
    pub newest_draft: Option<(Hash, u64)>,
}

/// A retry's check, run by preparation before admission and recording
/// nothing. It reads the parent by id, none `no-such-work-order`, then each
/// earlier attempt back to the chain's first order and this project's
/// releases, and has the retry judge answer: a pending or answered order, or
/// one whose chain an earlier attempt's return ended, `work-order-not-failed`
/// (decision 2), one already retried `work-order-retried`, naming the retry,
/// and one whose input this project purged `material-unavailable` (decision
/// 1). A start order is then refused `project-already-started` once a start
/// is approved, as project start is (D-19), and `start-order-pending` while
/// another start order is pending, so a project keeps one live start order
/// (D-02). A story's analyzer order is refused `analyzer-order-pending`
/// while another analyzer order of the story is pending, so a story keeps one
/// live analyzer order (D-09), and then `question-open` while a question of
/// the story's latest set is open, as `story-refine` is, so a retried order
/// neither drafts before the owner decided nor returns questions that would
/// abandon an open set. A phase's planner order is refused
/// `planner-order-pending` while another planner order of the phase is
/// pending, so a phase keeps one live planner order (D-11), and then
/// `checker-order-pending` while the phase's checker order is pending, since
/// a blocker it returns issues the phase's revision order (D-07). A phase's
/// checker order is refused `stale-draft` at slot `work_order`, naming the
/// newest draft, unless the draft its binding names is the phase's newest
/// plan draft and was not submitted again since the order was issued, so a
/// check is not run again on a draft a later submission took over, whatever
/// that submission's digest (D-10). A checker order pending beside a failed one
/// always comes from such a submission, so this answers for it too. Then it is
/// refused `planner-order-pending` while a planner order of the phase is
/// pending, since a round-1 blocker's revision declares none.
pub fn retry_check<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<CheckedRetry, Value> {
    let read = |id: &str| {
        order_document(store, project, id).map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))
    };
    let parent = read(id)?.ok_or_else(no_such_order)?;
    let mut documents = vec![observed(id, &parent)];
    let mut earlier = Vec::new();
    let mut next = parent_of(&parent.body)?;
    while let Some(at) = next {
        // The view holds every parent before its child, so a missing one
        // means the view cannot be read.
        let document = read(&at)?.ok_or_else(order_unreadable)?;
        documents.push(observed(&at, &document));
        next = parent_of(&document.body)?;
        earlier.push(document.body);
    }
    let releases =
        releases(store, project).map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
    let retry = judge_retry(&parent.body, &earlier, &releases.pairs())
        .map_err(|refusal| retry_refused(id, refusal))?;
    if is_start(&retry) {
        start_once(store, project)?;
        let pending = pending_start(store, project)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
        if let Some(order) = pending {
            let pending = order.get("id").and_then(Value::as_str).unwrap_or_default();
            return Err(Refusal::new(
                START_ORDER_PENDING,
                format!(
                    "the start planner's work order {pending} is pending, so no retry of {id} was issued, since a project has one live start order. Read that order with `document`, or fail it with `work-order-fail` and retry it with `work-order-retry`"
                ),
            )
            .slot("operation")
            .details(json!({"work_order": pending}))
            .value());
        }
    }
    if let Some(story) = analyzer_story(&retry) {
        let pending = pending_analyzer(store, project, story)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
        if let Some(order) = pending {
            return Err(analyzer_order_pending(
                story,
                &order,
                &format!("so no retry of {id} was issued"),
            ));
        }
        // The issue observes what the state was read from, so a set opened
        // since the check makes it `ledger-busy`.
        documents.extend(refinement_state(store, project, story)?.documents);
    }
    if let Some(phase) = planner_phase(&retry) {
        let pending = pending_planner(store, project, phase)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
        if let Some(order) = pending {
            return Err(planner_order_pending(
                phase,
                &order,
                &format!("so no retry of {id} was issued"),
            ));
        }
        // A blocker's revision must find no other live planner order of the
        // phase when the checker returns (D-07).
        let checking = pending_checker(store, project, phase)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
        if let Some(order) = checking {
            return Err(checker_order_pending(phase, &order, Refused::Retry));
        }
    }
    let mut newest_draft = None;
    if let Some(phase) = checker_phase(&retry) {
        let (binding, phase_plan) = order_binding(store, project, &parent.body, phase)?;
        let newest = store
            .find(
                project,
                DRAFT_VIEW,
                &newest_query(PLAN_KIND, &phase_stream(phase)),
            )
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
            .items
            .first()
            .and_then(draft_at)
            .ok_or_else(order_unreadable)?;
        // A draft and its order are recorded together, the draft first, so a
        // later sequence is a later submission, even of the same digest.
        let issued = parent
            .body
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(order_unreadable)?;
        if newest.0 != binding.plan_digest || newest.1 > issued {
            return Err(stale_checker_retry(id, newest.0));
        }
        let pending = pending_planner(store, project, phase)
            .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?;
        if let Some(order) = pending {
            return Err(planner_order_pending(
                phase,
                &order,
                &format!("so no retry of {id} was issued"),
            ));
        }
        documents.push(phase_plan);
        newest_draft = Some(newest);
    }
    let instructions = match parent.body.get("instructions") {
        Some(Value::Array(listed)) => listed
            .iter()
            .map(|entry| {
                entry
                    .get("identity")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(order_unreadable)?,
        _ => return Err(order_unreadable()),
    };
    Ok(CheckedRetry {
        retry,
        instructions,
        documents,
        retention: releases.version,
        newest_draft,
    })
}

/// Whether `retry` re-issues the start planner's order.
fn is_start(retry: &Retry) -> bool {
    retry.role == Role::Planner && retry.subject == START_SUBJECT
}

/// The story whose analyzer order `retry` re-issues, if it is one.
fn analyzer_story(retry: &Retry) -> Option<u32> {
    (retry.role == Role::Analyzer)
        .then(|| subject_story(&retry.subject))
        .flatten()
}

/// The phase whose planner order `retry` re-issues, if it is one.
fn planner_phase(retry: &Retry) -> Option<u32> {
    (retry.role == Role::Planner)
        .then(|| subject_phase(&retry.subject))
        .flatten()
}

/// The phase whose checker order `retry` re-issues, if it is one.
fn checker_phase(retry: &Retry) -> Option<u32> {
    (retry.role == Role::Checker)
        .then(|| subject_phase(&retry.subject))
        .flatten()
}

/// The digest and latest submission's sequence a `draft` document holds.
fn draft_at(document: &Document) -> Option<(Hash, u64)> {
    let body = &document.body;
    let digest = body.get("digest").and_then(Value::as_str)?;
    Some((Hash::from_hex(digest)?, body.get("seq")?.as_u64()?))
}

/// The refusal for a retry of checker order `id`, whose draft a later
/// submission took over, `newest` being that submission's digest, which can be
/// the same one.
fn stale_checker_retry(id: &str, newest: Hash) -> Value {
    let newest = newest.to_hex();
    Refusal::new(
        STALE_DRAFT,
        format!(
            "plan draft {newest} was submitted after the draft checker work order {id} checks, so its check is not run again and no retry was issued"
        ),
    )
    .slot("work_order")
    .details(json!({"newest": newest}))
    .value()
}

/// Order `id`'s document as a decision observes it.
fn observed(id: &str, document: &Document) -> ObservedDocument {
    ObservedDocument {
        view: WORK_ORDER_VIEW.into(),
        key: work_order_key(id),
        produced_seq: Some(document.produced_seq),
    }
}

/// The order an order document retries, if any.
fn parent_of(body: &Value) -> Result<Option<String>, Value> {
    match body.get("parent") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(parent)) => Ok(Some(parent.clone())),
        _ => Err(order_unreadable()),
    }
}

/// The answer for the retry judge's refusal of order `id`.
fn retry_refused(id: &str, refusal: RetryRefusal) -> Value {
    match refusal {
        RetryRefusal::NotFailed { state } => Refusal::new(
            WORK_ORDER_NOT_FAILED,
            format!(
                "work order {id} is {state}, so no retry was issued. Only a failed order is retried: a pending one is failed first with `work-order-fail`, and an answered one has returned"
            ),
        )
        .slot("work_order")
        .details(json!({"state": state}))
        .value(),
        RetryRefusal::EarlierAnswered { work_order } => Refusal::new(
            WORK_ORDER_NOT_FAILED,
            format!(
                "work order {work_order}, an earlier attempt of {id}, has returned, which ends the chain, so no retry was issued"
            ),
        )
        .slot("work_order")
        .details(json!({"state": ANSWERED, "work_order": work_order}))
        .value(),
        RetryRefusal::Retried { child } => Refusal::new(
            WORK_ORDER_RETRIED,
            format!(
                "work order {id} was already retried as {child}, so no second retry was issued. Fail or retry {child}, the newest attempt, instead"
            ),
        )
        .slot("work_order")
        .details(json!({"work_order": child}))
        .value(),
        RetryRefusal::MaterialUnavailable { input } => Refusal::new(
            MATERIAL_UNAVAILABLE,
            format!(
                "this project purged the {input} input of work order {id}, so no retry was issued. A retry carries its parent's inputs and never rebuilds them"
            ),
        )
        .slot("work_order")
        .details(json!({"input": input}))
        .value(),
        RetryRefusal::Exhausted => failed(
            LEDGER_UNAVAILABLE,
            format!("work order {id} is at the last attempt a chain can count, so no retry was issued"),
            PLACE_LEDGER,
        ),
        RetryRefusal::Unreadable { .. } => order_unreadable(),
    }
}

/// Issues the retry `checked` describes as order `id` under `command`: the
/// route for the parent's role at the judged attempt under `policy`, the
/// host's effective policy at the command's policy version, and the catalog
/// `names` read now (D-15); the registry's current evidence for each
/// instruction the parent listed; and the parent's inputs as they are held.
/// The decision observes every document the check read and expects the
/// `retention` stream where the check read it, so a retry, a submission or a
/// purge landing since makes the call `ledger-busy` with nothing recorded. A
/// retried start order also declares a start order's two absences, no
/// pending start order and no start approval, so a start approved since the
/// check makes it `ledger-busy` too, a retried analyzer order declares no
/// pending analyzer order of its story, and a retried planner or checker order
/// no pending planner or checker order of its phase, so one issued since does
/// the same. A retried checker order also confirms inside its transaction
/// that the newest plan draft is still the one the check read, since a draft
/// submitted since moves no document the check read once its own checker order
/// has failed.
pub fn issue_retry(
    store: &SqliteStore,
    command: &Command,
    checked: &CheckedRetry,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Value {
    let retry = &checked.retry;
    let route = match compose(
        policy,
        command.policy_version,
        host,
        retry.role,
        retry.attempt,
        names,
    ) {
        Ok(route) => route,
        Err(refusal) => return Refusal::new(refusal.code(), refusal.to_string()).value(),
    };
    // The worker reads the text served now, so the order carries its evidence.
    let mut instructions = Vec::with_capacity(checked.instructions.len());
    for identity in &checked.instructions {
        match crate::instruction::evidence(identity) {
            Ok(evidence) => instructions.push(evidence),
            Err(refusal) => return refusal,
        }
    }
    let inputs: Vec<Input<'_>> = retry.inputs.iter().map(Input::Held).collect();
    let absences: Vec<Absence> = if is_start(retry) {
        start_absences()
    } else if let Some(story) = analyzer_story(retry) {
        analyzer_absences(story)
    } else if let Some(phase) = planner_phase(retry).or_else(|| checker_phase(retry)) {
        let mut absences = planner_absences(phase);
        absences.extend(checker_absences(phase));
        absences
    } else {
        Vec::new()
    };
    // Every purge of this project moves this stream, and a transaction
    // cannot re-read the releases themselves.
    let streams = [(StreamName(RETENTION_STREAM.into()), checked.retention)];
    let mut further = Map::new();
    further.insert("parent".into(), json!(retry.parent));
    let issue = Issue {
        id,
        request: &retry.request,
        parent: Some(&retry.parent),
        subject: &retry.subject,
        route: &route,
        instructions: &instructions,
        inputs: &inputs,
        absences: &absences,
        streams: &streams,
        documents: &checked.documents,
        further: &further,
    };
    let recorded = match (checker_phase(retry), checked.newest_draft) {
        (Some(phase), Some(newest)) => record_checker_retry(store, command, &issue, phase, newest),
        _ => issue::record(store, command, Some(&issue)),
    };
    issue::answer(recorded, store)
}

/// Records `issue`, a retried checker order of phase `phase`, as
/// `issue::record` records an order, once the phase's newest plan draft is
/// confirmed to be `newest`.
fn record_checker_retry(
    store: &SqliteStore,
    command: &Command,
    issue: &Issue<'_>,
    phase: u32,
    newest: (Hash, u64),
) -> Result<Recorded, StoreError> {
    store.transact(command, &mut |tx| {
        confirm_newest_draft(tx, phase, newest)?;
        let mut receipt = Map::new();
        receipt.insert("status".into(), json!("ok"));
        receipt.extend(issue::issue_within(tx, issue, &command.recorded_at)?);
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: Value::Object(receipt),
            sensitive: false,
            observed: Observed {
                absences: issue.absences.to_vec(),
                documents: issue.documents.to_vec(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// The newest plan draft of phase `phase` is `newest`, by digest and
/// sequence, never by time: two submissions can share a time.
fn confirm_newest_draft(
    tx: &mut dyn Transaction,
    phase: u32,
    newest: (Hash, u64),
) -> Result<(), StoreError> {
    let now = tx
        .find(DRAFT_VIEW, &newest_query(PLAN_KIND, &phase_stream(phase)))?
        .items
        .first()
        .and_then(draft_at);
    if now != Some(newest) {
        return Err(StoreError::Stale(StaleInput::Document {
            view: DRAFT_VIEW.into(),
            key: draft_key(newest.0),
            seen: Some(newest.1),
            now: now.map(|(_, seq)| seq),
        }));
    }
    Ok(())
}

/// Serves one `work-order-retry` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the caller
/// with the instruction evidence attached and [`retry_check`] as its check,
/// and the next attempt is issued under a fresh lowercase UUID with the
/// prepared policy and the host catalog read now. A replay goes to the
/// recording with no order, and the store answers it from its record.
pub fn serve_retry(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_retry_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the issue to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(retry_check(store, project, &judged.work_order)?);
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
        return issue::answer(issue::record(store, &command, None), store);
    };
    let names = match host_names(store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
    };
    let id = uuid::Uuid::new_v4().to_string();
    issue_retry(
        store,
        &command,
        checked,
        &stepped.policy,
        preparation.host,
        &names,
        &id,
    )
}

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments the `route` query takes beside its `operation`: exactly one
/// of `role` and `work_order`. Unknown fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteShape {
    /// The role whose first attempt's route to explain, such as `planner`.
    /// Send this or `work_order`, not both.
    pub role: Option<String>,
    /// The id of a work order, as its receipt gave it, whose route to explain
    /// at the attempt the order records. Send this or `role`, not both.
    pub work_order: Option<String>,
}

/// The tagged form the `route` query's arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum RouteCall {
    #[serde(rename = "route")]
    Route(RouteShape),
}

/// What a `route` query asks to explain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteQuery {
    /// This role at attempt 1.
    Role(Role),
    /// The role and recorded attempt of the order with this id.
    WorkOrder(String),
}

/// Judges the `route` query's arguments: the shape, then exactly one of
/// `role` and `work_order`, then that one's form. A fault is the `refused`
/// answer.
pub fn judge_route_arguments(arguments: &Value) -> Result<RouteQuery, Value> {
    let shape = match serde_json::from_value::<RouteCall>(arguments.clone()) {
        Ok(RouteCall::Route(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    match (shape.role, shape.work_order) {
        (Some(role), None) => Role::parse(&role).map(RouteQuery::Role).ok_or_else(|| {
            Refusal::new(
                "invalid-arguments",
                "role must be one of planner, analyzer, checker, executor, verifier and reviewer",
            )
            .slot("role")
            .value()
        }),
        (None, Some(id)) => {
            judge_order_id(&id)?;
            Ok(RouteQuery::WorkOrder(id))
        }
        _ => Err(Refusal::new(
            "invalid-arguments",
            "route takes exactly one of role and work_order",
        )
        .slot("arguments")
        .value()),
    }
}

/// The note on a route explained at no recorded policy version.
const ROUTE_UNRECORDED: &str = "No policy recorded for this checkout matches the settings now, so this route is explained at policy version 0 and the next write records a new version";

/// A busy ledger's reason when a route was not explained.
const ROUTE_NOT_READ_BUSY: &str = "the ledger is busy, so the route was not read. Try it again";

/// Explains the route `query` names in `project` under `policy`, the host's
/// effective policy for the checkout at `root`, with the catalog `names`,
/// and records nothing (D-11). A role is explained at attempt 1 and an order
/// at the attempt it records, an unknown order `no-such-work-order`. The
/// policy version is the one this checkout and host's `policy` record holds
/// when the settings still match it at the catalog's version, and none
/// otherwise, with a note; the route is composed at that version, 0 when
/// none.
pub fn route_answer<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    root: &Path,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    query: &RouteQuery,
) -> Value {
    let (role, attempt) = match query {
        RouteQuery::Role(role) => (*role, NonZeroU32::MIN),
        RouteQuery::WorkOrder(id) => match order_document(store, project, id) {
            Ok(Some(document)) => match recorded_attempt(&document.body) {
                Some(found) => found,
                None => return order_unreadable(),
            },
            Ok(None) => return no_such_order(),
            Err(error) => return store_failed(&error, ROUTE_NOT_READ_BUSY),
        },
    };
    let recorded = match recorded_policy(root, policy) {
        Ok(recorded) => recorded,
        Err(error) => return failed(CONFIG_UNAVAILABLE, error.to_string(), "settings"),
    };
    let payload = effective_payload(&recorded, &project.0, names.version);
    let key = policy_key(&recorded.checkout, recorded.host);
    let version = match store.get(project, POLICY_VIEW, &key) {
        Ok(stored) => match judge_policy(&payload, stored.as_ref().map(|found| &found.body)) {
            PolicyJudgement::Unchanged { version } => Some(version),
            PolicyJudgement::Record => None,
        },
        Err(error) => return store_failed(&error, ROUTE_NOT_READ_BUSY),
    };
    let route = match compose(policy, version.unwrap_or(0), host, role, attempt, names) {
        Ok(route) => route,
        Err(refusal) => return Refusal::new(refusal.code(), refusal.to_string()).value(),
    };
    let mut answer = json!({"status": "ok", "route": route.record, "agent": route.agent,
        "policy_version": version});
    if version.is_none() {
        answer["note"] = json!(ROUTE_UNRECORDED);
    }
    answer
}

/// The role and attempt an order document records.
fn recorded_attempt(body: &Value) -> Option<(Role, NonZeroU32)> {
    let role = Role::parse(body.get("role")?.as_str()?)?;
    let attempt = u32::try_from(body.get("attempt")?.as_u64()?).ok()?;
    Some((role, NonZeroU32::new(attempt)?))
}

/// Serves one `route` query for the session's `preparation`. The arguments
/// are judged first; then the read is prepared with the settings read and
/// judged and nothing recorded, the host catalog is read and the route
/// explained.
pub fn serve_route(arguments: &Value, preparation: &Preparation) -> Value {
    let query = match judge_route_arguments(arguments) {
        Ok(query) => query,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let prepared = match prepare_settings_read(
        ledger.map(|ledger| &ledger.store),
        &preparation.caller,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(policy)) = (ledger, prepared.policy.as_ref()) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the read was prepared without a ledger or a policy",
            PLACE_LEDGER,
        );
    };
    let names = match host_names(&ledger.store, preparation.host) {
        Ok(names) => names,
        Err(error) => return store_failed(&error, ROUTE_NOT_READ_BUSY),
    };
    route_answer(
        &ledger.store,
        &prepared.project,
        &prepared.root,
        policy,
        preparation.host,
        &names,
        &query,
    )
}

/// The `invalid-arguments` answer for a shape fault, at slot `arguments`.
pub(crate) fn shape_fault(error: &serde_json::Error) -> Value {
    Refusal::new("invalid-arguments", bounded(error.to_string()))
        .slot("arguments")
        .value()
}

/// Whether `text` is a UUID in its one lowercase hyphenated spelling.
fn lowercase_uuid(text: &str) -> bool {
    uuid::Uuid::parse_str(text).is_ok_and(|parsed| parsed.hyphenated().to_string() == text)
}

/// Refuses a request id that is not a lowercase UUID, so a retry of the same
/// `call` cannot miss its replay by changing case or wrapping.
pub(crate) fn judge_request_id(request_id: &str, call: &str) -> Result<(), Value> {
    if lowercase_uuid(request_id) {
        return Ok(());
    }
    Err(Refusal::new(
        "invalid-arguments",
        format!(
            "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same {call} sends the same id"
        ),
    )
    .slot("request_id")
    .value())
}

/// Refuses an order id that is not a lowercase UUID, the one spelling an
/// order's receipt gives, so no read can miss the order it names.
pub(crate) fn judge_order_id(id: &str) -> Result<(), Value> {
    if lowercase_uuid(id) {
        return Ok(());
    }
    Err(Refusal::new(
        "invalid-arguments",
        "work_order must be the order's id as its receipt gives it, a UUID in lowercase hyphenated form",
    )
    .slot("work_order")
    .value())
}

/// The registry's evidence for the instruction a session named, if any.
pub(crate) fn judge_instruction(
    identity: Option<&str>,
) -> Result<Option<InstructionEvidence>, Value> {
    match identity.map(crate::instruction::evidence) {
        None => Ok(None),
        Some(Ok(evidence)) => Ok(Some(evidence)),
        // The registry's refusal names its read's `identity` argument; here
        // the session sent the identity as `instruction`.
        Some(Err(mut refusal)) => {
            refusal["slot"] = json!("instruction");
            Err(refusal)
        }
    }
}

/// Serde's text for a shape fault, cut to the echo limit with a closing
/// `...`, since it repeats an unknown field's name at whatever length the
/// caller sent.
fn bounded(text: String) -> String {
    const CUT: &str = "...";
    if text.len() <= MAX_ECHOED_BYTES {
        return text;
    }
    let mut end = MAX_ECHOED_BYTES - CUT.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT}", &text[..end])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    pub(crate) const B1: &str = "Build a ledger for agent work";
    const B2: &str = "Build a ledger for agent work.";

    fn start(request_id: &str, brief: &str) -> Value {
        json!({"operation": "project-start", "request_id": request_id, "brief": brief})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_start_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_brief_judged_blank_or_oversized_by_the_wrong_measure_is_caught() {
        let refusal = refused(&start(R1, " \n\t"));
        assert_eq!(refusal["code"], "brief-blank");
        assert_eq!(refusal["slot"], "brief");

        for brief in ["x".repeat(65_536), "é".repeat(32_768)] {
            let judged = judge_start_arguments(&start(R1, &brief)).expect("accepted");
            assert_eq!(judged.brief, brief);
        }

        let refusal = refused(&start(R1, &"é".repeat(32_769)));
        assert_eq!(refusal["code"], "brief-too-large");
        assert_eq!(refusal["slot"], "brief");
        let reason = refusal["reason"].as_str().unwrap();
        assert!(reason.contains("65536"), "{reason}");
        assert!(reason.contains("65538"), "{reason}");
    }

    #[test]
    fn a_start_request_digest_blind_to_the_brief_is_caught() {
        let request = |brief| judge_start_arguments(&start(R1, brief)).unwrap().request;
        let first = request(B1);
        let second = request(B2);
        assert_eq!(
            first.digest.to_hex(),
            "3d83fcc2baedc54269e332c5201f37226efbff6ce62eea636385255a4df2705f"
        );
        assert_eq!(
            second.digest.to_hex(),
            "2631f59ab1fa07a995b568fa83dd7fadc433faa40473627dc35dc1c9db55c16b"
        );
        for request in [&first, &second] {
            assert_eq!(request.kind, CommandKind("project.start".into()));
            assert_eq!(request.request_id, RequestId(R1.into()));
        }

        let mut path = start(R1, B1);
        path["path"] = json!("/real/r/BRIEF.md");
        let refusal = refused(&path);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");

        let refusal = refused(&start(&R1.to_uppercase(), B1));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");
    }

    fn policy(global: Option<&str>, project: &str) -> EffectivePolicy {
        use std::path::Path;

        use baley_core::policy::{Host, Schema, effective_policy};
        let global =
            global.map(|text| crate::settings::file(Path::new("/c/config.toml"), text.into()));
        let text = format!("[project]\nid = \"{ID}\"\nname = \"r\"\n{project}");
        let project = crate::settings::file(Path::new("/real/r/baley.toml"), text.into_bytes());
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            global.as_ref(),
            Some(&project),
        )
        .unwrap()
    }

    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const EFFORT_MAX: &str = "[roles.planner]\neffort = \"max\"\n";

    fn about(lines: &[String]) -> Vec<&'static str> {
        lines
            .iter()
            .map(|line| {
                assert!(line.contains("baley config interview"), "{line}");
                if line.starts_with("There is no global settings file") {
                    "global"
                } else if line.starts_with("The project file") {
                    "project"
                } else {
                    panic!("a line about neither file: {line}")
                }
            })
            .collect()
    }

    #[test]
    fn a_settings_line_shown_beside_settings_or_missing_without_them_is_caught() {
        let cases = [
            (None, "", vec!["global", "project"]),
            (Some(EFFORT_MAX), "", vec!["project"]),
            (None, "[roles.planner]\neffort = \"low\"\n", vec!["global"]),
            (
                None,
                "[host.claude-code]\nescalate_on_failure = true\n",
                vec!["global"],
            ),
            (
                Some(EFFORT_MAX),
                "[roles.planner]\neffort = \"low\"\n",
                vec![],
            ),
        ];
        for (global, project, expected) in cases {
            let lines = settings_lines(&policy(global, project));
            assert_eq!(about(&lines), expected, "{global:?} {project:?}: {lines:?}");
        }
    }

    #[test]
    fn a_settings_line_claiming_only_the_defaults_while_the_other_file_holds_settings_is_caught() {
        // Each missing file's line names the file still in force beside the defaults.
        let cases = [
            (Some(EFFORT_MAX), "", "the global settings file's settings"),
            (
                None,
                "[roles.planner]\neffort = \"low\"\n",
                "the project file's settings",
            ),
        ];
        for (global, project, other) in cases {
            let lines = settings_lines(&policy(global, project));
            let [line] = lines.as_slice() else {
                panic!("one line for the one missing file: {lines:?}");
            };
            assert!(line.contains(other), "{line}");
            assert!(line.contains("Baley's defaults apply"), "{line}");
            assert!(line.contains("`baley config interview`"), "{line}");
        }
    }

    /// Records a pending start planner order `id` in `project` under
    /// `request_id` at `at`, as project start records one for a checkout with
    /// no code, with brief B1.
    pub(crate) fn issue_start(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        request_id: &str,
        at: &str,
    ) {
        issue_start_with(store, project, id, request_id, at, B1);
    }

    /// [`issue_start`] with `brief` as the order's brief.
    pub(crate) fn issue_start_with(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        request_id: &str,
        at: &str,
        brief: &str,
    ) {
        use std::collections::BTreeSet;

        use baley_core::policy::{AcceptedNames, Schema, effective_policy};

        use crate::scope::fixtures::command_in;

        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let command = command_in(project, START_COMMAND, request_id, 1, at);
        let answer = super::issue_start_order(
            store,
            &command,
            &policy,
            Host::ClaudeCode,
            &names,
            false,
            brief,
            id,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    pub(crate) const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    #[test]
    fn a_start_check_passing_an_approved_start_or_a_pending_order_is_caught() {
        use baley_core::work_order::{WORK_ORDER_FAILED, failed_payload};
        use baley_store::{NewEvent, StreamName};

        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        assert_eq!(start_check(&store, &project()), Ok(()));

        issue_start(&store, &project(), I0, R1, T1);
        let refusal = start_check(&store, &project()).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "start-order-pending", "{refusal}");
        assert_eq!(refusal["slot"], "operation", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": I0}), "{refusal}");

        let digest = receipt(&submit_in(&store, &project_b(), &m(), R1, 1, T1).unwrap())["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        approve_draft_in(&store, &project_b(), digest);
        let refusal = start_check(&store, &project_b()).unwrap_err();
        assert_eq!(refusal["code"], "project-already-started", "{refusal}");

        append(
            &store,
            R2,
            T4,
            vec![NewEvent {
                stream: StreamName("roadmap".into()),
                type_name: WORK_ORDER_FAILED.into(),
                type_version: 1,
                git: None,
                payload: failed_payload(I0, "the planner stopped"),
                attachments: vec![],
            }],
        );
        assert_eq!(start_check(&store, &project()), Ok(()));
    }

    pub(crate) const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    pub(crate) const I2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";

    /// Records the failure of `id` in `project` through the failure's check
    /// and recording, under `request_id` at `at`.
    pub(crate) fn fail_order(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        request_id: &str,
        at: &str,
    ) {
        let kept = fail_check(store, project, id).unwrap();
        let command = crate::scope::fixtures::command_in(project, FAIL_COMMAND, request_id, 4, at);
        let answer = issue::answer(
            fail::record(store, &command, "the worker stopped", Some(&kept)),
            store,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    /// Records `value` as a start submission in `project` answering order
    /// `id`, under `request_id` at `at`.
    pub(crate) fn answer_order(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        value: &Value,
        request_id: &str,
        at: &str,
    ) {
        use baley_core::scope::judge_submission;
        use baley_store::Caller;

        use crate::scope::fixtures::{caller, command_in};
        let mut command = command_in(
            project,
            crate::scope::submit::SUBMIT_COMMAND,
            request_id,
            2,
            at,
        );
        command.caller = Some(Caller::Server(caller().with_work_order(id).unwrap()));
        crate::scope::submit::record(store, &command, &judge_submission(value).unwrap()).unwrap();
    }

    #[test]
    fn a_failure_recorded_for_an_unknown_or_settled_order_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        issue_start(&store, &project(), I2, R3, T3);
        answer_order(&store, &project(), I2, &m(), R4, T4);

        for (id, code, details) in [
            (I1, "no-such-work-order", Value::Null),
            (I0, "work-order-not-pending", json!({"state": "failed"})),
            (I2, "work-order-not-pending", json!({"state": "answered"})),
        ] {
            let before = head(&store);
            let refusal = fail_check(&store, &project(), id).unwrap_err();
            assert_eq!(refusal["status"], "refused", "{refusal}");
            assert_eq!(refusal["code"], code, "{refusal}");
            assert_eq!(refusal["slot"], "work_order", "{refusal}");
            if !details.is_null() {
                assert_eq!(refusal["details"], details, "{refusal}");
            }
            assert_eq!(head(&store), before);
        }
    }

    fn fail(work_order: &str, reason: Option<&str>) -> Value {
        let mut arguments = json!({"operation": "work-order-fail", "request_id": R1,
            "work_order": work_order});
        if let Some(reason) = reason {
            arguments["reason"] = json!(reason);
        }
        arguments
    }

    #[test]
    fn a_failure_reason_blank_oversized_or_an_order_id_in_another_form_accepted_is_caught() {
        let over = "x".repeat(4_097);
        let at = "x".repeat(4_096);
        for (arguments, slot) in [
            (fail(I0, Some("  ")), "reason"),
            (fail(I0, Some(&over)), "reason"),
            (
                fail(&I0.to_uppercase(), Some("the worker stopped")),
                "work_order",
            ),
            (fail(I0, None), "arguments"),
        ] {
            let refusal = judge_fail_arguments(&arguments).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{arguments}");
            assert_eq!(refusal["code"], "invalid-arguments", "{refusal}");
            assert_eq!(refusal["slot"], slot, "{refusal}");
        }
        let judged = judge_fail_arguments(&fail(I0, Some(&at))).expect("accepted");
        assert_eq!(judged.reason, at);
        assert_eq!(judged.work_order, I0);
    }

    const I3: &str = "8a4f5e6d-7182-4d9e-9fa0-2b3c4d5e6f70";

    /// Escalation on, everything else at the defaults.
    const ESCALATE: &str = "escalate_on_failure = true\n";

    fn no_names() -> AcceptedNames {
        AcceptedNames {
            names: std::collections::BTreeSet::new(),
            version: 0,
        }
    }

    fn head_in(store: &SqliteStore, project: &ProjectId) -> u64 {
        store.head(project).unwrap().map_or(0, |head| head.seq)
    }

    fn held(store: &SqliteStore, project: &ProjectId, id: &str) -> Option<Value> {
        crate::work_order::read::order(store, project, id).unwrap()
    }

    fn b1_hash() -> Hash {
        Hash(Sha256::digest(B1.as_bytes()).into())
    }

    /// Retries `parent` in `project` as order `id` through the retry's check
    /// and issue, under `request_id` at `at`, with the global file `global`.
    fn retry_in(
        store: &SqliteStore,
        project: &ProjectId,
        parent: &str,
        id: &str,
        request_id: &str,
        at: &str,
        global: &str,
    ) -> Value {
        let checked = retry_check(store, project, parent).expect("the retry's check passes");
        let command = crate::scope::fixtures::command_in(project, RETRY_COMMAND, request_id, 5, at);
        issue_retry(
            store,
            &command,
            &checked,
            &policy(Some(global), ""),
            Host::ClaudeCode,
            &no_names(),
            id,
        )
    }

    fn assert_refused(refusal: &Value, code: &str, slot: &str, details: &Value) {
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], slot, "{refusal}");
        assert_eq!(&refusal["details"], details, "{refusal}");
    }

    #[test]
    fn a_retry_stuck_at_its_attempt_escalated_from_pending_or_counted_twice_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        let checked = retry_check(&store, &project(), I0).expect("I0 failed");
        let command = command_in(&project(), RETRY_COMMAND, R3, 5, T3);
        let escalating = policy(Some(ESCALATE), "");
        let receipt = issue_retry(
            &store,
            &command,
            &checked,
            &escalating,
            Host::ClaudeCode,
            &no_names(),
            I2,
        );
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["work_order"], I2, "{receipt}");
        assert_eq!(receipt["request"], I0, "{receipt}");
        assert_eq!(receipt["parent"], I0, "{receipt}");
        assert_eq!(receipt["attempt"], 2, "{receipt}");
        assert_eq!(receipt["agent"], "bal-planner-xhigh", "{receipt}");
        let parent = held(&store, &project(), I0).unwrap();
        let child = held(&store, &project(), I2).expect("I2 is held");
        assert_eq!(child["request"], I0);
        assert_eq!(child["parent"], I0);
        assert_eq!(child["attempt"], 2);
        assert_eq!(child["route"]["rung"], "xhigh");
        assert_eq!(child["route"]["escalated"], true);
        assert_eq!(child["inputs"], parent["inputs"]);

        let before = head(&store);
        let refusal = retry_check(&store, &project(), I2).unwrap_err();
        assert_refused(
            &refusal,
            "work-order-not-failed",
            "work_order",
            &json!({"state": "pending"}),
        );
        assert_eq!(head(&store), before);

        // The same retry again, offered another fresh id, is the recorded one.
        let again = issue_retry(
            &store,
            &command,
            &checked,
            &escalating,
            Host::ClaudeCode,
            &no_names(),
            I3,
        );
        assert_eq!(again, receipt);
        let events = history(&store);
        let children: Vec<_> = of_type(&events, "work_order.issued")
            .into_iter()
            .filter(|event| event.payload["parent"] == I0)
            .collect();
        assert_eq!(children.len(), 1);
        assert_eq!(held(&store, &project(), I3), None);

        answer_order(&store, &project(), I2, &m(), R6, T4);
        issue_start(&store, &project(), I1, R5, T5);
        let fresh = held(&store, &project(), I1).expect("I1 is held");
        assert_eq!(fresh["attempt"], 1);
        assert_eq!(fresh["request"], I1);
        assert_eq!(fresh["parent"], Value::Null);
    }

    #[test]
    fn a_retry_resolved_under_the_parents_policy_or_over_a_purged_input_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        assert_eq!(
            held(&store, &project(), I0).unwrap()["route"]["rung"],
            "high"
        );
        fail_order(&store, &project(), I0, R2, T2);
        let receipt = retry_in(
            &store,
            &project(),
            I0,
            I2,
            R3,
            T3,
            "escalate_on_failure = false\n[roles.planner]\neffort = \"low\"\n",
        );
        assert_eq!(receipt["status"], "ok", "{receipt}");
        let route = &held(&store, &project(), I2).expect("I2 is held")["route"];
        assert_eq!(route["starting_rung"], "low", "{route}");
        assert_eq!(route["rung"], "low", "{route}");
        assert_eq!(route["escalated"], false, "{route}");

        issue_start(&store, &project_b(), I1, R1, T1);
        fail_order(&store, &project_b(), I1, R2, T2);
        purge_in(&store, &project_b(), b1_hash(), "owner purge");
        let before = head_in(&store, &project_b());
        let refusal = retry_check(&store, &project_b(), I1).unwrap_err();
        assert_refused(
            &refusal,
            "material-unavailable",
            "work_order",
            &json!({"input": "brief"}),
        );
        assert_eq!(head_in(&store, &project_b()), before);
    }

    #[test]
    fn a_retry_of_an_order_already_retried_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        let first = retry_in(&store, &project(), I0, I2, R3, T3, ESCALATE);
        assert_eq!(first["status"], "ok", "{first}");
        fail_order(&store, &project(), I2, R4, T4);
        let before = head(&store);
        let refusal = retry_check(&store, &project(), I0).unwrap_err();
        assert_refused(
            &refusal,
            "work-order-retried",
            "work_order",
            &json!({"work_order": I2}),
        );
        assert_eq!(head(&store), before);

        issue_start(&store, &project_b(), I1, R1, T1);
        fail_order(&store, &project_b(), I1, R2, T2);
        let first = retry_in(&store, &project_b(), I1, I3, R3, T3, ESCALATE);
        assert_eq!(first["status"], "ok", "{first}");
        answer_order(&store, &project_b(), I3, &m(), R4, T4);
        let before = head_in(&store, &project_b());
        let refusal = retry_check(&store, &project_b(), I1).unwrap_err();
        assert_refused(
            &refusal,
            "work-order-retried",
            "work_order",
            &json!({"work_order": I3}),
        );
        assert_eq!(head_in(&store, &project_b()), before);
    }

    #[test]
    fn a_retry_beside_a_pending_start_order_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project_b(), I1, R1, T1);
        fail_order(&store, &project_b(), I1, R2, T2);
        issue_start(&store, &project_b(), I2, R5, T3);
        let before = head_in(&store, &project_b());
        let refusal = retry_check(&store, &project_b(), I1).unwrap_err();
        assert_refused(
            &refusal,
            "start-order-pending",
            "operation",
            &json!({"work_order": I2}),
        );
        assert_eq!(head_in(&store, &project_b()), before);
    }

    #[test]
    fn a_start_order_retried_after_the_start_was_approved_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        approve_a_start(&store, T3);
        let before = head(&store);
        let refusal = retry_check(&store, &project(), I0).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "project-already-started", "{refusal}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_start_order_retry_recorded_over_an_approval_since_its_check_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        let checked = retry_check(&store, &project(), I0).expect("I0 failed");
        approve_a_start(&store, T3);
        let command = command_in(&project(), RETRY_COMMAND, R3, 5, T4);
        let answer = issue_retry(
            &store,
            &command,
            &checked,
            &policy(Some(ESCALATE), ""),
            Host::ClaudeCode,
            &no_names(),
            I2,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(of_type(&history(&store), "work_order.issued").len(), 1);
        assert_eq!(held(&store, &project(), I2), None);
    }

    #[test]
    fn a_retry_recorded_over_a_purge_since_its_check_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        issue_start(&store, &project_b(), I1, R1, T1);
        let checked = retry_check(&store, &project(), I0).expect("I0 failed");
        purge_in(&store, &project(), b1_hash(), "owner purge");
        let command = command_in(&project(), RETRY_COMMAND, R3, 5, T3);
        let answer = issue_retry(
            &store,
            &command,
            &checked,
            &policy(Some(ESCALATE), ""),
            Host::ClaudeCode,
            &no_names(),
            I2,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(of_type(&history(&store), "work_order.issued").len(), 1);
        assert_eq!(held(&store, &project(), I2), None);
        assert_eq!(held(&store, &project(), I0).unwrap()["child"], Value::Null);

        let refusal = retry_check(&store, &project(), I0).unwrap_err();
        assert_refused(
            &refusal,
            "material-unavailable",
            "work_order",
            &json!({"input": "brief"}),
        );
    }

    #[test]
    fn a_retry_recorded_over_a_submission_answering_its_order_or_an_earlier_attempt_since_its_check_is_caught()
     {
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        issue_start(&store, &project(), I0, R1, T1);
        fail_order(&store, &project(), I0, R2, T2);
        let checked = retry_check(&store, &project(), I0).expect("I0 failed");
        answer_order(&store, &project(), I0, &m(), R4, T3);
        let before = head(&store);
        let command = command_in(&project(), RETRY_COMMAND, R3, 5, T4);
        let answer = issue_retry(
            &store,
            &command,
            &checked,
            &policy(Some(ESCALATE), ""),
            Host::ClaudeCode,
            &no_names(),
            I2,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
        assert_eq!(held(&store, &project(), I2), None);

        // The submission answers the chain's first order, not the one retried.
        issue_start(&store, &project_b(), I1, R1, T1);
        fail_order(&store, &project_b(), I1, R2, T2);
        let first = retry_in(&store, &project_b(), I1, I2, R3, T3, ESCALATE);
        assert_eq!(first["status"], "ok", "{first}");
        fail_order(&store, &project_b(), I2, R4, T4);
        let checked = retry_check(&store, &project_b(), I2).expect("I2 failed");
        answer_order(&store, &project_b(), I1, &m(), R5, T5);
        let before = head_in(&store, &project_b());
        let command = command_in(&project_b(), RETRY_COMMAND, R6, 5, T6);
        let answer = issue_retry(
            &store,
            &command,
            &checked,
            &policy(Some(ESCALATE), ""),
            Host::ClaudeCode,
            &no_names(),
            I3,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head_in(&store, &project_b()), before);
        assert_eq!(held(&store, &project_b(), I3), None);
    }

    /// The `route` query judged from `arguments` beside its operation.
    fn route_query(arguments: Value) -> Result<RouteQuery, Value> {
        let mut call = json!({"operation": "route"});
        call.as_object_mut()
            .unwrap()
            .extend(arguments.as_object().unwrap().clone());
        judge_route_arguments(&call)
    }

    #[test]
    fn a_work_order_read_whole_past_the_bound_or_a_route_query_that_records_or_advances_is_caught()
    {
        use baley_core::policy::{Schema, effective_policy};

        use crate::mcp::document::{judge_arguments, read};
        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        let brief = "b".repeat(30_000);
        issue_start_with(&store, &project(), I0, R1, T1, &brief);
        let before = head(&store);

        let part = |n: u64| {
            let shape = judge_arguments(&json!({"operation": "document",
                "identity": {"kind": "work_order", "id": I0}, "part": n}))
            .expect("a work-order read is judged");
            read(&store, &project(), &shape)
        };
        let first = part(1);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["bound"], 24_576, "{first}");
        assert_eq!(first["part"], 1, "{first}");
        assert_eq!(first["next"], 2, "{first}");
        let mut joined = String::new();
        let mut at = first;
        loop {
            let body = at["body"].as_str().expect("each part has a body");
            assert!(
                body.len() <= 24_576,
                "{} bytes in {}",
                body.len(),
                at["part"]
            );
            joined.push_str(body);
            match at["next"].as_u64() {
                Some(next) => {
                    assert_eq!(Some(next), at["part"].as_u64().map(|n| n + 1));
                    at = part(next);
                }
                None => {
                    assert_eq!(at["next"], Value::Null, "{at}");
                    break;
                }
            }
        }
        let whole: Value = serde_json::from_str(&joined).expect("the parts join to JSON");
        assert_eq!(whole["status"], "ok");
        assert_eq!(whole["order"]["id"], I0);
        assert_eq!(whole["inputs"][0]["name"], "brief");
        assert_eq!(whole["inputs"][0]["text"], brief);

        let issued_under =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let query = route_query(json!({"work_order": I0})).expect("an order id is judged");
        let answer = route_answer(
            &store,
            &project(),
            Path::new("/w/r"),
            &issued_under,
            Host::ClaudeCode,
            &no_names(),
            &query,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["route"]["role"], "planner", "{answer}");
        assert_eq!(answer["route"]["attempt"], 1, "{answer}");
        assert_eq!(answer["route"]["rung"], "high", "{answer}");

        assert_eq!(head(&store), before);
        assert_eq!(of_type(&history(&store), "work_order.issued").len(), 1);
        assert_eq!(held(&store, &project(), I0).unwrap()["attempt"], 1);
    }

    #[test]
    fn a_route_query_version_taken_from_a_record_the_settings_no_longer_match_is_caught() {
        use baley_core::policy::recorded::recorded_policy;

        use crate::scope::fixtures::*;

        let (_dir, store) = two_projects();
        let root = Path::new("/w/r");
        let recorded_with = policy(Some(EFFORT_MAX), "");
        let recorded = recorded_policy(root, &recorded_with).unwrap();
        let version = crate::policy_step::record(
            &store,
            &project(),
            &recorded,
            0,
            RequestId(R1.into()),
            T1,
            None,
        )
        .unwrap();
        let before = head(&store);
        let query = route_query(json!({"role": "planner"})).expect("a role is judged");
        let explain = |policy: &EffectivePolicy| {
            route_answer(
                &store,
                &project(),
                root,
                policy,
                Host::ClaudeCode,
                &no_names(),
                &query,
            )
        };

        let matching = explain(&recorded_with);
        assert_eq!(matching["status"], "ok", "{matching}");
        assert_eq!(matching["policy_version"], version, "{matching}");
        assert_eq!(matching["route"]["rung"], "max", "{matching}");
        assert_eq!(matching["route"]["policy_version"], version, "{matching}");
        assert_eq!(matching.get("note"), None, "{matching}");

        let moved = explain(&policy(Some("[roles.planner]\neffort = \"low\"\n"), ""));
        assert_eq!(moved["status"], "ok", "{moved}");
        assert_eq!(moved["policy_version"], Value::Null, "{moved}");
        assert!(moved["note"].is_string(), "{moved}");
        assert_eq!(moved["route"]["rung"], "low", "{moved}");
        assert_eq!(moved["route"]["policy_version"], 0, "{moved}");

        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_route_query_in_another_form_accepted_is_caught() {
        for (arguments, slot) in [
            (json!({}), "arguments"),
            (json!({"role": "planner", "work_order": I0}), "arguments"),
            (json!({"role": "boss"}), "role"),
            (json!({"work_order": I0.to_uppercase()}), "work_order"),
        ] {
            let refusal = route_query(arguments.clone()).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{arguments}: {refusal}");
            assert_eq!(
                refusal["code"], "invalid-arguments",
                "{arguments}: {refusal}"
            );
            assert_eq!(refusal["slot"], slot, "{arguments}: {refusal}");
        }
        assert_eq!(
            route_query(json!({"role": "planner"})),
            Ok(RouteQuery::Role(Role::Planner))
        );
        assert_eq!(
            route_query(json!({"work_order": I0})),
            Ok(RouteQuery::WorkOrder(I0.into()))
        );
    }

    /// Story 1's questions order `id`, issued through the refine check and
    /// issue with its three inputs and then failed, in a store holding S1,
    /// approved.
    fn failed_analyzer(id: &str) -> (tempfile::TempDir, SqliteStore) {
        use crate::refine::order::check;
        use crate::refine::order::tests::{approved, issue_checked};
        use crate::scope::fixtures::*;

        let (dir, store) = approved();
        let checked = check(&store, &project(), 1).unwrap();
        let issued = issue_checked(&store, &checked, id, R1, T1);
        assert_eq!(issued["status"], "ok", "{issued}");
        fail_order(&store, &project(), id, R2, T2);
        (dir, store)
    }

    #[test]
    fn an_analyzer_retry_beside_a_live_order_of_its_story_is_caught() {
        use crate::refine::order::tests::issue_by_hand;
        use crate::scope::fixtures::*;

        let (_dir, store) = failed_analyzer(I0);
        issue_by_hand(&store, I1, 1, R3);
        let before = head(&store);
        let refusal = retry_check(&store, &project(), I0).unwrap_err();
        assert_refused(
            &refusal,
            "analyzer-order-pending",
            "operation",
            &json!({"work_order": I1}),
        );
        assert_eq!(head(&store), before);
    }

    #[test]
    fn an_analyzer_retry_racing_a_new_order_is_caught() {
        use crate::refine::order::tests::issue_by_hand;
        use crate::scope::fixtures::*;

        let (_dir, store) = failed_analyzer(I0);
        let checked = retry_check(&store, &project(), I0).expect("no other order is pending");
        issue_by_hand(&store, I1, 1, R3);
        let before = head(&store);
        let command = command_in(&project(), RETRY_COMMAND, R4, 5, T4);
        let answer = issue_retry(
            &store,
            &command,
            &checked,
            &policy(None, ""),
            Host::ClaudeCode,
            &no_names(),
            I2,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
        assert_eq!(held(&store, &project(), I2), None);
    }

    #[test]
    fn an_analyzer_retry_losing_its_story_or_inputs_is_caught() {
        use crate::scope::fixtures::*;

        let (_dir, store) = failed_analyzer(I0);
        let answer = retry_in(&store, &project(), I0, I2, R4, T4, "");
        assert_eq!(answer["status"], "ok", "{answer}");
        let events = history(&store);
        let issued = of_type(&events, "work_order.issued");
        let (first, retry) = (issued[0], issued[issued.len() - 1]);
        assert_eq!(retry.payload["id"], I2);
        assert_eq!(retry.stream, "roadmap");
        assert_eq!(retry.payload["subject"], "story/1");
        assert_eq!(retry.payload["parent"], I0);
        assert_eq!(retry.payload["attempt"], 2);
        assert_eq!(retry.payload["inputs"], first.payload["inputs"]);
    }

    #[test]
    fn an_analyzer_retry_over_an_open_set_is_caught() {
        use baley_core::questions::judge_set;

        use crate::questions::record::tests::{open_set, q1, story};
        use crate::refine::order::check;
        use crate::refine::order::tests::{answer_by_hand, approved, issue_checked};
        use crate::scope::fixtures::*;

        let (_dir, store) = approved();
        let first = open_set(&store, R1, &q1(), &story(1));
        answer_by_hand(&store, &first, "q1", 1, R2);
        answer_by_hand(&store, &first, "q2", 2, R3);
        let checked = check(&store, &project(), 1).unwrap();
        let drafting = issue_checked(&store, &checked, I3, R4, T4);
        assert_eq!(drafting["analyzer"], "drafting", "{drafting}");
        fail_order(&store, &project(), I3, R6, T5);
        let q3 = judge_set(&json!([{"id": "q3", "question": "Who reads the ledger?",
            "recommended": "The owner", "depends_on": []}]))
        .unwrap();
        let later = open_set(&store, R7, &q3, &story(1));

        let before = head(&store);
        let refusal = retry_check(&store, &project(), I3).unwrap_err();
        assert_refused(
            &refusal,
            "question-open",
            "operation",
            &json!({"set": later, "question": "q3"}),
        );
        assert_eq!(head(&store), before);
    }

    /// C2b, the retry of the round-2 checker order C2.
    const C2B: &str = "c2b0c2b0-2a2b-4c2d-8e2f-404142434445";
    /// C3, a checker order of a later draft of phase 1.
    const C3: &str = "c3c3c3c3-3a3b-4c3d-9e3f-404142434445";

    /// The retry `checked` issued as `id` under P0 for Claude Code with N0,
    /// under `request_id` at T8. Gives the answer.
    fn retried_under_p0(
        store: &SqliteStore,
        checked: &CheckedRetry,
        id: &str,
        request_id: &str,
    ) -> Value {
        use crate::planning::fixtures::{n0, p0};
        use crate::scope::fixtures::{T8, command};

        let command = command(RETRY_COMMAND, request_id, 9, T8);
        issue_retry(store, &command, checked, &p0(), Host::ClaudeCode, &n0(), id)
    }

    #[test]
    fn a_recheck_retry_rebound_or_reading_the_current_draft_is_caught() {
        use crate::planning::check_return::tests::{C2, V, c1_material, fw, lineage, returned};
        use crate::planning::fixtures::*;
        use crate::planning::order::tests::fail_by_hand;
        use crate::planning::submit::tests::as_drafted;
        use crate::refine::order::tests::{input_body, issued};
        use crate::scope::fixtures::*;

        let (_dir, store, _, _, _) = lineage(&pd(), 6);
        let c2 = issued(&store)
            .into_iter()
            .find(|event| event.payload["id"] == C2)
            .expect("C2 is issued");
        fail_by_hand(&store, C2, R5);
        let checked = retry_check(&store, &project(), C2).expect("C2 failed and nothing is live");
        let receipt = retried_under_p0(&store, &checked, C2B, R6);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["work_order"], C2B, "{receipt}");

        let c2b = issued(&store)
            .into_iter()
            .find(|event| event.payload["id"] == C2B)
            .expect("C2b is issued");
        assert_eq!(c2b.payload["request"], C2);
        assert_eq!(c2b.payload["parent"], C2);
        assert_eq!(c2b.payload["attempt"], 2);
        assert_eq!(c2b.payload["role"], "checker");
        assert_eq!(c2b.payload["inputs"], c2.payload["inputs"]);
        assert_eq!(input_body(&store, &c2b, "original"), as_drafted(pl2()));
        assert_eq!(input_body(&store, &c2b, "plan"), as_drafted(pl2r()));
        let events = history(&store);
        let issues = of_type(&events, "plan.check_issued");
        assert_eq!(issues.len(), 1);
        let material = issues[0].payload["material"].clone();

        let back = returned(&store, C2B, vec![fw()], &p0(), V, R7, T9, 9);
        assert_eq!(back["status"], "ok", "{back}");
        assert_eq!(back["round"], 2, "{back}");
        let events = history(&store);
        let results = of_type(&events, "plan.checked");
        assert_eq!(results.len(), 2);
        let result = &results[1].payload;
        assert_eq!(result["work_order"], C2B);
        assert_eq!(result["scope"], "diff");
        assert_eq!(result["scope_policy_version"], 6);
        assert_eq!(result["material"], material);

        // A fresh store, X2's body purged by the owner.
        let (_dir, store, _, _, _) = lineage(&pd(), 6);
        fail_by_hand(&store, C2, R5);
        let gone = c1_material(&store)[0].body.hash;
        purge_in(&store, &project(), gone, "the owner purged it");
        let before = head(&store);
        let refusal = retry_check(&store, &project(), C2).expect_err("the copy is purged");
        assert_refused(
            &refusal,
            "material-unavailable",
            "work_order",
            &json!({"input": "original"}),
        );
        assert_eq!(head(&store), before);
    }

    /// Store C with O, then PL2 through the gated recording under P0 as C1,
    /// failed by hand.
    fn c1_failed() -> (tempfile::TempDir, SqliteStore) {
        use crate::planning::check_return::tests::gated;
        use crate::planning::checker::tests::C1;
        use crate::planning::fixtures::*;
        use crate::planning::order::tests::{O, fail_by_hand};
        use crate::planning::submit::tests::order_o;
        use crate::scope::fixtures::*;

        let (dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl2(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        fail_by_hand(&store, C1, R3);
        (dir, store)
    }

    #[test]
    fn a_checker_retry_of_a_replaced_draft_is_caught() {
        use crate::planning::check_return::tests::gated;
        use crate::planning::checker::tests::C1;
        use crate::planning::fixtures::*;
        use crate::planning::order::tests::O;
        use crate::scope::fixtures::*;

        let (_dir, store) = c1_failed();
        let replaced = gated(&store, &pl3(), O, &poff(), C3, R4, T6);
        assert_eq!(replaced["status"], "ok", "{replaced}");
        let before = head(&store);
        let refusal = retry_check(&store, &project(), C1).expect_err("the draft is replaced");
        assert_refused(
            &refusal,
            "stale-draft",
            "work_order",
            &json!({"newest": replaced["digest"]}),
        );
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_checker_retry_issued_over_a_draft_submitted_since_its_check_is_caught() {
        use crate::planning::check_return::tests::gated;
        use crate::planning::checker::tests::{C1, C9};
        use crate::planning::fixtures::*;
        use crate::planning::order::tests::{O, fail_by_hand};
        use crate::scope::fixtures::*;

        let (_dir, store) = c1_failed();
        let checked = retry_check(&store, &project(), C1).expect("C1 checks the newest draft");
        let submitted = gated(&store, &pl3(), O, &p0(), C3, R4, T6);
        assert_eq!(submitted["check"]["work_order"], C3, "{submitted}");
        fail_by_hand(&store, C3, R5);

        let before = head(&store);
        let answer = retried_under_p0(&store, &checked, C9, R6);
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_checker_retry_issued_beside_a_pending_planner_order_is_caught() {
        use crate::planning::checker::tests::{C1, C9};
        use crate::planning::order::tests::{O2, prepared};
        use crate::scope::fixtures::*;

        let order_o2 = |store: &SqliteStore, request_id: &str| {
            let (command, o2) = prepared(store, &[1], O2, request_id, T7);
            let receipt = crate::planning::order::issue(store, &command, &o2);
            assert_eq!(receipt["status"], "ok", "{receipt}");
        };

        let (_dir, store) = c1_failed();
        order_o2(&store, R4);
        let before = head(&store);
        let refusal = retry_check(&store, &project(), C1).expect_err("O2 is pending");
        assert_refused(
            &refusal,
            "planner-order-pending",
            "operation",
            &json!({"work_order": O2}),
        );
        assert_eq!(head(&store), before);

        let (_dir, store) = c1_failed();
        let checked = retry_check(&store, &project(), C1).expect("no planner order is pending");
        order_o2(&store, R4);
        let before = head(&store);
        let answer = retried_under_p0(&store, &checked, C9, R5);
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_checker_retry_of_a_draft_submitted_again_is_caught() {
        use crate::mcp::questions::tests::{REVISION, relayed};
        use crate::planning::approve::tests::{approved, digest_of};
        use crate::planning::check_return::tests::{V, fw, gated, returned};
        use crate::planning::checker::tests::C1;
        use crate::planning::fixtures::*;
        use crate::planning::order::tests::O;
        use crate::questions::answer::tests::take;
        use crate::scope::fixtures::*;

        let (_dir, store) = c1_failed();
        let refused = |store: &SqliteStore, digest: &str| {
            let before = head(store);
            let refusal = retry_check(store, &project(), C1).expect_err("C1's draft came again");
            assert_refused(
                &refusal,
                "stale-draft",
                "work_order",
                &json!({"newest": digest}),
            );
            assert_eq!(head(store), before);
        };

        // The same bytes under O: one digest, a later submission.
        let again = gated(&store, &pl2(), O, &p0(), C3, R4, T6);
        assert_eq!(again["check"]["work_order"], C3, "{again}");
        let x2 = digest_of(&again);
        refused(&store, &x2.to_hex());

        let back = returned(&store, C3, vec![fw()], &p0(), V, R5, T7, 5);
        assert_eq!(back["status"], "ok", "{back}");
        refused(&store, &x2.to_hex());

        let set = back["set"].as_str().expect("a set id");
        let closed = relayed(&store, set, &json!([take("q1")]), R6, T8, REVISION);
        assert_eq!(closed["status"], "ok", "{closed}");
        let approval = approved(&store, x2, &p0(), R7, T9);
        assert_eq!(approval["status"], "ok", "{approval}");
        refused(&store, &x2.to_hex());
    }
}
