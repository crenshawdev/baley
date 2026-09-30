//! Detection's decisions on supplied bodies, observations, rows and
//! documents. Bodies follow each provider's API reference as read on
//! 2026-09-30, and expected values come from design 0003 and the phase's
//! decisions, never from running this code.

use super::*;
use crate::catalog::Provider;

fn models(listing: &ProviderListing) -> Vec<(&str, Option<u64>)> {
    listing.models().collect()
}

#[test]
fn an_openai_page_gives_each_id_with_its_creation_time() {
    let body = br#"{"object":"list","data":[
        {"id":"gpt-6-astra","object":"model","created":1686935002,"owned_by":"openai"},
        {"id":"gpt-6-luna","object":"model","created":1700000000,"owned_by":"system"}
    ]}"#;
    let listing = parse_page(Provider::OpenAi, body).expect("a sound page");
    assert_eq!(
        models(&listing),
        [
            ("gpt-6-astra", Some(1686935002)),
            ("gpt-6-luna", Some(1700000000))
        ]
    );
}

#[test]
fn a_deepseek_page_without_created_gives_its_ids_without_a_time() {
    let body = br#"{"object":"list","data":[
        {"id":"deepseek-v4-pro","object":"model","owned_by":"deepseek"},
        {"id":"deepseek-flash","object":"model","owned_by":"deepseek"}
    ]}"#;
    let listing = parse_page(Provider::DeepSeek, body).expect("a sound page");
    assert_eq!(
        models(&listing),
        [("deepseek-flash", None), ("deepseek-v4-pro", None)]
    );
}

#[test]
fn a_gemini_name_loses_its_models_prefix() {
    let body = br#"{"models":[{"name":"models/gemini-x","displayName":"Gemini X"}]}"#;
    let listing = parse_page(Provider::Gemini, body).expect("a sound page");
    assert_eq!(models(&listing), [("gemini-x", None)]);
}

#[test]
fn an_empty_openai_list_and_an_empty_gemini_object_are_valid_empty_listings() {
    let openai = parse_page(Provider::OpenAi, br#"{"object":"list","data":[]}"#);
    assert_eq!(openai, Some(ProviderListing::default()));
    let gemini = parse_page(Provider::Gemini, b"{}");
    assert_eq!(gemini, Some(ProviderListing::default()));
}

#[test]
fn a_repeated_id_counts_once() {
    let body = br#"{"data":[{"id":"gpt-x","created":5},{"id":"gpt-x","created":9}]}"#;
    let listing = parse_page(Provider::OpenAi, body).expect("a sound page");
    assert_eq!(models(&listing).len(), 1);
}

#[test]
fn an_odd_created_drops_the_time_but_keeps_the_id() {
    let body = br#"{"data":[{"id":"gpt-a","created":"yesterday"},{"id":"gpt-b","created":1.5}]}"#;
    let listing = parse_page(Provider::OpenAi, body).expect("a sound page");
    assert_eq!(models(&listing), [("gpt-a", None), ("gpt-b", None)]);
}

#[test]
fn a_body_outside_the_list_shape_is_malformed_not_a_listing() {
    let cases: [(Provider, &[u8], &str); 5] = [
        (Provider::OpenAi, b"<html>502</html>", "not JSON"),
        (Provider::OpenAi, br#"[{"id":"gpt-x"}]"#, "a JSON array"),
        (
            Provider::DeepSeek,
            br#"{"data":{"id":"x"}}"#,
            "data not a list",
        ),
        (Provider::OpenAi, br#"{"object":"list"}"#, "no data"),
        (
            Provider::Gemini,
            br#"{"models":{"name":"models/x"}}"#,
            "models not a list",
        ),
    ];
    for (provider, body, why) in cases {
        assert_eq!(parse_page(provider, body), None, "{why}");
    }
}

#[test]
fn one_bad_item_among_good_ones_rejects_the_whole_page() {
    let cases: [(Provider, &[u8], &str); 5] = [
        (
            Provider::OpenAi,
            br#"{"data":[{"id":"gpt-a"},{"id":7},{"id":"gpt-b"}]}"#,
            "a numeric id",
        ),
        (
            Provider::DeepSeek,
            br#"{"data":[{"id":"ds-a"},{"object":"model"}]}"#,
            "an item with no id",
        ),
        (
            Provider::Gemini,
            br#"{"models":[{"name":"models/g-a"},{"name":"g-b"}]}"#,
            "a name without models/",
        ),
        (
            Provider::OpenAi,
            br#"{"data":[{"id":"gpt-a"},{"id":""}]}"#,
            "an empty id",
        ),
        (
            Provider::Gemini,
            br#"{"models":[{"name":"models/g-a"},{"name":"models/"}]}"#,
            "an empty Gemini id",
        ),
    ];
    for (provider, body, why) in cases {
        assert_eq!(parse_page(provider, body), None, "{why}");
    }
}

#[test]
fn a_gemini_page_with_a_numeric_continuation_is_malformed_though_its_items_parse() {
    let body = br#"{"models":[{"name":"models/g-a"}],"nextPageToken":42}"#;
    assert_eq!(parse_page(Provider::Gemini, body), None);
}
