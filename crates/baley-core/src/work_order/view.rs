//! The `work_order` view: one document per issued order, keyed by its id,
//! holding what it was issued with and its state (design 0001 Views,
//! design 0012).
//!
//! An order is `pending` when issued, `failed` once `work_order.failed`
//! names it and `answered` once a return names it: a submission naming it,
//! for an analyzer's questions order the story question set naming it, and
//! for a checker order the `plan.checked` naming it, so the `live` index no
//! longer finds a checker order whose result is recorded.
//! The accepted result is the approved draft among an order's `drafts`:
//! an approval names only a digest, so no projector can reach the order
//! from it, and the binary reads it from the start approval when it reads
//! the order.

use baley_store::{
    Caller, Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order,
    Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::{WORK_ORDER_FAILED, WORK_ORDER_ISSUED};
use crate::plan::PLAN_CHECKED;
use crate::questions::QUESTIONS_OPENED;
use crate::scope::SCOPE_SUBMITTED;

/// The view's name.
pub const WORK_ORDER_VIEW: &str = "work_order";
/// The index that finds the live orders of one role and subject in one
/// state, newest first.
pub const WORK_ORDER_LIVE_INDEX: &str = "live";

/// An order waiting for its worker's answer.
pub const PENDING: &str = "pending";
/// An order recorded as failed.
pub const FAILED: &str = "failed";
/// An order a submission named.
pub const ANSWERED: &str = "answered";

/// The key field: the order's id.
const ID_FIELD: &str = "id";
/// The role the index leads with.
const ROLE_FIELD: &str = "role";
/// The subject, second in the index.
const SUBJECT_FIELD: &str = "subject";
/// The state, third in the index.
const STATE_FIELD: &str = "state";
/// The issue's sequence, newest first in the index.
const SEQ_FIELD: &str = "seq";
/// The failed order a retry was issued from.
const PARENT_FIELD: &str = "parent";
/// The retry issued from an order, or null.
const CHILD_FIELD: &str = "child";
/// The failure recorded for an order, or null.
const FAILURE_FIELD: &str = "failure";
/// What was returned under an order, first seen first: a draft's digest or
/// a story question set's id.
const DRAFTS_FIELD: &str = "drafts";

/// The view's declaration: the order id as a text key and one index over
/// role, subject and state, then the issue's sequence descending, so a
/// transaction can declare that no pending order of a role and subject
/// exists.
pub fn work_order_spec() -> ViewSpec {
    let text = |name: &str| IndexField {
        name: name.into(),
        kind: FieldKind::Text,
        order: Order::Ascending,
    };
    ViewSpec {
        name: WORK_ORDER_VIEW.into(),
        version: 3,
        key: vec![FieldSpec {
            name: ID_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: vec![IndexSpec {
            name: WORK_ORDER_LIVE_INDEX.into(),
            fields: vec![
                text(ROLE_FIELD),
                text(SUBJECT_FIELD),
                text(STATE_FIELD),
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

/// The key of the order with `id`.
pub fn work_order_key(id: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(id.to_owned())])
}

/// Keeps the `work_order` view current from `work_order.issued`,
/// `work_order.failed`, `scope.submitted`, `questions.opened` and
/// `plan.checked`.
///
/// Issued: a new document holding the payload's members plus `state`
/// `pending`, the event's `seq` and `recorded_at`, `failure` null, `drafts`
/// empty and `child` null. A retry also sets its parent's `child` to its id,
/// so a parent records the one retry issued from it. An id already held, a
/// parent not held or a parent that already has a child is refused.
///
/// Failed: the order gains `state` `failed` and `failure` `{reason, seq,
/// recorded_at}`. A failure for an order not held, or already failed, is
/// refused.
///
/// Submitted: when the caller carries the id of an order held, the digest
/// is added to its `drafts` once, and a pending order becomes `answered`; a
/// failed order stays failed. A submission naming no order, or one not
/// held, changes nothing: it is its own record, and the binary checks the
/// order before it records.
///
/// Opened: a story's question set naming an order held is that order's
/// return, so its id is added to `drafts` once and a pending order becomes
/// `answered`, as a submission does; a failed order stays failed. A set
/// naming no order held, and a plan draft's set, change nothing.
///
/// Checked: the checker order a result names in its `work_order` gets the
/// checked draft's digest added to its `drafts` once and becomes
/// `answered` when pending, as a submission does; a failed order stays
/// failed, so a late result never revives it. A result naming an order not
/// held changes nothing.
pub struct WorkOrderProjector {
    spec: ViewSpec,
}

impl WorkOrderProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: work_order_spec(),
        }
    }
}

impl Default for WorkOrderProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for WorkOrderProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[
            WORK_ORDER_ISSUED,
            WORK_ORDER_FAILED,
            SCOPE_SUBMITTED,
            QUESTIONS_OPENED,
            PLAN_CHECKED,
        ]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        let payload = &event.payload;
        let mut ids: Vec<&str> = match event.type_name.as_str() {
            WORK_ORDER_ISSUED => [ID_FIELD, PARENT_FIELD]
                .iter()
                .filter_map(|field| payload.get(*field).and_then(Value::as_str))
                .collect(),
            WORK_ORDER_FAILED => payload
                .get(ID_FIELD)
                .and_then(Value::as_str)
                .into_iter()
                .collect(),
            QUESTIONS_OPENED => story_set_order(event).into_iter().collect(),
            PLAN_CHECKED => checked_order(event).into_iter().collect(),
            _ => caller_order(event).into_iter().collect(),
        };
        ids.dedup();
        ids.into_iter().map(work_order_key).collect()
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: String| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let held = |id: &str| {
            let key = work_order_key(id);
            documents
                .iter()
                .find(|(at, _)| *at == key)
                .and_then(|(_, document)| document.as_object())
        };
        match event.type_name.as_str() {
            WORK_ORDER_ISSUED => {
                let Some(payload) = event.payload.as_object() else {
                    return Err(refuse("the payload is not an object".into()));
                };
                let id = text(payload, ID_FIELD).ok_or_else(|| refuse("id is not text".into()))?;
                for field in [ROLE_FIELD, SUBJECT_FIELD] {
                    if text(payload, field).is_none() {
                        return Err(refuse(format!("{field} is not text")));
                    }
                }
                if held(id).is_some() {
                    return Err(refuse(format!("order {id} is already held")));
                }
                let mut changes = Vec::new();
                match payload.get(PARENT_FIELD) {
                    Some(Value::Null) => {}
                    Some(Value::String(parent)) => {
                        let Some(document) = held(parent) else {
                            return Err(refuse(format!("parent {parent} is not held")));
                        };
                        if document.get(CHILD_FIELD) != Some(&Value::Null) {
                            return Err(refuse(format!("parent {parent} already has a retry")));
                        }
                        let mut document = document.clone();
                        document.insert(CHILD_FIELD.into(), json!(id));
                        changes.push(Change::Put {
                            key: work_order_key(parent),
                            body: Value::Object(document),
                        });
                    }
                    _ => return Err(refuse("parent is not text or null".into())),
                }
                let mut document = payload.clone();
                document.insert(STATE_FIELD.into(), json!(PENDING));
                document.insert(SEQ_FIELD.into(), json!(event.seq));
                document.insert("recorded_at".into(), json!(event.recorded_at));
                document.insert(FAILURE_FIELD.into(), Value::Null);
                document.insert(DRAFTS_FIELD.into(), json!([]));
                document.insert(CHILD_FIELD.into(), Value::Null);
                changes.push(Change::Put {
                    key: work_order_key(id),
                    body: Value::Object(document),
                });
                Ok(changes)
            }
            WORK_ORDER_FAILED => {
                let payload = event.payload.as_object();
                let field = |name| payload.and_then(|payload| text(payload, name));
                let id = field(ID_FIELD).ok_or_else(|| refuse("id is not text".into()))?;
                let reason = field("reason").ok_or_else(|| refuse("reason is not text".into()))?;
                let Some(document) = held(id) else {
                    return Err(refuse(format!("order {id} is not held")));
                };
                if text(document, STATE_FIELD) == Some(FAILED) {
                    return Err(refuse(format!("order {id} has already failed")));
                }
                let mut document = document.clone();
                document.insert(STATE_FIELD.into(), json!(FAILED));
                document.insert(
                    FAILURE_FIELD.into(),
                    json!({"reason": reason, "seq": event.seq, "recorded_at": event.recorded_at}),
                );
                Ok(vec![Change::Put {
                    key: work_order_key(id),
                    body: Value::Object(document),
                }])
            }
            QUESTIONS_OPENED => {
                // A set naming no order, or one not held, is its own record.
                let Some((id, document)) =
                    story_set_order(event).and_then(|id| held(id).map(|document| (id, document)))
                else {
                    return Ok(Vec::new());
                };
                let Some(set) = event.payload.get("set").and_then(Value::as_str) else {
                    return Err(refuse("set is not text".into()));
                };
                answer(id, document, set).map_err(refuse)
            }
            PLAN_CHECKED => {
                // A result naming an order not held is its own record.
                let Some((id, document)) =
                    checked_order(event).and_then(|id| held(id).map(|document| (id, document)))
                else {
                    return Ok(Vec::new());
                };
                let Some(digest) = event.payload.get("plan_digest").and_then(Value::as_str) else {
                    return Err(refuse("plan_digest is not text".into()));
                };
                answer(id, document, digest).map_err(refuse)
            }
            _ => {
                // A submission naming no order, or one not held, is its own record.
                let Some((id, document)) =
                    caller_order(event).and_then(|id| held(id).map(|document| (id, document)))
                else {
                    return Ok(Vec::new());
                };
                let Some(digest) = event.payload.get("digest").and_then(Value::as_str) else {
                    return Err(refuse("digest is not text".into()));
                };
                answer(id, document, digest).map_err(refuse)
            }
        }
    }
}

/// Order `id`'s `document` with `returned` added to its `drafts` once, and
/// `answered` when it was pending; a failed order stays failed.
fn answer(id: &str, document: &Map<String, Value>, returned: &str) -> Result<Vec<Change>, String> {
    let mut document = document.clone();
    let mut drafts = match document.get(DRAFTS_FIELD) {
        Some(Value::Array(drafts)) => drafts.clone(),
        _ => return Err("the order's drafts are not a list".into()),
    };
    if !drafts.iter().any(|draft| draft == returned) {
        drafts.push(json!(returned));
    }
    document.insert(DRAFTS_FIELD.into(), Value::Array(drafts));
    if text(&document, STATE_FIELD) == Some(PENDING) {
        document.insert(STATE_FIELD.into(), json!(ANSWERED));
    }
    Ok(vec![Change::Put {
        key: work_order_key(id),
        body: Value::Object(document),
    }])
}

/// The order a story's question set names; a plan draft's set names none.
fn story_set_order(event: &Event) -> Option<&str> {
    let payload = &event.payload;
    if payload.get("subject").and_then(Value::as_str) != Some("story") {
        return None;
    }
    payload.get("work_order").and_then(Value::as_str)
}

/// The checker order a `plan.checked` answers.
fn checked_order(event: &Event) -> Option<&str> {
    event.payload.get("work_order").and_then(Value::as_str)
}

/// The work order a submission's caller carries, in either form.
fn caller_order(event: &Event) -> Option<&str> {
    match event.caller.as_ref()? {
        Caller::Server(caller) => caller.work_order(),
        Caller::Hook(caller) => caller.work_order(),
    }
}

/// The text member `name` of `object`, if it is text.
fn text<'o>(object: &'o Map<String, Value>, name: &str) -> Option<&'o str> {
    object.get(name).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from the view's documented shape
    //! and the plan's test data, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{
        Actor, Hash, HookCaller, PayloadRef, ProjectId, RequestId, RetentionClass, ServerCaller,
    };

    use super::*;
    use crate::plan::CheckResult;
    use crate::policy::RecheckScope;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const I2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const I3: &str = "8a4f5e6d-7182-4d9e-9fa0-2b3c4d5e6f70";
    const T0: &str = "2026-10-18T09:00:00Z";
    const T1: &str = "2026-10-18T09:00:01Z";
    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";

    type View = BTreeMap<DocKey, Value>;

    fn event(seq: u64, stream: &str, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: stream.into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Baley,
            caller: None,
            recorded_at: format!("2026-10-18T09:00:{:02}Z", seq - 5),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    /// O1's payload: the start order.
    fn o1() -> Value {
        json!({"id": I0, "request": I0, "parent": null, "role": "planner",
            "subject": "start", "attempt": 1, "route": {"rung": "high"},
            "agent": "bal-planner-high",
            "instructions": [{"identity": "bal-start-planner", "version": "1",
                "hash": "c".repeat(64)}],
            "inputs": [{"name": "brief", "body": PayloadRef { hash: Hash([0xab; 32]),
                bytes: 300, class: RetentionClass::Record }.to_value()}]})
    }

    /// O1's payload with other members, such as O2's.
    fn order(id: &str, parent: &str, attempt: u32, agent: &str) -> Value {
        let mut payload = o1();
        payload["id"] = json!(id);
        payload["parent"] = json!(parent);
        payload["attempt"] = json!(attempt);
        payload["agent"] = json!(agent);
        payload
    }

    fn o2() -> Value {
        order(I1, I0, 2, "bal-planner-xhigh")
    }

    fn issued(seq: u64, payload: Value) -> Event {
        event(seq, "roadmap", "work_order.issued", payload)
    }

    fn failed(seq: u64, id: &str, reason: &str) -> Event {
        event(
            seq,
            "roadmap",
            "work_order.failed",
            json!({"id": id, "reason": reason}),
        )
    }

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn submitted(seq: u64, byte: u8, caller: Option<Caller>) -> Event {
        let payload = json!({"digest": digest(byte), "kind": "start", "target": "project",
            "body": PayloadRef { hash: Hash([byte; 32]), bytes: 40,
                class: RetentionClass::Record }.to_value()});
        Event {
            type_version: 2,
            caller,
            ..event(seq, "project", "scope.submitted", payload)
        }
    }

    fn server(order: Option<&str>) -> Caller {
        let caller = ServerCaller::new(
            "/work/proj",
            "/work/proj",
            "claude-code",
            SESSION,
            &json!(1),
        )
        .unwrap();
        Caller::Server(match order {
            Some(order) => caller.with_work_order(order).unwrap(),
            None => caller,
        })
    }

    fn hook(order: &str) -> Caller {
        let caller = HookCaller::new("claude-code", "/work/proj", "toolu_01ABC").unwrap();
        Caller::Hook(caller.with_work_order(order).unwrap())
    }

    /// Applies `event` to `view` as the store would, reading the documents
    /// at its keys first. A refusal leaves the view as it was.
    fn fold(view: &mut View, event: &Event) -> Result<(), ProjectorError> {
        let projector = WorkOrderProjector::new();
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

    fn doc<'v>(view: &'v View, id: &str) -> &'v Value {
        &view[&DocKey(vec![KeyValue::Text(id.into())])]
    }

    /// O1's document as issued at seq 5 and T0.
    fn pending_o1() -> Value {
        let mut document = o1();
        let fields = document.as_object_mut().unwrap();
        fields.insert("state".into(), json!("pending"));
        fields.insert("seq".into(), json!(5));
        fields.insert("recorded_at".into(), json!(T0));
        fields.insert("failure".into(), Value::Null);
        fields.insert("drafts".into(), json!([]));
        fields.insert("child".into(), Value::Null);
        document
    }

    fn with_o1() -> View {
        let mut view = View::new();
        fold(&mut view, &issued(5, o1())).unwrap();
        view
    }

    #[test]
    fn a_work_order_view_declared_without_its_live_index_is_caught() {
        let spec = work_order_spec();
        assert_eq!(spec.name, "work_order");
        assert_eq!(spec.version, 3);
        assert_eq!(spec.page_bound, 100);
        assert_eq!(
            spec.key,
            [FieldSpec {
                name: "id".into(),
                kind: FieldKind::Text,
            }]
        );
        assert_eq!(spec.indexes.len(), 1);
        assert_eq!(spec.indexes[0].name, "live");
        let text = |name: &str| IndexField {
            name: name.into(),
            kind: FieldKind::Text,
            order: Order::Ascending,
        };
        assert_eq!(
            spec.indexes[0].fields,
            [
                text("role"),
                text("subject"),
                text("state"),
                IndexField {
                    name: "seq".into(),
                    kind: FieldKind::Integer,
                    order: Order::Descending,
                },
            ]
        );
        assert_eq!(
            WorkOrderProjector::new().handles(),
            [
                "work_order.issued",
                "work_order.failed",
                "scope.submitted",
                "questions.opened",
                "plan.checked"
            ]
        );
    }

    #[test]
    fn an_issued_order_projected_without_its_members_or_not_pending_is_caught() {
        let mut view = with_o1();
        assert_eq!(view.len(), 1);
        assert_eq!(doc(&view, I0), &pending_o1());

        assert!(fold(&mut view, &issued(6, o1())).is_err());
        assert_eq!(view.len(), 1);

        fold(&mut view, &issued(7, o2())).unwrap();
        let retry = doc(&view, I1);
        assert_eq!(retry["request"], I0);
        assert_eq!(retry["parent"], I0);
        assert_eq!(retry["child"], Value::Null);
        assert_eq!(retry["state"], "pending");
        let mut parent = pending_o1();
        parent["child"] = json!(I1);
        assert_eq!(doc(&view, I0), &parent);
    }

    #[test]
    fn a_retry_child_projected_onto_no_parent_or_beside_a_second_child_is_caught() {
        let mut view = with_o1();
        fold(&mut view, &issued(6, o2())).unwrap();
        assert_eq!(doc(&view, I0)["child"], I1);
        let second = issued(7, order(I2, I0, 2, "bal-planner-high"));
        assert!(fold(&mut view, &second).is_err());
        let orphan = issued(8, order(I2, I3, 2, "bal-planner-high"));
        assert!(fold(&mut view, &orphan).is_err());
        assert_eq!(doc(&view, I0)["child"], I1);
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn a_failure_projected_onto_no_order_or_twice_is_caught() {
        let mut view = with_o1();
        fold(&mut view, &failed(6, I0, "no return")).unwrap();
        let mut failed_o1 = pending_o1();
        failed_o1["state"] = json!("failed");
        failed_o1["failure"] = json!({"reason": "no return", "seq": 6, "recorded_at": T1});
        assert_eq!(doc(&view, I0), &failed_o1);

        assert!(fold(&mut view, &failed(7, I1, "no return")).is_err());
        assert!(fold(&mut view, &failed(7, I0, "again")).is_err());
        assert_eq!(view.len(), 1);
        assert_eq!(doc(&view, I0), &failed_o1);

        fold(&mut view, &issued(8, o2())).unwrap();
        let mut retried = failed_o1;
        retried["child"] = json!(I1);
        assert_eq!(doc(&view, I0), &retried);
        fold(&mut view, &failed(9, I1, "no return")).unwrap();
        assert_eq!(doc(&view, I1)["state"], "failed");
        assert_eq!(doc(&view, I0), &retried);
    }

    #[test]
    fn a_submission_naming_its_order_left_unanswered_or_one_naming_none_changing_an_order_is_caught()
     {
        let drafts = |view: &View| doc(view, I0)["drafts"].clone();
        let mut view = with_o1();
        fold(&mut view, &submitted(8, 0x11, Some(server(Some(I0))))).unwrap();
        assert_eq!(doc(&view, I0)["state"], "answered");
        assert_eq!(drafts(&view), json!([digest(0x11)]));

        fold(&mut view, &submitted(9, 0x22, Some(server(Some(I0))))).unwrap();
        assert_eq!(drafts(&view), json!([digest(0x11), digest(0x22)]));
        fold(&mut view, &submitted(10, 0x11, Some(server(Some(I0))))).unwrap();
        assert_eq!(drafts(&view), json!([digest(0x11), digest(0x22)]));

        let before = view.clone();
        fold(&mut view, &submitted(11, 0x33, Some(server(None)))).unwrap();
        assert_eq!(view, before);
        fold(&mut view, &submitted(12, 0x33, Some(server(Some(I1))))).unwrap();
        assert_eq!(view, before);

        fold(&mut view, &submitted(13, 0x44, Some(hook(I0)))).unwrap();
        assert_eq!(
            drafts(&view),
            json!([digest(0x11), digest(0x22), digest(0x44)])
        );

        fold(&mut view, &failed(14, I0, "no return")).unwrap();
        fold(&mut view, &submitted(15, 0x33, Some(server(Some(I0))))).unwrap();
        assert_eq!(doc(&view, I0)["state"], "failed");
        assert_eq!(
            drafts(&view),
            json!([digest(0x11), digest(0x22), digest(0x44), digest(0x33)])
        );
    }

    const SECOND_SET: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";

    /// O1 as an analyzer's questions order for story 3.
    fn analyzer_o1() -> Value {
        let mut payload = o1();
        payload["id"] = json!(I1);
        payload["role"] = json!("analyzer");
        payload["subject"] = json!("story/3");
        payload
    }

    fn story_set(seq: u64, set: &str, order: &str) -> Event {
        event(
            seq,
            "roadmap",
            "questions.opened",
            json!({"set": set, "subject": "story", "story": 3, "work_order": order,
                "questions": [], "at": T0}),
        )
    }

    #[test]
    fn a_questions_order_left_pending_after_its_set_opened_is_caught() {
        let mut view = View::new();
        fold(&mut view, &issued(5, analyzer_o1())).unwrap();
        fold(&mut view, &story_set(6, I0, I1)).unwrap();
        assert_eq!(doc(&view, I1)["state"], "answered");
        assert_eq!(doc(&view, I1)["drafts"], json!([I0]));
        fold(&mut view, &story_set(7, SECOND_SET, I1)).unwrap();
        assert_eq!(doc(&view, I1)["state"], "answered");
        assert_eq!(doc(&view, I1)["drafts"], json!([I0, SECOND_SET]));
    }

    #[test]
    fn a_set_naming_no_held_order_or_a_plan_set_changing_an_order_is_caught() {
        let mut view = View::new();
        fold(&mut view, &issued(5, analyzer_o1())).unwrap();
        let before = view.clone();
        fold(&mut view, &story_set(6, I0, I3)).unwrap();
        assert_eq!(view, before);
        // A plan set never names an order; one carrying O1's id anyway must
        // still change nothing.
        let plan_set = event(
            7,
            "phase/2",
            "questions.opened",
            json!({"set": SECOND_SET, "subject": "plan", "phase": 2,
                "plan_digest": digest(0x5c), "work_order": I1, "questions": [], "at": T0}),
        );
        fold(&mut view, &plan_set).unwrap();
        assert_eq!(view, before);
    }

    #[test]
    fn a_failed_questions_order_revived_by_a_late_set_is_caught() {
        let mut view = View::new();
        fold(&mut view, &issued(5, analyzer_o1())).unwrap();
        fold(&mut view, &failed(6, I1, "no return")).unwrap();
        fold(&mut view, &story_set(7, I0, I1)).unwrap();
        assert_eq!(doc(&view, I1)["state"], "failed");
        assert_eq!(doc(&view, I1)["drafts"], json!([I0]));
    }

    /// O1 as the checker's order `id` for phase 1.
    fn checker(id: &str) -> Value {
        let mut payload = o1();
        payload["id"] = json!(id);
        payload["request"] = json!(id);
        payload["role"] = json!("checker");
        payload["subject"] = json!("phase/1");
        payload
    }

    /// A round-1 `plan.checked` of phase 1 answering `order` for the draft
    /// whose digest is `byte` repeated.
    fn checked(seq: u64, order: &str, byte: u8) -> Event {
        let result = CheckResult {
            phase: 1,
            plan_digest: Hash([byte; 32]),
            findings: &[],
            work_order: order,
            scope: RecheckScope::Full,
            previous_check: None,
            truth_versions: &[],
            material: &[],
            scope_policy_version: 3,
            revision: None,
        };
        event(seq, "phase/1", "plan.checked", result.payload())
    }

    #[test]
    fn a_checker_order_left_pending_after_its_result_or_a_failed_one_revived_is_caught() {
        let mut view = View::new();
        fold(&mut view, &issued(5, checker(I1))).unwrap();
        fold(&mut view, &checked(6, I1, 0x5c)).unwrap();
        assert_eq!(doc(&view, I1)["state"], "answered");
        assert_eq!(doc(&view, I1)["drafts"], json!([digest(0x5c)]));

        fold(&mut view, &issued(7, checker(I2))).unwrap();
        fold(&mut view, &failed(8, I2, "no return")).unwrap();
        fold(&mut view, &checked(9, I2, 0x6d)).unwrap();
        assert_eq!(doc(&view, I2)["state"], "failed");
        assert_eq!(doc(&view, I2)["drafts"], json!([digest(0x6d)]));

        let before = view.clone();
        fold(&mut view, &checked(10, I3, 0x5c)).unwrap();
        assert_eq!(view, before);
    }
}
