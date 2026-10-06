//! `capture` on `baley_apply`: one note or story recorded in the ledger as
//! `capture.recorded` on the project's `capture` stream, under the command
//! write preparation returned (design 0014 section 5, SUP-R1). Every event
//! the call appends carries the registry's evidence for the instruction the
//! session named.

use std::num::NonZeroU32;
use std::path::Path;

use baley_core::capture::{
    BLANK_TEXT, CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM, CaptureKind, TextForm,
    UNKNOWN_KIND, UnknownKind, capture_id, inline_payload, is_blank, judge_kind, judge_phase,
    stored_payload, text_form,
};
use baley_core::policy::Host;
use baley_store::{
    Command, CommandKind, Decision, InstructionEvidence, Ledger, NewEvent, Observed, OutcomeKind,
    Payloads, Recorded, Refusal as StoreRefusal, RequestId, RetentionClass, ServerCaller,
    StoreError, StreamName, request_digest,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::{Envelope, LEDGER_BUSY, Refusal};
use crate::ledger::answer::{AnswerUnread, answer_value};
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, WriteRequest, prepare};
use crate::process::Process;

/// The command kind every capture is recorded under.
pub const CAPTURE_COMMAND: &str = "capture.record";

/// The code for a capture whose exact bytes were purged before.
pub const TEXT_PURGED: &str = "text-purged";

/// The code for a request id already used for a different capture.
pub const REQUEST_ID_REUSE: &str = "request-id-reuse";

/// The place a ledger fault names.
pub(crate) const PLACE_LEDGER: &str = "ledger";

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `capture` takes beside its `operation`. Unknown fields are
/// refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureShape {
    /// A fresh UUID for this capture in lowercase hyphenated form. A retry of
    /// the same capture sends the same id.
    pub request_id: String,
    /// `note` or `story`.
    pub kind: String,
    /// The text to keep. It must not be blank.
    pub text: String,
    /// The phase the capture is about, numbered from 1.
    pub phase: Option<NonZeroU32>,
    /// The identity of the instruction the session followed, such as
    /// `bal-capture`.
    pub instruction: Option<String>,
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum CaptureCall {
    #[serde(rename = "capture")]
    Capture(CaptureShape),
}

/// A capture whose arguments passed every check made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedCapture {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The capture's kind.
    pub kind: CaptureKind,
    /// The text, as sent.
    pub text: String,
    /// The named phase, if any.
    pub phase: Option<u32>,
    /// The registry's evidence for the instruction the session named, if any.
    pub instruction: Option<InstructionEvidence>,
}

/// Judges `capture`'s arguments before anything is prepared or recorded:
/// the shape, then the request id, the kind, the text and the instruction,
/// in that order. A fault is the `refused` answer, which records nothing, so
/// a fixed retry may reuse its request id.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedCapture, Value> {
    let shape = match serde_json::from_value::<CaptureCall>(arguments.clone()) {
        Ok(CaptureCall::Capture(shape)) => shape,
        Err(error) => {
            return Err(Refusal::new("invalid-arguments", error.to_string())
                .slot("arguments")
                .value());
        }
    };
    // One spelling per id, so a retry cannot miss its replay by changing
    // case or wrapping.
    let canonical = uuid::Uuid::parse_str(&shape.request_id)
        .map(|parsed| parsed.hyphenated().to_string())
        .ok();
    if canonical.as_deref() != Some(shape.request_id.as_str()) {
        return Err(Refusal::new(
            "invalid-arguments",
            "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same capture sends the same id",
        )
        .slot("request_id")
        .value());
    }
    let kind = match judge_kind(&shape.kind) {
        Ok(kind) => kind,
        Err(unknown) => {
            let reason = match unknown {
                UnknownKind::Obsolete(name) => format!(
                    "`{name}` is no longer a capture kind. A capture is a `note` or a `story`"
                ),
                UnknownKind::Other => "a capture is a `note` or a `story`".to_string(),
            };
            return Err(Refusal::new(UNKNOWN_KIND, reason).slot("kind").value());
        }
    };
    if is_blank(&shape.text) {
        return Err(
            Refusal::new(BLANK_TEXT, "the capture text is empty or only whitespace")
                .slot("text")
                .value(),
        );
    }
    let instruction = match shape
        .instruction
        .as_deref()
        .map(crate::instruction::evidence)
    {
        None => None,
        Some(Ok(evidence)) => Some(evidence),
        // The registry's refusal names its read's `identity` argument; here
        // the session sent the identity as `instruction`.
        Some(Err(mut refusal)) => {
            refusal["slot"] = json!("instruction");
            return Err(refusal);
        }
    };
    let phase = shape.phase.map(NonZeroU32::get);
    let digest = request_digest(&json!({
        "command": CAPTURE_COMMAND,
        "kind": kind.as_str(),
        "text": shape.text,
        "phase": phase,
        "instruction": shape.instruction,
    }))
    // Strings and a u32 always have a canonical form.
    .expect("capture arguments are canonical");
    Ok(JudgedCapture {
        request: WriteRequest {
            kind: CommandKind(CAPTURE_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        kind,
        text: shape.text,
        phase,
        instruction,
    })
}

/// The caller a capture is prepared under: `caller` with `instruction`'s
/// evidence attached, or `caller` as it is when the session named none.
/// Preparation records checkout admission, the policy step and the command
/// under this one caller, so every event of the call carries the evidence.
fn evidenced(caller: &ServerCaller, instruction: Option<&InstructionEvidence>) -> ServerCaller {
    match instruction {
        None => caller.clone(),
        // The gate's caller carries no evidence, so this is its only entry.
        Some(evidence) => caller
            .clone()
            .with_instructions(vec![evidence.clone()])
            .expect("one entry is within the caller's instruction limit"),
    }
}

/// A function pointer keeps preparation from capturing a different caller.
type Prepare<S, T> =
    fn(Option<&S>, &ServerCaller, Option<&WriteRequest>, &Path, Host, &str, &mut dyn Process) -> T;

/// Selects the caller handed to preparation before it can append any events.
#[allow(clippy::too_many_arguments)]
fn prepare_capture<S, T>(
    judged: &JudgedCapture,
    caller: &ServerCaller,
    store: Option<&S>,
    config: &Path,
    host: Host,
    at: &str,
    process: &mut dyn Process,
    prepare: Prepare<S, T>,
) -> T {
    let caller = evidenced(caller, judged.instruction.as_ref());
    prepare(
        store,
        &caller,
        Some(&judged.request),
        config,
        host,
        at,
        process,
    )
}

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

/// Serves one `capture` call for the session's `preparation`: the arguments
/// are judged first, and a refusal is answered before anything is prepared
/// or recorded. Then the write is prepared under the caller with the
/// instruction evidence attached, which may record checkout admission and
/// the policy step, and the capture runs in its own transaction. A `failed`
/// preparation answer is returned as it is.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let prepared = match prepare_capture(
        &judged,
        &preparation.caller,
        ledger.map(|ledger| &ledger.store),
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
        prepare,
    ) {
        Ok(prepared) => prepared,
        Err(failed) => return serde_json::to_value(*failed).expect("a failed answer serializes"),
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    answer(
        record(
            &ledger.store,
            &command,
            judged.kind,
            &judged.text,
            judged.phase,
        ),
        &ledger.store,
    )
}

/// The answer for a capture's transaction result. A new or replayed outcome
/// answers the recorded answer, read through `payloads` when it was stored. A
/// recorded answer that cannot be read is a ledger fault, not a domain
/// outcome, so it answers `failed`.
pub fn answer(result: Result<Recorded, StoreError>, payloads: &dyn Payloads) -> Value {
    match result {
        Ok(Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) => {
            match answer_value(&outcome.answer, payloads) {
                Ok(value) => value,
                Err(AnswerUnread::Read(error)) => store_failed(&error, NOT_RECORDED_BUSY),
                Err(AnswerUnread::Gone(_)) => failed(
                    LEDGER_UNAVAILABLE,
                    "the capture is recorded, but its stored answer is no longer held",
                    PLACE_LEDGER,
                ),
                Err(AnswerUnread::Malformed) => failed(
                    LEDGER_UNAVAILABLE,
                    "the capture is recorded, but its stored answer cannot be read",
                    PLACE_LEDGER,
                ),
            }
        }
        Err(StoreError::Refused(StoreRefusal::RequestDigestMismatch { .. })) => Refusal::new(
            REQUEST_ID_REUSE,
            "this request_id was already used for a different capture, so nothing was recorded. A new capture needs a new request_id",
        )
        .slot("request_id")
        .value(),
        Err(StoreError::Refused(StoreRefusal::PayloadTombstoned(_))) => failed(
            TEXT_PURGED,
            "this exact text was purged from the ledger and cannot be stored again, so nothing was recorded",
            "text",
        ),
        Err(error) => store_failed(&error, NOT_RECORDED_BUSY),
    }
}

/// A busy ledger's reason when a capture was not recorded.
const NOT_RECORDED_BUSY: &str = "the ledger is busy, so the capture was not recorded. Try it again";

/// A store fault's answer, with `busy` as a busy ledger's reason: only a
/// busy ledger may be retried unchanged. `document` maps its read faults
/// here too.
pub(crate) fn store_failed(error: &StoreError, busy: &str) -> Value {
    match error {
        StoreError::Busy => failed(LEDGER_BUSY, busy, PLACE_LEDGER),
        error => failed(LEDGER_UNAVAILABLE, error.to_string(), PLACE_LEDGER),
    }
}

/// A `failed` answer, which is never recorded.
pub(crate) fn failed(code: &str, reason: impl Into<String>, place: &str) -> Value {
    serde_json::to_value(Envelope::<Value>::failed(code, reason.into(), place))
        .expect("a failed answer serializes")
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

    fn outcome_answer(recorded: &Recorded) -> (OutcomeKind, Value) {
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
        let (kind, receipt) = outcome_answer(&recorded);
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
        let (_, receipt) = outcome_answer(&recorded);
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
        assert_eq!(outcome_answer(&again), outcome_answer(&first));
        assert_eq!(outcome_answer(&again).1["recorded_at"], T1);
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
        let (kind, refusal) = outcome_answer(&recorded);
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
        assert_ne!(outcome_answer(&one).1["id"], outcome_answer(&two).1["id"]);
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

    fn arguments(extra: Value) -> Value {
        let mut base = json!({"operation": "capture", "request_id": REQUEST,
            "kind": "note", "text": "keep this"});
        for (name, value) in extra.as_object().unwrap() {
            base[name] = value.clone();
        }
        base
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_request_id_in_any_spelling_but_lowercase_hyphenated_accepted_is_caught() {
        for id in [
            "not-a-uuid",
            "9D0C1B7E-2F4A-4B6C-8D1E-3A5B7C9D0E2F",
            "{9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f}",
            "9d0c1b7e2f4a4b6c8d1e3a5b7c9d0e2f",
        ] {
            let refusal = refused(&arguments(json!({ "request_id": id })));
            assert_eq!(refusal["code"], "invalid-arguments", "{id}");
            assert_eq!(refusal["slot"], "request_id", "{id}");
        }
        let judged = judge_arguments(&arguments(json!({}))).unwrap();
        assert_eq!(judged.request.request_id, RequestId(REQUEST.into()));
        assert_eq!(judged.request.kind, CommandKind("capture.record".into()));
    }

    #[test]
    fn an_obsolete_kind_answered_as_invalid_arguments_or_without_its_name_is_caught() {
        for kind in ["todo", "seed"] {
            let refusal = refused(&arguments(json!({ "kind": kind })));
            assert_eq!(refusal["code"], "unknown-kind", "{kind}");
            assert_eq!(refusal["slot"], "kind");
            let reason = refusal["reason"].as_str().unwrap();
            assert!(reason.contains(&format!("`{kind}`")), "{reason}");
        }
        let refusal = refused(&arguments(json!({"kind": "Note"})));
        assert_eq!(refusal["code"], "unknown-kind");
        assert!(!refusal["reason"].as_str().unwrap().contains("Note"));
    }

    #[test]
    fn whitespace_only_text_accepted_is_caught() {
        let refusal = refused(&arguments(json!({"text": " \t\n "})));
        assert_eq!(refusal["code"], "blank-text");
        assert_eq!(refusal["slot"], "text");
    }

    #[test]
    fn a_by_field_or_phase_zero_accepted_is_caught() {
        for extra in [
            json!({"by": "someone"}),
            json!({"phase": 0}),
            json!({"phase": 1.5}),
        ] {
            let refusal = refused(&arguments(extra.clone()));
            assert_eq!(refusal["code"], "invalid-arguments", "{extra}");
            assert_eq!(refusal["slot"], "arguments", "{extra}");
        }
        let judged = judge_arguments(&arguments(json!({"phase": 2}))).unwrap();
        assert_eq!(judged.phase, Some(2));
    }

    #[test]
    fn a_digest_that_ignores_the_kind_text_phase_or_instruction_is_caught() {
        let digest = |extra: Value| judge_arguments(&arguments(extra)).unwrap().request.digest;
        let base = digest(json!({"instruction": "bal-help"}));
        assert_eq!(base, digest(json!({"instruction": "bal-help"})));
        for changed in [
            json!({"instruction": "bal-help", "kind": "story"}),
            json!({"instruction": "bal-help", "text": "keep that"}),
            json!({"instruction": "bal-help", "phase": 4}),
            json!({"instruction": "bal-read-contract"}),
            json!({}),
        ] {
            assert_ne!(digest(changed.clone()), base, "{changed}");
        }
    }

    /// Runs judged arguments through preparation's command and the
    /// transaction, as the served operation does.
    fn capture(store: &SqliteStore, arguments: &Value, at: &str) -> Result<Recorded, StoreError> {
        let judged = judge_arguments(arguments).unwrap();
        let command = prepared_command(&project(), &judged.request, 0, at, &caller());
        record(store, &command, judged.kind, &judged.text, judged.phase)
    }

    #[test]
    fn changed_text_accepted_under_one_request_id_through_its_arguments_is_caught() {
        let (_dir, store) = store();
        capture(&store, &arguments(json!({})), T1).unwrap();
        let reused = capture(&store, &arguments(json!({"text": "keep that"})), T2);
        assert!(
            matches!(
                reused,
                Err(StoreError::Refused(
                    StoreRefusal::RequestDigestMismatch { .. }
                ))
            ),
            "{reused:?}"
        );
        assert_eq!(of_type(&history(&store), "capture.recorded").len(), 1);
    }

    #[test]
    fn the_same_arguments_twice_recorded_as_two_captures_is_caught() {
        let (_dir, store) = store();
        capture(&store, &arguments(json!({})), T1).unwrap();
        let again = capture(&store, &arguments(json!({})), T2).unwrap();
        assert!(matches!(again, Recorded::Replayed { .. }), "{again:?}");
        assert_eq!(of_type(&history(&store), "capture.recorded").len(), 1);
    }

    /// A payload seam whose every read finds the ledger busy.
    struct BusyPayloads;

    impl Payloads for BusyPayloads {
        fn open(&self, _hash: &Hash) -> Result<PayloadBody<'_>, StoreError> {
            Err(StoreError::Busy)
        }

        fn status(&self, _hash: &Hash) -> Result<baley_store::PayloadStatus, StoreError> {
            Err(StoreError::Busy)
        }
    }

    fn new(kind: OutcomeKind, answer: Answer) -> Result<Recorded, StoreError> {
        Ok(Recorded::New {
            outcome: baley_store::Outcome { kind, answer },
            head: baley_store::Head {
                seq: 4,
                hash: Hash([1; 32]),
            },
        })
    }

    fn assert_failed(value: &Value, code: &str, place: &str, retryable: bool) {
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], code, "{value}");
        assert_eq!(value["place"], place, "{value}");
        assert_eq!(value["recorded"], false, "{value}");
        assert_eq!(value["retryable"], retryable, "{value}");
    }

    #[test]
    fn an_inline_or_replayed_answer_not_returned_as_recorded_is_caught() {
        let receipt = json!({"status": "ok", "id": "c1", "kind": "note", "phase": null,
            "bytes": 1, "form": "inline", "recorded_at": T1});
        let inline = Answer::Inline(receipt.clone());
        assert_eq!(
            answer(new(OutcomeKind::Done, inline.clone()), &BusyPayloads),
            receipt
        );
        let replayed = Ok(Recorded::Replayed {
            outcome: baley_store::Outcome {
                kind: OutcomeKind::Done,
                answer: inline,
            },
        });
        assert_eq!(answer(replayed, &BusyPayloads), receipt);
    }

    #[test]
    fn a_stored_answer_not_read_back_through_the_payloads_is_caught() {
        let (_dir, store) = store();
        let stored = json!({"status": "ok", "id": "c1"});
        let recorded = store
            .transact(&command(REQUEST, 1, T1), &mut |_tx| {
                Ok(Decision {
                    sensitive: true,
                    ..decision(OutcomeKind::Done, stored.clone())
                })
            })
            .unwrap();
        let Recorded::New { outcome, .. } = &recorded else {
            panic!("{recorded:?}");
        };
        assert!(matches!(outcome.answer, Answer::Stored(_)), "{recorded:?}");
        assert_eq!(answer(Ok(recorded), &store), stored);
    }

    #[test]
    fn an_unreadable_answer_answered_as_a_domain_outcome_or_busy_not_retryable_is_caught() {
        let (_dir, store) = store();
        let missing = PayloadRef {
            hash: Hash([5; 32]),
            bytes: 10,
            class: RetentionClass::Record,
        };
        let gone = Answer::Tombstone {
            reference: missing.clone(),
            status: baley_store::PayloadStatus::Purged { reason: "r".into() },
        };
        let malformed = Answer::Inline(json!(["not", "an", "object"]));
        for unreadable in [gone, Answer::Stored(missing.clone()), malformed] {
            let value = answer(new(OutcomeKind::Done, unreadable), &store);
            assert_failed(&value, "ledger-unavailable", "ledger", false);
        }
        let value = answer(
            new(OutcomeKind::Done, Answer::Stored(missing)),
            &BusyPayloads,
        );
        assert_failed(&value, "ledger-busy", "ledger", true);
    }

    #[test]
    fn a_busy_ledger_mapped_to_ledger_unavailable_or_not_retryable_is_caught() {
        let value = answer(Err(StoreError::Busy), &BusyPayloads);
        assert_failed(&value, "ledger-busy", "ledger", true);
    }

    #[test]
    fn a_reused_request_id_answered_as_failed_or_in_another_slot_is_caught() {
        let mismatch = StoreError::Refused(StoreRefusal::RequestDigestMismatch {
            request_id: RequestId(REQUEST.into()),
        });
        let value = answer(Err(mismatch), &BusyPayloads);
        assert_eq!(value["status"], "refused", "{value}");
        assert_eq!(value["code"], "request-id-reuse");
        assert_eq!(value["slot"], "request_id");
        assert!(value.get("retryable").is_none());
    }

    #[test]
    fn purged_text_marked_retryable_or_answered_at_another_place_is_caught() {
        let tombstoned = StoreError::Refused(StoreRefusal::PayloadTombstoned(Hash([5; 32])));
        let value = answer(Err(tombstoned), &BusyPayloads);
        assert_failed(&value, "text-purged", "text", false);
        assert!(value["reason"].as_str().unwrap().contains("purged"));
    }

    #[test]
    fn another_store_error_answered_as_retryable_or_as_a_refusal_is_caught() {
        let down = StoreError::Unavailable("disk full".into());
        let value = answer(Err(down), &BusyPayloads);
        assert_failed(&value, "ledger-unavailable", "ledger", false);
        assert!(value["reason"].as_str().unwrap().contains("disk full"));
    }

    #[test]
    fn an_unknown_instruction_accepted_or_refused_in_the_identity_slot_is_caught() {
        let refusal = refused(&arguments(json!({"instruction": "bal-nothing"})));
        assert_eq!(refusal["code"], "unknown-instruction");
        assert_eq!(refusal["slot"], "instruction");
    }

    #[test]
    fn an_unserved_instruction_accepted_or_refused_without_its_build_is_caught() {
        let refusal = refused(&arguments(json!({"instruction": "bal-plan"})));
        assert_eq!(refusal["code"], "instruction-unavailable");
        assert_eq!(refusal["slot"], "instruction");
        assert_eq!(refusal["details"]["build"], 4);
    }

    /// The caller a capture with `arguments` is prepared under.
    fn prepared_caller(arguments: &Value) -> ServerCaller {
        let judged = judge_arguments(arguments).unwrap();
        observe_preparation(&judged)
    }

    /// Observes preparation's caller without running its external operations.
    fn observe_preparation(judged: &JudgedCapture) -> ServerCaller {
        prepare_capture(
            judged,
            &caller(),
            None::<&SqliteStore>,
            Path::new("/config"),
            Host::ClaudeCode,
            T1,
            &mut crate::process::Recorded::new(),
            |_, selected, _, _, _, _, _| selected.clone(),
        )
    }

    #[test]
    fn evidence_missing_from_the_caller_or_not_the_registrys_is_caught() {
        let formed = prepared_caller(&arguments(json!({"instruction": "bal-help"})));
        let crate::instruction::Lookup::Served { entry, text } =
            crate::instruction::lookup("bal-help")
        else {
            panic!("bal-help is served");
        };
        let [evidence] = formed.instructions() else {
            panic!("expected one entry, got {:?}", formed.instructions());
        };
        assert_eq!(
            (evidence.identity(), evidence.version(), evidence.hash()),
            (entry.identity, text.version, text.hash)
        );
        let base = caller();
        assert_eq!(formed.project_directory(), base.project_directory());
        assert_eq!(formed.working_directory(), base.working_directory());
        assert_eq!(formed.host(), base.host());
        assert_eq!(formed.baley_session(), base.baley_session());
        assert_eq!(formed.call(), base.call());
    }

    #[test]
    fn preparation_receives_a_caller_without_the_captures_instruction_evidence() {
        let evidence = InstructionEvidence::new("bal-capture", "1", &"ab".repeat(32)).unwrap();
        let judged = JudgedCapture {
            request: WriteRequest {
                kind: CommandKind(CAPTURE_COMMAND.into()),
                request_id: RequestId(REQUEST.into()),
                digest: Hash([1; 32]),
            },
            kind: CaptureKind::Note,
            text: "keep this".into(),
            phase: None,
            instruction: Some(evidence.clone()),
        };
        // Observe the input at preparation, before it returns a domain command.
        let observed = observe_preparation(&judged);
        assert_eq!(observed.instructions(), &[evidence]);
        assert_eq!(
            observed,
            caller()
                .with_instructions(judged.instruction.into_iter().collect())
                .unwrap()
        );
    }

    #[test]
    fn evidence_invented_for_a_capture_that_named_no_instruction_is_caught() {
        let formed = prepared_caller(&arguments(json!({})));
        assert!(formed.instructions().is_empty());
        assert_eq!(formed, caller());
    }

    #[test]
    fn a_capture_and_its_completion_recorded_without_the_evidence_is_caught() {
        let (_dir, store) = store();
        let arguments = arguments(json!({"instruction": "bal-help"}));
        let judged = judge_arguments(&arguments).unwrap();
        let formed = prepared_caller(&arguments);
        let command = prepared_command(&project(), &judged.request, 0, T1, &formed);
        record(&store, &command, judged.kind, &judged.text, judged.phase).unwrap();
        let expected = crate::instruction::evidence("bal-help").unwrap();
        let events = history(&store);
        for type_name in ["capture.recorded", "command.completed"] {
            let found = of_type(&events, type_name);
            assert_eq!(found.len(), 1, "{type_name}");
            let Some(Caller::Server(recorded)) = &found[0].caller else {
                panic!("{type_name} has no server caller");
            };
            assert_eq!(
                recorded.instructions(),
                std::slice::from_ref(&expected),
                "{type_name}"
            );
        }
    }
}
