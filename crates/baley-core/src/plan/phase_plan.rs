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

use super::event::{
    PLAN_APPROVED, PLAN_CHECK_ISSUED, PLAN_CHECKED, PLAN_CONTINUATION, PLAN_CONTINUATION_FOLLOWED,
    PLAN_REPLACED,
};
use crate::questions::QUESTIONS_OPENED;
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
        version: 2,
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
/// `plan.replaced`, `phase.withdrawn`, a plan draft's `questions.opened`,
/// `plan.continuation`, `plan.continuation_followed`, `plan.checked` and
/// `plan.check_issued`.
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
/// - `draft`: the phase's one unapproved plan draft, `{digest, set}` from
///   the latest plan draft's set, or null; its approval clears it (D-03).
/// - `continuation`: the phase's latest planning continuation, `{stories,
///   edit, work_order, followed}`, or null (D-09, D-11).
/// - `checks`: the phase's check lineages that bought a revision, in the
///   order their round 1 returned, each `{revision, first: {seq,
///   plan_digest}, recheck}`: the planner revision order round 1 issued,
///   that result's sequence and draft, and `{work_order, seq}` of the
///   round-2 issue or null (PLN-R16, D-02). Every lineage is kept.
///
/// An approval whose number is not above `highest` is refused, so no
/// number is projected twice (PLN-R14, D-07), and so is one whose size would
/// take the phase's size past a `u64`. A replacement of a plan not
/// in force, or at a digest other than the one approved, is refused; the
/// replaced plan keeps its digest, stories and size. A withdrawal, a draft
/// or a continuation of a phase with no document creates one. A story's
/// question set changes nothing. A followed marker naming another edit or
/// order than the latest continuation's, or one already followed, is
/// refused.
///
/// A round-1 `plan.checked` with a revision order appends a lineage, and
/// one with none adds nothing. A `plan.check_issued` sets `recheck` on the
/// lineage whose `first` its `previous_check` names, and is refused when
/// none does or that lineage's round 2 was already issued, so no third
/// round is projected. A round-2 `plan.checked` changes nothing and is
/// refused unless its lineage holds a round 2; a round other than 1 or 2
/// is refused. A check never changes `draft`, which only a plan draft's
/// set sets.
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
        &[
            PLAN_APPROVED,
            PLAN_REPLACED,
            PHASE_WITHDRAWN,
            QUESTIONS_OPENED,
            PLAN_CONTINUATION,
            PLAN_CONTINUATION_FOLLOWED,
            PLAN_CHECKED,
            PLAN_CHECK_ISSUED,
        ]
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
        // A story's question set is no plan draft, so it changes nothing here.
        if event.type_name == QUESTIONS_OPENED && !plan_set(&event.payload) {
            return Ok(Vec::new());
        }
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
            QUESTIONS_OPENED => open_draft(&mut document, payload),
            PLAN_CONTINUATION => continuation(&mut document, payload),
            PLAN_CONTINUATION_FOLLOWED => follow(&mut document, payload),
            PLAN_CHECKED => checked(&mut document, payload, event.seq),
            PLAN_CHECK_ISSUED => check_issued(&mut document, payload, event.seq),
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
        "draft": null,
        "continuation": null,
        "checks": [],
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
    // The approved draft no longer waits.
    if document.get("draft").and_then(|draft| draft.get("digest")) == Some(&json!(digest)) {
        document.insert("draft".into(), Value::Null);
    }
    Ok(())
}

/// Makes a plan draft's set the phase's current draft, replacing any
/// earlier one, since a phase holds one unapproved draft (D-03).
fn open_draft(document: &mut Map<String, Value>, payload: &Value) -> Result<(), String> {
    let digest = text(payload, "plan_digest")?;
    let set = text(payload, "set")?;
    document.insert("draft".into(), json!({"digest": digest, "set": set}));
    Ok(())
}

/// Makes a continuation the phase's latest, not yet followed.
fn continuation(document: &mut Map<String, Value>, payload: &Value) -> Result<(), String> {
    let stories = wholes(payload, "stories")?;
    let edit = text(payload, "edit")?;
    let work_order = text(payload, "work_order")?;
    document.insert(
        "continuation".into(),
        json!({"stories": stories, "edit": edit, "work_order": work_order, "followed": false}),
    );
    Ok(())
}

/// Marks the latest continuation followed. One naming another edit or
/// order, or one already followed, is refused, so an order is handed to a
/// session once (D-09, D-11).
fn follow(document: &mut Map<String, Value>, payload: &Value) -> Result<(), String> {
    let edit = text(payload, "edit")?;
    let work_order = text(payload, "work_order")?;
    let Some(Value::Object(latest)) = document.get_mut("continuation") else {
        return Err("the phase has no continuation".into());
    };
    if latest.get("edit") != Some(&json!(edit))
        || latest.get("work_order") != Some(&json!(work_order))
    {
        return Err(format!(
            "the latest continuation is not edit {edit} with order {work_order}"
        ));
    }
    if latest.get("followed") != Some(&json!(false)) {
        return Err(format!("order {work_order} was already followed"));
    }
    latest.insert("followed".into(), json!(true));
    Ok(())
}

/// Starts a lineage for a round-1 result that bought a revision, after
/// every earlier one, so a revision order's submit still finds its own
/// lineage once a later one starts (D-02). A round-1 result with no
/// revision has no round 2 to find, so it adds nothing. A round-2 result
/// changes nothing, but one whose round 2 was never issued is refused.
fn checked(document: &mut Map<String, Value>, payload: &Value, seq: u64) -> Result<(), String> {
    let digest = text(payload, "plan_digest")?;
    match whole(payload, "round")? {
        1 => match payload.get("revision") {
            Some(Value::Null) => Ok(()),
            Some(Value::String(revision)) => {
                let lineage = json!({
                    "revision": revision,
                    "first": {"seq": seq, "plan_digest": digest},
                    "recheck": null,
                });
                checks(document)?.push(lineage);
                Ok(())
            }
            _ => Err("revision is not text or null".into()),
        },
        2 => {
            let first = previous_check(payload)?;
            let issued = checks(document)?
                .iter()
                .any(|lineage| lineage["first"] == first && !lineage["recheck"].is_null());
            if issued {
                Ok(())
            } else {
                Err(format!("no round 2 was issued against the check {first}"))
            }
        }
        round => Err(format!("round {round} is not 1 or 2")),
    }
}

/// Binds round 2 to the lineage whose first check it names. One naming no
/// lineage, or a lineage whose round 2 was already issued, is refused, so
/// no third round is ever projected (D-02, D-09).
fn check_issued(
    document: &mut Map<String, Value>,
    payload: &Value,
    seq: u64,
) -> Result<(), String> {
    let work_order = text(payload, "work_order")?;
    let first = previous_check(payload)?;
    let lineage = checks(document)?
        .iter_mut()
        .find(|lineage| lineage["first"] == first)
        .ok_or_else(|| format!("no lineage starts at the check {first}"))?;
    if !lineage["recheck"].is_null() {
        return Err(format!("round 2 of the check {first} was already issued"));
    }
    lineage["recheck"] = json!({"work_order": work_order, "seq": seq});
    Ok(())
}

/// The `previous_check` a round-2 event names, rebuilt from its two
/// members so it compares equal to a lineage's `first`.
fn previous_check(payload: &Value) -> Result<Value, String> {
    let previous = payload
        .get("previous_check")
        .ok_or("previous_check is missing")?;
    let seq = previous
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or("previous_check's seq is not a whole number")?;
    let digest = text(previous, "plan_digest")?;
    Ok(json!({"seq": seq, "plan_digest": digest}))
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
            let plan_size = held["size"]
                .as_u64()
                .ok_or("a held plan's size is not a whole number")?;
            // A total past u64 would panic or wrap, so it is refused.
            size = size
                .checked_add(plan_size)
                .ok_or("the phase's size would exceed a u64")?;
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

fn checks(document: &mut Map<String, Value>) -> Result<&mut Vec<Value>, String> {
    match document.get_mut("checks") {
        Some(Value::Array(checks)) => Ok(checks),
        _ => Err("the document's checks are not a list".into()),
    }
}

/// Whether a `questions.opened` payload is a plan draft's set.
fn plan_set(payload: &Value) -> bool {
    payload.get("subject").and_then(Value::as_str) == Some("plan")
}

/// The phase an event names: `number` on `phase.withdrawn`, `phase` on a
/// plan event or a plan draft's set, none on a story's set.
fn phase_of(event: &Event) -> Option<u32> {
    let member = match event.type_name.as_str() {
        PHASE_WITHDRAWN => "number",
        QUESTIONS_OPENED if !plan_set(&event.payload) => return None,
        _ => PHASE_FIELD,
    };
    whole(&event.payload, member).ok()
}

/// The member `name` of `payload` as a whole number within `u32`.
pub(super) fn whole(payload: &Value, name: &str) -> Result<u32, String> {
    payload
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{name} is not a whole number"))
}

/// The member `name` of `payload` as text.
pub(super) fn text<'a>(payload: &'a Value, name: &str) -> Result<&'a str, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} is not text"))
}

/// The member `name` of `payload` as a list of whole numbers within `u32`.
pub(super) fn wholes(payload: &Value, name: &str) -> Result<Vec<u32>, String> {
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
    use crate::plan::event::{
        CheckIssue, CheckResult, PlanApproval, PreviousCheck, continuation_followed_payload,
        continuation_payload, replaced_payload,
    };
    use crate::plan::fixtures::{c1, c2, o1};
    use crate::policy::RecheckScope;
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
                "status": "active", "draft": null, "continuation": null, "checks": [],
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
                "status": "inactive", "draft": null, "continuation": null, "checks": []})
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
        // A replaced plan's number stays assigned though nothing is in force.
        refused(
            &[a(1, 1, 1, x(), &[1], 1), replaced(2, 1, 1, x(), y())],
            &a(3, 1, 1, y(), &[1], 1),
        );
    }

    #[test]
    fn a_phase_size_overflowing_its_total_is_caught() {
        let mut huge = a(1, 1, 1, x(), &[1], 1);
        huge.payload["size"] = json!(u64::MAX);
        let mut view = fold(&[huge]);
        assert_eq!(view[&one()]["size"], json!(u64::MAX));
        let before = view.clone();
        assert!(step(&mut view, &a(2, 2, 1, y(), &[1], 1)).is_err());
        assert_eq!(view, before);
    }

    #[test]
    fn a_phase_event_missing_from_the_handled_types_is_caught() {
        assert_eq!(
            PhasePlanProjector::new().handles(),
            [
                "plan.approved",
                "plan.replaced",
                "phase.withdrawn",
                "questions.opened",
                "plan.continuation",
                "plan.continuation_followed",
                "plan.checked",
                "plan.check_issued",
            ]
        );
    }

    /// A plan draft's set `set` of phase 1 for the draft at `digest`.
    fn plan_set(seq: u64, set: &str, digest: Hash) -> Event {
        event(
            seq,
            "questions.opened",
            json!({"set": set, "subject": "plan", "phase": 1, "plan_digest": digest.to_hex(),
                "questions": [], "at": T}),
        )
    }

    #[test]
    fn a_plan_draft_kept_after_a_newer_draft_or_its_approval_is_caught() {
        let draft = |set: &str, digest: Hash| json!({"digest": digest.to_hex(), "set": set});
        let mut view = View::new();
        step(&mut view, &plan_set(1, "A", x())).unwrap();
        assert_eq!(view[&one()]["draft"], draft("A", x()));
        step(&mut view, &plan_set(2, "B", y())).unwrap();
        assert_eq!(view[&one()]["draft"], draft("B", y()));

        let story = event(
            3,
            "questions.opened",
            json!({"set": "C", "subject": "story", "story": 1, "work_order": "W1",
                "questions": [], "at": T}),
        );
        step(&mut view, &story).unwrap();
        assert_eq!(view.len(), 1);
        assert_eq!(view[&one()]["draft"], draft("B", y()));

        step(&mut view, &a(4, 1, 1, y(), &[1], 1)).unwrap();
        assert_eq!(view[&one()]["draft"], Value::Null);
    }

    #[test]
    fn a_continuation_replaced_by_an_older_one_or_followed_twice_is_caught() {
        let mut view = View::new();
        let first = event(
            1,
            "plan.continuation",
            continuation_payload(1, &[1, 2], x(), "O1"),
        );
        step(&mut view, &first).unwrap();
        assert_eq!(
            view[&one()]["continuation"],
            json!({"stories": [1, 2], "edit": x().to_hex(), "work_order": "O1",
                "followed": false})
        );
        let second = event(
            2,
            "plan.continuation",
            continuation_payload(1, &[2, 1], y(), "O2"),
        );
        step(&mut view, &second).unwrap();
        assert_eq!(
            view[&one()]["continuation"],
            json!({"stories": [2, 1], "edit": y().to_hex(), "work_order": "O2",
                "followed": false})
        );

        let followed = |seq, edit, order| {
            event(
                seq,
                "plan.continuation_followed",
                continuation_followed_payload(1, edit, order),
            )
        };
        // Stale in both, in only the edit, and in only the order.
        for (seq, edit, order) in [(3, x(), "O1"), (3, x(), "O2"), (3, y(), "O1")] {
            let before = view.clone();
            assert!(step(&mut view, &followed(seq, edit, order)).is_err());
            assert_eq!(view, before);
        }
        step(&mut view, &followed(4, y(), "O2")).unwrap();
        assert_eq!(
            view[&one()]["continuation"],
            json!({"stories": [2, 1], "edit": y().to_hex(), "work_order": "O2",
                "followed": true})
        );
        let before = view.clone();
        assert!(step(&mut view, &followed(5, y(), "O2")).is_err());
        assert_eq!(view, before);
    }

    const V: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const V2: &str = "1b2c3d4e-5f60-4b7c-8d9e-0f1a2b3c4d5e";
    const W2: &str = "2c3d4e5f-6071-4c8d-9eaf-1a2b3c4d5e6f";
    const W3: &str = "3d4e5f60-7182-4d9e-8fb0-2b3c4d5e6f70";

    /// The round-1 result `seq`, `{seq, plan_digest}`, as a re-check names it.
    fn first(seq: u64, plan_digest: Hash) -> PreviousCheck {
        PreviousCheck { seq, plan_digest }
    }

    /// K: a `plan.checked` of phase 1 at `seq` for the draft at `digest`
    /// with `revision`; round 2 exactly when it names `previous`.
    fn k(seq: u64, digest: Hash, revision: Option<&str>, previous: Option<PreviousCheck>) -> Event {
        let result = CheckResult {
            phase: 1,
            plan_digest: digest,
            findings: &[],
            work_order: "4e5f6071-8293-4eaf-9fc1-3c4d5e6f7081",
            scope: RecheckScope::Full,
            previous_check: previous,
            truth_versions: &[],
            material: &[],
            scope_policy_version: 3,
            revision,
        };
        event(seq, "plan.checked", result.payload())
    }

    /// I: a `plan.check_issued` of phase 1 at `seq` for `order`, naming
    /// `previous` as its first check.
    fn i(seq: u64, order: &str, previous: PreviousCheck) -> Event {
        let issue = CheckIssue {
            phase: 1,
            work_order: order,
            previous_check: previous,
            original_plan_digest: previous.plan_digest,
            plan_digest: y(),
            truth_versions: &[],
            scope: RecheckScope::Full,
            scope_policy_version: 3,
            material: &[],
        };
        event(seq, "plan.check_issued", issue.payload())
    }

    fn lineage(revision: &str, seq: u64, digest: Hash, recheck: Value) -> Value {
        json!({"revision": revision, "first": {"seq": seq, "plan_digest": digest.to_hex()},
            "recheck": recheck})
    }

    #[test]
    fn a_round_two_projected_without_its_lineage_or_a_second_time_is_caught() {
        let refused = |view: &mut View, event: &Event| {
            let before = view.clone();
            assert!(step(view, event).is_err(), "{}", event.payload);
            assert_eq!(*view, before);
        };
        let mut view = fold(&[k(5, x(), Some(V), None)]);
        assert_eq!(
            view[&one()]["checks"],
            json!([lineage(V, 5, x(), Value::Null)])
        );
        step(&mut view, &i(7, W2, first(5, x()))).unwrap();
        assert_eq!(
            view[&one()]["checks"][0]["recheck"],
            json!({"work_order": W2, "seq": 7})
        );
        refused(&mut view, &i(9, W3, first(5, x())));

        let mut view = fold(&[k(5, x(), Some(V), None)]);
        refused(&mut view, &i(7, W2, first(4, x())));

        let mut view = fold(&[k(5, x(), None, None)]);
        assert_eq!(view[&one()]["checks"], json!([]));
        refused(&mut view, &i(7, W2, first(5, x())));

        let mut view = fold(&[k(5, x(), Some(V), None)]);
        refused(&mut view, &k(8, y(), None, Some(first(5, x()))));
    }

    #[test]
    fn a_new_lineage_dropping_an_earlier_ones_revision_is_caught() {
        let mut view = fold(&[k(5, x(), Some(V), None), k(11, y(), Some(V2), None)]);
        assert_eq!(
            view[&one()]["checks"],
            json!([
                lineage(V, 5, x(), Value::Null),
                lineage(V2, 11, y(), Value::Null)
            ])
        );
        step(&mut view, &i(13, W2, first(5, x()))).unwrap();
        assert_eq!(
            view[&one()]["checks"],
            json!([
                lineage(V, 5, x(), json!({"work_order": W2, "seq": 13})),
                lineage(V2, 11, y(), Value::Null)
            ])
        );
    }
}
