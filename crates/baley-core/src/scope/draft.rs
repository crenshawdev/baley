//! The `draft` view: one document per submitted digest, holding the
//! sequence of that digest's latest submission, so the newest draft of a
//! kind is the first document its index gives (design 0004 section 5).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, Hash, IndexField, IndexSpec, KeyValue, Order,
    PayloadRef, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::SCOPE_SUBMITTED;

/// The view's name.
pub const DRAFT_VIEW: &str = "draft";
/// The index that gives a kind's drafts newest first.
pub const DRAFT_NEWEST_INDEX: &str = "newest";

/// The key field: the submission's digest in lowercase hex.
const DIGEST_FIELD: &str = "digest";
/// The change kind the index leads with.
const KIND_FIELD: &str = "kind";
/// The latest submission's sequence, newest first in the index.
const SEQ_FIELD: &str = "seq";

/// The view's declaration: the digest as a text key and one index over the
/// change kind, then the latest submission's sequence descending.
pub fn draft_spec() -> ViewSpec {
    ViewSpec {
        name: DRAFT_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: DIGEST_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: vec![IndexSpec {
            name: DRAFT_NEWEST_INDEX.into(),
            fields: vec![
                IndexField {
                    name: KIND_FIELD.into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                },
                IndexField {
                    name: SEQ_FIELD.into(),
                    kind: FieldKind::Integer,
                    order: Order::Descending,
                },
            ],
        }],
        page_bound: 100,
    }
}

/// The key of the draft with `digest`.
pub fn draft_key(digest: Hash) -> DocKey {
    DocKey(vec![KeyValue::Text(digest.to_hex())])
}

/// Keeps the `draft` view current from `scope.submitted`.
///
/// A document holds the digest, the change kind, the sequence and time of
/// the digest's latest submission and the hex hash of its body. Submitting
/// a digest again replaces its document, so it is the newest draft again.
/// None of the body is copied in.
pub struct DraftProjector {
    spec: ViewSpec,
}

impl DraftProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self { spec: draft_spec() }
    }
}

impl Default for DraftProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for DraftProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[SCOPE_SUBMITTED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        submitted_digest(&event.payload)
            .map(draft_key)
            .into_iter()
            .collect()
    }

    // The document is replaced whole, so the one already there is not read.
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
        let Some(digest) = submitted_digest(&event.payload) else {
            return Err(refuse("digest is missing or not a digest"));
        };
        let kind = match event.payload.get(KIND_FIELD) {
            Some(Value::String(kind)) if !kind.is_empty() => kind,
            _ => return Err(refuse("kind is missing, not text or empty")),
        };
        let Some(body) = event.payload.get("body").and_then(PayloadRef::from_value) else {
            return Err(refuse("body is missing or not a reference object"));
        };
        Ok(vec![Change::Put {
            key: draft_key(digest),
            body: json!({
                DIGEST_FIELD: digest.to_hex(),
                KIND_FIELD: kind,
                SEQ_FIELD: event.seq,
                "recorded_at": event.recorded_at,
                "hash": body.hash.to_hex(),
            }),
        }])
    }
}

/// The digest a `scope.submitted` payload names, if it names one.
fn submitted_digest(payload: &Value) -> Option<Hash> {
    payload
        .get(DIGEST_FIELD)
        .and_then(Value::as_str)
        .and_then(Hash::from_hex)
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from the view's documented shape,
    //! never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, PayloadRef, ProjectId, RequestId, RetentionClass};
    use serde_json::json;

    use super::*;

    fn submitted(seq: u64, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "project".into(),
            stream_version: seq,
            type_name: "scope.submitted".into(),
            type_version: 1,
            actor: Actor::Baley,
            caller: None,
            recorded_at: at(seq),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    fn at(seq: u64) -> String {
        format!("2026-10-13T09:00:{seq:02}Z")
    }

    /// A `scope.submitted` payload for digest `digest`, whose body hashes to
    /// `body`.
    fn payload(digest: u8, body: u8) -> Value {
        json!({"digest": format!("{digest:02x}").repeat(32), "kind": "start",
            "body": PayloadRef { hash: Hash([body; 32]), bytes: 300,
                class: RetentionClass::Record }.to_value()})
    }

    #[test]
    fn a_resubmitted_digest_left_at_its_first_sequence_is_caught() {
        let projector = DraftProjector::new();
        let mut view = BTreeMap::new();
        for event in [
            submitted(4, payload(0xaa, 0x1a)),
            submitted(5, payload(0xbb, 0x1b)),
            submitted(6, payload(0xaa, 0x1a)),
        ] {
            let read: Vec<(DocKey, Value)> = projector
                .keys(&event)
                .into_iter()
                .filter_map(|key| view.get(&key).cloned().map(|body| (key, body)))
                .collect();
            for change in projector.apply(&event, &read).unwrap() {
                match change {
                    Change::Put { key, body } => {
                        view.insert(key, body);
                    }
                    Change::Delete { key } => {
                        view.remove(&key);
                    }
                }
            }
        }
        let key = |digest: &str| DocKey(vec![KeyValue::Text(digest.repeat(32))]);
        assert_eq!(view.len(), 2);
        assert_eq!(
            view[&key("aa")],
            json!({"digest": "aa".repeat(32), "kind": "start", "seq": 6,
                "recorded_at": "2026-10-13T09:00:06Z", "hash": "1a".repeat(32)})
        );
        assert_eq!(
            view[&key("bb")],
            json!({"digest": "bb".repeat(32), "kind": "start", "seq": 5,
                "recorded_at": "2026-10-13T09:00:05Z", "hash": "1b".repeat(32)})
        );
    }

    #[test]
    fn a_draft_view_that_lists_its_newest_last_is_caught() {
        let spec = draft_spec();
        assert_eq!(
            spec.key,
            [FieldSpec {
                name: "digest".into(),
                kind: FieldKind::Text,
            }]
        );
        assert_eq!(spec.indexes.len(), 1);
        assert_eq!(
            spec.indexes[0].fields,
            [
                IndexField {
                    name: "kind".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                },
                IndexField {
                    name: "seq".into(),
                    kind: FieldKind::Integer,
                    order: Order::Descending,
                },
            ]
        );
    }

    #[test]
    fn a_submission_payload_without_a_digest_kind_or_body_projected_is_caught() {
        for member in ["digest", "kind", "body"] {
            let mut incomplete = payload(0xaa, 0x1a);
            incomplete.as_object_mut().unwrap().remove(member);
            let error = DraftProjector::new()
                .apply(&submitted(4, incomplete), &[])
                .expect_err(member);
            assert!(error.0.contains("scope.submitted at seq 4"), "{}", error.0);
        }
    }
}
