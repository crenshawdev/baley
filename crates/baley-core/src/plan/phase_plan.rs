//! The `phase_plan` view: one document per phase, keyed by its number,
//! holding the phase's planning state (design 0005 section 6 Views, PLN-R9,
//! PLN-R14, Figure 3).
//!
//! A phase is active while one of its plans is in force and it is not
//! withdrawn. Completion (`phase.completed`) is Build 5's, so until then
//! only a withdrawal or every plan replaced away ends a phase's activity,
//! and a project with an approved plan cannot plan another phase (D-02).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order, Projector,
    ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::{PLAN_APPROVED, PLAN_REPLACED};
use crate::scope::PHASE_WITHDRAWN;

/// The view's name.
pub const PHASE_PLAN_VIEW: &str = "phase_plan";
/// The index that lists phases by status, then number, so the active
/// phases are found without reading every phase.
pub const PHASE_PLAN_STATUS_INDEX: &str = "by_status";
/// The status of a phase with a plan in force that is not withdrawn.
pub const ACTIVE: &str = "active";
/// The status of every other phase.
pub const INACTIVE: &str = "inactive";
/// The state of a plan in force.
pub const IN_FORCE: &str = "in-force";
/// The state of a plan a later plan replaced.
pub const REPLACED: &str = "replaced";

/// The key field: the phase number.
const PHASE_FIELD: &str = "phase";
/// The phase's status, which the index leads with.
const STATUS_FIELD: &str = "status";

/// The view's declaration: the phase number as an integer key and one
/// index over the status, then the number.
pub fn phase_plan_spec() -> ViewSpec {
    ViewSpec {
        name: PHASE_PLAN_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: PHASE_FIELD.into(),
            kind: FieldKind::Integer,
        }],
        indexes: vec![IndexSpec {
            name: PHASE_PLAN_STATUS_INDEX.into(),
            fields: vec![
                IndexField {
                    name: STATUS_FIELD.into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                },
                IndexField {
                    name: PHASE_FIELD.into(),
                    kind: FieldKind::Integer,
                    order: Order::Ascending,
                },
            ],
        }],
        page_bound: 100,
    }
}

/// The key of phase `phase`'s document.
pub fn phase_plan_key(phase: u32) -> DocKey {
    DocKey(vec![KeyValue::Integer(i64::from(phase))])
}

/// Keeps the `phase_plan` view current from `plan.approved`,
/// `plan.replaced` and `phase.withdrawn`.
///
/// A document holds these members:
/// - `phase`: the phase's number.
/// - `plans`: its approved plans in number order, each `{plan, digest,
///   stories, size, state, replaced_by}`, `state` `in-force` or `replaced`
///   and `replaced_by` the replacing draft's digest or null.
/// - `highest`: the highest plan number ever assigned, replaced plans
///   included, 0 before the first.
/// - `size`: the tasks of the plans in force, each plan counted once
///   however many stories it serves.
/// - `withdrawn`: whether the phase was withdrawn.
/// - `status`: `active` while a plan is in force and the phase is not
///   withdrawn, `inactive` otherwise, worked out on every change.
///
/// An approval whose number is not above `highest` is refused, so no
/// number is projected twice (PLN-R14, D-07). A replacement of a plan not
/// in force, or at a digest other than the one approved, is refused; the
/// replaced plan keeps its digest, stories and size. A withdrawal of a
/// phase with no document creates one.
pub struct PhasePlanProjector {
    spec: ViewSpec,
}

impl PhasePlanProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: phase_plan_spec(),
        }
    }
}

impl Default for PhasePlanProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for PhasePlanProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[PLAN_APPROVED, PLAN_REPLACED, PHASE_WITHDRAWN]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        phase_of(event).map(phase_plan_key).into_iter().collect()
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
        let phase = phase_of(event).ok_or_else(|| refuse("the phase is not a phase number"))?;
        let key = phase_plan_key(phase);
        let mut document = match documents.iter().find(|(at, _)| *at == key) {
            Some((_, Value::Object(document))) => document.clone(),
            Some(_) => return Err(refuse("the phase's document is not an object")),
            None => fresh(phase),
        };
        let payload = &event.payload;
        match event.type_name.as_str() {
            PLAN_APPROVED => approve(&mut document, payload),
            PLAN_REPLACED => replace(&mut document, payload),
            PHASE_WITHDRAWN => {
                document.insert("withdrawn".into(), json!(true));
                Ok(())
            }
            _ => Err("the view does not handle this event".into()),
        }
        .and_then(|()| settle(&mut document))
        .map_err(|message| refuse(&message))?;
        Ok(vec![Change::Put {
            key,
            body: Value::Object(document),
        }])
    }
}

/// The document of a phase nothing has been projected for.
fn fresh(phase: u32) -> Map<String, Value> {
    let Value::Object(document) = json!({
        PHASE_FIELD: phase,
        "plans": [],
        "highest": 0,
        "size": 0,
        "withdrawn": false,
        STATUS_FIELD: INACTIVE,
    }) else {
        unreachable!("a JSON object literal is an object")
    };
    document
}

/// Adds the approved plan in force and raises `highest` to its number.
fn approve(document: &mut Map<String, Value>, payload: &Value) -> Result<(), String> {
    let plan = whole(payload, "plan")?;
    let digest = text(payload, "digest")?;
    let stories = wholes(payload, "stories")?;
    let size = payload
        .get("size")
        .and_then(Value::as_u64)
        .ok_or("size is not a whole number")?;
    let highest = document
        .get("highest")
        .and_then(Value::as_u64)
        .ok_or("the document's highest is not a whole number")?;
    if u64::from(plan) <= highest {
        return Err(format!(
            "plan {plan} is not above the highest number assigned, {highest}"
        ));
    }
    plans(document)?.push(json!({
        "plan": plan,
        "digest": digest,
        "stories": stories,
        "size": size,
        "state": IN_FORCE,
        "replaced_by": null,
    }));
    document.insert("highest".into(), json!(plan));
    Ok(())
}

/// Marks the plan in force at `old_digest` replaced by `new_digest`.
fn replace(document: &mut Map<String, Value>, payload: &Value) -> Result<(), String> {
    let plan = whole(payload, "plan")?;
    let old = text(payload, "old_digest")?;
    let new = text(payload, "new_digest")?;
    let held = plans(document)?
        .iter_mut()
        .find(|held| held["plan"] == plan && held["state"] == IN_FORCE)
        .ok_or_else(|| format!("plan {plan} is not in force"))?;
    if held["digest"] != old {
        return Err(format!("plan {plan} was not approved at {old}"));
    }
    held["state"] = json!(REPLACED);
    held["replaced_by"] = json!(new);
    Ok(())
}

/// Works out the size and status from the plans in force and the
/// withdrawal, so neither can drift from the plans.
fn settle(document: &mut Map<String, Value>) -> Result<(), String> {
    let mut size = 0u64;
    let mut in_force = false;
    for held in plans(document)?.iter() {
        if held["state"] == IN_FORCE {
            in_force = true;
            size += held["size"]
                .as_u64()
                .ok_or("a held plan's size is not a whole number")?;
        }
    }
    let withdrawn = document
        .get("withdrawn")
        .and_then(Value::as_bool)
        .ok_or("the document's withdrawn is not true or false")?;
    let status = if in_force && !withdrawn {
        ACTIVE
    } else {
        INACTIVE
    };
    document.insert("size".into(), json!(size));
    document.insert(STATUS_FIELD.into(), json!(status));
    Ok(())
}

fn plans(document: &mut Map<String, Value>) -> Result<&mut Vec<Value>, String> {
    match document.get_mut("plans") {
        Some(Value::Array(plans)) => Ok(plans),
        _ => Err("the document's plans are not a list".into()),
    }
}

/// The phase an event names: `number` on `phase.withdrawn`, `phase` on a
/// plan event.
fn phase_of(event: &Event) -> Option<u32> {
    let member = if event.type_name == PHASE_WITHDRAWN {
        "number"
    } else {
        PHASE_FIELD
    };
    whole(&event.payload, member).ok()
}

fn whole(payload: &Value, name: &str) -> Result<u32, String> {
    payload
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{name} is not a whole number"))
}

fn text<'a>(payload: &'a Value, name: &str) -> Result<&'a str, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} is not text"))
}

fn wholes(payload: &Value, name: &str) -> Result<Vec<u32>, String> {
    payload
        .get(name)
        .and_then(Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .map(|value| value.as_u64().and_then(|value| u32::try_from(value).ok()))
                .collect()
        })
        .ok_or_else(|| format!("{name} is not a list of whole numbers"))
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0005 Figure 3, PLN-R9,
    //! PLN-R14, D-02, D-07 and the plan's test data, never from running this
    //! code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};

    use super::*;
    use crate::plan::event::{PlanApproval, replaced_payload};
    use crate::plan::fixtures::{c1, c2, o1};
    use crate::scope::phase_withdrawn_payload;

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const T: &str = "2026-10-31T09:00:00Z";

    type View = BTreeMap<DocKey, Value>;

    fn x() -> Hash {
        Hash([0x5c; 32])
    }

    fn y() -> Hash {
        Hash([0x6d; 32])
    }

    fn z() -> Hash {
        Hash([0x7e; 32])
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

    /// A(n, p, d, s, k): plan `n` of phase `p` approved at digest `d` for
    /// stories `s` with size `k`, map `[C1, C2, O1]`, no suite and set S.
    fn a(seq: u64, n: u32, p: u32, d: Hash, s: &[u32], k: usize) -> Event {
        let map = [c1(), c2(), o1()];
        let body = PayloadRef {
            hash: y(),
            bytes: 1_024,
            class: RetentionClass::Record,
        };
        let approval = PlanApproval {
            phase: p,
            plan: n,
            digest: d,
            stories: s,
            size: k,
            suite: None,
            question_set: S,
            evidence_map: &map,
            body: &body,
            owner: "Ada Owner",
            at: T,
        };
        event(seq, "plan.approved", approval.payload())
    }

    fn replaced(seq: u64, phase: u32, plan: u32, old: Hash, new: Hash) -> Event {
        event(
            seq,
            "plan.replaced",
            replaced_payload(phase, plan, old, new, "Ada Owner", T),
        )
    }

    fn withdrawn(seq: u64, phase: u32) -> Event {
        event(seq, "phase.withdrawn", phase_withdrawn_payload(phase, z()))
    }

    /// Applies `event` to `view` as the store would, or gives the refusal.
    fn step(view: &mut View, event: &Event) -> Result<(), ProjectorError> {
        let projector = PhasePlanProjector::new();
        let read: Vec<(DocKey, Value)> = projector
            .keys(event)
            .into_iter()
            .filter_map(|key| view.get(&key).cloned().map(|body| (key, body)))
            .collect();
        for change in projector.apply(event, &read)? {
            match change {
                Change::Put { key, body } => {
                    view.insert(key, body);
                }
                Change::Delete { key } => {
                    view.remove(&key);
                }
            }
        }
        Ok(())
    }

    fn fold(events: &[Event]) -> View {
        let mut view = View::new();
        for event in events {
            step(&mut view, event).unwrap();
        }
        view
    }

    fn held(
        plan: u32,
        digest: Hash,
        stories: &[u32],
        size: u32,
        replaced_by: Option<Hash>,
    ) -> Value {
        json!({"plan": plan, "digest": digest.to_hex(), "stories": stories, "size": size,
            "state": if replaced_by.is_some() { "replaced" } else { "in-force" },
            "replaced_by": replaced_by.map(|digest| digest.to_hex())})
    }

    fn one() -> DocKey {
        DocKey(vec![KeyValue::Integer(1)])
    }

    #[test]
    fn a_phase_size_counted_per_story_or_left_after_replacement_is_caught() {
        let approvals = [
            a(1, 1, 1, x(), &[1, 2], 2),
            a(2, 2, 1, y(), &[1], 1),
            a(3, 3, 1, z(), &[2], 3),
        ];
        let view = fold(&approvals);
        let phase = &view[&one()];
        assert_eq!(phase["size"], 6);
        assert_eq!(phase["highest"], 3);
        assert_eq!(phase["status"], "active");

        let mut events = approvals.to_vec();
        events.push(replaced(4, 1, 1, x(), z()));
        let view = fold(&events);
        assert_eq!(view.len(), 1);
        assert_eq!(
            view[&one()],
            json!({"phase": 1, "highest": 3, "size": 4, "withdrawn": false,
                "status": "active",
                "plans": [held(1, x(), &[1, 2], 2, Some(z())), held(2, y(), &[1], 1, None),
                    held(3, z(), &[2], 3, None)]})
        );
    }

    #[test]
    fn a_phase_left_active_after_withdrawal_or_its_last_plan_replaced_is_caught() {
        let mut view = View::new();
        step(&mut view, &a(1, 1, 1, x(), &[1], 1)).unwrap();
        assert_eq!(view[&one()]["status"], "active");
        step(&mut view, &withdrawn(2, 1)).unwrap();
        assert_eq!(view[&one()]["status"], "inactive");
        assert_eq!(view[&one()]["withdrawn"], true);

        let two = DocKey(vec![KeyValue::Integer(2)]);
        let mut view = View::new();
        step(&mut view, &a(1, 1, 2, x(), &[1], 1)).unwrap();
        assert_eq!(view[&two]["status"], "active");
        step(&mut view, &replaced(2, 2, 1, x(), y())).unwrap();
        assert_eq!(view[&two]["status"], "inactive");
        assert_eq!(view[&two]["size"], 0);

        let view = fold(&[withdrawn(1, 3)]);
        assert_eq!(
            view[&DocKey(vec![KeyValue::Integer(3)])],
            json!({"phase": 3, "plans": [], "highest": 0, "size": 0, "withdrawn": true,
                "status": "inactive"})
        );
    }

    #[test]
    fn a_plan_number_reused_or_a_plan_replaced_twice_accepted_is_caught() {
        let refused = |first: &[Event], second: &Event| {
            let mut view = fold(first);
            let before = view.clone();
            assert!(step(&mut view, second).is_err(), "{}", second.payload);
            assert_eq!(view, before);
        };
        refused(&[a(1, 2, 1, x(), &[1], 1)], &a(2, 2, 1, y(), &[1], 1));
        refused(&[a(1, 2, 1, x(), &[1], 1)], &a(2, 1, 1, y(), &[1], 1));
        refused(
            &[a(1, 1, 1, x(), &[1], 1), replaced(2, 1, 1, x(), y())],
            &replaced(3, 1, 1, x(), y()),
        );
        refused(&[a(1, 1, 1, x(), &[1], 1)], &replaced(2, 1, 1, y(), z()));
    }
}
