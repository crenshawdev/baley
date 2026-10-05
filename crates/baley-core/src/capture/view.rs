//! The `capture` view: one document per `capture.recorded`, keyed by the
//! event's sequence and found by capture id (design 0001 Views, design 0014
//! section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order,
    PayloadRef, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::CAPTURE_RECORDED;

/// The view's name.
pub const CAPTURE_VIEW: &str = "capture";
/// The index that finds a capture by its id.
pub const CAPTURE_ID_INDEX: &str = "by_id";

/// The key field: the sequence of the capture's `capture.recorded`.
const SEQ_FIELD: &str = "seq";
/// The capture id field the index covers.
const ID_FIELD: &str = "id";

/// The view's declaration: the event sequence as an integer key and one
/// index over the capture id.
///
/// The key is the sequence because the store's `payload.purged` names
/// released bodies by `[seq, hash]`, and a projector reaches only documents
/// whose keys it derives from the event. The id is the only index: an index
/// field must be non-null text in every body, and a capture's phase can be
/// null.
pub fn capture_spec() -> ViewSpec {
    ViewSpec {
        name: CAPTURE_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: SEQ_FIELD.into(),
            kind: FieldKind::Integer,
        }],
        indexes: vec![IndexSpec {
            name: CAPTURE_ID_INDEX.into(),
            fields: vec![IndexField {
                name: ID_FIELD.into(),
                kind: FieldKind::Text,
                order: Order::Ascending,
            }],
        }],
        page_bound: 100,
    }
}

/// The key of the capture recorded at `seq`.
pub fn capture_key(seq: u64) -> DocKey {
    // Sequences stay within the canonical integer range, so they fit.
    DocKey(vec![KeyValue::Integer(
        i64::try_from(seq).expect("a sequence fits an i64"),
    )])
}

/// Keeps the `capture` view current from `capture.recorded`.
///
/// A document holds the sequence, id, kind, phase, byte count and recording
/// time, then either the inline text or the body's hash with a purge state
/// of `present`. Inline text is already permanent in the hashed event, so
/// keeping it here brings back nothing a purge removed. Payload text is never
/// copied in. The caller is not kept: the hashed envelope is the record of
/// who captured.
pub struct CaptureProjector {
    spec: ViewSpec,
}

impl CaptureProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: capture_spec(),
        }
    }
}

impl Default for CaptureProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for CaptureProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[CAPTURE_RECORDED]
    }

    // Each capture gets a document of its own, so none is read first.
    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        Vec::new()
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let text = |member: &str| match payload.get(member) {
            Some(Value::String(value)) if !value.is_empty() => Some(value),
            _ => None,
        };
        let (Some(id), Some(kind)) = (text(ID_FIELD), text("kind")) else {
            return Err(refuse("id or kind is missing, not text or empty"));
        };
        let mut body = Map::new();
        body.insert(SEQ_FIELD.into(), json!(event.seq));
        body.insert(ID_FIELD.into(), json!(id));
        body.insert("kind".into(), json!(kind));
        body.insert("phase".into(), member(payload, "phase"));
        body.insert("bytes".into(), member(payload, "bytes"));
        body.insert("recorded_at".into(), json!(event.recorded_at));
        match (payload.get("text"), payload.get("body")) {
            (Some(Value::String(inline)), None) => {
                body.insert("text".into(), json!(inline));
            }
            (None, Some(reference)) => {
                let Some(reference) = PayloadRef::from_value(reference) else {
                    return Err(refuse("body is not a reference object"));
                };
                body.insert("hash".into(), json!(reference.hash.to_hex()));
                body.insert("state".into(), json!("present"));
            }
            _ => return Err(refuse("needs exactly one of inline text or a body")),
        }
        Ok(vec![Change::Put {
            key: capture_key(event.seq),
            body: Value::Object(body),
        }])
    }
}

/// A payload member as given, `null` when absent.
fn member(payload: &Map<String, Value>, name: &str) -> Value {
    payload.get(name).cloned().unwrap_or(Value::Null)
}
