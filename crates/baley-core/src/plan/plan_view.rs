//! The `plan` view: one document per approved plan, keyed by its phase and
//! number, holding the plan's approval binding (design 0005 section 6
//! Views, D-07, D-17).
//!
//! Only `plan.approved` and `plan.replaced` reach it, so an unapproved
//! draft never appears. It does not handle `payload.purged`: the record
//! holds the hash of the draft's body, never its text, so a purge of the
//! body leaves the record whole (D-06).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, PayloadRef, Projector, ProjectorError,
    ViewSpec,
};
use serde_json::{Value, json};

use super::event::{PLAN_APPROVED, PLAN_REPLACED};
use super::phase_plan::{REPLACED, text, whole, wholes};

/// The view's name.
pub const PLAN_VIEW: &str = "plan";
/// The state of an approved plan no later plan replaced.
pub const APPROVED: &str = "approved";

/// The view's declaration: the phase and the plan number as integer keys,
/// and no index.
pub fn plan_spec() -> ViewSpec {
    ViewSpec {
        name: PLAN_VIEW.into(),
        version: 1,
        key: vec![
            FieldSpec {
                name: "phase".into(),
                kind: FieldKind::Integer,
            },
            FieldSpec {
                name: "plan".into(),
                kind: FieldKind::Integer,
            },
        ],
        indexes: Vec::new(),
        page_bound: 100,
    }
}

/// The key of plan `plan` of phase `phase`.
pub fn plan_key(phase: u32, plan: u32) -> DocKey {
    DocKey(vec![
        KeyValue::Integer(i64::from(phase)),
        KeyValue::Integer(i64::from(plan)),
    ])
}

/// Keeps the `plan` view current from `plan.approved` and `plan.replaced`.
///
/// An approval puts a document holding `phase`, `plan`, the approved
/// `digest`, `stories`, `size`, `suite` (null when none was set),
/// `question_set`, `body` (the hex hash of the draft's `record` payload),
/// `owner`, `at`, `state` `approved` and `replaced_by` null. A replacement
/// sets `state` `replaced` and `replaced_by` to the new digest and keeps
/// every other field, so the binding stays readable (D-07); one for a plan
/// not held is refused.
pub struct PlanProjector {
    spec: ViewSpec,
}

impl PlanProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self { spec: plan_spec() }
    }
}

impl Default for PlanProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for PlanProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[PLAN_APPROVED, PLAN_REPLACED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        key_of(&event.payload).into_iter().collect()
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
        let payload = &event.payload;
        let key = key_of(payload).ok_or_else(|| refuse("the phase or plan is not a number"))?;
        let body = if event.type_name == PLAN_REPLACED {
            let new = text(payload, "new_digest").map_err(|message| refuse(&message))?;
            let Some((_, Value::Object(held))) = documents.iter().find(|(at, _)| *at == key) else {
                return Err(refuse("the replaced plan is not held"));
            };
            let mut held = held.clone();
            held.insert("state".into(), json!(REPLACED));
            held.insert("replaced_by".into(), json!(new));
            Value::Object(held)
        } else {
            approved(payload).map_err(|message| refuse(&message))?
        };
        Ok(vec![Change::Put { key, body }])
    }
}

/// The document of a `plan.approved` payload.
fn approved(payload: &Value) -> Result<Value, String> {
    let suite = match payload.get("suite") {
        Some(Value::Null) => None,
        Some(Value::String(suite)) => Some(suite.as_str()),
        _ => return Err("suite is not text or null".into()),
    };
    let body = payload
        .get("body")
        .and_then(PayloadRef::from_value)
        .ok_or("body is not a payload reference")?;
    Ok(json!({
        "phase": whole(payload, "phase")?,
        "plan": whole(payload, "plan")?,
        "digest": text(payload, "digest")?,
        "stories": wholes(payload, "stories")?,
        "size": payload.get("size").and_then(Value::as_u64).ok_or("size is not a whole number")?,
        "suite": suite,
        "question_set": text(payload, "question_set")?,
        "body": body.hash.to_hex(),
        "owner": text(payload, "owner")?,
        "at": text(payload, "at")?,
        "state": APPROVED,
        "replaced_by": null,
    }))
}

/// The key a plan event names: its phase and its plan number.
pub(super) fn key_of(payload: &Value) -> Option<DocKey> {
    Some(plan_key(
        whole(payload, "phase").ok()?,
        whole(payload, "plan").ok()?,
    ))
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0005 section 6, D-06,
    //! D-07, D-17 and the plan's test data, never from running this code.

    use baley_store::{Actor, Hash, ProjectId, PurgedEvent, RequestId, RetentionClass};

    use super::*;
    use crate::plan::event::{PlanApproval, replaced_payload};
    use crate::plan::fixtures::{c1, c2, o1};
    use crate::scope::submitted_payload;

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const T: &str = "2026-10-31T09:00:00Z";

    fn x() -> Hash {
        Hash([0x5c; 32])
    }

    fn y() -> Hash {
        Hash([0x6d; 32])
    }

    /// R: a `record` reference with hash Y and 1,024 bytes.
    fn r() -> PayloadRef {
        PayloadRef {
            hash: y(),
            bytes: 1_024,
            class: RetentionClass::Record,
        }
    }

    fn event(seq: u64, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "phase/1".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: format!("2026-10-31T09:00:{seq:02}Z"),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    #[test]
    fn an_approved_plan_record_dropped_or_rebound_on_replacement_is_caught() {
        let map = [c1(), c2(), o1()];
        let body = r();
        let approval = PlanApproval {
            phase: 1,
            plan: 1,
            digest: x(),
            stories: &[1],
            size: 1,
            suite: Some("cargo test"),
            question_set: S,
            evidence_map: &map,
            body: &body,
            owner: "Ada Owner",
            at: T,
        };
        let approved = event(1, "plan.approved", approval.payload());
        let projector = PlanProjector::new();
        let key = DocKey(vec![KeyValue::Integer(1), KeyValue::Integer(1)]);
        assert_eq!(projector.keys(&approved), std::slice::from_ref(&key));
        let record = json!({"phase": 1, "plan": 1, "digest": "5c".repeat(32), "stories": [1],
            "size": 1, "suite": "cargo test", "question_set": S, "body": "6d".repeat(32),
            "owner": "Ada Owner", "at": T, "state": "approved", "replaced_by": null});
        assert_eq!(
            projector.apply(&approved, &[]).unwrap(),
            [Change::Put {
                key: key.clone(),
                body: record.clone(),
            }]
        );

        let replaced = event(
            2,
            "plan.replaced",
            replaced_payload(1, 1, x(), y(), "Ada Owner", T),
        );
        assert_eq!(projector.keys(&replaced), std::slice::from_ref(&key));
        assert_eq!(
            projector
                .apply(&replaced, &[(key.clone(), record)])
                .unwrap(),
            [Change::Put {
                key,
                body: json!({"phase": 1, "plan": 1, "digest": "5c".repeat(32),
                    "stories": [1], "size": 1, "suite": "cargo test", "question_set": S,
                    "body": "6d".repeat(32), "owner": "Ada Owner", "at": T,
                    "state": "replaced", "replaced_by": "6d".repeat(32)}),
            }]
        );
    }

    #[test]
    fn a_draft_or_a_purge_reaching_the_plan_view_is_caught() {
        let projector = PlanProjector::new();
        assert_eq!(projector.handles(), ["plan.approved", "plan.replaced"]);
        let draft = event(
            1,
            "scope.submitted",
            submitted_payload(x(), "plan", "phase/1", &r()),
        );
        assert!(projector.keys(&draft).is_empty());
        let purge = event(
            2,
            "payload.purged",
            PurgedEvent {
                requested: vec![y()],
                released: vec![(1, y())],
                removed: vec![],
                shared: vec![],
                reason: "pasted a secret".into(),
            }
            .to_value(),
        );
        assert!(projector.keys(&purge).is_empty());

        let replaced = event(
            3,
            "plan.replaced",
            replaced_payload(1, 4, x(), y(), "Ada Owner", T),
        );
        assert!(projector.apply(&replaced, &[]).is_err());
    }
}
