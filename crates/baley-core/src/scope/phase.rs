//! The `phase` view: one document per declared phase, keyed by its number
//! (design 0004 section 6). The `phase_detail` view: one document per
//! `phase.declared`, keyed by its sequence, with the purge state of a detail
//! body its project released (design 0001 Views).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order,
    PAYLOAD_PURGED, PayloadRef, Projector, ProjectorError, PurgedEvent, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::PHASE_DECLARED;
use super::mark_purged;

/// The view's name.
pub const PHASE_VIEW: &str = "phase";
/// The index that lists phases by status, then number.
pub const PHASE_STATUS_INDEX: &str = "by_status";
/// The detail view's name.
pub const PHASE_DETAIL_VIEW: &str = "phase_detail";

/// The key field: the phase number.
const NUMBER_FIELD: &str = "number";
/// The phase's status, which the index leads with.
const STATUS_FIELD: &str = "status";
/// The sequence of the `phase.declared` a document came from.
const DECLARED_SEQ_FIELD: &str = "declared_seq";
/// The detail view's key field: the `phase.declared` sequence.
const SEQ_FIELD: &str = "seq";

/// The view's declaration: the phase number as an integer key and one
/// index over the status, then the number.
pub fn phase_spec() -> ViewSpec {
    ViewSpec {
        name: PHASE_VIEW.into(),
        version: 2,
        key: vec![FieldSpec {
            name: NUMBER_FIELD.into(),
            kind: FieldKind::Integer,
        }],
        indexes: vec![IndexSpec {
            name: PHASE_STATUS_INDEX.into(),
            fields: vec![
                IndexField {
                    name: STATUS_FIELD.into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                },
                IndexField {
                    name: NUMBER_FIELD.into(),
                    kind: FieldKind::Integer,
                    order: Order::Ascending,
                },
            ],
        }],
        page_bound: 100,
    }
}

/// The key of phase `number`.
pub fn phase_key(number: u32) -> DocKey {
    DocKey(vec![KeyValue::Integer(i64::from(number))])
}

/// Keeps the `phase` view current from `phase.declared`.
///
/// A document holds the phase's number, name, goal, dependencies, story
/// ids, status `declared`, the approving digest, the declaration's time
/// and sequence, and the hex hash of its detail body, never the detail
/// text. The sequence finds the detail's `phase_detail` document, which
/// knows whether this project purged it. A later
/// declaration of the same number replaces the document. Context, plans,
/// execution and completion are later builds' events.
pub struct PhaseProjector {
    spec: ViewSpec,
}

impl PhaseProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self { spec: phase_spec() }
    }
}

impl Default for PhaseProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for PhaseProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[PHASE_DECLARED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        declared_number(&event.payload)
            .map(phase_key)
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
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let fault = || refuse("a member is missing or of the wrong type");
        let number = declared_number(&event.payload).ok_or_else(fault)?;
        let detail = payload
            .get("detail")
            .and_then(PayloadRef::from_value)
            .ok_or_else(fault)?;
        Ok(vec![Change::Put {
            key: phase_key(number),
            body: json!({
                NUMBER_FIELD: number,
                "name": text(payload, "name").ok_or_else(fault)?,
                "goal": text(payload, "goal").ok_or_else(fault)?,
                "detail": detail.hash.to_hex(),
                "depends_on": wholes(payload, "depends_on").ok_or_else(fault)?,
                "stories": wholes(payload, "stories").ok_or_else(fault)?,
                STATUS_FIELD: "declared",
                "submission": text(payload, "submission").ok_or_else(fault)?,
                "declared_at": event.recorded_at,
                DECLARED_SEQ_FIELD: event.seq,
            }),
        }])
    }
}

/// The detail view's declaration: the `phase.declared` sequence as an
/// integer key, and no index, since a phase document names its sequence.
///
/// The key is the sequence because the store's `payload.purged` names
/// released bodies by `[seq, hash]`, and a projector reaches only documents
/// whose keys it derives from the event; the `phase` view's number key is
/// not in the purge.
pub fn phase_detail_spec() -> ViewSpec {
    ViewSpec {
        name: PHASE_DETAIL_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: SEQ_FIELD.into(),
            kind: FieldKind::Integer,
        }],
        indexes: Vec::new(),
        page_bound: 100,
    }
}

/// The key of the detail declared at `seq`.
pub fn phase_detail_key(seq: u64) -> DocKey {
    // Sequences stay within the canonical integer range, so they fit.
    DocKey(vec![KeyValue::Integer(
        i64::try_from(seq).expect("a sequence fits an i64"),
    )])
}

/// Keeps the `phase_detail` view current from `phase.declared` and the
/// store's `payload.purged`.
///
/// A document holds the declaration's sequence, the phase number and the
/// hex hash of its detail body, never the text. When this project releases
/// that body, the document gains `state` `purged` and the purge's reason,
/// and is kept so the phase answers a tombstone. The state is per project,
/// so another project holding the same bytes keeps its text.
pub struct PhaseDetailProjector {
    spec: ViewSpec,
}

impl PhaseDetailProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: phase_detail_spec(),
        }
    }
}

impl Default for PhaseDetailProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for PhaseDetailProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[PHASE_DECLARED, PAYLOAD_PURGED]
    }

    // A declaration gets a document of its own, so none is read first. A
    // purge names each released reference by the sequence that holds it.
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
            .map(|(seq, _)| phase_detail_key(*seq))
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
            return Ok(mark_purged(&purge, documents));
        }
        let fault = || refuse("the number or detail is missing or of the wrong type");
        let number = declared_number(&event.payload).ok_or_else(fault)?;
        let detail = event
            .payload
            .get("detail")
            .and_then(PayloadRef::from_value)
            .ok_or_else(fault)?;
        Ok(vec![Change::Put {
            key: phase_detail_key(event.seq),
            body: json!({
                SEQ_FIELD: event.seq,
                NUMBER_FIELD: number,
                "hash": detail.hash.to_hex(),
            }),
        }])
    }
}

/// The phase number a `phase.declared` payload names, if it names one.
fn declared_number(payload: &Value) -> Option<u32> {
    let number = payload.get(NUMBER_FIELD)?.as_u64()?;
    u32::try_from(number).ok()
}

fn text<'a>(payload: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    payload.get(name).and_then(Value::as_str)
}

fn wholes(payload: &Map<String, Value>, name: &str) -> Option<Vec<u64>> {
    payload
        .get(name)?
        .as_array()?
        .iter()
        .map(Value::as_u64)
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0004's phase shape and
    //! the plan's approved S1, never from running this code.

    use baley_store::{
        Actor, Hash, IndexField, Order, PayloadRef, ProjectId, RequestId, RetentionClass,
    };
    use serde_json::json;

    use super::super::event::phase_declared_payload;
    use super::*;

    const AT: &str = "2026-10-13T09:00:09Z";

    #[test]
    fn a_declared_phase_keyed_off_its_number_or_holding_its_detail_text_is_caught() {
        let detail = PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 18,
            class: RetentionClass::Record,
        };
        let event = Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq: 9,
            stream: "roadmap".into(),
            stream_version: 5,
            type_name: "phase.declared".into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: AT.into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload: phase_declared_payload(
                2,
                "Approval",
                "The owner approves",
                &detail,
                &[1],
                &[2],
                Hash([0x5c; 32]),
            ),
            prev_hash: None,
            hash: Hash([0; 32]),
        };
        let projector = PhaseProjector::new();
        assert_eq!(projector.keys(&event), [DocKey(vec![KeyValue::Integer(2)])]);
        assert_eq!(
            projector.apply(&event, &[]).unwrap(),
            [Change::Put {
                key: DocKey(vec![KeyValue::Integer(2)]),
                body: json!({"number": 2, "name": "Approval", "goal": "The owner approves",
                    "detail": "ab".repeat(32), "depends_on": [1], "stories": [2],
                    "status": "declared", "submission": "5c".repeat(32), "declared_at": AT,
                    "declared_seq": 9}),
            }]
        );
    }

    fn event(seq: u64, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: AT.into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    #[test]
    fn a_phase_detail_purge_lost_or_marked_on_another_declaration_is_caught() {
        let detail = PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 18,
            class: RetentionClass::Record,
        };
        let declared = event(
            9,
            "phase.declared",
            phase_declared_payload(
                2,
                "Approval",
                "The owner approves",
                &detail,
                &[1],
                &[2],
                Hash([0x5c; 32]),
            ),
        );
        let projector = PhaseDetailProjector::new();
        let at = |seq| DocKey(vec![KeyValue::Integer(seq)]);
        assert!(projector.keys(&declared).is_empty());
        let held = json!({"seq": 9, "number": 2, "hash": "ab".repeat(32)});
        assert_eq!(
            projector.apply(&declared, &[]).unwrap(),
            [Change::Put {
                key: at(9),
                body: held.clone(),
            }]
        );

        let purge = event(
            13,
            "payload.purged",
            baley_store::PurgedEvent {
                requested: vec![Hash([0xab; 32]), Hash([0xcd; 32])],
                released: vec![(9, Hash([0xab; 32])), (12, Hash([0xcd; 32]))],
                removed: vec![],
                shared: vec![Hash([0xab; 32]), Hash([0xcd; 32])],
                reason: "pasted a secret".into(),
            }
            .to_value(),
        );
        assert_eq!(projector.keys(&purge), [at(9), at(12)]);
        // The declaration at 12 holds another body, so its reference is not
        // the one released there.
        let other = json!({"seq": 12, "number": 1, "hash": "ef".repeat(32)});
        assert_eq!(
            projector
                .apply(&purge, &[(at(9), held), (at(12), other)])
                .unwrap(),
            [Change::Put {
                key: at(9),
                body: json!({"seq": 9, "number": 2, "hash": "ab".repeat(32),
                    "state": "purged", "reason": "pasted a secret"}),
            }]
        );
    }

    #[test]
    fn a_phase_view_changed_by_a_submission_or_lacking_its_status_index_is_caught() {
        let projector = PhaseProjector::new();
        assert_eq!(projector.handles(), ["phase.declared"]);
        let spec = projector.spec();
        assert_eq!(
            spec.key,
            [FieldSpec {
                name: "number".into(),
                kind: FieldKind::Integer,
            }]
        );
        assert_eq!(spec.indexes.len(), 1);
        assert_eq!(
            spec.indexes[0].fields,
            [
                IndexField {
                    name: "status".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                },
                IndexField {
                    name: "number".into(),
                    kind: FieldKind::Integer,
                    order: Order::Ascending,
                },
            ]
        );
    }
}
