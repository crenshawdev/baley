use serde_json::json;

use super::*;

/// The twenty front doors not served yet, with their owning builds, written out from the
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
    let unserved = ENTRIES.iter().filter(|entry| entry.text.is_none()).count();
    assert_eq!(unserved, UNAVAILABLE.len());
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

/// Every (identity, version, hash) this registry has served, written out by
/// hand. Rows are only ever added: a changed text gets a new version and a new
/// row, and the old row stays so the old pair is never reused.
const SERVED_EVER: &[(&str, &str, &str)] = &[
    (
        "bal-help",
        "1",
        "6a546a62197f3070cdfa4725732e5f8e3c00d21addf3b77696459c0dcb469cad",
    ),
    (
        "bal-read-contract",
        "1",
        "4feb8474b44d0f4b85290797064cc1e57a369f5b3afdc0ca866afe8ea7d6c217",
    ),
];

fn served() -> impl Iterator<Item = (&'static str, &'static Text)> {
    ENTRIES
        .iter()
        .filter_map(|entry| entry.text.as_ref().map(|text| (entry.identity, text)))
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_served_text_changed_without_its_pinned_hash_or_a_new_version_row_fails() {
    assert!(served().count() >= 1);
    for (identity, text) in served() {
        assert_eq!(
            sha256_hex(text.body),
            text.hash,
            "{identity}: the text no longer matches its pinned hash"
        );
        assert!(
            SERVED_EVER.contains(&(identity, text.version, text.hash)),
            "{identity} version {} hash {} is not in the served-ever list",
            text.version,
            text.hash
        );
    }
}

#[test]
fn one_identity_and_version_never_carry_two_hashes_in_the_served_ever_list() {
    for (index, (identity, version, hash)) in SERVED_EVER.iter().enumerate() {
        for (other_identity, other_version, other_hash) in &SERVED_EVER[index + 1..] {
            if identity == other_identity && version == other_version {
                assert_eq!(hash, other_hash, "{identity} {version} has two hashes");
            }
        }
    }
}

#[test]
fn no_served_text_carries_inherited_lifecycle_or_disk_skill_wording() {
    let forbidden = [
        "`document`",
        "\"document\"",
        "document-search",
        ".planning",
        "skills/",
        "SKILL.md",
        "CLAUDE_PLUGIN_ROOT",
    ];
    for (identity, text) in served() {
        for word in forbidden {
            assert!(!text.body.contains(word), "{identity} contains {word}");
        }
    }
}

#[test]
fn the_help_text_asks_for_its_identity_on_apply_calls_only() {
    let Lookup::Served { text, .. } = lookup("bal-help") else {
        panic!("bal-help is served");
    };
    let body = text.body;
    assert!(body.contains("`bal-help`") && body.contains("`instruction`"));
    assert!(body.contains("on every `baley_apply` call"));
    assert!(body.contains("Never send it on a `baley_query` call"));
}

#[test]
fn the_read_contract_stays_within_a_description_and_keeps_its_paging_and_host_tools_rules() {
    let Lookup::Served { text, .. } = lookup("bal-read-contract") else {
        panic!("bal-read-contract is served");
    };
    let body = text.body;
    assert!(
        body.chars().count() <= 2048,
        "{} characters",
        body.chars().count()
    );
    for needle in [
        "\"operation\":\"instruction\"",
        "\"identity\"",
        "part",
        "next",
        "host's own",
    ] {
        assert!(body.contains(needle), "the read contract lost {needle}");
    }
}

#[test]
fn the_help_evidence_takes_its_version_and_hash_from_the_registry_not_the_binary() {
    let evidence = evidence("bal-help").unwrap();
    // The list is append-only, so the newest row is the last one for the identity.
    let (_, version, hash) = SERVED_EVER
        .iter()
        .rfind(|(identity, _, _)| *identity == "bal-help")
        .unwrap();
    assert_eq!(evidence.identity(), "bal-help");
    assert_eq!(evidence.version(), *version);
    assert_eq!(evidence.hash(), *hash);
    assert_ne!(evidence.version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn every_served_entry_fits_the_caller_records_limits_as_evidence() {
    for (identity, text) in served() {
        let evidence = evidence(identity).unwrap_or_else(|_| panic!("{identity} is refused"));
        assert_eq!(evidence.identity(), identity);
        assert_eq!(evidence.version(), text.version);
        assert_eq!(evidence.hash(), text.hash);
    }
}

#[test]
fn an_unavailable_or_unknown_identity_is_refused_and_given_no_evidence() {
    assert_eq!(
        evidence("bal-plan").unwrap_err(),
        unavailable("bal-plan", 4)
    );
    assert_eq!(evidence("no-such-instruction").unwrap_err(), unknown());
}
