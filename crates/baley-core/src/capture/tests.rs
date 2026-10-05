//! Captures' decisions on supplied values. Expected payloads are written out
//! from the event's documented shape, never from running this code.

use baley_store::{EventSchema, Hash, PayloadRef, RetentionClass};
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
