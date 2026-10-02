//! Checkout admission's decisions on supplied values. Expected payloads are
//! written out from the event's documented shape, never from running this
//! code.

use baley_store::{
    Actor, Change, DocKey, Event, EventSchema, FieldKind, FieldSpec, Hash, IndexField, IndexSpec,
    KeyValue, Order, ProjectId, Projector, RequestId, ViewSpec,
};
use serde_json::{Value, json};

use super::*;
use crate::registry::Registry;

fn checkout(root_commit: Option<&str>, remote_url: Option<&str>) -> Checkout {
    Checkout {
        path: "/r".into(),
        root_commit: root_commit.map(str::to_owned),
        remote_url: remote_url.map(str::to_owned),
    }
}

#[test]
fn a_seen_payload_dropping_or_renaming_a_member_is_caught() {
    assert_eq!(
        seen_payload(&checkout(
            Some("abc123"),
            Some("https://github.com/o/r.git")
        )),
        json!({
            "path": "/r",
            "root_commit": "abc123",
            "remote_url": "https://github.com/o/r.git",
        })
    );
}

#[test]
fn a_seen_payload_omitting_or_blanking_an_absent_field_is_caught() {
    let payload = seen_payload(&checkout(None, None));
    assert_eq!(
        payload,
        json!({"path": "/r", "root_commit": null, "remote_url": null})
    );
    assert_eq!(payload.as_object().map(|members| members.len()), Some(3));
}

#[test]
fn checkout_seen_registered_at_any_version_but_1_is_caught() {
    let mut registry = Registry::new();
    register_checkout_events(&mut registry).unwrap();
    assert!(registry.reads("checkout.seen", 1));
    assert!(!registry.reads("checkout.seen", 2));
}

const PROJECT_ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

fn event(seq: u64, payload: Value) -> Event {
    Event {
        project_id: ProjectId(PROJECT_ID.into()),
        seq,
        stream: "project".into(),
        stream_version: seq,
        type_name: "checkout.seen".into(),
        type_version: 1,
        actor: Actor::Baley,
        recorded_at: "2026-10-01T09:00:00Z".into(),
        request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
        git: None,
        policy_version: 0,
        payload,
        prev_hash: None,
        hash: Hash([0; 32]),
    }
}

fn seen(path: &str, root_commit: Option<&str>, remote_url: Option<&str>) -> Value {
    seen_payload(&Checkout {
        path: path.into(),
        root_commit: root_commit.map(str::to_owned),
        remote_url: remote_url.map(str::to_owned),
    })
}

fn put(changes: Vec<Change>) -> (DocKey, Value) {
    match <[Change; 1]>::try_from(changes) {
        Ok([Change::Put { key, body }]) => (key, body),
        other => panic!("expected one put, got {other:?}"),
    }
}

fn key(path: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(path.into())])
}

#[test]
fn a_checkout_view_keyed_or_indexed_by_remote_url_is_caught() {
    assert_eq!(
        checkout_spec(),
        ViewSpec {
            name: "checkout".into(),
            version: 1,
            key: vec![FieldSpec {
                name: "path".into(),
                kind: FieldKind::Text,
            }],
            indexes: vec![IndexSpec {
                name: "by_path".into(),
                fields: vec![IndexField {
                    name: "path".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                }],
            }],
            page_bound: 100,
        }
    );
}

#[test]
fn a_later_checkout_seen_merged_into_the_stored_row_instead_of_replacing_it_is_caught() {
    let projector = CheckoutProjector::new();
    let first = seen("/r", Some("c1"), Some("https://h/o/r.git"));
    let second = seen("/r", None, None);
    let (first_key, first_body) = put(projector.apply(&event(2, first.clone()), &[]).unwrap());
    let (second_key, second_body) = put(projector.apply(&event(5, second.clone()), &[]).unwrap());
    assert_eq!(first_key, key("/r"));
    assert_eq!(second_key, key("/r"));
    assert_eq!(first_body, first);
    assert_eq!(second_body, second);
}

#[test]
fn two_checkouts_of_one_project_sharing_a_row_is_caught() {
    let projector = CheckoutProjector::new();
    let (one, _) = put(projector
        .apply(&event(2, seen("/a", None, None)), &[])
        .unwrap());
    let (other, _) = put(projector
        .apply(&event(3, seen("/b", None, None)), &[])
        .unwrap());
    assert_eq!(one, key("/a"));
    assert_eq!(other, key("/b"));
}

#[test]
fn a_malformed_checkout_seen_accepted_into_the_view_is_caught() {
    let projector = CheckoutProjector::new();
    let cases = [
        json!("a string"),
        json!({"root_commit": null, "remote_url": null}),
        json!({"path": "", "root_commit": null, "remote_url": null}),
        json!({"path": 7, "root_commit": null, "remote_url": null}),
        json!({"path": "/r", "remote_url": null}),
        json!({"path": "/r", "root_commit": "", "remote_url": null}),
        json!({"path": "/r", "root_commit": 7, "remote_url": null}),
        json!({"path": "/r", "root_commit": null}),
        json!({"path": "/r", "root_commit": null, "remote_url": ""}),
        json!({"path": "/r", "root_commit": null, "remote_url": 7}),
    ];
    for payload in cases {
        let refused = projector
            .apply(&event(9, payload.clone()), &[])
            .expect_err(&format!("{payload} must be refused"));
        assert!(
            refused.0.starts_with("checkout.seen at seq 9: "),
            "{}",
            refused.0
        );
    }
}

#[test]
fn a_checkout_projector_handling_or_removing_anything_beyond_checkout_seen_is_caught() {
    let projector = CheckoutProjector::new();
    assert_eq!(projector.handles(), ["checkout.seen"]);
    assert!(projector.keys(&event(2, seen("/r", None, None))).is_empty());
}
