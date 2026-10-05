//! Captures' decisions on supplied values. Expected payloads are written out
//! from the event's documented shape, never from running this code.

use baley_store::{
    Actor, Change, DocKey, Event, EventSchema, FieldKind, FieldSpec, Hash, IndexField, IndexSpec,
    KeyValue, Order, PayloadRef, ProjectId, Projector, RequestId, RetentionClass, ViewSpec,
};
use serde_json::{Value, json};

use super::*;
use crate::registry::Registry;

const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
const OTHER_REQUEST: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

fn body() -> PayloadRef {
    PayloadRef {
        hash: Hash([0xab; 32]),
        bytes: 4097,
        class: RetentionClass::Record,
    }
}

#[test]
fn two_captures_of_the_same_text_under_different_requests_sharing_an_id_is_caught() {
    let one = capture_id(REQUEST, CaptureKind::Note, "keep this", None);
    let other = capture_id(OTHER_REQUEST, CaptureKind::Note, "keep this", None);
    assert_ne!(one, other);
}

#[test]
fn a_capture_id_that_ignores_the_kind_the_text_or_the_phase_is_caught() {
    let base = capture_id(REQUEST, CaptureKind::Note, "keep this", None);
    assert_eq!(
        base,
        capture_id(REQUEST, CaptureKind::Note, "keep this", None)
    );
    assert_eq!(base.len(), 64);
    assert!(
        base.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
    for changed in [
        capture_id(REQUEST, CaptureKind::Story, "keep this", None),
        capture_id(REQUEST, CaptureKind::Note, "keep that", None),
        capture_id(REQUEST, CaptureKind::Note, "keep this", Some(3)),
    ] {
        assert_ne!(changed, base);
    }
}

#[test]
fn a_threshold_that_counts_characters_instead_of_utf8_bytes_is_caught() {
    // 1,024 four-byte characters: exactly 4,096 bytes.
    let at_limit = "🦀".repeat(1024);
    assert_eq!(text_form(&at_limit), TextForm::Inline);
    assert_eq!(text_form(&"x".repeat(4097)), TextForm::Payload);
    // 1,025 characters but 4,100 bytes: fewer than 4,096 characters would
    // still be over the limit in bytes.
    let multi_byte = format!("{}é", "🦀".repeat(1024));
    assert!(multi_byte.chars().count() < 4096);
    assert_eq!(text_form(&multi_byte), TextForm::Payload);
    let short_chars = "é".repeat(2049);
    assert!(short_chars.chars().count() < 4096);
    assert_eq!(text_form(&short_chars), TextForm::Payload);
}

#[test]
fn an_inline_payload_dropping_a_member_or_carrying_a_caller_or_time_is_caught() {
    assert_eq!(
        inline_payload("c1", CaptureKind::Story, None, "é!"),
        json!({"id": "c1", "kind": "story", "phase": null, "bytes": 3, "text": "é!"})
    );
}

#[test]
fn a_stored_payload_that_keeps_the_text_or_drops_the_reference_is_caught() {
    let payload = stored_payload("c1", CaptureKind::Note, Some(2), &body());
    assert_eq!(
        payload,
        json!({"id": "c1", "kind": "note", "phase": 2, "bytes": 4097,
            "body": {"payload": "ab".repeat(32), "bytes": 4097, "class": "record"}})
    );
    let members = |value: &Value| {
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(!members(&payload).contains(&"text".to_string()));
    assert!(!members(&payload).contains(&"by".to_string()));
    let inline = inline_payload("c1", CaptureKind::Note, None, "x");
    assert!(!members(&inline).contains(&"by".to_string()));
}

#[test]
fn capture_recorded_registered_at_any_version_but_1_is_caught() {
    let mut registry = Registry::new();
    register_capture_events(&mut registry).unwrap();
    assert!(registry.reads("capture.recorded", 1));
    assert!(!registry.reads("capture.recorded", 2));
    assert!(!registry.reads("capture.recorded", 0));
}

#[test]
fn blank_judged_by_emptiness_alone_letting_whitespace_through_is_caught() {
    for text in ["", "   ", "\t\n \r\n"] {
        assert!(is_blank(text), "{text:?} is blank");
    }
}

#[test]
fn text_with_a_nul_or_other_control_character_refused_as_blank_is_caught() {
    for text in ["a\0b", "bell\u{7}", "\u{1b}[31mred"] {
        assert!(!is_blank(text), "{text:?} is not blank");
    }
}

#[test]
fn an_obsolete_kind_accepted_or_a_near_spelling_taken_for_a_kind_is_caught() {
    assert_eq!(judge_kind("note"), Ok(CaptureKind::Note));
    assert_eq!(judge_kind("story"), Ok(CaptureKind::Story));
    assert_eq!(judge_kind("todo"), Err(UnknownKind::Obsolete("todo")));
    assert_eq!(judge_kind("seed"), Err(UnknownKind::Obsolete("seed")));
    assert_eq!(judge_kind("Note"), Err(UnknownKind::Other));
    assert_eq!(judge_kind(""), Err(UnknownKind::Other));
}

#[test]
fn a_named_phase_accepted_when_observed_absent_is_caught() {
    assert_eq!(judge_phase(None, false), Ok(()));
    assert_eq!(judge_phase(Some(3), false), Err("no-such-phase"));
    assert_eq!(judge_phase(Some(3), true), Ok(()));
}

const PROJECT_ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
const AT: &str = "2026-10-05T09:00:00Z";

fn event(type_name: &str, seq: u64, payload: Value) -> Event {
    Event {
        project_id: ProjectId(PROJECT_ID.into()),
        seq,
        stream: "capture".into(),
        stream_version: 1,
        type_name: type_name.into(),
        type_version: 1,
        actor: Actor::Baley,
        caller: None,
        recorded_at: AT.into(),
        request_id: RequestId(REQUEST.into()),
        git: None,
        policy_version: 0,
        payload,
        prev_hash: None,
        hash: Hash([0; 32]),
    }
}

fn recorded(seq: u64, payload: Value) -> Event {
    event("capture.recorded", seq, payload)
}

fn put(changes: Vec<Change>) -> (DocKey, Value) {
    match <[Change; 1]>::try_from(changes) {
        Ok([Change::Put { key, body }]) => (key, body),
        other => panic!("expected one put, got {other:?}"),
    }
}

#[test]
fn an_inline_capture_keyed_by_its_id_or_losing_its_text_is_caught() {
    let payload = inline_payload("c1", CaptureKind::Note, None, "keep this");
    let (key, body) = put(CaptureProjector::new()
        .apply(&recorded(7, payload), &[])
        .unwrap());
    assert_eq!(key, DocKey(vec![KeyValue::Integer(7)]));
    assert_eq!(
        body,
        json!({"seq": 7, "id": "c1", "kind": "note", "phase": null, "bytes": 9,
            "recorded_at": AT, "text": "keep this"})
    );
}

#[test]
fn a_payload_capture_copying_text_or_missing_its_hash_and_state_is_caught() {
    let payload = stored_payload("c2", CaptureKind::Story, None, &body());
    let (key, document) = put(CaptureProjector::new()
        .apply(&recorded(9, payload), &[])
        .unwrap());
    assert_eq!(key, DocKey(vec![KeyValue::Integer(9)]));
    assert_eq!(
        document,
        json!({"seq": 9, "id": "c2", "kind": "story", "phase": null, "bytes": 4097,
            "recorded_at": AT, "hash": "ab".repeat(32), "state": "present"})
    );
}

#[test]
fn a_capture_view_keyed_by_text_or_declaring_a_phase_index_is_caught() {
    assert_eq!(
        capture_spec(),
        ViewSpec {
            name: "capture".into(),
            version: 1,
            key: vec![FieldSpec {
                name: "seq".into(),
                kind: FieldKind::Integer,
            }],
            indexes: vec![IndexSpec {
                name: "by_id".into(),
                fields: vec![IndexField {
                    name: "id".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                }],
            }],
            page_bound: 100,
        }
    );
}

#[test]
fn a_malformed_capture_recorded_accepted_into_the_view_is_caught() {
    let reference = body().to_value();
    for payload in [
        json!("not an object"),
        json!({"kind": "note", "text": "x"}),
        json!({"id": "c1", "text": "x"}),
        json!({"id": "c1", "kind": "note"}),
        json!({"id": "c1", "kind": "note", "text": "x", "body": reference}),
        json!({"id": "c1", "kind": "note", "body": {"payload": "zz"}}),
    ] {
        let error = CaptureProjector::new()
            .apply(&recorded(4, payload.clone()), &[])
            .expect_err(&format!("{payload} must be refused"));
        assert!(error.0.contains("capture.recorded at seq 4"), "{}", error.0);
    }
}

#[test]
fn a_capture_projector_missing_purges_or_reading_documents_for_a_new_capture_is_caught() {
    let projector = CaptureProjector::new();
    assert_eq!(projector.handles(), ["capture.recorded", "payload.purged"]);
    let payload = inline_payload("c1", CaptureKind::Note, None, "x");
    assert!(projector.keys(&recorded(3, payload)).is_empty());
}

const REASON: &str = "the owner asked";

fn purge(seq: u64, released: Value) -> Event {
    event(
        "payload.purged",
        seq,
        json!({"requested": ["ab".repeat(32)], "released": released,
            "removed": ["ab".repeat(32)], "shared": [], "reason": REASON}),
    )
}

/// The payload capture recorded at seq 9, as the view holds it.
fn present() -> (DocKey, Value) {
    let payload = stored_payload("c2", CaptureKind::Story, None, &body());
    put(CaptureProjector::new()
        .apply(&recorded(9, payload), &[])
        .unwrap())
}

#[test]
fn a_purge_of_a_captures_own_body_deleting_it_or_leaving_it_present_is_caught() {
    let projector = CaptureProjector::new();
    let event = purge(12, json!([[9, "ab".repeat(32)]]));
    assert_eq!(projector.keys(&event), [DocKey(vec![KeyValue::Integer(9)])]);
    let (key, document) = put(projector.apply(&event, &[present()]).unwrap());
    assert_eq!(key, DocKey(vec![KeyValue::Integer(9)]));
    assert_eq!(
        document,
        json!({"seq": 9, "id": "c2", "kind": "story", "phase": null, "bytes": 4097,
            "recorded_at": AT, "hash": "ab".repeat(32), "state": "purged", "reason": REASON})
    );
}

#[test]
fn a_purge_of_another_sequence_marking_this_capture_is_caught() {
    let event = purge(12, json!([[10, "ab".repeat(32)]]));
    let changes = CaptureProjector::new().apply(&event, &[present()]).unwrap();
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn a_purge_of_another_hash_marking_this_capture_or_an_inline_one_is_caught() {
    let event = purge(12, json!([[7, "ab".repeat(32)], [9, "cd".repeat(32)]]));
    let inline = put(CaptureProjector::new()
        .apply(
            &recorded(7, inline_payload("c1", CaptureKind::Note, None, "x")),
            &[],
        )
        .unwrap());
    let changes = CaptureProjector::new()
        .apply(&event, &[inline, present()])
        .unwrap();
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn a_malformed_purge_payload_accepted_into_the_view_is_caught() {
    for payload in [
        json!("not an object"),
        json!({"released": [[9, "ab".repeat(32)]], "reason": REASON}),
        json!({"requested": [], "released": [[9]], "removed": [], "shared": [], "reason": REASON}),
    ] {
        let event = event("payload.purged", 12, payload.clone());
        let error = CaptureProjector::new()
            .apply(&event, &[present()])
            .expect_err(&format!("{payload} must be refused"));
        assert!(error.0.contains("payload.purged at seq 12"), "{}", error.0);
    }
}

#[test]
fn a_capture_and_its_purge_folding_differently_on_a_rebuild_is_caught() {
    let fold = || {
        let projector = CaptureProjector::new();
        let payload = stored_payload("c2", CaptureKind::Story, None, &body());
        let recorded = put(projector.apply(&recorded(9, payload), &[]).unwrap());
        put(projector
            .apply(&purge(12, json!([[9, "ab".repeat(32)]])), &[recorded])
            .unwrap())
    };
    assert_eq!(fold(), fold());
}
