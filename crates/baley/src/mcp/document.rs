//! `document` on `baley_query`: a record read back by identity. A capture's
//! text comes from the `capture` view when inline, or from its body by hash,
//! and a body this project purged answers a tombstone (design 0012
//! operations, design 0014 section 6). A draft by its digest, the accepted
//! roadmap and an accepted story come from `scope::read` (design 0004
//! section 5), the same reads the owner's commands print.

use std::io::Read;
use std::path::Path;

use baley_core::capture::{CAPTURE_ID_INDEX, CAPTURE_VIEW, NO_SUCH_PHASE};
use baley_core::scope::UNKNOWN_DRAFT;
use baley_store::{
    Hash, IndexQuery, KeyValue, PageRequest, PayloadBody, PayloadStatus, Payloads, ProjectId, Views,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::handler::Preparation;
use crate::mcp::parts::{Cut, PART_BOUND, cut};
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, prepare};
use crate::scope::read::{
    self as scope, NO_SUCH_STORY, Preview, PreviewError, ReadError, Roadmap, Story,
};

/// The code for a capture id the project's `capture` view does not hold.
pub const NO_SUCH_CAPTURE: &str = "no-such-capture";

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `document` takes beside its `operation`. Unknown fields are
/// refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentShape {
    /// The record to read, by its kind and id.
    pub identity: Identity,
    /// One-based part for texts over 24,576 bytes; defaults to 1.
    /// Concatenate the returned bodies in order.
    pub part: Option<usize>,
}

/// A record's identity, tagged by its kind. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Identity {
    /// A note or story captured in the current project.
    #[serde(rename = "capture")]
    Capture {
        /// The capture id from the capture's receipt.
        id: String,
    },
    /// A scope draft as its latest submission recorded it.
    #[serde(rename = "draft")]
    Draft {
        /// The draft's digest from the scope-submit receipt.
        digest: String,
    },
    /// The project's accepted roadmap: its description, phases and stories.
    #[serde(rename = "roadmap")]
    Roadmap {},
    /// One accepted story.
    #[serde(rename = "story")]
    Story {
        /// The story id Baley assigned when the scope was approved.
        id: u32,
    },
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum DocumentCall {
    #[serde(rename = "document")]
    Document(DocumentShape),
}

/// Judges `document`'s arguments. A shape fault is `invalid-arguments`,
/// answered before anything is prepared.
pub fn judge_arguments(arguments: &Value) -> Result<DocumentShape, Value> {
    match serde_json::from_value::<DocumentCall>(arguments.clone()) {
        Ok(DocumentCall::Document(shape)) => Ok(shape),
        Err(error) => Err(Refusal::new("invalid-arguments", error.to_string())
            .slot("arguments")
            .value()),
    }
}

/// A capture's document as the `capture` view holds it. Inline text is
/// kept; a body is named by its hash, with this project's purge state.
#[derive(Debug, Deserialize)]
pub struct CaptureDocument {
    kind: String,
    phase: Option<u32>,
    bytes: u64,
    recorded_at: String,
    text: Option<String>,
    hash: Option<String>,
    state: Option<String>,
    reason: Option<String>,
}

impl CaptureDocument {
    /// Reads a view document's body, or none when it is not a capture's.
    pub fn from_value(body: &Value) -> Option<Self> {
        serde_json::from_value(body.clone()).ok()
    }

    /// Where this capture's answer comes from. A body this project purged is
    /// a tombstone and is never opened, even when another project still
    /// holds the same bytes. None when the document names neither text nor
    /// a readable hash.
    pub fn source(&self) -> Option<Source<'_>> {
        match (&self.text, &self.hash, self.state.as_deref()) {
            (Some(text), None, _) => Some(Source::Ready(Content::Text(text))),
            (None, Some(_), Some("purged")) => Some(Source::Ready(Content::Purged(
                self.reason.as_deref().unwrap_or_default(),
            ))),
            (None, Some(hash), _) => Hash::from_hex(hash).map(Source::Open),
            _ => None,
        }
    }
}

/// What reading a capture needs.
#[derive(Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// The answer's content is known from the view.
    Ready(Content<'a>),
    /// The body stored under this hash is to be opened.
    Open(Hash),
}

/// A capture's content: its text, or the reason its body was purged.
#[derive(Debug, PartialEq, Eq)]
pub enum Content<'a> {
    /// The capture's text.
    Text(&'a str),
    /// The body was purged, for this reason.
    Purged(&'a str),
}

/// The answer for `identity`'s `document` with `content` and the requested
/// `part`. Every answer, and every part of a long text, carries the
/// identity, the capture's kind, phase, byte count and recording time. A
/// purged body answers `ok` with a tombstone, since the record exists and a
/// purge is a lasting state.
pub fn answer(
    identity: &Identity,
    document: &CaptureDocument,
    content: Content<'_>,
    part: Option<usize>,
) -> Value {
    let mut answer = json!({"status": "ok", "identity": identity, "kind": document.kind,
        "phase": document.phase, "bytes": document.bytes, "recorded_at": document.recorded_at});
    match content {
        Content::Purged(reason) => {
            answer["tombstone"] = json!({"state": "purged", "reason": reason});
        }
        Content::Text(text) => match cut(text, part) {
            Cut::Whole(text) => answer["text"] = json!(text),
            Cut::Part { body, part, next } => {
                answer["bound"] = json!(PART_BOUND);
                answer["part"] = json!(part);
                answer["body"] = json!(body);
                answer["next"] = json!(next);
            }
            Cut::Absent => return part_not_found(),
        },
    }
    answer
}

/// The refusal for a capture id the project does not hold. It echoes none of
/// the id, so a long or odd id cannot grow the answer.
pub fn no_such_capture() -> Value {
    Refusal::new(
        NO_SUCH_CAPTURE,
        "this project holds no capture with that id",
    )
    .slot("identity")
    .value()
}

/// Serves one `document` call for the session's `preparation`: the
/// arguments are judged first, and a shape fault is answered before anything
/// is prepared. Then the read is prepared, which finds the project and
/// checks the ledger knows it and records nothing, and the capture is read
/// from that project. A `failed` preparation answer is returned as it is.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let shape = match judge_arguments(arguments) {
        Ok(shape) => shape,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // A read reads no settings, so the config folder is never opened.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let prepared = match prepare(
        ledger.map(|ledger| &ledger.store),
        &preparation.caller,
        None,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
    ) {
        Ok(prepared) => prepared,
        Err(failed) => return serde_json::to_value(*failed).expect("a failed answer serializes"),
    };
    let Some(ledger) = ledger else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the read was prepared without a ledger",
            PLACE_LEDGER,
        );
    };
    read(&ledger.store, &prepared.project, &shape)
}

/// A busy ledger's reason when a capture was not read.
const NOT_READ_BUSY: &str = "the ledger is busy, so the capture was not read. Try it again";

/// A busy ledger's reason when scope was not read.
const SCOPE_NOT_READ_BUSY: &str = "the ledger is busy, so the scope was not read. Try it again";

/// Reads the record `shape` names from `project` in `store`. A capture is
/// read from its view document by id, then its body only when the view says
/// this project still holds it; a draft, the roadmap and a story through
/// `scope::read`. Nothing is appended.
pub fn read<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    shape: &DocumentShape,
) -> Value {
    let identity = &shape.identity;
    match identity {
        Identity::Capture { .. } => match find_capture(store, project, shape) {
            Ok(document) => read_document(store, project, shape, &document),
            Err(answer) => answer,
        },
        // A digest not written as the receipt writes it names no draft.
        Identity::Draft { digest } => {
            match Hash::from_hex(digest).filter(|h| h.to_hex() == *digest) {
                Some(hash) => {
                    draft_answer(identity, scope::preview(store, project, hash), shape.part)
                }
                None => draft_answer(identity, Err(PreviewError::UnknownDraft), shape.part),
            }
        }
        Identity::Roadmap {} => {
            roadmap_answer(identity, scope::roadmap(store, project), shape.part)
        }
        Identity::Story { id } => {
            story_answer(identity, scope::story(store, project, *id), shape.part)
        }
    }
}

/// The answer for a draft read: its preview, in parts when large, or why it
/// has none. An unknown draft echoes none of the digest.
pub fn draft_answer(
    identity: &Identity,
    read: Result<Preview, PreviewError>,
    part: Option<usize>,
) -> Value {
    match read {
        Ok(preview) => in_parts(
            json!({"status": "ok", "identity": identity, "kind": preview.kind,
                "digest": preview.digest.to_hex(), "recorded_at": preview.recorded_at,
                "submission": preview.submission}),
            part,
        ),
        Err(PreviewError::UnknownDraft) => Refusal::new(
            UNKNOWN_DRAFT,
            "no submission in this project has that digest",
        )
        .slot("identity")
        .value(),
        Err(PreviewError::Unreadable(reason)) => failed(LEDGER_UNAVAILABLE, reason, PLACE_LEDGER),
        Err(PreviewError::Store(error)) => store_failed(&error, SCOPE_NOT_READ_BUSY),
    }
}

/// The answer for a roadmap read, in parts when large.
pub fn roadmap_answer(
    identity: &Identity,
    read: Result<Roadmap, ReadError>,
    part: Option<usize>,
) -> Value {
    match read {
        Ok(roadmap) => in_parts(
            json!({"status": "ok", "identity": identity, "description": roadmap.description,
                "phases": roadmap.phases, "stories": roadmap.stories}),
            part,
        ),
        Err(error) => read_failed(error),
    }
}

/// The answer for a story read, in parts when large. The story's own status
/// is nested under `story`, so it never stands in for the answer's.
pub fn story_answer(
    identity: &Identity,
    read: Result<Story, ReadError>,
    part: Option<usize>,
) -> Value {
    match read {
        Ok(story) => in_parts(
            json!({"status": "ok", "identity": identity, "story": story}),
            part,
        ),
        Err(error) => read_failed(error),
    }
}

/// Why accepted scope was not read. A missing phase or story echoes none of
/// its number, as a missing capture echoes none of its id.
fn read_failed(error: ReadError) -> Value {
    match error {
        ReadError::NoSuchPhase => Refusal::new(NO_SUCH_PHASE, "this project has no such phase")
            .slot("identity")
            .value(),
        ReadError::NoSuchStory => Refusal::new(NO_SUCH_STORY, "this project has no such story")
            .slot("identity")
            .value(),
        ReadError::BodyGone => failed(
            LEDGER_UNAVAILABLE,
            "the scope is recorded, but a body it names is no longer held",
            PLACE_LEDGER,
        ),
        ReadError::Unreadable(reason) => failed(LEDGER_UNAVAILABLE, reason, PLACE_LEDGER),
        ReadError::Store(error) => store_failed(&error, SCOPE_NOT_READ_BUSY),
    }
}

/// `answer` whole when its JSON text fits one part, or one numbered part of
/// that text, as `help` cuts its answer.
fn in_parts(answer: Value, part: Option<usize>) -> Value {
    let text = serde_json::to_string(&answer).expect("a document answer serializes");
    match cut(&text, part) {
        Cut::Whole(_) => answer,
        Cut::Part { body, part, next } => json!({"status": "ok", "bound": PART_BOUND,
            "part": part, "body": body, "next": next}),
        Cut::Absent => part_not_found(),
    }
}

fn part_not_found() -> Value {
    Refusal::new(
        "document-part-not-found",
        "the requested document part is absent",
    )
    .slot("part")
    .value()
}

fn find_capture<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    shape: &DocumentShape,
) -> Result<CaptureDocument, Value> {
    let Identity::Capture { id } = &shape.identity else {
        return Err(no_such_capture());
    };
    let query = IndexQuery {
        index: CAPTURE_ID_INDEX.into(),
        equals: vec![KeyValue::Text(id.clone())],
        page: PageRequest {
            limit: 1,
            after: None,
        },
    };
    let found = match store.find(project, CAPTURE_VIEW, &query) {
        Ok(page) => page.items.into_iter().next(),
        Err(error) => return Err(store_failed(&error, NOT_READ_BUSY)),
    };
    let Some(found) = found else {
        return Err(no_such_capture());
    };
    CaptureDocument::from_value(&found.body).ok_or_else(unreadable)
}

fn unreadable() -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        "the capture is recorded, but its view document cannot be read",
        PLACE_LEDGER,
    )
}

/// Resolves a view observation's content. A purge after that observation
/// needs the project's current view, since a body's status is global.
fn read_document<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    shape: &DocumentShape,
    document: &CaptureDocument,
) -> Value {
    let hash = match document.source() {
        Some(Source::Ready(content)) => {
            return answer(&shape.identity, document, content, shape.part);
        }
        Some(Source::Open(hash)) => hash,
        None => return unreadable(),
    };
    match store.open(&hash) {
        Err(error) => store_failed(&error, NOT_READ_BUSY),
        Ok(PayloadBody::Gone(status)) => {
            let current;
            let document = if matches!(status, PayloadStatus::Purged { .. }) {
                current = match find_capture(store, project, shape) {
                    Ok(document) => document,
                    Err(answer) => return answer,
                };
                &current
            } else {
                document
            };
            gone_answer(shape, document, &status)
        }
        Ok(PayloadBody::Present(mut reader)) => {
            let mut bytes = Vec::new();
            if let Err(error) = reader.read_to_end(&mut bytes) {
                return failed(LEDGER_UNAVAILABLE, error.to_string(), PLACE_LEDGER);
            }
            // The body was stored from the capture's UTF-8 text.
            match String::from_utf8(bytes) {
                Ok(text) => answer(&shape.identity, document, Content::Text(&text), shape.part),
                Err(_) => failed(
                    LEDGER_UNAVAILABLE,
                    "the capture is recorded, but its body is not UTF-8 text",
                    PLACE_LEDGER,
                ),
            }
        }
    }
}

/// A global tombstone needs this project's view to confirm its purge reason.
fn gone_answer(shape: &DocumentShape, document: &CaptureDocument, status: &PayloadStatus) -> Value {
    if let (PayloadStatus::Purged { .. }, Some(Source::Ready(Content::Purged(reason)))) =
        (status, document.source())
    {
        return answer(
            &shape.identity,
            document,
            Content::Purged(reason),
            shape.part,
        );
    }
    failed(
        LEDGER_UNAVAILABLE,
        "the capture is recorded, but its body is not held whole",
        PLACE_LEDGER,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "2026-10-05T09:00:00Z";
    const ID: &str = "c2";

    fn identity() -> Identity {
        Identity::Capture { id: ID.into() }
    }

    fn document(body: Value) -> CaptureDocument {
        let mut base = json!({"seq": 9, "id": ID, "kind": "story", "phase": null,
            "bytes": 4097, "recorded_at": AT});
        for (name, value) in body.as_object().unwrap() {
            base[name] = value.clone();
        }
        CaptureDocument::from_value(&base).expect("a capture document")
    }

    /// The metadata every answer and part carries, written from D-09.
    fn metadata(answer: &Value, bytes: u64) {
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["identity"], json!({"kind": "capture", "id": ID}));
        assert_eq!(answer["kind"], "story");
        assert_eq!(answer["phase"], Value::Null);
        assert_eq!(answer["bytes"], bytes);
        assert_eq!(answer["recorded_at"], AT);
    }

    #[test]
    fn a_purged_body_opened_instead_of_answered_as_a_tombstone_is_caught() {
        let purged = document(json!({"hash": "ab".repeat(32), "state": "purged",
            "reason": "pasted a secret"}));
        assert_eq!(
            purged.source(),
            Some(Source::Ready(Content::Purged("pasted a secret")))
        );
        let present = document(json!({"hash": "ab".repeat(32), "state": "present"}));
        assert_eq!(present.source(), Some(Source::Open(Hash([0xab; 32]))));
    }

    #[test]
    fn a_confirmed_purge_answered_with_the_global_reason_is_caught() {
        let current = document(json!({"hash": "ab".repeat(32), "state": "purged",
            "reason": "this project removed it"}));
        let shape = DocumentShape {
            identity: identity(),
            part: None,
        };
        let status = PayloadStatus::Purged {
            reason: "another project removed it".into(),
        };
        let value = gone_answer(&shape, &current, &status);
        metadata(&value, 4097);
        assert_eq!(
            value["tombstone"],
            json!({"state": "purged", "reason": "this project removed it"})
        );
        assert!(value.get("text").is_none(), "{value}");
    }

    #[test]
    fn an_unconfirmed_purge_answered_with_the_global_reason_is_caught() {
        let shape = DocumentShape {
            identity: identity(),
            part: None,
        };
        let status = PayloadStatus::Purged {
            reason: "another project removed it".into(),
        };
        for body in [
            json!({"hash": "ab".repeat(32), "state": "present"}),
            json!({"text": "still inline"}),
            json!({}),
        ] {
            let current = document(body);
            let value = gone_answer(&shape, &current, &status);
            assert_eq!(value["status"], "failed", "{value}");
            assert_eq!(value["code"], "ledger-unavailable");
            assert_eq!(value["place"], "ledger");
            assert!(value.get("tombstone").is_none(), "{value}");
            assert!(value.get("text").is_none(), "{value}");
        }
    }

    #[test]
    fn an_inline_capture_answered_without_its_text_or_its_metadata_is_caught() {
        let inline = document(json!({"kind": "note", "bytes": 9, "text": "keep this"}));
        let Some(Source::Ready(content)) = inline.source() else {
            panic!("inline text is answered from the view");
        };
        assert_eq!(
            answer(&identity(), &inline, content, None),
            json!({"status": "ok", "identity": {"kind": "capture", "id": ID},
                "kind": "note", "phase": null, "bytes": 9, "recorded_at": AT,
                "text": "keep this"})
        );
    }

    #[test]
    fn a_long_text_part_missing_the_metadata_or_not_joining_back_is_caught() {
        // 24,577 bytes: one byte, then two-byte characters across the bound.
        let text = format!("x{}", "é".repeat(PART_BOUND / 2));
        assert_eq!(text.len(), PART_BOUND + 1);
        let bytes = text.len() as u64;
        let held = document(json!({"bytes": bytes}));
        let mut joined = String::new();
        let mut part = 1;
        loop {
            let value = answer(&identity(), &held, Content::Text(&text), Some(part));
            metadata(&value, bytes);
            assert!(value.get("text").is_none(), "{value}");
            assert_eq!(value["bound"], PART_BOUND);
            assert_eq!(value["part"], part);
            joined.push_str(value["body"].as_str().unwrap());
            match value["next"].as_u64() {
                Some(next) => part = usize::try_from(next).unwrap(),
                None => break,
            }
        }
        assert_eq!(part, 2);
        assert_eq!(joined, text);
    }

    #[test]
    fn a_part_that_does_not_exist_answered_or_refused_in_another_slot_is_caught() {
        let held = document(json!({"bytes": 5, "text": "short"}));
        for part in [0, 2] {
            let value = answer(&identity(), &held, Content::Text("short"), Some(part));
            assert_eq!(value["status"], "refused", "{value}");
            assert_eq!(value["code"], "document-part-not-found");
            assert_eq!(value["slot"], "part");
        }
    }

    #[test]
    fn an_unknown_field_or_an_identity_kind_not_served_accepted_is_caught() {
        let base = json!({"operation": "document", "identity": {"kind": "capture", "id": ID}});
        let judged = judge_arguments(&base).unwrap();
        assert_eq!(judged.identity, identity());
        for arguments in [
            json!({"operation": "document", "identity": {"kind": "capture", "id": ID},
                "instruction": "bal-capture"}),
            json!({"operation": "document", "identity": {"kind": "capture", "id": ID,
                "phase": 1}}),
            json!({"operation": "document", "identity": {"kind": "plan", "id": ID}}),
            json!({"operation": "document", "identity": {"kind": "capture", "id": ID},
                "part": -1}),
        ] {
            let refusal = judge_arguments(&arguments).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{arguments}");
            assert_eq!(refusal["code"], "invalid-arguments", "{arguments}");
            assert_eq!(refusal["slot"], "arguments", "{arguments}");
        }
    }

    mod reads {
        use baley_core::capture::{CaptureKind, capture_id};
        use baley_store::{
            Actor, Admin, Command, CommandKind, HistoryFilter, Ledger, PayloadRef, RequestId,
            ServerCaller,
        };
        use baley_store_sqlite::SqliteStore;

        use super::super::*;
        use crate::mcp::capture::{CAPTURE_COMMAND, record};
        use crate::mcp::prepare::{WriteRequest, prepared_command};

        const T0: &str = "2026-10-05T09:00:00Z";
        const T1: &str = "2026-10-05T09:00:01Z";
        const A: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
        const B: &str = "7a2d3b5f-9c0e-4f1a-8b2c-3d4e5f6a7b8c";
        const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
        const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
        const PURGE: &str = "5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b";

        fn project(id: &str) -> ProjectId {
            ProjectId(id.into())
        }

        /// A real store in a fresh temporary directory, holding A and B.
        fn store() -> (tempfile::TempDir, SqliteStore) {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let store =
                crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
            store.create_project(&project(A), "a", T0).unwrap();
            store.create_project(&project(B), "b", T0).unwrap();
            (dir, store)
        }

        /// Records `text` as a note in `id`'s project and gives its capture id.
        fn capture(store: &SqliteStore, id: &str, text: &str) -> String {
            let request = WriteRequest {
                kind: CommandKind(CAPTURE_COMMAND.into()),
                request_id: RequestId(REQUEST.into()),
                digest: Hash([1; 32]),
            };
            let caller =
                ServerCaller::new("/real/r", "/real/r", "claude-code", SESSION, &json!(7)).unwrap();
            let command = prepared_command(&project(id), &request, 0, T1, &caller);
            record(store, &command, CaptureKind::Note, text, None).unwrap();
            capture_id(REQUEST, CaptureKind::Note, text, None)
        }

        /// The body hash of the one long capture in `id`'s project.
        fn body_hash(store: &SqliteStore, id: &str) -> Hash {
            let page = PageRequest {
                limit: 100,
                after: None,
            };
            let events = store
                .history(&project(id), 1..=1000, &HistoryFilter::default(), page)
                .unwrap()
                .items;
            let captured = events
                .iter()
                .find(|event| event.type_name == "capture.recorded")
                .expect("a capture");
            PayloadRef::from_value(&captured.payload["body"])
                .expect("a body")
                .hash
        }

        fn purge(store: &SqliteStore, id: &str, hash: Hash, reason: &str) {
            let command = Command {
                project: project(id),
                kind: CommandKind("payload.purge".into()),
                request_id: RequestId(PURGE.into()),
                digest: Hash([9; 32]),
                scope: vec![],
                policy_version: 0,
                recorded_at: T1.into(),
                actor: Actor::Owner,
                caller: None,
            };
            store.purge(&command, &[hash], reason).unwrap();
        }

        fn head(store: &SqliteStore, id: &str) -> u64 {
            store.head(&project(id)).unwrap().map_or(0, |head| head.seq)
        }

        fn shape(id: &str) -> DocumentShape {
            DocumentShape {
                identity: Identity::Capture { id: id.into() },
                part: None,
            }
        }

        /// 4,097 bytes: one over the inline limit, so stored as a payload.
        fn long() -> String {
            format!("{}x", "🦀".repeat(1024))
        }

        #[test]
        fn a_short_capture_not_read_back_or_its_read_appending_is_caught() {
            let (_dir, store) = store();
            let id = capture(&store, A, "keep this");
            let before = head(&store, A);
            let value = read(&store, &project(A), &shape(&id));
            assert_eq!(
                value,
                json!({"status": "ok", "identity": {"kind": "capture", "id": id},
                    "kind": "note", "phase": null, "bytes": 9, "recorded_at": T1,
                    "text": "keep this"})
            );
            assert_eq!(head(&store, A), before);
        }

        #[test]
        fn a_purged_long_capture_served_or_its_tombstone_lost_on_rebuild_is_caught() {
            let (_dir, store) = store();
            let text = long();
            let id = capture(&store, A, &text);
            capture(&store, B, &text);
            purge(&store, A, body_hash(&store, A), "pasted a secret");
            // B keeps the body, so the store would still open it: only the
            // purge state A's view rebuilt from its events gives the tombstone.
            store.rebuild(&project(A)).unwrap();
            assert_eq!(
                read(&store, &project(A), &shape(&id)),
                json!({"status": "ok", "identity": {"kind": "capture", "id": id},
                    "kind": "note", "phase": null, "bytes": 4097, "recorded_at": T1,
                    "tombstone": {"state": "purged", "reason": "pasted a secret"}})
            );
        }

        #[test]
        fn a_purged_body_resurrected_from_another_projects_copy_is_caught() {
            let (_dir, store) = store();
            let text = long();
            let in_a = capture(&store, A, &text);
            let in_b = capture(&store, B, &text);
            let hash = body_hash(&store, A);
            purge(&store, A, hash, "pasted a secret");
            // B still requires the body, so the store would hand it over: only
            // A's view can say A released it.
            let PayloadBody::Present(mut reader) = store.open(&hash).unwrap() else {
                panic!("B keeps the body");
            };
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, text.as_bytes());

            let from_a = read(&store, &project(A), &shape(&in_a));
            assert_eq!(
                from_a["tombstone"],
                json!({"state": "purged", "reason": "pasted a secret"}),
                "{from_a}"
            );
            assert!(from_a.get("text").is_none(), "{from_a}");
            let from_b = read(&store, &project(B), &shape(&in_b));
            assert_eq!(from_b["text"], json!(text), "{from_b}");
            assert!(from_b.get("tombstone").is_none(), "{from_b}");
        }

        #[test]
        fn concurrent_project_purges_answer_with_another_projects_reason() {
            let (_dir, store) = store();
            let text = long();
            let in_a = shape(&capture(&store, A, &text));
            let in_b = shape(&capture(&store, B, &text));
            let seen_a = find_capture(&store, &project(A), &in_a).unwrap();
            let seen_b = find_capture(&store, &project(B), &in_b).unwrap();
            let hash = body_hash(&store, A);
            assert_eq!(seen_a.source(), Some(Source::Open(hash)));
            assert_eq!(seen_b.source(), Some(Source::Open(hash)));

            purge(&store, A, hash, "A removed it");
            purge(&store, B, hash, "B removed it");
            // Resume both reads from their observations before either purge.
            for (id, shape, seen, reason) in [
                (A, &in_a, &seen_a, "A removed it"),
                (B, &in_b, &seen_b, "B removed it"),
            ] {
                let value = read_document(&store, &project(id), shape, seen);
                assert_eq!(value["status"], "ok", "{value}");
                assert_eq!(
                    value["tombstone"],
                    json!({"state": "purged", "reason": reason}),
                    "{id}: {value}"
                );
                assert!(value.get("text").is_none(), "{value}");
            }
        }

        #[test]
        fn an_unknown_id_answered_as_anything_but_no_such_capture_or_appending_is_caught() {
            let (_dir, store) = store();
            capture(&store, A, "keep this");
            let before = head(&store, A);
            let value = read(&store, &project(A), &shape(&"0".repeat(64)));
            assert_eq!(value["status"], "refused", "{value}");
            assert_eq!(value["code"], "no-such-capture");
            assert_eq!(value["slot"], "identity");
            assert_eq!(head(&store, A), before);
        }
    }

    mod scope_kinds {
        use super::super::*;
        use crate::scope::fixtures::{M_DIGEST, m};
        use crate::scope::read::{Description, RoadmapPhase};

        const AT: &str = "2026-10-13T09:00:01Z";

        fn draft() -> Identity {
            Identity::Draft {
                digest: M_DIGEST.into(),
            }
        }

        fn preview(submission: Value) -> Preview {
            Preview {
                kind: "start".into(),
                digest: Hash::from_hex(M_DIGEST).unwrap(),
                seq: 2,
                recorded_at: AT.into(),
                submission,
            }
        }

        fn deferred() -> Story {
            Story {
                id: 3,
                sentence: "Teammates see the backlog".into(),
                status: "deferred".into(),
                phase: None,
                truths: 0,
            }
        }

        fn refused(value: &Value, code: &str, slot: &str) {
            assert_eq!(value["status"], "refused", "{value}");
            assert_eq!(value["code"], code, "{value}");
            assert_eq!(value["slot"], slot, "{value}");
        }

        #[test]
        fn a_draft_answer_missing_its_submission_or_identity_is_caught() {
            assert_eq!(
                draft_answer(&draft(), Ok(preview(m())), None),
                json!({"status": "ok", "identity": {"kind": "draft", "digest": M_DIGEST},
                    "kind": "start", "digest": M_DIGEST, "recorded_at": AT,
                    "submission": {"description": {"what": "w", "core_value": "v",
                            "constraints": []},
                        "stories": [{"label": "s", "sentence": "t", "status": "active"}],
                        "roadmap": [{"name": "n", "goal": "g", "detail": "d",
                            "depends_on": [], "stories": ["s"]}]}})
            );
        }

        #[test]
        fn a_story_whose_status_overwrites_the_answer_status_is_caught() {
            assert_eq!(
                story_answer(&Identity::Story { id: 3 }, Ok(deferred()), None),
                json!({"status": "ok", "identity": {"kind": "story", "id": 3},
                    "story": {"id": 3, "sentence": "Teammates see the backlog",
                        "status": "deferred", "phase": null, "truths": 0}})
            );
        }

        #[test]
        fn a_roadmap_answer_out_of_order_or_missing_its_description_is_caught() {
            let phase = |number, name: &str, goal: &str, depends_on, stories| RoadmapPhase {
                number,
                name: name.into(),
                goal: goal.into(),
                status: "declared".into(),
                depends_on,
                stories,
            };
            let story = |id, sentence: &str, phase| Story {
                id,
                sentence: sentence.into(),
                status: "active".into(),
                phase,
                truths: 0,
            };
            let roadmap = Roadmap {
                description: Some(Description {
                    what: "A ledger for agent work".into(),
                    core_value: "Nothing moves on an unproven claim".into(),
                    constraints: vec!["One binary".into()],
                }),
                phases: vec![
                    phase(1, "Records", "Steps are recorded", vec![], vec![1]),
                    phase(2, "Approval", "The owner approves", vec![1], vec![2]),
                ],
                stories: vec![
                    story(1, "Every step is recorded", Some(1)),
                    story(2, "The owner approves scope", Some(2)),
                    deferred(),
                ],
            };
            let value = roadmap_answer(&Identity::Roadmap {}, Ok(roadmap), None);
            assert_eq!(value["status"], "ok", "{value}");
            assert_eq!(value["identity"], json!({"kind": "roadmap"}));
            assert_eq!(
                value["description"],
                json!({"what": "A ledger for agent work",
                    "core_value": "Nothing moves on an unproven claim",
                    "constraints": ["One binary"]})
            );
            assert_eq!(
                value["phases"],
                json!([
                    {"number": 1, "name": "Records", "goal": "Steps are recorded",
                        "status": "declared", "depends_on": [], "stories": [1]},
                    {"number": 2, "name": "Approval", "goal": "The owner approves",
                        "status": "declared", "depends_on": [1], "stories": [2]}
                ])
            );
            assert_eq!(
                value["stories"],
                json!([
                    {"id": 1, "sentence": "Every step is recorded", "status": "active",
                        "phase": 1, "truths": 0},
                    {"id": 2, "sentence": "The owner approves scope", "status": "active",
                        "phase": 2, "truths": 0},
                    {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                        "phase": null, "truths": 0}
                ])
            );
        }

        #[test]
        fn a_large_answer_served_whole_or_cut_off_its_json_is_caught() {
            let mut submission = m();
            submission["roadmap"][0]["detail"] = json!("a".repeat(30_000));
            let whole = json!({"status": "ok", "identity": {"kind": "draft", "digest": M_DIGEST},
                "kind": "start", "digest": M_DIGEST, "recorded_at": AT,
                "submission": submission.clone()});
            let first = draft_answer(&draft(), Ok(preview(submission.clone())), Some(1));
            let second = draft_answer(&draft(), Ok(preview(submission.clone())), Some(2));
            for (value, part, next) in [(&first, 1, json!(2)), (&second, 2, Value::Null)] {
                let mut keys: Vec<&String> = value.as_object().unwrap().keys().collect();
                keys.sort();
                assert_eq!(keys, ["body", "bound", "next", "part", "status"], "{value}");
                assert_eq!(value["status"], "ok");
                assert_eq!(value["bound"], 24_576);
                assert_eq!(value["part"], part);
                assert_eq!(value["next"], next);
            }
            assert_eq!(first["body"].as_str().unwrap().len(), 24_576);
            let joined = format!(
                "{}{}",
                first["body"].as_str().unwrap(),
                second["body"].as_str().unwrap()
            );
            assert_eq!(serde_json::from_str::<Value>(&joined).unwrap(), whole);
            let third = draft_answer(&draft(), Ok(preview(submission)), Some(3));
            refused(&third, "document-part-not-found", "part");
        }

        #[test]
        fn an_identity_of_a_new_kind_with_a_wrong_or_extra_field_accepted_is_caught() {
            for identity in [
                json!({"kind": "story", "id": "1"}),
                json!({"kind": "roadmap", "id": 1}),
                json!({"kind": "draft"}),
            ] {
                let arguments = json!({"operation": "document", "identity": identity});
                let refusal = judge_arguments(&arguments).expect_err("refused");
                refused(&refusal, "invalid-arguments", "arguments");
            }
        }

        #[test]
        fn an_unknown_draft_or_story_answered_ok_or_echoing_its_argument_is_caught() {
            let digest = "ab".repeat(32);
            let unknown = draft_answer(
                &Identity::Draft {
                    digest: digest.clone(),
                },
                Err(PreviewError::UnknownDraft),
                None,
            );
            refused(&unknown, "unknown-draft", "identity");
            assert!(!unknown["reason"].as_str().unwrap().contains(&digest));
            let missing = story_answer(
                &Identity::Story { id: 99 },
                Err(ReadError::NoSuchStory),
                None,
            );
            refused(&missing, "no-such-story", "identity");
            assert!(!missing["reason"].as_str().unwrap().contains("99"));
        }
    }

    #[test]
    fn a_gone_body_answered_as_text_or_without_its_reason_is_caught() {
        let held = document(json!({"hash": "ab".repeat(32), "state": "present"}));
        let content = Content::Purged("pasted a secret");
        let value = answer(&identity(), &held, content, None);
        metadata(&value, 4097);
        assert!(value.get("text").is_none(), "{value}");
        assert_eq!(
            value["tombstone"],
            json!({"state": "purged", "reason": "pasted a secret"})
        );
    }
}
