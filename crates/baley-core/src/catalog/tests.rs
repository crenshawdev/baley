//! The catalog's decisions on supplied values. Expected values come from
//! design 0003 (CFG-R19 to CFG-R23) and the phase's decisions, never from
//! running this code.

use std::collections::BTreeSet;

use baley_store::EventSchema;
use serde_json::json;

use super::*;
use crate::policy::is_project_id;
use crate::registry::Registry;

#[test]
fn tier_parse_folds_no_case_and_takes_no_rung_name() {
    assert_eq!(Tier::parse("flagship"), Some(Tier::Flagship));
    assert_eq!(Tier::parse("balanced"), Some(Tier::Balanced));
    assert_eq!(Tier::parse("cheap"), Some(Tier::Cheap));
    assert_eq!(Tier::parse("Cheap"), None);
    assert_eq!(Tier::parse("high"), None);
    assert_eq!(Tier::parse(""), None);
}

#[test]
fn each_catalog_name_parses_to_its_own_catalog_and_round_trips() {
    let expected = [
        ("claude-code", Catalog::Host(Host::ClaudeCode)),
        ("codex", Catalog::Host(Host::Codex)),
        ("openai", Catalog::Provider(Provider::OpenAi)),
        ("gemini", Catalog::Provider(Provider::Gemini)),
        ("deepseek", Catalog::Provider(Provider::DeepSeek)),
    ];
    for (name, catalog) in expected {
        assert_eq!(Catalog::parse(name), Ok(catalog), "{name}");
        assert_eq!(catalog.name(), name);
    }
}

#[test]
fn anthropic_a_case_variant_and_the_empty_name_are_unknown_providers() {
    for name in ["anthropic", "Codex", ""] {
        let refusal = Catalog::parse(name).expect_err(name);
        assert_eq!(refusal.code(), "unknown-provider", "{name}");
    }
    assert_eq!(
        Catalog::parse("anthropic").unwrap_err().to_string(),
        "unknown-provider: \"anthropic\" is no model catalog; \
         catalogs: claude-code, codex, openai, gemini, deepseek"
    );
}

#[test]
fn the_reserved_user_project_is_no_id_a_project_file_could_name() {
    assert_eq!(USER_PROJECT, "user");
    assert!(!is_project_id(USER_PROJECT));
}

#[test]
fn claude_code_has_exactly_its_four_aliases_and_codex_none() {
    let claude: BTreeSet<&str> = host_aliases(Host::ClaudeCode).iter().copied().collect();
    let expected: BTreeSet<&str> = ["opus", "sonnet", "haiku", "fable"].into_iter().collect();
    assert_eq!(claude, expected);
    assert_eq!(host_aliases(Host::ClaudeCode).len(), 4);
    assert!(host_aliases(Host::Codex).is_empty());
}

#[test]
fn no_two_exact_hint_rows_give_one_id_two_tiers() {
    let mut seen = BTreeSet::new();
    for row in EXACT_HINTS {
        assert!(seen.insert((row.provider, row.id)), "{row:?}");
    }
}

#[test]
fn no_two_prefix_rows_share_a_provider_and_prefix() {
    let mut seen = BTreeSet::new();
    for row in PREFIX_HINTS {
        assert!(seen.insert((row.provider, row.prefix)), "{row:?}");
    }
}

#[test]
fn no_provider_is_left_out_of_the_exact_hint_rows() {
    for provider in Provider::ALL {
        assert!(
            EXACT_HINTS.iter().any(|row| row.provider == provider),
            "{provider:?}"
        );
    }
}

#[test]
fn no_hint_row_has_an_empty_id_or_prefix() {
    assert!(EXACT_HINTS.iter().all(|row| !row.id.is_empty()));
    assert!(PREFIX_HINTS.iter().all(|row| !row.prefix.is_empty()));
}

#[test]
fn every_models_event_type_reads_at_version_1_and_no_later() {
    let mut registry = Registry::new();
    register_model_events(&mut registry).unwrap();
    for type_name in [
        "models.seeded",
        "models.owner_changed",
        "models.detected",
        "models.detection_failed",
    ] {
        assert!(registry.reads(type_name, 1), "{type_name}");
        assert!(!registry.reads(type_name, 2), "{type_name}");
    }
}

#[test]
fn an_owner_addition_without_a_tier_has_no_tier_key_and_one_with_a_tier_names_it() {
    let openai = Catalog::Provider(Provider::OpenAi);
    assert_eq!(
        owner_changed_payload(openai, "gpt-x", OwnerChange::Added(None), 4),
        json!({"catalog": "openai", "name": "gpt-x", "change": "added", "catalog_version": 4})
    );
    assert_eq!(
        owner_changed_payload(openai, "gpt-x", OwnerChange::Added(Some(Tier::Cheap)), 4),
        json!({
            "catalog": "openai", "name": "gpt-x", "change": "added",
            "tier": "cheap", "catalog_version": 4
        })
    );
}

#[test]
fn an_owner_removal_payload_holds_exactly_catalog_name_change_and_version() {
    let codex = Catalog::Host(Host::Codex);
    assert_eq!(
        owner_changed_payload(codex, "o-max", OwnerChange::Removed, 9),
        json!({"catalog": "codex", "name": "o-max", "change": "removed", "catalog_version": 9})
    );
}

#[test]
fn a_seed_payload_keeps_every_field_name_and_flag_of_its_rows() {
    let rows = [
        HintRow {
            provider: Provider::Gemini,
            id: "gem-a",
            tier: Tier::Flagship,
            high_effort: true,
        },
        HintRow {
            provider: Provider::DeepSeek,
            id: "deep-b",
            tier: Tier::Cheap,
            high_effort: false,
        },
    ];
    assert_eq!(
        seeded_payload(3, 12, &rows),
        json!({
            "hint_version": 3,
            "catalog_version": 12,
            "rows": [
                {"provider": "gemini", "name": "gem-a", "tier": "flagship", "high_effort": true},
                {"provider": "deepseek", "name": "deep-b", "tier": "cheap", "high_effort": false},
            ]
        })
    );
}
