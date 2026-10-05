//! The `capture` view: one document per `capture.recorded`, keyed by the
//! event's sequence and found by capture id, with the purge state of a body
//! its project released (design 0001 Views, design 0014 section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order,
    PAYLOAD_PURGED, PayloadRef, Projector, ProjectorError, PurgedEvent, ViewSpec,
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

/// Keeps the `capture` view current from `capture.recorded` and the store's
/// `payload.purged`.
///
/// A document holds the sequence, id, kind, phase, byte count and recording
/// time, then either the inline text or the body's hash with a purge state
/// of `present`. Inline text is already permanent in the hashed event, so
/// keeping it here brings back nothing a purge removed. Payload text is never
/// copied in. The caller is not kept: the hashed envelope is the record of
/// who captured.
///
/// When this project releases a body, the document's state becomes `purged`
/// with the purge's reason. The document is never deleted: a purged capture
/// is still known by its id and answers a tombstone. The state is per
/// project, so another project holding the same bytes keeps its text.
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
        &[CAPTURE_RECORDED, PAYLOAD_PURGED]
    }

    // A new capture gets a document of its own, so none is read first. A
    // purge names each released reference by the sequence that holds it,
    // which is the key of the capture recorded there, if any.
    fn keys(&self, event: &Event) -> Vec<DocKey> {
        if event.type_name != PAYLOAD_PURGED {
            return Vec::new();
        }
        let Some(purge) = PurgedEvent::from_value(&event.payload) else {
            // `apply` refuses it with a reason.
            return Vec::new();
        };
        let mut keys: Vec<DocKey> = purge
            .released
            .iter()
            .map(|(seq, _)| capture_key(*seq))
            .collect();
        keys.dedup();
        keys
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        if event.type_name == PAYLOAD_PURGED {
            let Some(purge) = PurgedEvent::from_value(&event.payload) else {
                return Err(refuse("the purge payload cannot be read"));
            };
            return Ok(purged(&purge, documents));
        }
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

/// The documents `purge` marks purged: each at a released sequence whose
/// body hash is the one released there. Inline captures hold no hash and
/// stay as they are.
fn purged(purge: &PurgedEvent, documents: &[(DocKey, Value)]) -> Vec<Change> {
    documents
        .iter()
        .filter_map(|(key, document)| {
            let Value::Object(fields) = document else {
                return None;
            };
            let hash = fields.get("hash").and_then(Value::as_str)?;
            let released = purge
                .released
                .iter()
                .any(|(seq, released)| capture_key(*seq) == *key && released.to_hex() == hash);
            if !released {
                return None;
            }
            let mut body = fields.clone();
            body.insert("state".into(), json!("purged"));
            body.insert("reason".into(), json!(purge.reason));
            Some(Change::Put {
                key: key.clone(),
                body: Value::Object(body),
            })
        })
        .collect()
}

/// A payload member as given, `null` when absent.
fn member(payload: &Map<String, Value>, name: &str) -> Value {
    payload.get(name).cloned().unwrap_or(Value::Null)
}
