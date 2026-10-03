//! Which providers the catalog and detection know, checked from outside the
//! files that must not name a provider Baley no longer has.

use baley::detection::key_name;
use baley::models::named_providers;
use baley_core::catalog::{Catalog, Provider};

#[test]
fn gemini_is_no_model_catalog_and_the_refusal_names_the_three_that_are() {
    let refusal = Catalog::parse("gemini").unwrap_err();
    assert_eq!(refusal.code(), "unknown-provider");
    assert_eq!(
        refusal.to_string(),
        "unknown-provider: \"gemini\" is no model catalog; \
         catalogs: claude-code, openai, deepseek"
    );
}

#[test]
fn gemini_is_no_provider_models_update_detects() {
    let refusal = named_providers(&["gemini".to_string()]).unwrap_err();
    assert!(
        refusal.starts_with(
            "unknown-provider: \"gemini\" is no provider Baley detects; providers: openai, deepseek"
        ),
        "{refusal}"
    );
}

#[test]
fn the_providers_are_openai_then_deepseek_each_under_its_own_key_name() {
    assert_eq!(Provider::ALL, [Provider::OpenAi, Provider::DeepSeek]);
    let names = Provider::ALL.map(key_name);
    assert_eq!(names, ["OPENAI_API_KEY", "DEEPSEEK_API_KEY"]);
}
