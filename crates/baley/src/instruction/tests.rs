use serde_json::json;

use super::*;

/// The twenty front doors with their owning builds, written out from the
/// design rather than read from the table under test.
const UNAVAILABLE: &[(&str, u32)] = &[
    ("bal-context", 4),
    ("bal-plan", 4),
    ("bal-review", 4),
    ("bal-plan-review", 4),
    ("bal-decision-review", 4),
    ("bal-minimalism-review", 4),
    ("bal-execute", 5),
    ("bal-verify", 5),
    ("bal-audit", 5),
    ("bal-coverage", 5),
    ("bal-land", 6),
    ("bal-milestone", 6),
    ("bal-undo", 6),
    ("bal-progress", 7),
    ("bal-suggest", 7),
    ("bal-task", 8),
    ("bal-debug", 8),
    ("bal-spike", 8),
    ("bal-why", 8),
    ("bal-capture", 3),
];

#[test]
fn an_unavailable_front_door_is_never_presented_as_usable_and_names_its_owning_build() {
    assert_eq!(UNAVAILABLE.len(), 20);
    for (identity, build) in UNAVAILABLE {
        assert_eq!(
            lookup(identity),
            Lookup::Unavailable {
                identity,
                build: *build
            },
            "{identity}"
        );
    }
    assert_eq!(ENTRIES.len(), UNAVAILABLE.len());
}

#[test]
fn the_unavailable_refusal_names_the_build_in_words_and_as_an_integer_in_slot_identity() {
    let refusal = unavailable("bal-plan", 4);
    assert_eq!(refusal["status"], "refused");
    assert_eq!(refusal["code"], "instruction-unavailable");
    assert_eq!(refusal["slot"], "identity");
    assert_eq!(refusal["details"], json!({"build": 4}));
    let reason = refusal["reason"].as_str().unwrap();
    assert!(reason.contains("Build 4") && reason.contains("bal-plan"));
}

#[test]
fn a_path_shaped_identity_is_unknown_and_no_file_content_reaches_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("bal-help");
    let skill = dir.path().join("skills/bal-help/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&plain, "MARKER-PLAIN-FILE\n").unwrap();
    std::fs::write(&skill, "MARKER-SKILL-FILE\n").unwrap();
    let spellings = [
        plain.to_str().unwrap(),
        skill.to_str().unwrap(),
        "skills/bal-help/SKILL.md",
    ];
    for spelling in spellings {
        assert_eq!(lookup(spelling), Lookup::Unknown, "{spelling}");
    }
    let refusal = unknown().to_string();
    assert!(!refusal.contains("MARKER"));
    assert!(refusal.contains("unknown-instruction") && refusal.contains("identity"));
}

#[test]
fn a_loose_spelling_of_a_registered_identity_is_unknown() {
    assert_eq!(lookup("BAL-PLAN"), Lookup::Unknown);
    assert_eq!(lookup("/bal-plan"), Lookup::Unknown);
    assert_eq!(lookup("bal-plan "), Lookup::Unknown);
    assert_eq!(lookup(""), Lookup::Unknown);
}
