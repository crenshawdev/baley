//! The `capture.recorded` event: its type, its registration, the capture id,
//! the byte threshold between inline text and a payload, and the payload in
//! both forms (design 0014 section 6, design 0001 Payloads).

use baley_store::{PayloadRef, request_digest};
use serde_json::{Value, json};

use crate::registry::{Registry, RegistryError};

/// One note or story captured: `{id, kind, phase, bytes}` with either the
/// `text` itself or a `body` reference object. Built by [`inline_payload`] or
/// [`stored_payload`].
pub const CAPTURE_RECORDED: &str = "capture.recorded";
/// The current `capture.recorded` payload version.
pub const CAPTURE_RECORDED_VERSION: u32 = 1;
/// The one stream per project every capture goes on. There is never a
/// stream per capture.
pub const CAPTURE_STREAM: &str = "capture";

/// The most UTF-8 bytes of text a `capture.recorded` holds inline. Longer
/// text is stored as a `record` payload, so a purge can release it.
pub const INLINE_TEXT_LIMIT: usize = 4096;

/// Registers `capture.recorded` at version 1, with no upcasters.
pub fn register_capture_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, [])
}

/// The kinds a capture may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    /// A record kept as it is, never a queue item.
    Note,
    /// A candidate for the backlog, which the owner may promote to a story.
    Story,
}

impl CaptureKind {
    /// The kind as the payload and the caller spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Story => "story",
        }
    }
}

/// Where a capture's text is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextForm {
    /// In the event payload itself.
    Inline,
    /// As a `record` payload the event references.
    Payload,
}

impl TextForm {
    /// The form as a receipt names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Payload => "payload",
        }
    }
}

/// Where `text` is kept: inline through [`INLINE_TEXT_LIMIT`] UTF-8 bytes, a
/// payload above. Bytes, never characters, since the limit bounds storage.
pub fn text_form(text: &str) -> TextForm {
    if text.len() <= INLINE_TEXT_LIMIT {
        TextForm::Inline
    } else {
        TextForm::Payload
    }
}

/// The capture id: the lowercase hex digest of the request id, kind, text
/// and phase. The request id is inside, so two separate captures of the same
/// text never share an id, and a retry of one request always gets the same.
pub fn capture_id(request_id: &str, kind: CaptureKind, text: &str, phase: Option<u32>) -> String {
    request_digest(&json!({
        "request_id": request_id,
        "kind": kind.as_str(),
        "text": text,
        "phase": phase,
    }))
    // Strings and a u32 always have a canonical form.
    .expect("a capture id input is canonical")
    .to_hex()
}

/// The payload of a capture whose text is inline. It holds no caller and no
/// time: the envelope carries both, hashed with the event.
pub fn inline_payload(id: &str, kind: CaptureKind, phase: Option<u32>, text: &str) -> Value {
    json!({
        "id": id,
        "kind": kind.as_str(),
        "phase": phase,
        "bytes": text.len(),
        "text": text,
    })
}

/// The payload of a capture whose text the caller already stored as
/// `body`. It holds the reference object and none of the text.
pub fn stored_payload(id: &str, kind: CaptureKind, phase: Option<u32>, body: &PayloadRef) -> Value {
    json!({
        "id": id,
        "kind": kind.as_str(),
        "phase": phase,
        "bytes": body.bytes,
        "body": body.to_value(),
    })
}
