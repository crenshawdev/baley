//! A start or change submission recorded as a draft (design 0004 section
//! 5): its body stored as a `record` payload and one `scope.submitted` on the
//! `project` stream, in the submission's own domain transaction. Accepted
//! scope does not change until the owner approves the draft by its digest.
//!
//! A start names the start planner's work order it answers, which is checked
//! before its write is prepared, and its events carry that order and the
//! order's instructions on their caller.
//!
//! A change is checked against accepted scope before its write is prepared,
//! so a change scope rules out records nothing at all (D-08), and its
//! transaction confirms the scope it was checked against has not moved.

use std::collections::BTreeMap;

use baley_core::plan::{
    PLAN_CONTINUATION, PLAN_CONTINUATION_VERSION, continuation_payload, phase_stream,
};
use baley_core::policy::Role;
use baley_core::scope::{
    ChangeKind, ChangeSubmission, PROJECT_ALREADY_STARTED, PROJECT_STREAM, PhaseFields,
    SCOPE_APPROVED, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION, START_KIND, START_TARGET,
    ScopeRefusal, Submission, SubmissionRefusal, judge_change, judge_scope_change,
    submitted_payload,
};
use baley_core::work_order::{NO_SUCH_WORK_ORDER, START_SUBJECT, WRONG_WORK_ORDER};
use baley_store::{
    Absence, Command, Decision, EVENT_PAGE_BOUND, EventMatch, Hash, HistoryFilter,
    InstructionEvidence, Ledger, NewEvent, Observed, ObservedDocument, OutcomeKind, PageRequest,
    Payloads, ProjectId, REQUEST_VIEW, Recorded, Refusal as StoreRefusal, RetentionClass,
    StoreError, StreamName, Views, request_key, request_state,
};
use serde_json::{Map, Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::ledger::answer::{AnswerUnread, answer_value};
use crate::mcp::capture::{PLACE_LEDGER, REQUEST_ID_REUSE, TEXT_PURGED, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::scope::read::{self, ReadError, ScopeRead};

/// The command kind every scope submission is recorded under.
pub const SUBMIT_COMMAND: &str = "scope.submit";

/// A busy ledger's reason when a scope call recorded nothing.
pub(crate) const NOT_RECORDED_BUSY: &str =
    "the ledger is busy, so nothing was recorded. Try it again";

/// The start approval on the project stream, as an absence or a
/// confirmation matches it.
pub fn start_approval() -> EventMatch {
    EventMatch {
        type_name: SCOPE_APPROVED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::from([("kind".to_string(), json!(START_KIND))]),
    }
}

/// Whether `project`'s events hold a `scope.approved` of kind `start`, read
/// from the events and never from a view (D-05). It pages the approvals
/// until one of kind `start` turns up or the events end.
pub fn start_approved<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
) -> Result<bool, StoreError> {
    let filter = HistoryFilter {
        types: vec![SCOPE_APPROVED.into()],
        git_commit: None,
    };
    let start = json!(START_KIND);
    let mut after = None;
    loop {
        let page = store.history(
            project,
            1..=u64::MAX,
            &filter,
            PageRequest {
                limit: EVENT_PAGE_BOUND,
                after,
            },
        )?;
        if page.items.iter().any(|event| {
            event.stream == PROJECT_STREAM && event.payload.get("kind") == Some(&start)
        }) {
            return Ok(true);
        }
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(false),
        }
    }
}

/// The start-once check a submission is prepared with: it lets the write go
/// on while no start is approved, and otherwise ends the call refused
/// `project-already-started` with nothing recorded. A read fault is a
/// store error's `failed` answer.
pub fn start_once<L: Ledger + ?Sized>(store: &L, project: &ProjectId) -> Result<(), Value> {
    match start_approved(store, project) {
        Ok(false) => Ok(()),
        Ok(true) => Err(Refusal::new(
            PROJECT_ALREADY_STARTED,
            "this project's first scope is already approved, so no other start can be submitted. Accepted scope changes through the scope-change commands",
        )
        .slot("submission")
        .value()),
        Err(error) => Err(store_failed(&error, NOT_RECORDED_BUSY)),
    }
}

/// The order check a start submission is prepared with (D-04): the work
/// order `id` read from `project`, none `no-such-work-order`, then one that
/// is not the start planner's `wrong-work-order`, both slot `work_order`,
/// then [`start_once`]. An answered order passes, so a later submission
/// naming it is the newest draft (D-03). A pass gives back the order's
/// instruction evidence, which the submission's caller then carries (D-24).
/// It records nothing.
pub fn start_order_check<S: Ledger + Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: &str,
) -> Result<Vec<InstructionEvidence>, Value> {
    let order = crate::work_order::read::order(store, project, id)
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
        .ok_or_else(|| {
            Refusal::new(
                NO_SUCH_WORK_ORDER,
                "this project holds no work order with that id, so nothing was recorded",
            )
            .slot("work_order")
            .value()
        })?;
    let text = |field: &str| order.get(field).and_then(Value::as_str);
    if text("role") != Some(Role::Planner.name()) || text("subject") != Some(START_SUBJECT) {
        return Err(Refusal::new(
            WRONG_WORK_ORDER,
            "that work order is not the start planner's, so a start submission cannot answer it, and nothing was recorded",
        )
        .slot("work_order")
        .value());
    }
    start_once(store, project)?;
    order_evidence(&order).ok_or_else(|| {
        failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a work order whose instructions cannot be read",
            PLACE_LEDGER,
        )
    })
}

/// The instruction evidence an order document lists, in order.
fn order_evidence(order: &Value) -> Option<Vec<InstructionEvidence>> {
    order
        .get("instructions")?
        .as_array()?
        .iter()
        .map(|entry| {
            let text = |field: &str| entry.get(field).and_then(Value::as_str);
            InstructionEvidence::new(text("identity")?, text("version")?, text("hash")?).ok()
        })
        .collect()
}

/// Records one accepted start `submission` under `command`, the command
/// preparation returned, in one domain transaction: the body as a `record`
/// payload and one `scope.submitted` attaching it. Every call records, so a
/// resubmission makes its draft the newest again (D-04); only a retry of the
/// same request replays.
///
/// The decision declares the start approval it relied on being absent, so
/// a start approved after the start-once read makes the store refuse the
/// command as stale with nothing recorded. The answer is the receipt, which
/// never holds the submission. The store's result comes back unchanged.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    submission: &Submission,
) -> Result<Recorded, StoreError> {
    // A receipt over 4 KiB would be stored as its own answer payload, so it
    // carries counts, never the content.
    let receipt = json!({
        "status": "ok",
        "digest": submission.digest().to_hex(),
        "kind": START_KIND,
        "stories": submission.stories.len(),
        "phases": submission.phases.len(),
        "recorded_at": command.recorded_at,
    });
    store.transact(command, &mut |tx| {
        let body = tx.put_payload(submission.body(), RetentionClass::Record)?;
        tx.append(NewEvent {
            stream: StreamName(PROJECT_STREAM.into()),
            type_name: SCOPE_SUBMITTED.into(),
            type_version: SCOPE_SUBMITTED_VERSION,
            git: None,
            payload: submitted_payload(submission.digest(), START_KIND, START_TARGET, &body),
            attachments: vec![body],
        })?;
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: receipt.clone(),
            sensitive: false,
            observed: Observed {
                absences: vec![Absence::Event(start_approval())],
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// Checks `change` against accepted scope as `project`'s ledger holds it
/// now: the scope read, then the scope judgement, recording nothing. A
/// change scope rules out is the `refused` answer, slot `submission`; a
/// read fault is a store error's `failed` answer. A pass gives back the
/// read, which the recording confirms. It runs before the write is
/// prepared, so a refusal records no checkout admission, policy step or
/// `command.completed` either (D-08).
pub fn check_change<S: Ledger + Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    change: &ChangeSubmission,
) -> Result<ScopeRead, Value> {
    let read = read::scope(store, project).map_err(|error| scope_unread(&error))?;
    let accepted = read
        .accepted()
        .ok_or_else(|| scope_unread(&ReadError::Unreadable(String::new())))?;
    judge_scope_change(change, &accepted, read.started, &read.progress())
        .map_err(|refusal| scope_refused(&refusal, "submission"))?;
    Ok(read)
}

/// A scope read fault's answer: a busy ledger may be retried.
pub(crate) fn scope_unread(error: &ReadError) -> Value {
    match error {
        ReadError::Store(error) => store_failed(error, NOT_RECORDED_BUSY),
        _ => failed(
            LEDGER_UNAVAILABLE,
            "the project's accepted scope cannot be read, so nothing was recorded",
            PLACE_LEDGER,
        ),
    }
}

/// The `refused` answer for a change accepted scope rules out, at `slot`:
/// `submission` when it is submitted, `digest` when it is approved (D-12).
/// The reason names the phases and stories the details hold, since the
/// command line prints only the code and the reason.
pub fn scope_refused(refusal: &ScopeRefusal, slot: &str) -> Value {
    let reason = match refusal {
        ScopeRefusal::ProjectNotStarted => {
            "this project's first scope is not approved yet, so there is no accepted scope to change and nothing was recorded".to_string()
        }
        ScopeRefusal::NoSuchPhase { phase } => {
            format!("this project has no declared phase {phase}, so nothing was recorded")
        }
        ScopeRefusal::NoSuchStory { story } => {
            format!("this project has no story {story}, so nothing was recorded")
        }
        ScopeRefusal::Malformed { section, field } => format!(
            "the change's {field} in {section} does not fit accepted scope, so nothing was recorded"
        ),
        ScopeRefusal::StoryDoubleAssigned { story, phase } => format!(
            "story {story} is already served by phase {phase}, and a story belongs to one phase at most, so nothing was recorded"
        ),
        ScopeRefusal::DependencyCycle { phases } => format!(
            "{} would depend on each other in a cycle, so no order can deliver them and nothing was recorded",
            listed("phase", phases)
        ),
        ScopeRefusal::DependencyOrder { phase, depends_on } => format!(
            "phase {phase} would come before phase {depends_on}, which it depends on, so nothing was recorded"
        ),
        ScopeRefusal::OrderIncomplete { order } => format!(
            "the order must hold each of {} exactly once, so nothing was recorded",
            listed("entry", order)
        ),
        ScopeRefusal::StoryUndecided { story } => format!(
            "the withdrawal gives story {story} of the phase no decision, so nothing was recorded. Each of its stories needs a phase to move to or a drop"
        ),
        ScopeRefusal::PhaseHasDependents { phases } => format!(
            "{} depend on this phase, so it cannot be withdrawn and nothing was recorded. Change their dependencies first",
            listed("phase", phases)
        ),
        ScopeRefusal::StoryNotServed { story, status } => format!(
            "story {story} is {status}, which is final, so it cannot change and nothing was recorded"
        ),
        ScopeRefusal::ExecutionPresent { phase, undo } => format!(
            "phase {phase} has execution present, so it cannot be withdrawn until {undo} undoes it, and nothing was recorded"
        ),
        ScopeRefusal::PhaseComplete { phase } => format!(
            "phase {phase} is complete, so it cannot be withdrawn or given other stories, and nothing was recorded"
        ),
        ScopeRefusal::StoryShipped { story, phase } => format!(
            "story {story} was served by phase {phase}, which is complete, so it cannot be moved or dropped and nothing was recorded"
        ),
    };
    let answer = Refusal::new(refusal.code(), reason).slot(slot);
    match refusal.details() {
        Some(details) => answer.details(details),
        None => answer,
    }
    .value()
}

/// `numbers` in words: `phase 2`, `phases 2 and 3`, `phases 1, 2 and 3`.
/// An order's entries are ids or numbers, so they are named bare.
fn listed(noun: &str, numbers: &[u32]) -> String {
    let words: Vec<String> = numbers.iter().map(u32::to_string).collect();
    let joined = match words.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    };
    match (noun, numbers.len()) {
        ("entry", _) => joined,
        (_, 1) => format!("{noun} {joined}"),
        _ => format!("{noun}s {joined}"),
    }
}

/// Records one checked `change` under `command`, the command preparation
/// returned, in one domain transaction: the body as a `record` payload and
/// one `scope.submitted` attaching it, with the change's kind and target.
/// Every call records, so the same content submitted again is the newest
/// draft again; only a retry of the same request replays, from the store
/// before any decision runs.
///
/// Before it appends anything the decision confirms every scope document
/// `read`, the check's, named is unchanged, so a change whose scope moved
/// since its check records nothing and `answer` gives `failed`
/// `ledger-busy`. A decision asked to run without the check's read records
/// nothing. The answer is the receipt, which never holds the change. It
/// records no planning continuation and observes nothing more; T9's `phase
/// plan` records one through [`submit_phase_edit`].
pub fn record_change<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    change: &ChangeSubmission,
    read: Option<&ScopeRead>,
) -> Result<Recorded, StoreError> {
    record_with(store, command, change, read, None)
}

/// A planning continuation recorded with a `phase edit` of phase `phase`'s
/// stories (D-09): the selection the edit commits and the planner order its
/// approval issues, with the documents the caller read when it chose to
/// draft and what it relied on being absent.
#[derive(Debug, Clone, Copy)]
pub struct PlanningContinuation<'a> {
    /// The selection, in order.
    pub stories: &'a [u32],
    /// The id of the planner order the edit's approval issues, fixed now so
    /// a rerun finds the same order.
    pub work_order: &'a str,
    /// The documents the caller read when it chose to draft, at the sequence
    /// that produced each, so an order issued or a story moved since makes
    /// the call `ledger-busy` with nothing recorded.
    pub documents: &'a [ObservedDocument],
    /// What the caller relied on being absent when it chose to draft, such
    /// as an active phase an empty read of `phase_plan`'s status index left
    /// no document for, so one recorded since makes the call `ledger-busy`
    /// with nothing recorded.
    pub absences: &'a [Absence],
}

/// [`record_change`] with a planning continuation of phase `phase`: in the
/// same transaction, after the draft, one `plan.continuation` on `phase/<n>`
/// naming the phase, the selection in order, the draft's digest and the
/// order, and the decision observes the continuation's documents and
/// declares its absences. Without one it records and observes exactly what
/// `record_change` does.
fn record_with<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    change: &ChangeSubmission,
    read: Option<&ScopeRead>,
    continuation: Option<(u32, PlanningContinuation<'_>)>,
) -> Result<Recorded, StoreError> {
    let kind = change.kind().as_str();
    let receipt = json!({
        "status": "ok",
        "digest": change.digest().to_hex(),
        "kind": kind,
        "recorded_at": command.recorded_at,
    });
    store.transact(command, &mut |tx| {
        let Some(read) = read else {
            return Err(StoreError::Unavailable(
                "a change submission reached its transaction without its check's read".into(),
            ));
        };
        read::confirm(tx, read)?;
        let body = tx.put_payload(change.body(), RetentionClass::Record)?;
        tx.append(NewEvent {
            stream: StreamName(PROJECT_STREAM.into()),
            type_name: SCOPE_SUBMITTED.into(),
            type_version: SCOPE_SUBMITTED_VERSION,
            git: None,
            payload: submitted_payload(change.digest(), kind, change.target(), &body),
            attachments: vec![body],
        })?;
        let mut observed = Observed::default();
        if let Some((phase, continuation)) = continuation {
            tx.append(NewEvent {
                stream: StreamName(phase_stream(phase)),
                type_name: PLAN_CONTINUATION.into(),
                type_version: PLAN_CONTINUATION_VERSION,
                git: None,
                payload: continuation_payload(
                    phase,
                    continuation.stories,
                    change.digest(),
                    continuation.work_order,
                ),
                attachments: Vec::new(),
            })?;
            observed.documents = continuation.documents.to_vec();
            observed.absences = continuation.absences.to_vec();
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: receipt.clone(),
            sensitive: false,
            observed,
            git: None,
        })
    })
}

/// Records a `phase-edit` of phase `phase` changing `fields` under
/// `command`, which its caller prepared, from plain values, so T9's `phase
/// plan` can record an edit without MCP or the command line (D-18). The
/// members are built as a session sends them, so the same edit has the same
/// body and digest; then they are judged, checked against accepted scope and
/// recorded as `check_change` and `record_change` do. A request the ledger
/// already holds is replayed before the check, as preparation does over MCP,
/// so a retry of a completed edit gets its recorded answer however scope has
/// moved since; a check that fails looks the request up again, for an
/// original that committed after the first lookup. The answer is the digest the recorded receipt names, or the
/// answer that refused or failed it; a receipt the store replayed for another
/// change recorded under the same command is a reused request id, since this
/// edit was never recorded. The draft is approved through `approve::judge`
/// and `approve::record`.
///
/// T9's `phase plan` passes `continuation` for a changed selection, and the
/// edit is then recorded with its `plan.continuation` in one transaction or
/// not at all, observing the documents the caller read and declaring what it
/// relied on being absent (D-09). With none,
/// the edit records exactly what `record_change` records.
pub fn submit_phase_edit<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    phase: u32,
    fields: &PhaseFields,
    continuation: Option<PlanningContinuation<'_>>,
) -> Result<Hash, Value> {
    let mut members = Map::new();
    members.insert("phase".into(), json!(phase));
    let texts = [
        ("name", &fields.name),
        ("goal", &fields.goal),
        ("detail", &fields.detail),
    ];
    for (name, text) in texts {
        if let Some(text) = text {
            members.insert(name.into(), json!(text));
        }
    }
    let lists = [
        ("depends_on", &fields.depends_on),
        ("stories", &fields.stories),
    ];
    for (name, numbers) in lists {
        if let Some(numbers) = numbers {
            members.insert(name.into(), json!(numbers));
        }
    }
    let change = judge_change(ChangeKind::PhaseEdit, &Value::Object(members))
        .map_err(|refusal| submission_refused(&refusal))?;
    let continuation = continuation.map(|continuation| (phase, continuation));
    let result = if held(store, command)? {
        record_with(store, command, &change, None, continuation)
    } else {
        match check_change(store, &command.project, &change) {
            Ok(read) => record_with(store, command, &change, Some(&read), continuation),
            // The original can commit between the lookup and the check, and
            // the check may refuse because of it, so the request is looked
            // up again before a refusal is answered.
            Err(refusal) if !held(store, command)? => return Err(refusal),
            Err(_) => record_with(store, command, &change, None, continuation),
        }
    };
    recorded_digest(answer(result, store), &change)
}

/// The digest `answer`, a scope call's recorded answer, names when it is
/// `change`'s. An `ok` receipt for another change is a reused request id;
/// any other answer is returned as it is.
fn recorded_digest(answer: Value, change: &ChangeSubmission) -> Result<Hash, Value> {
    if answer["status"] != "ok" {
        return Err(answer);
    }
    match answer
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex)
    {
        Some(digest) if digest == change.digest() => Ok(digest),
        _ => Err(request_id_reused()),
    }
}

/// The `refused` answer for a request id already used for another scope
/// call.
fn request_id_reused() -> Value {
    Refusal::new(
        REQUEST_ID_REUSE,
        "this request_id was already used for a different scope call, so nothing was recorded. A new call needs a new request_id",
    )
    .slot("request_id")
    .value()
}

/// Whether the ledger holds a record of `command`'s request, looked up as
/// preparation looks up a write over MCP. A record that cannot be read is a
/// ledger fault.
fn held<S: Views + ?Sized>(store: &S, command: &Command) -> Result<bool, Value> {
    match store.get(&command.project, REQUEST_VIEW, &request_key(command)) {
        Ok(None) => Ok(false),
        Ok(Some(document)) if request_state(&document.body).is_some() => Ok(true),
        Ok(Some(_)) => Err(failed(
            LEDGER_UNAVAILABLE,
            "the ledger holds a record of this request that cannot be read",
            PLACE_LEDGER,
        )),
        Err(error) => Err(store_failed(&error, NOT_RECORDED_BUSY)),
    }
}

/// The `refused` answer for a submission judged unacceptable, with the
/// fault's place in its details.
pub fn submission_refused(refusal: &SubmissionRefusal) -> Value {
    let (reason, details) = match refusal {
        SubmissionRefusal::Malformed { section, field } => (
            "a field of the submission is missing, blank, of the wrong type, unknown or out of range, so nothing was recorded. The details name it",
            json!({"section": section, "field": field}),
        ),
        SubmissionRefusal::StoryDoubleAssigned { story, phases } => (
            "a story is listed by two phases, and a story belongs to one phase at most, so nothing was recorded",
            json!({"story": story, "phases": phases}),
        ),
        SubmissionRefusal::DependencyCycle { phases } => (
            "phases depend on each other in a cycle, so no order can deliver them and nothing was recorded",
            json!({"phases": phases}),
        ),
    };
    Refusal::new(refusal.code(), reason)
        .slot("submission")
        .details(details)
        .value()
}

/// A busy ledger's reason when a recorded answer was not read.
const NOT_READ_BUSY: &str = "the ledger is busy, so the recorded answer was not read. Try it again";

/// The answer for a scope call's transaction result. A new or replayed
/// outcome answers the recorded answer, read through `payloads` when it was
/// stored. A ledger that moved after the call was judged is `ledger-busy`,
/// since repeating the call judges it afresh.
pub fn answer(result: Result<Recorded, StoreError>, payloads: &dyn Payloads) -> Value {
    match result {
        Ok(Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) => {
            match answer_value(&outcome.answer, payloads) {
                Ok(value) => value,
                Err(AnswerUnread::Read(error)) => store_failed(&error, NOT_READ_BUSY),
                Err(AnswerUnread::Gone(_)) => failed(
                    LEDGER_UNAVAILABLE,
                    "the call is recorded, but its stored answer is no longer held",
                    PLACE_LEDGER,
                ),
                Err(AnswerUnread::Malformed) => failed(
                    LEDGER_UNAVAILABLE,
                    "the call is recorded, but its stored answer cannot be read",
                    PLACE_LEDGER,
                ),
            }
        }
        Err(StoreError::Refused(StoreRefusal::RequestDigestMismatch { .. })) => request_id_reused(),
        Err(StoreError::Refused(StoreRefusal::PayloadTombstoned(_))) => failed(
            TEXT_PURGED,
            "this exact text was purged from the ledger and cannot be stored again, so nothing was recorded",
            "submission",
        ),
        Err(StoreError::Stale(_)) => failed(
            LEDGER_BUSY,
            "the project's scope changed while this call was prepared, so nothing was recorded. The same call can be repeated, and it is judged afresh",
            PLACE_LEDGER,
        ),
        Err(error) => store_failed(&error, NOT_RECORDED_BUSY),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Read;
    use std::ops::RangeInclusive;

    use baley_core::scope::{DRAFT_NEWEST_INDEX, DRAFT_VIEW};
    use baley_store::{
        Anchor, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim, DecideReconcile, DocKey,
        Document, Event, Hash, Head, IndexQuery, KeyValue, Page, PayloadBody, PayloadRef,
        PayloadStatus, ReconcileAuthority, RequestId, StaleInput, VerifyReport, Views,
    };

    use super::*;
    use crate::scope::fixtures::*;

    fn assert_failed(value: &Value, code: &str, place: &str, retryable: bool) {
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], code, "{value}");
        assert_eq!(value["place"], place, "{value}");
        assert_eq!(value["recorded"], false, "{value}");
        assert_eq!(value["retryable"], retryable, "{value}");
    }

    #[test]
    fn a_submission_recorded_off_the_project_stream_or_without_its_body_is_caught() {
        let (_dir, store) = store();
        let recorded = submit(&store, &m(), R1, 1, T1).unwrap();

        let events = history(&store);
        let submitted = of_type(&events, "scope.submitted");
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].stream, "project");
        // The store refuses a reference its event does not attach, so this
        // reference is the event's one attachment.
        assert_eq!(
            submitted[0].payload,
            json!({"digest": M_DIGEST, "kind": "start", "target": "project",
                "body": {"payload": M_DIGEST, "bytes": 202, "class": "record"}})
        );
        let body = PayloadRef::from_value(&submitted[0].payload["body"]).unwrap();
        let PayloadBody::Present(mut reader) = store.open(&body.hash).unwrap() else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), M_CANONICAL);

        assert_eq!(
            receipt(&recorded),
            json!({"status": "ok", "digest": M_DIGEST, "kind": "start", "stories": 1,
                "phases": 1, "recorded_at": T1})
        );
    }

    #[test]
    fn a_resubmission_recording_nothing_or_a_retry_recording_twice_is_caught() {
        let (_dir, store) = store();
        let first = submit(&store, &m(), R1, 1, T1).unwrap();
        let second = submit(&store, &m(), R2, 1, T2).unwrap();
        assert!(matches!(second, Recorded::New { .. }), "{second:?}");
        let third = submit(&store, &m(), R1, 1, T3).unwrap();
        assert!(matches!(third, Recorded::Replayed { .. }), "{third:?}");
        assert_eq!(receipt(&third), receipt(&first));

        approve_a_start(&store, T3);
        let fourth = submit(&store, &m(), R1, 1, T3).unwrap();
        assert!(matches!(fourth, Recorded::Replayed { .. }), "{fourth:?}");
        assert_eq!(receipt(&fourth), receipt(&first));
        assert_eq!(of_type(&history(&store), "scope.submitted").len(), 2);
    }

    #[test]
    fn the_newest_draft_after_a_b_a_taken_as_b_is_caught() {
        let (_dir, store) = store();
        let a = receipt(&submit(&store, &s1(), R1, 1, T1).unwrap())["digest"].clone();
        let b = receipt(&submit(&store, &s2(), R2, 2, T2).unwrap())["digest"].clone();
        assert_ne!(a, b);
        submit(&store, &s1(), R3, 3, T3).unwrap();

        let query = IndexQuery {
            index: DRAFT_NEWEST_INDEX.into(),
            equals: vec![KeyValue::Text("start".into())],
            page: PageRequest {
                limit: 1,
                after: None,
            },
        };
        let page = store.find(&project(), DRAFT_VIEW, &query).unwrap();
        assert_eq!(page.items[0].body["digest"], a);
    }

    #[test]
    fn a_submission_after_an_approved_start_recorded_is_caught() {
        let (_dir, store) = store();
        approve_a_start(&store, T1);
        let before = head(&store);

        let refusal = start_once(&store, &project()).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "project-already-started", "{refusal}");
        assert_eq!(refusal["slot"], "submission", "{refusal}");
        assert_eq!(head(&store), before);
        assert!(of_type(&history(&store), "scope.submitted").is_empty());
    }

    #[test]
    fn a_submission_recorded_after_a_start_approved_since_its_read_is_caught() {
        let (_dir, store) = store();
        start_once(&store, &project()).unwrap();
        approve_a_start(&store, T1);
        let after = head(&store);

        let result = submit(&store, &m(), R1, 1, T2);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        let events = history(&store);
        assert!(of_type(&events, "scope.submitted").is_empty());
        assert_eq!(of_type(&events, "command.completed").len(), 1);
        assert_failed(&answer(result, &store), "ledger-busy", "ledger", true);
    }

    fn digest_of(recorded: &Recorded) -> Hash {
        Hash::from_hex(receipt(recorded)["digest"].as_str().unwrap()).unwrap()
    }

    /// The fixtures' store with S1 approved at T3.
    fn approved() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let (dir, store) = two_projects();
        approve_draft(
            &store,
            digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap()),
        );
        (dir, store)
    }

    #[test]
    fn a_change_submission_breaking_a_scope_rule_recorded_or_answered_otherwise_is_caught() {
        use baley_core::scope::{ChangeKind as K, SubmissionRefusal, judge_change};

        let (_dir, store) = approved();
        append(&store, R2, T3, vec![story_dropped(3)]);
        let before = head(&store);
        let cases = [
            (
                K::PhaseReorder,
                json!({"order": [1]}),
                "order-incomplete",
                json!({"order": [1, 2]}),
            ),
            (
                K::BacklogReorder,
                json!({"order": [1, 2, 2]}),
                "order-incomplete",
                json!({"order": [1, 2]}),
            ),
            (
                K::PhaseReorder,
                json!({"order": [2, 1]}),
                "dependency-order",
                json!({"phase": 2, "depends_on": 1}),
            ),
            (
                K::PhaseWithdraw,
                json!({"phase": 2, "decisions": []}),
                "story-undecided",
                json!({"story": 2}),
            ),
            (
                K::PhaseWithdraw,
                json!({"phase": 1, "decisions": [{"story": 1, "to": "drop"}]}),
                "phase-has-dependents",
                json!({"phases": [2]}),
            ),
            (
                K::StoryEdit,
                json!({"story": 3, "sentence": "Teammates read the backlog"}),
                "story-not-served",
                json!({"story": 3, "status": "dropped"}),
            ),
        ];
        for (kind, value, code, details) in cases {
            let refusal = check_change(&store, &project(), &change(kind, &value)).unwrap_err();
            assert_eq!(refusal["status"], "refused", "{value}: {refusal}");
            assert_eq!(refusal["code"], code, "{value}: {refusal}");
            assert_eq!(refusal["slot"], "submission", "{value}: {refusal}");
            assert_eq!(refusal["details"], details, "{value}: {refusal}");
        }
        assert_eq!(head(&store), before);

        let capture = json!({"sentence": "s", "status": "active", "phase": null, "capture": "c-1"});
        assert_eq!(
            judge_change(K::StoryDeclare, &capture),
            Err(SubmissionRefusal::Malformed {
                section: "capture".into(),
                field: "capture".into()
            })
        );

        let other = || store.head(&project_b()).unwrap().map_or(0, |head| head.seq);
        let before = other();
        let refusal =
            check_change(&store, &project_b(), &change(K::ProjectDescribe, &d1())).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "project-not-started", "{refusal}");
        assert_eq!(refusal["slot"], "submission", "{refusal}");
        assert_eq!(other(), before);
    }

    #[test]
    fn a_change_draft_recorded_without_its_kind_target_or_body_is_caught() {
        use baley_core::scope::{ChangeKind, ROADMAP_VIEW, roadmap_key};

        let (_dir, store) = approved();
        let roadmap = || store.get(&project(), ROADMAP_VIEW, &roadmap_key()).unwrap();
        let before = roadmap();
        let submitted_before = of_type(&history(&store), "scope.submitted").len();
        let recorded = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R4, T4).unwrap();

        let events = history(&store);
        let submitted = of_type(&events, "scope.submitted");
        assert_eq!(submitted.len(), submitted_before + 1);
        let new = submitted.last().unwrap();
        assert_eq!(new.stream, "project");
        assert_eq!(new.type_version, 2);
        assert_eq!(
            new.payload,
            json!({"digest": E_B_DIGEST, "kind": "phase-edit", "target": "2",
                "body": {"payload": E_B_DIGEST, "bytes": 72, "class": "record"}})
        );
        let body = PayloadRef::from_value(&new.payload["body"]).unwrap();
        let PayloadBody::Present(mut reader) = store.open(&body.hash).unwrap() else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), E_B_CANONICAL);
        assert_eq!(
            receipt(&recorded),
            json!({"status": "ok", "digest": E_B_DIGEST, "kind": "phase-edit",
                "recorded_at": T4})
        );
        assert_eq!(roadmap(), before);
    }

    #[test]
    fn a_change_draft_recorded_after_its_scope_moved_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let edit = change(ChangeKind::PhaseEdit, &e_b());
        let read = check_change(&store, &project(), &edit).unwrap();
        append(&store, R3, T4, vec![story_dropped(3)]);
        let after = head(&store);

        let command = change_command(&project(), &edit, R2, T5);
        let result = record_change(&store, &command, &edit, Some(&read));
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        assert_failed(&answer(result, &store), "ledger-busy", "ledger", true);
    }

    #[test]
    fn a_retried_change_submission_recorded_twice_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let first = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R4, T4).unwrap();
        let again = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R4, T5).unwrap();
        assert!(matches!(again, Recorded::Replayed { .. }), "{again:?}");
        assert_eq!(receipt(&again), receipt(&first));
        let events = history(&store);
        let drafts = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["digest"] == E_B_DIGEST)
            .count();
        assert_eq!(drafts, 1);
    }

    #[test]
    fn a_phase_edit_recorded_from_plain_values_under_another_digest_is_caught() {
        use baley_core::scope::PhaseFields;

        let (_dir, store) = approved();
        let goal = PhaseFields {
            goal: Some("The owner approves every change".into()),
            ..PhaseFields::default()
        };
        let digest =
            submit_phase_edit(&store, &command(SUBMIT_COMMAND, R4, 4, T4), 2, &goal, None).unwrap();
        assert_eq!(digest.to_hex(), E_B_DIGEST);
        let events = history(&store);
        let drafts: Vec<&Value> = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["digest"] == E_B_DIGEST)
            .map(|event| &event.payload["target"])
            .collect();
        assert_eq!(drafts, ["2"]);

        let before = head(&store);
        let empty = PhaseFields::default();
        let refusal =
            submit_phase_edit(&store, &command(SUBMIT_COMMAND, R6, 6, T5), 2, &empty, None)
                .unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "malformed-submission", "{refusal}");
        assert_eq!(refusal["slot"], "submission", "{refusal}");
        assert_eq!(
            refusal["details"],
            json!({"section": "submission", "field": "submission"})
        );
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_retried_phase_edit_refused_after_its_phase_was_withdrawn_is_caught() {
        use baley_core::scope::PhaseFields;

        let (_dir, store) = approved();
        let goal = PhaseFields {
            goal: Some("The owner approves every change".into()),
            ..PhaseFields::default()
        };
        let edit = command(SUBMIT_COMMAND, R4, 4, T4);
        submit_phase_edit(&store, &edit, 2, &goal, None).unwrap();
        append(
            &store,
            R6,
            T5,
            vec![
                phase_withdrawn(2),
                story_reassigned(2, 1),
                phase_reordered(&[1]),
            ],
        );
        let before = head(&store);

        let again = submit_phase_edit(&store, &edit, 2, &goal, None);
        assert_eq!(again, Ok(Hash::from_hex(E_B_DIGEST).unwrap()));
        assert_eq!(head(&store), before);
        let drafts = of_type(&history(&store), "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["digest"] == E_B_DIGEST)
            .count();
        assert_eq!(drafts, 1);
    }

    /// The real store, with `land` run once just after the first request
    /// lookup returns: the window in which a retry's original can commit
    /// between the lookup and the check.
    struct CommitsAtLookup<'a> {
        store: &'a baley_store_sqlite::SqliteStore,
        land: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl Ledger for CommitsAtLookup<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::transact(self.store, command, decide)
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut DecideClaim<'_>,
        ) -> Result<Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &ClaimId,
            owner: &ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &ClaimOwner,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &ClaimId,
            authority: ReconcileAuthority,
            decide: &mut DecideReconcile<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: RangeInclusive<u64>,
            filter: &HistoryFilter,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::history(self.store, project, range, filter, page)
        }

        fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&Anchor>,
        ) -> Result<VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    impl Views for CommitsAtLookup<'_> {
        fn get(
            &self,
            project: &ProjectId,
            view: &str,
            key: &DocKey,
        ) -> Result<Option<Document>, StoreError> {
            let read = Views::get(self.store, project, view, key);
            if view == REQUEST_VIEW {
                let land = self.land.borrow_mut().take();
                if let Some(land) = land {
                    land();
                }
            }
            read
        }

        fn get_many(
            &self,
            project: &ProjectId,
            view: &str,
            keys: &[DocKey],
        ) -> Result<Vec<Option<Document>>, StoreError> {
            Views::get_many(self.store, project, view, keys)
        }

        fn find(
            &self,
            project: &ProjectId,
            view: &str,
            query: &IndexQuery,
        ) -> Result<Page<Document>, StoreError> {
            Views::find(self.store, project, view, query)
        }
    }

    impl Payloads for CommitsAtLookup<'_> {
        fn open(&self, hash: &Hash) -> Result<PayloadBody<'_>, StoreError> {
            Payloads::open(self.store, hash)
        }

        fn status(&self, hash: &Hash) -> Result<PayloadStatus, StoreError> {
            Payloads::status(self.store, hash)
        }
    }

    #[test]
    fn a_retried_phase_edit_refused_when_its_original_committed_after_the_lookup_is_caught() {
        use baley_core::scope::PhaseFields;

        let (_dir, store) = approved();
        let goal = PhaseFields {
            goal: Some("The owner approves every change".into()),
            ..PhaseFields::default()
        };
        let edit = command(SUBMIT_COMMAND, R4, 4, T4);
        // The original records E_B and then phase 2 is withdrawn, so the
        // retry's check refuses an edit its original already recorded.
        let racing = CommitsAtLookup {
            store: &store,
            land: RefCell::new(Some(Box::new(|| {
                submit_phase_edit(&store, &edit, 2, &goal, None).unwrap();
                append(
                    &store,
                    R6,
                    T5,
                    vec![
                        phase_withdrawn(2),
                        story_reassigned(2, 1),
                        phase_reordered(&[1]),
                    ],
                );
            }))),
        };

        let retry = submit_phase_edit(&racing, &edit, 2, &goal, None);
        assert!(racing.land.borrow().is_none(), "the lookup was not made");
        assert_eq!(retry, Ok(Hash::from_hex(E_B_DIGEST).unwrap()));
        let drafts = of_type(&history(&store), "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["digest"] == E_B_DIGEST)
            .count();
        assert_eq!(drafts, 1);
    }

    #[test]
    fn a_phase_edit_answered_with_a_digest_its_replay_never_recorded_is_caught() {
        use baley_core::scope::PhaseFields;

        let (_dir, store) = approved();
        let goal = PhaseFields {
            goal: Some("The owner approves every change".into()),
            ..PhaseFields::default()
        };
        let edit = command(SUBMIT_COMMAND, R4, 4, T4);
        submit_phase_edit(&store, &edit, 2, &goal, None).unwrap();
        let before = head(&store);

        let name = PhaseFields {
            name: Some("Approvals".into()),
            ..PhaseFields::default()
        };
        let other = submit_phase_edit(&store, &edit, 2, &name, None).unwrap_err();
        assert_eq!(other["status"], "refused", "{other}");
        assert_eq!(other["code"], "request-id-reuse", "{other}");
        assert_eq!(other["slot"], "request_id", "{other}");
        assert_eq!(head(&store), before);

        let again = submit_phase_edit(&store, &edit, 2, &goal, None);
        assert_eq!(again, Ok(Hash::from_hex(E_B_DIGEST).unwrap()));
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_reused_id_purged_body_or_moved_or_busy_ledger_answered_wrongly_is_caught() {
        let (_dir, store) = store();
        let mismatch = StoreError::Refused(StoreRefusal::RequestDigestMismatch {
            request_id: RequestId(R1.into()),
        });
        let value = answer(Err(mismatch), &store);
        assert_eq!(value["status"], "refused", "{value}");
        assert_eq!(value["code"], "request-id-reuse", "{value}");
        assert_eq!(value["slot"], "request_id", "{value}");

        let tombstoned = StoreError::Refused(StoreRefusal::PayloadTombstoned(Hash([5; 32])));
        assert_failed(
            &answer(Err(tombstoned), &store),
            "text-purged",
            "submission",
            false,
        );
        let moved = StoreError::Stale(StaleInput::Absence(Absence::Event(start_approval())));
        assert_failed(&answer(Err(moved), &store), "ledger-busy", "ledger", true);
        assert_failed(
            &answer(Err(StoreError::Busy), &store),
            "ledger-busy",
            "ledger",
            true,
        );
        let down = StoreError::Unavailable("disk full".into());
        assert_failed(
            &answer(Err(down), &store),
            "ledger-unavailable",
            "ledger",
            false,
        );
    }

    #[test]
    fn a_submission_bound_to_an_order_that_is_not_the_start_planners_is_caught() {
        use baley_core::policy::Role;
        use baley_core::work_order::{
            IssuedOrder, WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION, issued_payload,
        };

        use crate::mcp::work_order::tests::{I0, I1};

        let route = json!({});
        let order = |id: &str, role: Role, subject: &str| NewEvent {
            stream: StreamName("roadmap".into()),
            type_name: WORK_ORDER_ISSUED.into(),
            type_version: WORK_ORDER_ISSUED_VERSION,
            git: None,
            payload: issued_payload(&IssuedOrder {
                id,
                request: id,
                parent: None,
                role,
                subject,
                attempt: 1,
                route: &route,
                agent: "bal-checker-high",
                instructions: &[],
                inputs: &[],
            }),
            attachments: vec![],
        };
        let (_dir, store) = store();
        append(
            &store,
            R1,
            T1,
            vec![
                order(I0, Role::Checker, "start"),
                order(I1, Role::Planner, "phase/2"),
            ],
        );
        let before = head(&store);
        for id in [I0, I1] {
            let refusal = start_order_check(&store, &project(), id).unwrap_err();
            assert_eq!(refusal["status"], "refused", "{id}: {refusal}");
            assert_eq!(refusal["code"], "wrong-work-order", "{id}: {refusal}");
            assert_eq!(refusal["slot"], "work_order", "{id}: {refusal}");
        }
        assert_eq!(head(&store), before);
    }

    /// The JSON body a `scope.submitted` attaches.
    fn submitted_body(store: &baley_store_sqlite::SqliteStore, event: &Event) -> Value {
        let body = PayloadRef::from_value(&event.payload["body"]).unwrap();
        let PayloadBody::Present(mut reader) = store.open(&body.hash).unwrap() else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn on_stream<'a>(events: &'a [Event], stream: &str) -> Vec<&'a Event> {
        events
            .iter()
            .filter(|event| event.stream == stream)
            .collect()
    }

    #[test]
    fn a_continuation_recorded_apart_from_its_edit_is_caught() {
        use baley_core::scope::{ChangeKind, PHASE_VIEW, PhaseFields, phase_key};

        use crate::planning::fixtures::store_c;
        use crate::scope::approve::{self, APPROVE_COMMAND};

        const O: &str = "1f2e3d4c-5b6a-4978-8695-a4b3c2d1e0f1";
        let (_dir, store) = store_c();
        let stories = PhaseFields {
            stories: Some(vec![1, 4]),
            ..PhaseFields::default()
        };
        let continuation = PlanningContinuation {
            stories: &[1, 4],
            work_order: O,
            documents: &[],
            absences: &[],
        };
        let edit = command(SUBMIT_COMMAND, R1, 1, T4);
        let digest = submit_phase_edit(&store, &edit, 1, &stories, Some(continuation)).unwrap();

        let events = history(&store);
        let drafts: Vec<&Event> = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["kind"] == "phase-edit")
            .collect();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].payload["digest"], digest.to_hex());
        assert_eq!(
            submitted_body(&store, drafts[0]),
            json!({"kind": "phase-edit", "phase": 1, "stories": [1, 4]})
        );
        let continuations = of_type(&events, "plan.continuation");
        assert_eq!(continuations.len(), 1);
        assert_eq!(continuations[0].stream, "phase/1");
        assert_eq!(
            continuations[0].payload,
            json!({"phase": 1, "stories": [1, 4], "edit": digest.to_hex(), "work_order": O})
        );
        assert_eq!(continuations[0].request_id, drafts[0].request_id);
        assert_eq!(continuations[0].request_id.0, R1);

        // Phase 2's document as the caller read it, before an approved edit
        // of phase 2 moves it.
        let read = store
            .get(&project(), PHASE_VIEW, &phase_key(2))
            .unwrap()
            .expect("phase 2 is held");
        let observed = [ObservedDocument {
            view: PHASE_VIEW.into(),
            key: phase_key(2),
            produced_seq: Some(read.produced_seq),
        }];
        let e_b = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R2, T5).unwrap();
        let judged = approve::judge(&store, &project(), digest_of(&e_b)).unwrap();
        approve::record(
            &store,
            &command(APPROVE_COMMAND, R3, 3, T5),
            Some(&judged),
            "Ada Owner",
        )
        .unwrap();
        let before = head(&store);
        let continuation = PlanningContinuation {
            documents: &observed,
            ..continuation
        };
        let busy = submit_phase_edit(
            &store,
            &command(SUBMIT_COMMAND, R4, 1, T6),
            1,
            &stories,
            Some(continuation),
        )
        .unwrap_err();
        assert_failed(&busy, "ledger-busy", "ledger", true);
        assert_eq!(head(&store), before);
        assert_eq!(of_type(&history(&store), "plan.continuation").len(), 1);
    }

    #[test]
    fn a_phase_edit_recorded_differently_once_continuations_exist_is_caught() {
        use baley_core::scope::PhaseFields;

        use crate::planning::fixtures::store_c;

        let (_dir, store) = store_c();
        let goal = PhaseFields {
            goal: Some("The owner approves every change".into()),
            ..PhaseFields::default()
        };
        let digest =
            submit_phase_edit(&store, &command(SUBMIT_COMMAND, R1, 1, T4), 2, &goal, None).unwrap();
        assert_eq!(digest.to_hex(), E_B_DIGEST);
        let events = history(&store);
        let drafts = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["digest"] == E_B_DIGEST)
            .count();
        assert_eq!(drafts, 1);
        assert!(on_stream(&events, "phase/2").is_empty());
        assert!(of_type(&events, "plan.continuation").is_empty());
    }
}
