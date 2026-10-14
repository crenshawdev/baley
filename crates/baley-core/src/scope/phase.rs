//! The `phase` view: one document per declared phase, keyed by its number
//! (design 0004 section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order,
    PayloadRef, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::PHASE_DECLARED;

/// The view's name.
pub const PHASE_VIEW: &str = "phase";
/// The index that lists phases by status, then number.
pub const PHASE_STATUS_INDEX: &str = "by_status";

/// The key field: the phase number.
const NUMBER_FIELD: &str = "number";
/// The phase's status, which the index leads with.
const STATUS_FIELD: &str = "status";

/// The view's declaration: the phase number as an integer key and one
/// index over the status, then the number.
pub fn phase_spec() -> ViewSpec {
    ViewSpec {
        name: PHASE_VIEW.into(),
        version: 1,
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
/// ids, status `declared`, the approving digest, the declaration's time,
/// and the hex hash of its detail body, never the detail text. A later
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
                    "status": "declared", "submission": "5c".repeat(32), "declared_at": AT}),
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
