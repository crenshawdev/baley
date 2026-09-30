//! The catalog's decisions on supplied values. Expected values come from
//! design 0003 (CFG-R19 to CFG-R23) and the phase's decisions, never from
//! running this code.

use super::*;
use crate::policy::is_project_id;

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
