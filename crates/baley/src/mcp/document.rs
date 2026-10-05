//! `document` on `baley_query`: a record read back by identity. A capture is
//! the only kind so far. Its text comes from the `capture` view when inline,
//! or from its body by hash, and a body this project purged answers a
//! tombstone (design 0012 operations, design 0014 section 6).

use baley_store::{Hash, PayloadStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::parts::{Cut, PART_BOUND, cut};

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

impl<'a> Content<'a> {
    /// The content a gone body stands for: a purge's tombstone. None for any
    /// other status, since a capture's body is a `record`, never reduced.
    pub fn gone(status: &'a PayloadStatus) -> Option<Self> {
        match status {
            PayloadStatus::Purged { reason } => Some(Self::Purged(reason)),
            _ => None,
        }
    }
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
            Cut::Absent => {
                return Refusal::new(
                    "document-part-not-found",
                    "the requested document part is absent",
                )
                .slot("part")
                .value();
            }
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

    #[test]
    fn a_gone_body_answered_as_text_or_without_its_reason_is_caught() {
        let held = document(json!({"hash": "ab".repeat(32), "state": "present"}));
        let status = PayloadStatus::Purged {
            reason: "pasted a secret".into(),
        };
        let content = Content::gone(&status).expect("a purge is a tombstone");
        let value = answer(&identity(), &held, content, None);
        metadata(&value, 4097);
        assert!(value.get("text").is_none(), "{value}");
        assert_eq!(
            value["tombstone"],
            json!({"state": "purged", "reason": "pasted a secret"})
        );
    }
}
