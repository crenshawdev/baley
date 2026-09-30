//! Detection's decisions on supplied bodies, observations, rows and
//! documents. Bodies follow each provider's API reference as read on
//! 2026-09-30, and expected values come from design 0003 and the phase's
//! decisions, never from running this code.

use serde_json::json;

use super::*;
use crate::catalog::{HintRow, Placement, PrefixRow, Provider, Tier};

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

#[test]
fn a_gemini_page_gives_its_next_page_token() {
    let body = br#"{"models":[{"name":"models/g-a"}],"nextPageToken":"t2"}"#;
    assert_eq!(next_page(Provider::Gemini, body), Some("t2".to_owned()));
}

#[test]
fn a_last_gemini_page_with_no_or_an_empty_token_gives_no_next_page() {
    let last = br#"{"models":[{"name":"models/g-a"}]}"#;
    assert_eq!(next_page(Provider::Gemini, last), None);
    let empty = br#"{"models":[],"nextPageToken":""}"#;
    assert_eq!(next_page(Provider::Gemini, empty), None);
    assert_eq!(next_page(Provider::Gemini, b"not json"), None);
}

#[test]
fn openai_and_deepseek_never_page_even_when_a_body_carries_a_token() {
    let body = br#"{"object":"list","data":[{"id":"x"}],"nextPageToken":"t2"}"#;
    assert_eq!(next_page(Provider::OpenAi, body), None);
    assert_eq!(next_page(Provider::DeepSeek, body), None);
}

#[test]
fn a_page_without_a_continuation_completes_the_listing_up_to_the_20th() {
    assert_eq!(paging(1, None), Paging::Complete);
    assert_eq!(paging(20, None), Paging::Complete);
}

#[test]
fn a_continuation_is_followed_after_each_of_pages_1_to_19() {
    for page in 1..=19 {
        assert_eq!(
            paging(page, Some(format!("t{}", page + 1))),
            Paging::Follow(format!("t{}", page + 1)),
            "page {page}"
        );
    }
}

#[test]
fn a_20th_page_that_still_continues_cuts_the_listing_short_and_is_not_followed() {
    assert_eq!(PAGE_BOUND, 20);
    assert_eq!(paging(20, Some("t21".to_owned())), Paging::CutShort);
}

// A key-shaped marker that must never reach a category, a Debug or a payload.
const SENTINEL: &str = "sk-SENTINEL-4f1c9e";

fn response(status: u16, body: &[u8]) -> ObservedResponse {
    ObservedResponse {
        status,
        body: body.to_vec(),
        cut_short: false,
    }
}

fn observed(responses: Vec<ObservedResponse>) -> Observation {
    Observation {
        responses,
        ..Observation::default()
    }
}

fn gemini_error(reason: &str) -> Vec<u8> {
    json!({"error": {
        "code": 400,
        "message": "API key not valid. Please pass a valid API key.",
        "status": "INVALID_ARGUMENT",
        "details": [{
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": reason,
            "domain": "googleapis.com",
        }],
    }})
    .to_string()
    .into_bytes()
}

#[test]
fn a_401_or_403_is_unauthorized_for_every_provider() {
    for provider in Provider::ALL {
        for status in [401, 403] {
            let observation = observed(vec![response(status, b"{}")]);
            assert_eq!(
                classify(provider, &observation),
                Err(Category::Unauthorized),
                "{provider:?} {status}"
            );
        }
    }
}

#[test]
fn a_gemini_400_is_unauthorized_only_for_an_api_key_invalid_reason() {
    let invalid = observed(vec![response(400, &gemini_error("API_KEY_INVALID"))]);
    assert_eq!(
        classify(Provider::Gemini, &invalid),
        Err(Category::Unauthorized)
    );
    let other = observed(vec![response(400, &gemini_error("FIELD_INVALID"))]);
    assert_eq!(classify(Provider::Gemini, &other), Err(Category::Http(400)));
}

#[test]
fn the_gemini_400_rule_never_applies_to_openai() {
    let body = gemini_error("API_KEY_INVALID");
    let observation = observed(vec![response(400, &body)]);
    assert_eq!(
        classify(Provider::OpenAi, &observation),
        Err(Category::Http(400))
    );
}

#[test]
fn a_429_is_rate_limited_and_other_statuses_keep_their_number() {
    let cases = [
        (429, Category::RateLimited),
        (503, Category::Http(503)),
        (301, Category::Http(301)),
    ];
    for (status, category) in cases {
        let observation = observed(vec![response(status, b"")]);
        assert_eq!(
            classify(Provider::DeepSeek, &observation),
            Err(category),
            "{status}"
        );
    }
}

#[test]
fn a_200_outside_the_list_shape_or_with_one_bad_item_is_malformed() {
    let shape = observed(vec![response(200, br#"{"error":"nope"}"#)]);
    assert_eq!(classify(Provider::OpenAi, &shape), Err(Category::Malformed));
    let bad_item = observed(vec![response(
        200,
        br#"{"data":[{"id":"gpt-a"},{"id":null}]}"#,
    )]);
    assert_eq!(
        classify(Provider::OpenAi, &bad_item),
        Err(Category::Malformed)
    );
}

#[test]
fn a_malformed_later_page_rejects_the_listing_though_the_first_was_sound() {
    let observation = observed(vec![
        response(
            200,
            br#"{"models":[{"name":"models/g-a"}],"nextPageToken":"t2"}"#,
        ),
        response(200, br#"{"models":"broken"}"#),
    ]);
    assert_eq!(
        classify(Provider::Gemini, &observation),
        Err(Category::Malformed)
    );
}

#[test]
fn a_transport_failure_after_a_good_page_is_offline_not_a_listing() {
    let observation = Observation {
        responses: vec![response(
            200,
            br#"{"models":[{"name":"models/g-a"}],"nextPageToken":"t2"}"#,
        )],
        transport_failed: true,
        page_bound_hit: false,
    };
    assert_eq!(
        classify(Provider::Gemini, &observation),
        Err(Category::Offline)
    );
}

#[test]
fn a_200_body_cut_short_is_incomplete_even_when_what_was_kept_parses() {
    let mut cut = response(200, br#"{"data":[{"id":"gpt-a"}]}"#);
    cut.cut_short = true;
    assert_eq!(
        classify(Provider::OpenAi, &observed(vec![cut])),
        Err(Category::Incomplete)
    );
}

#[test]
fn good_pages_stopped_by_the_page_bound_are_incomplete_not_a_listing() {
    let page = response(
        200,
        br#"{"models":[{"name":"models/g-a"}],"nextPageToken":"more"}"#,
    );
    let observation = Observation {
        responses: vec![page; PAGE_BOUND],
        transport_failed: false,
        page_bound_hit: true,
    };
    assert_eq!(
        classify(Provider::Gemini, &observation),
        Err(Category::Incomplete)
    );
}

#[test]
fn two_good_gemini_pages_give_the_union_of_their_ids() {
    let observation = observed(vec![
        response(
            200,
            br#"{"models":[{"name":"models/g-a"},{"name":"models/g-b"}],"nextPageToken":"t2"}"#,
        ),
        response(
            200,
            br#"{"models":[{"name":"models/g-b"},{"name":"models/g-c"}]}"#,
        ),
    ]);
    let listing = classify(Provider::Gemini, &observation).expect("a listing");
    assert_eq!(
        models(&listing),
        [("g-a", None), ("g-b", None), ("g-c", None)]
    );
}

#[test]
fn a_200_empty_list_is_a_valid_empty_listing() {
    let observation = observed(vec![response(200, br#"{"object":"list","data":[]}"#)]);
    assert_eq!(
        classify(Provider::OpenAi, &observation),
        Ok(ProviderListing::default())
    );
}

#[test]
fn an_observation_with_no_response_is_a_category_never_an_empty_listing() {
    assert_eq!(
        classify(Provider::OpenAi, &Observation::default()),
        Err(Category::Offline)
    );
}

#[test]
fn each_category_is_recorded_under_exactly_its_name() {
    let cases = [
        (Category::Offline, "offline"),
        (Category::Incomplete, "incomplete"),
        (Category::Malformed, "malformed"),
        (Category::Unauthorized, "unauthorized"),
        (Category::RateLimited, "rate-limited"),
        (Category::Http(503), "http-503"),
        (Category::KeysFileExposed, "keys-file-exposed"),
        (Category::KeysFileInvalid, "keys-file-invalid"),
        (Category::KeysFileUnreadable, "keys-file-unreadable"),
    ];
    for (category, name) in cases {
        assert_eq!(category.name(), name);
    }
}

#[test]
fn a_401_echoing_the_key_gives_a_category_without_it() {
    let body = json!({"error": {
        "message": format!("Incorrect API key provided: {SENTINEL}."),
        "type": "invalid_request_error",
        "code": "invalid_api_key",
    }})
    .to_string();
    let observation = observed(vec![response(401, body.as_bytes())]);
    let category = classify(Provider::OpenAi, &observation).expect_err("a failure");
    assert!(!category.name().contains(SENTINEL));
    assert!(!format!("{category:?}").contains(SENTINEL));
}

#[test]
fn an_observations_debug_shows_body_lengths_never_body_bytes() {
    let body = format!(r#"{{"error":{{"message":"bad key {SENTINEL}"}}}}"#);
    let observation = observed(vec![response(401, body.as_bytes())]);
    let shown = format!("{observation:?}");
    assert!(!shown.contains(SENTINEL), "{shown}");
    assert!(
        shown.contains(&format!("body_len: {}", body.len())),
        "{shown}"
    );
}

const fn exact_row(provider: Provider, id: &'static str, tier: Tier, high_effort: bool) -> HintRow {
    HintRow {
        provider,
        id,
        tier,
        high_effort,
    }
}

const fn prefix_row(
    provider: Provider,
    prefix: &'static str,
    tier: Tier,
    high_effort: bool,
) -> PrefixRow {
    PrefixRow {
        provider,
        prefix,
        tier,
        high_effort,
    }
}

fn tagged(tier: Tier, high_effort: bool, placed: Placement) -> Option<Tag> {
    Some(Tag {
        tier,
        high_effort,
        placed,
    })
}

#[test]
fn an_exact_row_beats_a_prefix_row_that_also_matches() {
    let exact = [exact_row(
        Provider::OpenAi,
        "gpt-6-astra",
        Tier::Cheap,
        false,
    )];
    let prefixes = [prefix_row(Provider::OpenAi, "gpt-6", Tier::Flagship, true)];
    assert_eq!(
        tag(Provider::OpenAi, "gpt-6-astra", &exact, &prefixes),
        tagged(Tier::Cheap, false, Placement::Hint)
    );
}

#[test]
fn the_longest_matching_prefix_row_wins_whatever_its_place_in_the_table() {
    let long_last = [
        prefix_row(Provider::OpenAi, "gpt-6", Tier::Balanced, false),
        prefix_row(Provider::OpenAi, "gpt-6-astra", Tier::Flagship, true),
    ];
    let long_first = [long_last[1], long_last[0]];
    for prefixes in [long_last, long_first] {
        assert_eq!(
            tag(Provider::OpenAi, "gpt-6-astra-2026-09-01", &[], &prefixes),
            tagged(Tier::Flagship, true, Placement::Prefix)
        );
    }
}

#[test]
fn another_providers_row_never_tags_an_id_with_the_same_text() {
    let exact = [exact_row(
        Provider::DeepSeek,
        "shared-1",
        Tier::Flagship,
        true,
    )];
    let prefixes = [prefix_row(Provider::DeepSeek, "shared", Tier::Cheap, true)];
    assert_eq!(tag(Provider::OpenAi, "shared-1", &exact, &prefixes), None);
    assert_eq!(tag(Provider::OpenAi, "shared-2", &exact, &prefixes), None);
}

#[test]
fn an_id_no_row_names_or_starts_is_left_untagged() {
    let exact = [exact_row(
        Provider::OpenAi,
        "gpt-6-astra",
        Tier::Flagship,
        true,
    )];
    let prefixes = [prefix_row(
        Provider::OpenAi,
        "gpt-6-astra",
        Tier::Flagship,
        true,
    )];
    assert_eq!(tag(Provider::OpenAi, "gpt-6", &exact, &prefixes), None);
    assert_eq!(tag(Provider::OpenAi, "o9-mini", &exact, &prefixes), None);
}

fn candidate(name: &str, tier: Tier, high_effort: bool, created: Option<u64>) -> Candidate<'_> {
    Candidate {
        name,
        tier,
        high_effort,
        created,
    }
}

#[test]
fn best_fit_takes_the_candidate_sharing_the_longest_run_of_segments() {
    let candidates = [
        candidate("gemini-3.8-flash", Tier::Cheap, false, None),
        candidate("gemini-3.8-pro", Tier::Flagship, true, None),
    ];
    assert_eq!(
        best_fit("gemini-3.8-pro-exp", &candidates),
        Tag {
            tier: Tier::Flagship,
            high_effort: true,
            placed: Placement::BestFit
        }
    );
    // The longer run wins though the other candidate would take every tie.
    let shorter_but_newer = [
        candidate("gemini-3.8-pro", Tier::Flagship, true, None),
        candidate("gemini-3.8-zeta", Tier::Cheap, false, Some(999)),
    ];
    assert_eq!(
        best_fit("gemini-3.8-pro-exp", &shorter_but_newer).tier,
        Tier::Flagship
    );
}

#[test]
fn best_fit_matches_whole_segments_not_characters() {
    let candidates = [
        candidate("gpt-6", Tier::Flagship, true, None),
        candidate("gpt-60", Tier::Cheap, false, None),
    ];
    assert_eq!(best_fit("gpt-60-mini", &candidates).tier, Tier::Cheap);
    // Characters would pick `gpt-6` (5 shared against 4). Segments tie at one,
    // and the name that sorts last wins.
    let tied = [
        candidate("gpt-6", Tier::Flagship, true, None),
        candidate("gpt-x", Tier::Cheap, false, None),
    ];
    assert_eq!(best_fit("gpt-60-mini", &tied).tier, Tier::Cheap);
}

#[test]
fn best_fit_breaks_an_equal_run_toward_the_newest_creation_time() {
    let candidates = [
        candidate("gpt-7-b", Tier::Cheap, false, Some(200)),
        candidate("gpt-7-a", Tier::Flagship, false, Some(100)),
    ];
    assert_eq!(best_fit("gpt-7-z", &candidates).tier, Tier::Cheap);
    let reversed = [candidates[1], candidates[0]];
    assert_eq!(best_fit("gpt-7-z", &reversed).tier, Tier::Cheap);
}

#[test]
fn best_fit_breaks_an_equal_run_toward_a_candidate_with_a_time() {
    let candidates = [
        candidate("gpt-7-b", Tier::Flagship, false, None),
        candidate("gpt-7-a", Tier::Cheap, false, Some(100)),
    ];
    assert_eq!(best_fit("gpt-7-z", &candidates).tier, Tier::Cheap);
}

#[test]
fn best_fit_breaks_an_equal_run_without_times_toward_the_name_that_sorts_last() {
    let candidates = [
        candidate("gpt-7-b", Tier::Cheap, false, None),
        candidate("gpt-7-a", Tier::Flagship, false, None),
    ];
    assert_eq!(best_fit("gpt-7-z", &candidates).tier, Tier::Cheap);
    let reversed = [candidates[1], candidates[0]];
    assert_eq!(best_fit("gpt-7-z", &reversed).tier, Tier::Cheap);
}

#[test]
fn best_fit_takes_the_winners_high_effort_flag_not_another_candidates() {
    let candidates = [
        candidate("deepseek-v5-pro", Tier::Flagship, false, None),
        candidate("deepseek-v5", Tier::Balanced, true, None),
    ];
    let placed = best_fit("deepseek-v5-pro-0930", &candidates);
    assert_eq!((placed.tier, placed.high_effort), (Tier::Flagship, false));
}

#[test]
fn an_id_of_no_known_family_is_balanced_without_high_effort_placed_best_fit() {
    let fallback = Tag {
        tier: Tier::Balanced,
        high_effort: false,
        placed: Placement::BestFit,
    };
    let candidates = [candidate("gpt-6", Tier::Flagship, true, Some(1))];
    assert_eq!(best_fit("text-embedding-3-large", &candidates), fallback);
    assert_eq!(best_fit("whisper-1", &[]), fallback);
}
