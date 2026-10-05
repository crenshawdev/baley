//! `capture` on `baley_apply`: one note or story recorded in the ledger as
//! `capture.recorded` on the project's `capture` stream, under the command
//! write preparation returned (design 0014 section 5, SUP-R1).

use baley_core::capture::{
    CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM, CaptureKind, TextForm, capture_id,
    inline_payload, judge_phase, stored_payload, text_form,
};
use baley_store::{
    Command, Decision, Ledger, NewEvent, Observed, OutcomeKind, Recorded, RetentionClass,
    StoreError, StreamName,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;

/// The command kind every capture is recorded under.
pub const CAPTURE_COMMAND: &str = "capture.record";

/// Records one capture of `kind`, `text` and `phase` under `command`, the
/// command preparation returned, in one domain transaction.
///
/// The command is used as given, so the actor is Baley and the caller is the
/// server caller preparation put on it. A named phase is refused inside the
/// transaction, recording `command.completed` and nothing else. Text over
/// the inline limit is stored as a `record` payload the event attaches. The
/// answer is the receipt, which never holds the text: a long receipt would be
/// stored as its own answer payload that the capture's purge never releases.
/// The store's result comes back unchanged.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    kind: CaptureKind,
    text: &str,
    phase: Option<u32>,
) -> Result<Recorded, StoreError> {
    let id = capture_id(&command.request_id.0, kind, text, phase);
    store.transact(command, &mut |tx| {
        // Build 4 reads its `phase` view here; until then no phase exists.
        if let (Err(code), Some(named)) = (judge_phase(phase, false), phase) {
            return Ok(decision(
                OutcomeKind::Refused,
                Refusal::new(
                    code,
                    format!(
                        "this project has no phase {named}, so the capture was not recorded. Capture it without a phase"
                    ),
                )
                .slot("phase")
                .phase(named)
                .value(),
            ));
        }
        let form = text_form(text);
        let (payload, attachments) = match form {
            TextForm::Inline => (inline_payload(&id, kind, phase, text), Vec::new()),
            TextForm::Payload => {
                let body = tx.put_payload(text.as_bytes(), RetentionClass::Record)?;
                (stored_payload(&id, kind, phase, &body), vec![body])
            }
        };
        tx.append(NewEvent {
            stream: StreamName(CAPTURE_STREAM.into()),
            type_name: CAPTURE_RECORDED.into(),
            type_version: CAPTURE_RECORDED_VERSION,
            git: None,
            payload,
            attachments,
        })?;
        Ok(decision(
            OutcomeKind::Done,
            json!({
                "status": "ok",
                "id": id,
                "kind": kind.as_str(),
                "phase": phase,
                "bytes": text.len(),
                "form": form.as_str(),
                "recorded_at": command.recorded_at,
            }),
        ))
    })
}

/// A capture's decision: it observes nothing, carries no git facts and is not
/// sensitive.
fn decision(kind: OutcomeKind, answer: Value) -> Decision {
    Decision {
        kind,
        answer,
        sensitive: false,
        observed: Observed::default(),
        git: None,
    }
}

#[cfg(test)]
mod tests {
    use baley_core::capture::{CaptureKind, capture_id};
    use baley_store::{
        Actor, Admin, Answer, Caller, CommandKind, Event, Hash, HistoryFilter, Ledger, OutcomeKind,
        PageRequest, PayloadBody, PayloadRef, Payloads, ProjectId, Recorded,
        Refusal as StoreRefusal, RequestId, ServerCaller, StoreError,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};
    use std::io::Read;

    use super::*;
    use crate::mcp::prepare::{WriteRequest, prepared_command};

    const T0: &str = "2026-10-05T09:00:00Z";
    const T1: &str = "2026-10-05T09:00:01Z";
    const T2: &str = "2026-10-05T09:00:02Z";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    const OTHER_REQUEST: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

    fn project() -> ProjectId {
        ProjectId(ID.into())
    }

    /// A real store in a fresh temporary directory, holding the project.
    fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        store.create_project(&project(), "sample", T0).unwrap();
        (dir, store)
    }

    fn caller() -> ServerCaller {
        ServerCaller::new("/real/r", "/real/r/p", "claude-code", SESSION, &json!(7)).unwrap()
    }

    fn command(request_id: &str, digest: u8, at: &str) -> Command {
        let request = WriteRequest {
            kind: CommandKind(CAPTURE_COMMAND.into()),
            request_id: RequestId(request_id.into()),
            digest: Hash([digest; 32]),
        };
        prepared_command(&project(), &request, 0, at, &caller())
    }

    fn history(store: &SqliteStore) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .history(&project(), 1..=1000, &HistoryFilter::default(), page)
            .unwrap()
            .items
    }

    fn of_type<'a>(events: &'a [Event], type_name: &str) -> Vec<&'a Event> {
        events
            .iter()
            .filter(|event| event.type_name == type_name)
            .collect()
    }

    fn head(store: &SqliteStore) -> u64 {
        store.head(&project()).unwrap().map_or(0, |head| head.seq)
    }

    fn answer(recorded: &Recorded) -> (OutcomeKind, Value) {
        let outcome = match recorded {
            Recorded::New { outcome, .. } | Recorded::Replayed { outcome } => outcome,
        };
        match &outcome.answer {
            Answer::Inline(value) => (outcome.kind, value.clone()),
            other => panic!("expected an inline answer, got {other:?}"),
        }
    }

    /// 1,024 four-byte characters: exactly the 4,096-byte limit.
    fn at_limit() -> String {
        "🦀".repeat(1024)
    }

    fn over_limit() -> String {
        format!("{}x", at_limit())
    }

    #[test]
    fn a_threshold_counted_in_characters_or_a_long_text_left_unattached_is_caught() {
        let (_dir, store) = store();
        let short = at_limit();
        record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            &short,
            None,
        )
        .unwrap();
        let long = over_limit();
        record(
            &store,
            &command(OTHER_REQUEST, 2, T1),
            CaptureKind::Note,
            &long,
            None,
        )
        .unwrap();

        let events = history(&store);
        let captures = of_type(&events, "capture.recorded");
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0].stream, "capture");
        assert_eq!(captures[0].payload["text"], json!(short));
        assert!(captures[0].payload.get("body").is_none());

        assert!(captures[1].payload.get("text").is_none());
        let body = PayloadRef::from_value(&captures[1].payload["body"]).expect("a reference");
        assert_eq!(body.class, RetentionClass::Record);
        assert_eq!(body.bytes, 4097);
        let PayloadBody::Present(mut reader) = store.open(&body.hash).unwrap() else {
            panic!("the body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, long.as_bytes());
    }

    #[test]
    fn a_capture_recorded_as_the_owner_or_without_the_server_caller_is_caught() {
        let (_dir, store) = store();
        record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Story,
            "x",
            None,
        )
        .unwrap();
        let events = history(&store);
        for type_name in ["capture.recorded", "command.completed"] {
            let found = of_type(&events, type_name);
            assert_eq!(found.len(), 1, "{type_name}");
            assert_eq!(found[0].actor, Actor::Baley, "{type_name}");
            assert_eq!(
                found[0].caller,
                Some(Caller::Server(caller())),
                "{type_name}"
            );
        }
    }

    #[test]
    fn a_receipt_that_carries_the_text_or_misses_one_of_its_six_fields_is_caught() {
        let (_dir, store) = store();
        let short = "keep this note";
        let recorded = record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            short,
            None,
        )
        .unwrap();
        let (kind, receipt) = answer(&recorded);
        assert_eq!(kind, OutcomeKind::Done);
        assert_eq!(
            receipt,
            json!({"status": "ok",
                "id": capture_id(REQUEST, CaptureKind::Note, short, None),
                "kind": "note", "phase": null, "bytes": 14, "form": "inline",
                "recorded_at": T1})
        );

        let long = over_limit();
        let recorded = record(
            &store,
            &command(OTHER_REQUEST, 2, T1),
            CaptureKind::Story,
            &long,
            None,
        )
        .unwrap();
        let (_, receipt) = answer(&recorded);
        assert_eq!(receipt["form"], "payload");
        assert_eq!(receipt["bytes"], 4097);
        let serialized = serde_json::to_string(&receipt).unwrap();
        assert!(!serialized.contains("🦀"), "{serialized}");
    }

    #[test]
    fn a_replay_that_records_a_second_capture_or_rebuilds_its_receipt_is_caught() {
        let (_dir, store) = store();
        let first = record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            "x",
            None,
        )
        .unwrap();
        let again = record(
            &store,
            &command(REQUEST, 1, T2),
            CaptureKind::Note,
            "x",
            None,
        )
        .unwrap();
        assert!(matches!(again, Recorded::Replayed { .. }), "{again:?}");
        assert_eq!(answer(&again), answer(&first));
        assert_eq!(answer(&again).1["recorded_at"], T1);
        assert_eq!(of_type(&history(&store), "capture.recorded").len(), 1);
    }

    #[test]
    fn a_named_phase_recorded_as_a_capture_or_its_long_text_stored_anyway_is_caught() {
        let (_dir, store) = store();
        let before = head(&store);
        let long = over_limit();
        let recorded = record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Story,
            &long,
            Some(3),
        )
        .unwrap();
        assert!(matches!(recorded, Recorded::New { .. }), "{recorded:?}");
        let (kind, refusal) = answer(&recorded);
        assert_eq!(kind, OutcomeKind::Refused);
        assert_eq!(refusal["status"], "refused");
        assert_eq!(refusal["code"], "no-such-phase");
        assert_eq!(refusal["slot"], "phase");

        let events = history(&store);
        assert!(of_type(&events, "capture.recorded").is_empty());
        assert_eq!(of_type(&events, "command.completed").len(), 1);
        assert_eq!(head(&store), before + 1);
        for event in &events {
            assert!(event.payload.get("body").is_none(), "{event:?}");
        }
    }

    #[test]
    fn changed_input_accepted_under_one_request_id_is_caught() {
        let (_dir, store) = store();
        record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            "x",
            None,
        )
        .unwrap();
        let before = head(&store);
        let reused = record(
            &store,
            &command(REQUEST, 2, T2),
            CaptureKind::Note,
            "y",
            None,
        );
        assert!(
            matches!(
                reused,
                Err(StoreError::Refused(
                    StoreRefusal::RequestDigestMismatch { .. }
                ))
            ),
            "{reused:?}"
        );
        assert_eq!(head(&store), before);
    }

    #[test]
    fn two_requests_with_the_same_text_sharing_a_capture_id_is_caught() {
        let (_dir, store) = store();
        let one = record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            "x",
            None,
        )
        .unwrap();
        let two = record(
            &store,
            &command(OTHER_REQUEST, 1, T1),
            CaptureKind::Note,
            "x",
            None,
        )
        .unwrap();
        assert_ne!(answer(&one).1["id"], answer(&two).1["id"]);
    }

    #[test]
    fn purged_bytes_stored_again_by_a_new_capture_is_caught() {
        let (_dir, store) = store();
        let long = over_limit();
        record(
            &store,
            &command(REQUEST, 1, T1),
            CaptureKind::Note,
            &long,
            None,
        )
        .unwrap();
        let captured = of_type(&history(&store), "capture.recorded")[0].clone();
        let body = PayloadRef::from_value(&captured.payload["body"]).unwrap();
        let purge = Command {
            project: project(),
            kind: CommandKind("payload.purge".into()),
            request_id: RequestId("5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b".into()),
            digest: Hash([9; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T1.into(),
            actor: Actor::Owner,
            caller: None,
        };
        store
            .purge(&purge, &[body.hash], "pasted a secret")
            .unwrap();
        let before = head(&store);

        let again = record(
            &store,
            &command(OTHER_REQUEST, 2, T2),
            CaptureKind::Note,
            &long,
            None,
        );
        assert!(
            matches!(
                again,
                Err(StoreError::Refused(StoreRefusal::PayloadTombstoned(_)))
            ),
            "{again:?}"
        );
        assert_eq!(head(&store), before);
    }
}
