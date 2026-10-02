//! Checkout admission's decisions on supplied values. Expected payloads are
//! written out from the event's documented shape, never from running this
//! code.

use baley_store::EventSchema;
use serde_json::json;

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
