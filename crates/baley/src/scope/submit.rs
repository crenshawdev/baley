//! A start submission recorded as a draft (design 0004 section 5): its body
//! stored as a `record` payload and one `scope.submitted` on the `project`
//! stream, in the submission's own domain transaction. Accepted scope does
//! not change until the owner approves the draft by its digest.

use std::collections::BTreeMap;

use baley_core::scope::{
    PROJECT_ALREADY_STARTED, PROJECT_STREAM, SCOPE_APPROVED, SCOPE_SUBMITTED,
    SCOPE_SUBMITTED_VERSION, START_KIND, Submission, submitted_payload,
};
use baley_store::{
    Absence, Command, Decision, EVENT_PAGE_BOUND, EventMatch, HistoryFilter, Ledger, NewEvent,
    Observed, OutcomeKind, PageRequest, Payloads, ProjectId, Recorded, Refusal as StoreRefusal,
    RetentionClass, StoreError, StreamName,
};
use serde_json::{Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::ledger::answer::{AnswerUnread, answer_value};
use crate::mcp::capture::{PLACE_LEDGER, REQUEST_ID_REUSE, TEXT_PURGED, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;

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
            payload: submitted_payload(submission.digest(), START_KIND, &body),
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
        Err(StoreError::Refused(StoreRefusal::RequestDigestMismatch { .. })) => Refusal::new(
            REQUEST_ID_REUSE,
            "this request_id was already used for a different scope call, so nothing was recorded. A new call needs a new request_id",
        )
        .slot("request_id")
        .value(),
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
    use std::io::Read;

    use baley_core::scope::{DRAFT_NEWEST_INDEX, DRAFT_VIEW, judge_submission};
    use baley_store::{
        Answer, Hash, IndexQuery, KeyValue, PayloadBody, PayloadRef, RequestId, StaleInput, Views,
    };

    use super::*;
    use crate::scope::fixtures::*;

    fn submit(
        store: &impl Ledger,
        value: &Value,
        request: &str,
        digest: u8,
        at: &str,
    ) -> Result<Recorded, StoreError> {
        let submission = judge_submission(value).unwrap();
        record(
            store,
            &command(SUBMIT_COMMAND, request, digest, at),
            &submission,
        )
    }

    fn receipt(recorded: &Recorded) -> Value {
        let (Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) = recorded;
        match &outcome.answer {
            Answer::Inline(value) => value.clone(),
            other => panic!("expected an inline answer, got {other:?}"),
        }
    }

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
            json!({"digest": M_DIGEST, "kind": "start",
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
}
