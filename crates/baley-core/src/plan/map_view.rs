//! The `evidence_map` view: one document per approved plan, keyed by its
//! phase and number, holding the plan's evidence items exactly as approved
//! (design 0005 section 6 Views, D-07, D-17).
//!
//! Only `plan.approved` and `plan.replaced` reach it, so a draft's map never
//! appears. Readiness against the current truths is never stored: it is
//! worked out when the map is read.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::{PLAN_APPROVED, PLAN_REPLACED};
use super::judge::{ApprovedItem, EvidenceKind};
use super::phase_plan::{REPLACED, text, whole};
use super::plan_view::{APPROVED, key_of, plan_key};

/// The view's name.
pub const EVIDENCE_MAP_VIEW: &str = "evidence_map";

/// The view's declaration: the phase and the plan number as integer keys,
/// and no index.
pub fn evidence_map_spec() -> ViewSpec {
    ViewSpec {
        name: EVIDENCE_MAP_VIEW.into(),
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

/// The key of plan `plan` of phase `phase`'s map.
pub fn evidence_map_key(phase: u32, plan: u32) -> DocKey {
    plan_key(phase, plan)
}

/// Keeps the `evidence_map` view current from `plan.approved` and
/// `plan.replaced`.
///
/// An approval puts a document holding `phase`, `plan`, `items` (the
/// event's inline map, each item whole as approved, its check ids, truth
/// versions and `level` among them) and `state` `approved`. A replacement
/// sets `state` `replaced` and keeps the items, so the replaced plan's map
/// stays readable (D-07); one for a plan not held is refused.
pub struct EvidenceMapProjector {
    spec: ViewSpec,
}

impl EvidenceMapProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: evidence_map_spec(),
        }
    }
}

impl Default for EvidenceMapProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for EvidenceMapProjector {
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
            let Some((_, Value::Object(held))) = documents.iter().find(|(at, _)| *at == key) else {
                return Err(refuse("the replaced plan's map is not held"));
            };
            let mut held = held.clone();
            held.insert("state".into(), json!(REPLACED));
            Value::Object(held)
        } else {
            let Some(Value::Array(items)) = payload.get("evidence_map") else {
                return Err(refuse("evidence_map is not a list"));
            };
            json!({
                "phase": whole(payload, "phase").map_err(|message| refuse(&message))?,
                "plan": whole(payload, "plan").map_err(|message| refuse(&message))?,
                "items": items,
                "state": APPROVED,
            })
        };
        Ok(vec![Change::Put { key, body }])
    }
}

/// The items of an `evidence_map` document as the plan judge takes them,
/// each with the document's plan number and the item whole as its
/// definition, so the facts the binary gathers come from one reading. The
/// caller chooses which documents to read, a replaced plan's or not.
/// `None` when the plan number or any item cannot be read: an item is never
/// skipped.
pub fn approved_items(document: &Value) -> Option<Vec<ApprovedItem>> {
    let plan = whole(document, "plan").ok()?;
    document
        .get("items")?
        .as_array()?
        .iter()
        .map(|item| {
            let kind = match text(item, "kind").ok()? {
                "artifact" => EvidenceKind::Artifact,
                "link" => EvidenceKind::Link,
                "check" => EvidenceKind::Check,
                "observation" => EvidenceKind::Observation,
                _ => return None,
            };
            Some(ApprovedItem {
                plan,
                id: text(item, "id").ok()?.to_owned(),
                story: whole(item, "story").ok()?,
                truth: text(item, "truth").ok()?.to_owned(),
                truth_version: whole(item, "truth_version").ok()?,
                kind,
                definition: item.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected documents and items are written out from design 0005
    //! section 6, D-07, D-17, phase 8 D-06 and the plan's test data, never
    //! from running this code.

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};

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

    /// Plan 2 of phase 1 approved at X for story 1 with size 1 and `map`.
    fn approved_two(map: &[Value]) -> Event {
        let body = r();
        let approval = PlanApproval {
            phase: 1,
            plan: 2,
            digest: x(),
            stories: &[1],
            size: 1,
            suite: None,
            question_set: S,
            evidence_map: map,
            body: &body,
            owner: "Ada Owner",
            at: T,
        };
        event(1, "plan.approved", approval.payload())
    }

    fn put(changes: Vec<Change>) -> (DocKey, Value) {
        let [Change::Put { key, body }] = &changes[..] else {
            panic!("one document put, got {changes:?}");
        };
        (key.clone(), body.clone())
    }

    #[test]
    fn an_evidence_map_changed_on_replay_or_holding_a_draft_is_caught() {
        let projector = EvidenceMapProjector::new();
        let approved = approved_two(&[c1(), c2(), o1()]);
        let first = put(projector.apply(&approved, &[]).unwrap());
        let second = put(projector.apply(&approved, &[]).unwrap());
        assert_eq!(first, second);
        let (key, document) = first;
        assert_eq!(
            key,
            DocKey(vec![
                baley_store::KeyValue::Integer(1),
                baley_store::KeyValue::Integer(2)
            ])
        );
        assert_eq!(document["items"], json!([c1(), c2(), o1()]));
        assert_eq!(document["items"][0]["spec"]["level"], "unit");
        assert_eq!(document["items"][1]["spec"]["level"], "integration");
        assert_eq!(document["state"], "approved");

        let draft = event(
            2,
            "scope.submitted",
            submitted_payload(x(), "plan", "phase/1", &r()),
        );
        assert!(projector.keys(&draft).is_empty());

        let replaced = event(
            3,
            "plan.replaced",
            replaced_payload(1, 2, x(), y(), "Ada Owner", T),
        );
        assert_eq!(projector.keys(&replaced), std::slice::from_ref(&key));
        let (_, after) = put(projector.apply(&replaced, &[(key, document)]).unwrap());
        assert_eq!(after["items"], json!([c1(), c2(), o1()]));
        assert_eq!(after["state"], "replaced");
    }

    #[test]
    fn a_map_read_into_items_of_another_plan_or_kind_is_caught() {
        let projector = EvidenceMapProjector::new();
        let (_, document) = put(projector.apply(&approved_two(&[c1(), o1()]), &[]).unwrap());
        assert_eq!(
            approved_items(&document),
            Some(vec![
                ApprovedItem {
                    plan: 2,
                    id: "c1".into(),
                    story: 1,
                    truth: "t1".into(),
                    truth_version: 1,
                    kind: EvidenceKind::Check,
                    definition: c1(),
                },
                ApprovedItem {
                    plan: 2,
                    id: "o1".into(),
                    story: 1,
                    truth: "t2".into(),
                    truth_version: 2,
                    kind: EvidenceKind::Observation,
                    definition: o1(),
                },
            ])
        );

        let mut broken = document;
        broken["items"][1].as_object_mut().unwrap().remove("kind");
        assert_eq!(approved_items(&broken), None);
    }
}
