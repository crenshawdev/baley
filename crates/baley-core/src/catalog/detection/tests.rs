//! Detection's decisions on supplied bodies, observations, rows and
//! documents. Bodies follow each provider's API reference as read on
//! 2026-09-30, and expected values come from design 0003 and the phase's
//! decisions, never from running this code.

use serde_json::{Value, json};

use super::*;
use crate::catalog::{
    HINT_VERSION, HintRow, MODELS_DETECTED, MODELS_DETECTED_VERSION, MODELS_DETECTION_FAILED,
    MODELS_DETECTION_FAILED_VERSION, Placement, PrefixRow, Provider, Tier,
};

fn models(listing: &ProviderListing) -> Vec<(&str, Option<u64>)> {
    listing.models().collect()
}

#[test]
fn an_openai_body_gives_each_id_with_its_creation_time() {
    let body = br#"{"object":"list","data":[
        {"id":"gpt-6-astra","object":"model","created":1686935002,"owned_by":"openai"},
        {"id":"gpt-6-luna","object":"model","created":1700000000,"owned_by":"system"}
    ]}"#;
    let listing = parse_body(Provider::OpenAi, body).expect("a sound body");
    assert_eq!(
        models(&listing),
        [
            ("gpt-6-astra", Some(1686935002)),
            ("gpt-6-luna", Some(1700000000))
        ]
    );
}

#[test]
fn a_deepseek_body_without_created_gives_its_ids_without_a_time() {
    let body = br#"{"object":"list","data":[
        {"id":"deepseek-v4-pro","object":"model","owned_by":"deepseek"},
        {"id":"deepseek-flash","object":"model","owned_by":"deepseek"}
    ]}"#;
    let listing = parse_body(Provider::DeepSeek, body).expect("a sound body");
    assert_eq!(
        models(&listing),
        [("deepseek-flash", None), ("deepseek-v4-pro", None)]
    );
}

#[test]
fn a_gemini_name_loses_its_models_prefix() {
    let body = br#"{"models":[{"name":"models/gemini-x","displayName":"Gemini X"}]}"#;
    let listing = parse_body(Provider::Gemini, body).expect("a sound body");
    assert_eq!(models(&listing), [("gemini-x", None)]);
}

#[test]
fn an_empty_openai_list_and_an_empty_gemini_object_are_valid_empty_listings() {
    let openai = parse_body(Provider::OpenAi, br#"{"object":"list","data":[]}"#);
    assert_eq!(openai, Some(ProviderListing::default()));
    let gemini = parse_body(Provider::Gemini, b"{}");
    assert_eq!(gemini, Some(ProviderListing::default()));
}

#[test]
fn a_repeated_id_counts_once() {
    let body = br#"{"data":[{"id":"gpt-x","created":5},{"id":"gpt-x","created":9}]}"#;
    let listing = parse_body(Provider::OpenAi, body).expect("a sound body");
    assert_eq!(models(&listing).len(), 1);
}

#[test]
fn an_odd_created_drops_the_time_but_keeps_the_id() {
    let body = br#"{"data":[{"id":"gpt-a","created":"yesterday"},{"id":"gpt-b","created":1.5}]}"#;
    let listing = parse_body(Provider::OpenAi, body).expect("a sound body");
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
        assert_eq!(parse_body(provider, body), None, "{why}");
    }
}

#[test]
fn one_bad_item_among_good_ones_rejects_the_whole_body() {
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
        assert_eq!(parse_body(provider, body), None, "{why}");
    }
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

fn observed(response: ObservedResponse) -> Observation {
    Observation {
        response: Some(response),
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
            let observation = observed(response(status, b"{}"));
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
    let invalid = observed(response(400, &gemini_error("API_KEY_INVALID")));
    assert_eq!(
        classify(Provider::Gemini, &invalid),
        Err(Category::Unauthorized)
    );
    let other = observed(response(400, &gemini_error("FIELD_INVALID")));
    assert_eq!(classify(Provider::Gemini, &other), Err(Category::Http(400)));
}

#[test]
fn the_gemini_400_rule_never_applies_to_openai() {
    let body = gemini_error("API_KEY_INVALID");
    let observation = observed(response(400, &body));
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
        let observation = observed(response(status, b""));
        assert_eq!(
            classify(Provider::DeepSeek, &observation),
            Err(category),
            "{status}"
        );
    }
}

#[test]
fn a_200_outside_the_list_shape_or_with_one_bad_item_is_malformed() {
    let shape = observed(response(200, br#"{"error":"nope"}"#));
    assert_eq!(classify(Provider::OpenAi, &shape), Err(Category::Malformed));
    let bad_item = observed(response(200, br#"{"data":[{"id":"gpt-a"},{"id":null}]}"#));
    assert_eq!(
        classify(Provider::OpenAi, &bad_item),
        Err(Category::Malformed)
    );
}

#[test]
fn a_transport_failure_beside_a_good_response_is_offline_not_a_listing() {
    let observation = Observation {
        response: Some(response(
            200,
            br#"{"object":"list","data":[{"id":"gpt-a","created":5}]}"#,
        )),
        transport_failed: true,
    };
    assert_eq!(
        classify(Provider::OpenAi, &observation),
        Err(Category::Offline)
    );
}

#[test]
fn a_200_body_cut_short_is_incomplete_even_when_what_was_kept_parses() {
    let mut cut = response(200, br#"{"data":[{"id":"gpt-a"}]}"#);
    cut.cut_short = true;
    assert_eq!(
        classify(Provider::OpenAi, &observed(cut)),
        Err(Category::Incomplete)
    );
}

#[test]
fn a_200_empty_list_is_a_valid_empty_listing() {
    let observation = observed(response(200, br#"{"object":"list","data":[]}"#));
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
    let observation = observed(response(401, body.as_bytes()));
    let category = classify(Provider::OpenAi, &observation).expect_err("a failure");
    assert!(!category.name().contains(SENTINEL));
    assert!(!format!("{category:?}").contains(SENTINEL));
}

#[test]
fn an_observations_debug_shows_body_lengths_never_body_bytes() {
    let body = format!(r#"{{"error":{{"message":"bad key {SENTINEL}"}}}}"#);
    let observation = observed(response(401, body.as_bytes()));
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
        candidate("deepseek-v9-flash", Tier::Cheap, false, None),
        candidate("deepseek-v9-pro", Tier::Flagship, true, None),
    ];
    assert_eq!(
        best_fit("deepseek-v9-pro-exp", &candidates),
        Tag {
            tier: Tier::Flagship,
            high_effort: true,
            placed: Placement::BestFit
        }
    );
    // The longer run wins though the other candidate would take every tie.
    let shorter_but_newer = [
        candidate("deepseek-v9-pro", Tier::Flagship, true, None),
        candidate("deepseek-v9-prover", Tier::Cheap, false, Some(999)),
    ];
    assert_eq!(
        best_fit("deepseek-v9-pro-exp", &shorter_but_newer).tier,
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

// A catalog document entry as the projector stores it.
fn entry(id: &str, source: &str, tier: Option<&str>, high_effort: bool, placed: &str) -> Value {
    let mut entry = json!({
        "id": id, "source": source, "high_effort": high_effort, "placed": placed,
        "first_seen": "2026-09-01T00:00:00Z", "accepted_seq": 1, "owner_removed": false,
    });
    if let Some(tier) = tier {
        entry["tier"] = tier.into();
    }
    entry
}

fn owner_removed(id: &str) -> Value {
    let mut removed = entry(id, "owner", None, false, "owner");
    removed["owner_removed"] = true.into();
    removed.as_object_mut().unwrap().remove("accepted_seq");
    removed
}

fn document(provider: &str, entries: Vec<Value>) -> Value {
    json!({"catalog": provider, "entries": entries})
}

// A provider's body listing `ids`, each with its creation time.
fn listing_of(provider: Provider, ids: &[(&str, Option<u64>)]) -> ProviderListing {
    let data: Vec<Value> = ids
        .iter()
        .map(|(id, created)| match created {
            Some(at) => json!({"id": id, "created": at}),
            None => json!({"id": id}),
        })
        .collect();
    let body = json!({"object": "list", "data": data});
    parse_body(provider, body.to_string().as_bytes()).expect("a sound body")
}

fn added_tag(diff: &Diff, id: &str) -> Option<Tag> {
    diff.added
        .iter()
        .find(|added| added.id == id)
        .map(|added| added.tag)
}

fn row<'a>(diff: &'a Diff, id: &str) -> Option<&'a ReportRow> {
    diff.report.rows.iter().find(|row| row.id == id)
}

const ASTRA: [HintRow; 1] = [exact_row(
    Provider::OpenAi,
    "gpt-6-astra",
    Tier::Flagship,
    true,
)];

#[test]
fn a_listed_seeded_id_is_added_again_placed_hint_and_counted_unchanged() {
    let doc = document(
        "openai",
        vec![entry("gpt-6-astra", "seed", Some("flagship"), true, "hint")],
    );
    let listing = listing_of(Provider::OpenAi, &[("gpt-6-astra", Some(10))]);
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert_eq!(
        added_tag(&diff, "gpt-6-astra"),
        tagged(Tier::Flagship, true, Placement::Hint)
    );
    assert_eq!(diff.report.count(IdChange::Unchanged), 1);
    assert_eq!(diff.report.count(IdChange::New), 0);
}

#[test]
fn a_best_fit_id_matched_by_a_prefix_row_now_is_added_placed_prefix() {
    let doc = document(
        "openai",
        vec![entry(
            "gpt-7-nova-0901",
            "detected",
            Some("balanced"),
            false,
            "best-fit",
        )],
    );
    let prefixes = [prefix_row(
        Provider::OpenAi,
        "gpt-7-nova",
        Tier::Flagship,
        true,
    )];
    let listing = listing_of(Provider::OpenAi, &[("gpt-7-nova-0901", None)]);
    let diff = diff(Provider::OpenAi, &listing, &[], &prefixes, Some(&doc));
    assert_eq!(
        added_tag(&diff, "gpt-7-nova-0901"),
        tagged(Tier::Flagship, true, Placement::Prefix)
    );
}

#[test]
fn a_listed_id_the_owner_removed_is_neither_added_nor_reported() {
    let doc = document("openai", vec![owner_removed("gpt-6-astra")]);
    let listing = listing_of(Provider::OpenAi, &[("gpt-6-astra", None)]);
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert!(diff.added.is_empty(), "{diff:?}");
    assert!(diff.report.rows.is_empty(), "{diff:?}");
}

#[test]
fn unlisted_seed_and_detected_ids_are_removed_but_owner_entries_and_removals_are_not() {
    let doc = document(
        "openai",
        vec![
            entry("gpt-6-astra", "seed", Some("flagship"), true, "hint"),
            entry("gpt-old", "detected", Some("cheap"), false, "best-fit"),
            entry("gpt-mine", "owner", Some("cheap"), false, "owner"),
            entry("gpt-untiered", "owner", None, false, "owner"),
            owner_removed("gpt-gone"),
            // The owner removed a seeded id: the projector keeps its source.
            {
                let mut seeded = entry("gpt-6-luna", "seed", Some("cheap"), true, "hint");
                seeded["owner_removed"] = true.into();
                seeded
            },
        ],
    );
    let listing = listing_of(Provider::OpenAi, &[]);
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert_eq!(diff.removed, ["gpt-6-astra", "gpt-old"]);
    assert_eq!(
        row(&diff, "gpt-old").map(|row| (row.change, row.tier, row.placed)),
        Some((IdChange::Removed, Some(Tier::Cheap), Placement::BestFit))
    );
}

#[test]
fn a_new_id_takes_its_family_tier_from_each_kind_of_candidate() {
    let family = |tier: &str, high_effort: bool, placed: &str, source: &str| {
        document(
            "deepseek",
            vec![entry(
                "deepseek-v9-pro",
                source,
                Some(tier),
                high_effort,
                placed,
            )],
        )
    };
    let cases = [
        (
            "hint",
            family("cheap", true, "hint", "seed"),
            Tier::Cheap,
            true,
        ),
        (
            "prefix",
            family("flagship", false, "prefix", "detected"),
            Tier::Flagship,
            false,
        ),
        (
            "owner",
            family("cheap", false, "owner", "owner"),
            Tier::Cheap,
            false,
        ),
    ];
    let listing = listing_of(Provider::DeepSeek, &[("deepseek-v9-pro-exp", None)]);
    for (kind, doc, tier, high_effort) in cases {
        let diff = diff(Provider::DeepSeek, &listing, &[], &[], Some(&doc));
        assert_eq!(
            added_tag(&diff, "deepseek-v9-pro-exp"),
            tagged(tier, high_effort, Placement::BestFit),
            "{kind}"
        );
    }

    // A tagged id of this same listing, with no document at all.
    let rows = [exact_row(Provider::OpenAi, "gpt-9", Tier::Cheap, true)];
    let listing = listing_of(Provider::OpenAi, &[("gpt-9", None), ("gpt-9-mini", None)]);
    let diff = diff(Provider::OpenAi, &listing, &rows, &[], None);
    assert_eq!(
        added_tag(&diff, "gpt-9-mini"),
        tagged(Tier::Cheap, true, Placement::BestFit)
    );
}

#[test]
fn a_new_id_takes_nothing_from_a_best_fit_entry_or_an_untiered_owner_entry() {
    let fallback = tagged(Tier::Balanced, false, Placement::BestFit);
    let docs = [
        document(
            "deepseek",
            vec![entry(
                "deepseek-v9-pro",
                "detected",
                Some("flagship"),
                true,
                "best-fit",
            )],
        ),
        document(
            "deepseek",
            vec![entry("deepseek-v9-pro", "owner", None, true, "owner")],
        ),
    ];
    let listing = listing_of(Provider::DeepSeek, &[("deepseek-v9-pro-exp", None)]);
    for doc in docs {
        let diff = diff(Provider::DeepSeek, &listing, &[], &[], Some(&doc));
        assert_eq!(added_tag(&diff, "deepseek-v9-pro-exp"), fallback, "{doc}");
    }
}

#[test]
fn an_id_placed_by_best_fit_this_run_is_no_candidate_for_another() {
    // `gem-a-x` takes flagship from `gem-a-x-base`. Were it a candidate, its
    // creation time would win `gem-a-y` its tie against `gem-a-z` (cheap).
    let doc = document(
        "openai",
        vec![
            entry("gem-a-x-base", "seed", Some("flagship"), true, "hint"),
            entry("gem-a-z", "seed", Some("cheap"), false, "hint"),
        ],
    );
    let listing = listing_of(Provider::OpenAi, &[("gem-a-x", Some(5)), ("gem-a-y", None)]);
    let diff = diff(Provider::OpenAi, &listing, &[], &[], Some(&doc));
    assert_eq!(
        added_tag(&diff, "gem-a-x").map(|tag| tag.tier),
        Some(Tier::Flagship)
    );
    assert_eq!(
        added_tag(&diff, "gem-a-y").map(|tag| tag.tier),
        Some(Tier::Cheap)
    );
}

#[test]
fn an_embedding_id_of_no_known_family_is_added_balanced_by_best_fit() {
    let doc = document(
        "openai",
        vec![entry("gpt-6-astra", "seed", Some("flagship"), true, "hint")],
    );
    let listing = listing_of(
        Provider::OpenAi,
        &[("gpt-6-astra", None), ("text-embedding-3-large", None)],
    );
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert_eq!(
        added_tag(&diff, "text-embedding-3-large"),
        tagged(Tier::Balanced, false, Placement::BestFit)
    );
}

#[test]
fn a_listed_owner_entry_reports_its_own_tier_and_placement_and_is_unchanged() {
    let doc = document(
        "openai",
        vec![entry("gpt-6-astra", "owner", Some("cheap"), false, "owner")],
    );
    let listing = listing_of(Provider::OpenAi, &[("gpt-6-astra", None)]);
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert_eq!(
        row(&diff, "gpt-6-astra"),
        Some(&ReportRow {
            id: "gpt-6-astra".into(),
            change: IdChange::Unchanged,
            tier: Some(Tier::Cheap),
            placed: Placement::Owner,
        })
    );
}

#[test]
fn the_counts_are_taken_against_the_document_before_the_event() {
    let doc = document(
        "openai",
        vec![
            entry("gpt-6-astra", "seed", Some("flagship"), true, "hint"),
            entry("gpt-kept", "detected", Some("cheap"), false, "best-fit"),
            entry("gpt-old-1", "detected", Some("cheap"), false, "best-fit"),
            entry("gpt-old-2", "seed", Some("cheap"), false, "hint"),
            entry("gpt-old-3", "detected", Some("cheap"), false, "best-fit"),
            owner_removed("gpt-gone"),
        ],
    );
    let listing = listing_of(
        Provider::OpenAi,
        &[
            ("gpt-6-astra", None),
            ("gpt-kept", None),
            ("gpt-new", None),
            ("gpt-gone", None),
        ],
    );
    let diff = diff(Provider::OpenAi, &listing, &ASTRA, &[], Some(&doc));
    assert_eq!(diff.report.count(IdChange::New), 1);
    assert_eq!(diff.report.count(IdChange::Unchanged), 2);
    assert_eq!(diff.report.count(IdChange::Removed), 3);
}

#[test]
fn added_and_removed_are_in_id_order() {
    let doc = document(
        "openai",
        vec![
            entry("gpt-z-old", "detected", Some("cheap"), false, "best-fit"),
            entry("gpt-a-old", "detected", Some("cheap"), false, "best-fit"),
        ],
    );
    let listing = listing_of(Provider::OpenAi, &[("gpt-z", None), ("gpt-a", None)]);
    let diff = diff(Provider::OpenAi, &listing, &[], &[], Some(&doc));
    let added: Vec<&str> = diff.added.iter().map(|added| added.id.as_str()).collect();
    assert_eq!(added, ["gpt-a", "gpt-z"]);
    assert_eq!(diff.removed, ["gpt-a-old", "gpt-z-old"]);
}

#[test]
fn a_listing_records_detected_with_the_compiled_hint_and_given_catalog_versions() {
    // The compiled rows tag `gpt-6-astra`; nothing places the embedding id.
    let doc = document(
        "openai",
        vec![
            entry("gpt-6-astra", "seed", Some("flagship"), true, "hint"),
            entry("gpt-6-luna", "seed", Some("cheap"), true, "hint"),
        ],
    );
    let listing = listing_of(
        Provider::OpenAi,
        &[
            ("gpt-6-astra", Some(1)),
            ("text-embedding-3-large", Some(2)),
        ],
    );
    let chosen = choose_event(Provider::OpenAi, Ok(listing), Some(&doc), 7);
    assert_eq!(chosen.type_name, MODELS_DETECTED);
    assert_eq!(chosen.type_version, MODELS_DETECTED_VERSION);
    assert_eq!(
        chosen.payload,
        json!({
            "provider": "openai",
            "added": [
                {"id": "gpt-6-astra", "tier": "flagship", "high_effort": true, "placed": "hint"},
                {
                    "id": "text-embedding-3-large", "tier": "balanced",
                    "high_effort": false, "placed": "best-fit"
                },
            ],
            "removed": ["gpt-6-luna"],
            "catalog_version": 7,
            "hint_version": HINT_VERSION,
        })
    );
    let Outcome::Detected(report) = chosen.outcome else {
        panic!("a listing is reported as detected");
    };
    assert_eq!(report.count(IdChange::New), 1);
}

#[test]
fn an_incomplete_listing_records_models_detection_failed_with_no_removed() {
    let doc = document(
        "deepseek",
        vec![entry(
            "deepseek-v9-pro",
            "detected",
            Some("flagship"),
            true,
            "best-fit",
        )],
    );
    let chosen = choose_event(Provider::DeepSeek, Err(Category::Incomplete), Some(&doc), 4);
    assert_eq!(chosen.type_name, MODELS_DETECTION_FAILED);
    assert_eq!(chosen.type_version, MODELS_DETECTION_FAILED_VERSION);
    assert_eq!(
        chosen.payload,
        json!({"provider": "deepseek", "category": "incomplete", "catalog_version": 4})
    );
    assert_eq!(chosen.outcome, Outcome::Failed(Category::Incomplete));
}

#[test]
fn text_beside_the_ids_in_a_200_body_never_reaches_the_detected_payload() {
    let body = json!({"object": "list", "data": [
        {"id": "gpt-6-astra", "object": "model", "created": 1, "owned_by": SENTINEL},
        {"id": "gpt-6-luna", "object": "model", "created": 2, "owned_by": SENTINEL},
    ]})
    .to_string();
    let observation = observed(response(200, body.as_bytes()));
    let classified = classify(Provider::OpenAi, &observation);
    let chosen = choose_event(Provider::OpenAi, classified, None, 0);
    assert_eq!(chosen.type_name, MODELS_DETECTED);
    assert!(!chosen.payload.to_string().contains(SENTINEL));
}

#[test]
fn a_401_body_echoing_the_key_never_reaches_the_failure_payload() {
    let body = format!(r#"{{"error":{{"message":"Incorrect API key provided: {SENTINEL}"}}}}"#);
    let observation = observed(response(401, body.as_bytes()));
    let classified = classify(Provider::DeepSeek, &observation);
    let chosen = choose_event(Provider::DeepSeek, classified, None, 0);
    assert_eq!(chosen.type_name, MODELS_DETECTION_FAILED);
    assert!(!chosen.payload.to_string().contains(SENTINEL));
}
