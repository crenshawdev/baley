//! The catalog's decisions on supplied values. Expected values come from
//! design 0003 (CFG-R19 to CFG-R23) and the phase's decisions, never from
//! running this code.

use std::collections::{BTreeMap, BTreeSet};

use baley_store::{
    Actor, Change, DocKey, Event, EventSchema, Hash, KeyValue, ProjectId, Projector, RequestId,
};
use serde_json::{Value, json};

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
fn openai_and_deepseek_have_exact_hint_rows_and_gemini_has_none() {
    let has_rows = |provider| EXACT_HINTS.iter().any(|row| row.provider == provider);
    assert!(has_rows(Provider::OpenAi));
    assert!(has_rows(Provider::DeepSeek));
    assert!(!has_rows(Provider::Gemini));
    assert!(
        !PREFIX_HINTS
            .iter()
            .any(|row| row.provider == Provider::Gemini)
    );
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

// A `models.*` event on the user project, as the store hands it to a
// projector.
fn event(seq: u64, type_name: &str, recorded_at: &str, payload: Value) -> Event {
    Event {
        project_id: ProjectId("user".into()),
        seq,
        stream: "models".into(),
        stream_version: seq,
        type_name: type_name.into(),
        type_version: 1,
        actor: Actor::Baley,
        recorded_at: recorded_at.into(),
        request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
        git: None,
        policy_version: 0,
        payload,
        prev_hash: None,
        hash: Hash([0; 32]),
    }
}

fn key(text: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(text.into())])
}

// Hands the projector exactly the documents its keys name, and keeps what it
// puts, as the store does between events.
fn project(documents: &mut BTreeMap<DocKey, Value>, event: &Event) -> Vec<Change> {
    let projector = ModelCatalogProjector::new();
    let given: Vec<(DocKey, Value)> = projector
        .keys(event)
        .into_iter()
        .filter_map(|key| documents.get(&key).map(|body| (key, body.clone())))
        .collect();
    let changes = projector.apply(event, &given).expect("the event applies");
    for change in &changes {
        match change {
            Change::Put { key, body } => documents.insert(key.clone(), body.clone()),
            Change::Delete { key } => documents.remove(key),
        };
    }
    changes
}

fn seeded(seq: u64, at: &str, hint_version: u64, rows: Value) -> Event {
    let payload = json!({"hint_version": hint_version, "catalog_version": 0, "rows": rows});
    event(seq, "models.seeded", at, payload)
}

fn owner(seq: u64, catalog: &str, name: &str, change: &str, tier: Option<&str>) -> Event {
    let mut payload =
        json!({"catalog": catalog, "name": name, "change": change, "catalog_version": 0});
    if let Some(tier) = tier {
        payload["tier"] = tier.into();
    }
    event(seq, "models.owner_changed", "2026-09-29T12:00:00Z", payload)
}

fn state(catalog_version: u64, hint_version: Option<u64>) -> Value {
    let mut body = json!({"catalog": "state", "catalog_version": catalog_version});
    if let Some(hint) = hint_version {
        body["hint_version"] = hint.into();
    }
    body
}

fn doc(catalog: &str, entries: Value) -> Value {
    json!({"catalog": catalog, "entries": entries})
}

// An entry as a document stores it. `extra` adds or overrides fields.
fn stored(
    id: &str,
    source: &str,
    tier: &str,
    high_effort: bool,
    placed: &str,
    extra: Value,
) -> Value {
    let mut entry = json!({
        "id": id, "source": source, "tier": tier, "high_effort": high_effort,
        "placed": placed, "first_seen": "2026-09-01T00:00:00Z", "accepted_seq": 1,
        "owner_removed": false,
    });
    for (field, value) in extra.as_object().unwrap() {
        entry[field] = value.clone();
    }
    entry
}

fn documents(pairs: &[(&str, Value)]) -> BTreeMap<DocKey, Value> {
    pairs
        .iter()
        .map(|(name, body)| (key(name), body.clone()))
        .collect()
}

#[test]
fn a_first_seed_records_every_row_and_moves_both_versions() {
    let mut docs = BTreeMap::new();
    let rows = json!([
        {"provider": "openai", "name": "oa-1", "tier": "flagship", "high_effort": true},
        {"provider": "gemini", "name": "gm-1", "tier": "cheap", "high_effort": false},
    ]);
    project(&mut docs, &seeded(1, "2026-09-29T10:00:00Z", 1, rows));
    let expected = documents(&[
        ("state", state(1, Some(1))),
        (
            "openai",
            doc(
                "openai",
                json!([{
                    "id": "oa-1", "source": "seed", "tier": "flagship", "high_effort": true,
                    "placed": "hint", "first_seen": "2026-09-29T10:00:00Z", "accepted_seq": 1,
                    "owner_removed": false,
                }]),
            ),
        ),
        (
            "gemini",
            doc(
                "gemini",
                json!([{
                    "id": "gm-1", "source": "seed", "tier": "cheap", "high_effort": false,
                    "placed": "hint", "first_seen": "2026-09-29T10:00:00Z", "accepted_seq": 1,
                    "owner_removed": false,
                }]),
            ),
        ),
    ]);
    assert_eq!(docs, expected);
}

#[test]
fn a_seed_that_only_retiers_an_id_leaves_the_catalog_version() {
    let mut docs = BTreeMap::new();
    let first =
        json!([{"provider": "openai", "name": "oa-1", "tier": "flagship", "high_effort": true}]);
    project(&mut docs, &seeded(1, "2026-09-29T10:00:00Z", 1, first));
    let second =
        json!([{"provider": "openai", "name": "oa-1", "tier": "cheap", "high_effort": true}]);
    project(&mut docs, &seeded(2, "2026-09-30T10:00:00Z", 2, second));
    assert_eq!(docs[&key("state")], state(1, Some(2)));
    let entry = &docs[&key("openai")]["entries"][0];
    assert_eq!(entry["tier"], "cheap");
    assert_eq!(entry["first_seen"], "2026-09-29T10:00:00Z");
}

#[test]
fn a_seed_drops_a_withdrawn_seed_id_but_never_a_detected_one() {
    let detected = stored(
        "oa-det",
        "detected",
        "balanced",
        false,
        "best-fit",
        json!({}),
    );
    let mut docs = documents(&[
        ("state", state(5, Some(1))),
        (
            "openai",
            doc(
                "openai",
                json!([
                    detected,
                    stored("oa-old", "seed", "cheap", false, "hint", json!({}))
                ]),
            ),
        ),
    ]);
    let rows =
        json!([{"provider": "gemini", "name": "gm-1", "tier": "cheap", "high_effort": false}]);
    project(&mut docs, &seeded(7, "2026-09-29T10:00:00Z", 2, rows));
    assert_eq!(docs[&key("openai")], doc("openai", json!([detected])));
    assert_eq!(docs[&key("state")], state(7, Some(2)));
}

#[test]
fn a_seed_neither_retiers_an_owner_entry_nor_brings_back_an_owner_removal() {
    let owner_entry = stored("oa-own", "owner", "cheap", false, "owner", json!({}));
    let removed = stored(
        "oa-gone",
        "seed",
        "cheap",
        false,
        "hint",
        json!({"owner_removed": true}),
    );
    let openai = doc("openai", json!([removed, owner_entry]));
    let mut docs = documents(&[("state", state(3, Some(1))), ("openai", openai.clone())]);
    let rows = json!([
        {"provider": "openai", "name": "oa-own", "tier": "flagship", "high_effort": true},
        {"provider": "openai", "name": "oa-gone", "tier": "balanced", "high_effort": true},
    ]);
    project(&mut docs, &seeded(4, "2026-09-29T10:00:00Z", 2, rows));
    assert_eq!(docs[&key("openai")], openai);
    assert_eq!(docs[&key("state")], state(3, Some(2)));
}

#[test]
fn an_owner_addition_takes_its_tier_or_none_and_moves_the_version() {
    let mut docs = BTreeMap::new();
    project(
        &mut docs,
        &owner(3, "codex", "o-cheap", "added", Some("cheap")),
    );
    assert_eq!(docs[&key("state")], state(3, None));
    project(&mut docs, &owner(4, "codex", "o-plain", "added", None));
    assert_eq!(docs[&key("state")], state(4, None));
    let expected = doc(
        "codex",
        json!([
            {
                "id": "o-cheap", "source": "owner", "tier": "cheap", "high_effort": false,
                "placed": "owner", "first_seen": "2026-09-29T12:00:00Z", "accepted_seq": 3,
                "owner_removed": false,
            },
            {
                "id": "o-plain", "source": "owner", "high_effort": false,
                "placed": "owner", "first_seen": "2026-09-29T12:00:00Z", "accepted_seq": 4,
                "owner_removed": false,
            },
        ]),
    );
    assert_eq!(docs[&key("codex")], expected);
}

#[test]
fn an_owner_tier_over_a_detected_id_keeps_its_high_effort_flag() {
    let detected = stored("oa-det", "detected", "cheap", true, "best-fit", json!({}));
    let mut docs = documents(&[
        ("state", state(2, None)),
        ("openai", doc("openai", json!([detected]))),
    ]);
    project(
        &mut docs,
        &owner(5, "openai", "oa-det", "added", Some("flagship")),
    );
    let expected = stored("oa-det", "owner", "flagship", true, "owner", json!({}));
    assert_eq!(docs[&key("openai")], doc("openai", json!([expected])));
}

#[test]
fn an_owner_addition_of_an_already_accepted_id_leaves_the_version() {
    let seed = stored("oa-1", "seed", "cheap", false, "hint", json!({}));
    let mut docs = documents(&[
        ("state", state(2, Some(1))),
        ("openai", doc("openai", json!([seed]))),
    ]);
    project(&mut docs, &owner(6, "openai", "oa-1", "added", None));
    assert_eq!(docs[&key("state")], state(2, Some(1)));
    assert_eq!(docs[&key("openai")]["entries"][0]["source"], "owner");
}

#[test]
fn an_owner_removal_hides_a_seed_id_and_moves_the_version() {
    let seed = stored("oa-1", "seed", "cheap", false, "hint", json!({}));
    let mut docs = documents(&[
        ("state", state(2, Some(1))),
        ("openai", doc("openai", json!([seed]))),
    ]);
    project(&mut docs, &owner(6, "openai", "oa-1", "removed", None));
    let hidden = stored(
        "oa-1",
        "seed",
        "cheap",
        false,
        "hint",
        json!({"owner_removed": true}),
    );
    assert_eq!(docs[&key("openai")], doc("openai", json!([hidden])));
    assert_eq!(docs[&key("state")], state(6, Some(1)));
}

#[test]
fn keys_name_the_state_and_every_document_the_event_changes() {
    let projector = ModelCatalogProjector::new();
    let seed = seeded(1, "2026-09-29T10:00:00Z", 1, json!([]));
    assert_eq!(
        projector.keys(&seed),
        vec![key("state"), key("openai"), key("gemini"), key("deepseek")]
    );
    let change = owner(2, "claude-code", "claude-x", "added", None);
    assert_eq!(
        projector.keys(&change),
        vec![key("state"), key("claude-code")]
    );
}

#[test]
fn an_owner_change_without_a_name_is_refused_not_applied() {
    let projector = ModelCatalogProjector::new();
    let payload = json!({"catalog": "openai", "change": "added", "catalog_version": 0});
    let broken = event(2, "models.owner_changed", "2026-09-29T12:00:00Z", payload);
    let error = projector.apply(&broken, &[]).unwrap_err();
    assert!(error.0.contains("name"), "{}", error.0);
}

fn detected(seq: u64, at: &str, provider: &str, added: Value, removed: Value) -> Event {
    let payload = json!({
        "provider": provider, "added": added, "removed": removed,
        "catalog_version": 0, "hint_version": 1,
    });
    event(seq, "models.detected", at, payload)
}

#[test]
fn a_detection_removing_an_owner_entry_leaves_it_accepted_with_its_tier() {
    let owner_entry = stored("oa-own", "owner", "cheap", false, "owner", json!({}));
    let openai = doc("openai", json!([owner_entry]));
    let mut docs = documents(&[("state", state(2, None)), ("openai", openai.clone())]);
    project(
        &mut docs,
        &detected(
            3,
            "2026-09-29T13:00:00Z",
            "openai",
            json!([]),
            json!(["oa-own"]),
        ),
    );
    assert_eq!(docs[&key("openai")], openai);
    assert_eq!(docs[&key("state")], state(2, None));
}

#[test]
fn a_detection_never_brings_back_an_id_the_owner_removed() {
    let mut docs = BTreeMap::new();
    let found =
        json!([{"id": "oa-1", "tier": "balanced", "high_effort": false, "placed": "best-fit"}]);
    project(
        &mut docs,
        &detected(
            1,
            "2026-09-29T13:00:00Z",
            "openai",
            found.clone(),
            json!([]),
        ),
    );
    project(&mut docs, &owner(2, "openai", "oa-1", "removed", None));
    project(
        &mut docs,
        &detected(3, "2026-09-30T13:00:00Z", "openai", found, json!([])),
    );
    let entry = &docs[&key("openai")]["entries"][0];
    assert_eq!(entry["owner_removed"], true);
    assert_eq!(docs[&key("state")], state(2, None));
}

#[test]
fn a_failed_detection_names_no_document_and_leaves_the_version() {
    let mut docs = BTreeMap::new();
    let found = json!([{"id": "gm-1", "tier": "cheap", "high_effort": true, "placed": "prefix"}]);
    project(
        &mut docs,
        &detected(3, "2026-09-29T13:00:00Z", "gemini", found, json!([])),
    );
    let payload = json!({"provider": "gemini", "category": "auth", "catalog_version": 3});
    let failed = event(
        4,
        "models.detection_failed",
        "2026-09-29T14:00:00Z",
        payload,
    );
    assert_eq!(
        ModelCatalogProjector::new().keys(&failed),
        Vec::<DocKey>::new()
    );
    assert_eq!(project(&mut docs, &failed), Vec::<Change>::new());
    assert_eq!(docs[&key("state")], state(3, None));
}

#[test]
fn an_empty_detection_verifies_every_entry_of_its_provider_and_leaves_the_version() {
    let seed = stored("oa-seed", "seed", "cheap", false, "hint", json!({}));
    let owner_entry = stored("oa-own", "owner", "flagship", true, "owner", json!({}));
    let mut docs = documents(&[
        ("state", state(4, Some(1))),
        ("openai", doc("openai", json!([owner_entry, seed]))),
    ]);
    let refresh = detected(5, "2026-09-29T15:00:00Z", "openai", json!([]), json!([]));
    assert_eq!(
        ModelCatalogProjector::new().keys(&refresh),
        vec![key("state"), key("openai")]
    );
    project(&mut docs, &refresh);
    let seen = json!({"last_verified": "2026-09-29T15:00:00Z"});
    let expected = doc(
        "openai",
        json!([
            stored("oa-own", "owner", "flagship", true, "owner", seen.clone()),
            stored("oa-seed", "seed", "cheap", false, "hint", seen),
        ]),
    );
    assert_eq!(docs[&key("openai")], expected);
    assert_eq!(docs[&key("state")], state(4, Some(1)));
}

#[test]
fn a_detection_removing_a_detected_id_drops_it_and_moves_the_version() {
    let gone = stored("oa-old", "detected", "cheap", false, "best-fit", json!({}));
    let mut docs = documents(&[
        ("state", state(2, None)),
        ("openai", doc("openai", json!([gone]))),
    ]);
    project(
        &mut docs,
        &detected(
            6,
            "2026-09-29T16:00:00Z",
            "openai",
            json!([]),
            json!(["oa-old"]),
        ),
    );
    assert_eq!(docs[&key("openai")], doc("openai", json!([])));
    assert_eq!(docs[&key("state")], state(6, None));
}

#[test]
fn a_detected_id_keeps_the_events_tier_and_flag_not_the_compiled_tables() {
    let row = EXACT_HINTS
        .first()
        .expect("the hint table has an exact row");
    let other = Tier::ALL
        .into_iter()
        .find(|tier| *tier != row.tier)
        .unwrap();
    let found = json!([{
        "id": row.id, "tier": other.name(), "high_effort": !row.high_effort, "placed": "best-fit",
    }]);
    let mut docs = BTreeMap::new();
    let provider = row.provider.name();
    project(
        &mut docs,
        &detected(1, "2026-09-29T17:00:00Z", provider, found, json!([])),
    );
    let entry = &docs[&key(provider)]["entries"][0];
    assert_eq!(entry["tier"], other.name());
    assert_eq!(entry["high_effort"], !row.high_effort);
}

#[test]
fn a_detection_sets_an_owner_entrys_high_effort_flag_but_not_its_tier() {
    let mut docs = BTreeMap::new();
    project(
        &mut docs,
        &owner(1, "openai", "oa-own", "added", Some("cheap")),
    );
    let found =
        json!([{"id": "oa-own", "tier": "flagship", "high_effort": true, "placed": "best-fit"}]);
    project(
        &mut docs,
        &detected(2, "2026-09-29T18:00:00Z", "openai", found, json!([])),
    );
    let entry = &docs[&key("openai")]["entries"][0];
    assert_eq!(entry["high_effort"], true);
    assert_eq!(entry["tier"], "cheap");
    assert_eq!(entry["placed"], "owner");
    assert_eq!(entry["source"], "owner");
}

#[test]
fn opus_is_a_claude_code_name_and_no_codex_name_with_no_documents() {
    let claude = accepted_names("claude-code", None, None).unwrap();
    let codex = accepted_names("codex", None, None).unwrap();
    assert!(claude.names.contains("opus"));
    assert!(!codex.names.contains("opus"));
    assert_eq!((claude.version, codex.version), (0, 0));
}

#[test]
fn the_lookup_refuses_anthropic_and_a_case_variant_as_unknown_providers() {
    for name in ["anthropic", "Claude-Code"] {
        let refusal = accepted_names(name, None, None).unwrap_err();
        assert_eq!(refusal.code(), "unknown-provider", "{name}");
    }
}

#[test]
fn the_lookup_returns_the_state_documents_catalog_version() {
    let found = accepted_names("openai", None, Some(&state(7, Some(1)))).unwrap();
    assert_eq!(found.version, 7);
}

#[test]
fn a_provider_accepts_its_seed_detected_and_owner_ids_but_not_a_removed_one() {
    let openai = doc(
        "openai",
        json!([
            stored("oa-seed", "seed", "cheap", false, "hint", json!({})),
            stored("oa-det", "detected", "balanced", true, "prefix", json!({})),
            stored("oa-own", "owner", "flagship", false, "owner", json!({})),
            stored(
                "oa-gone",
                "detected",
                "cheap",
                false,
                "hint",
                json!({"owner_removed": true})
            ),
        ]),
    );
    let found = accepted_names("openai", Some(&openai), None).unwrap();
    let expected: BTreeSet<String> = ["oa-seed", "oa-det", "oa-own"].map(String::from).into();
    assert_eq!(found.names, expected);
}

#[test]
fn a_hosts_owner_entries_are_accepted_beside_its_aliases() {
    let codex = doc(
        "codex",
        json!([{
            "id": "o-own", "source": "owner", "high_effort": false, "placed": "owner",
            "first_seen": "2026-09-29T12:00:00Z", "accepted_seq": 3, "owner_removed": false,
        }]),
    );
    let found = accepted_names("codex", Some(&codex), None).unwrap();
    assert_eq!(found.names, BTreeSet::from(["o-own".to_owned()]));
}

#[test]
fn the_listing_shows_aliases_untiered_detected_placement_and_no_removed_id() {
    let openai = doc(
        "openai",
        json!([
            stored(
                "oa-gone",
                "seed",
                "cheap",
                false,
                "hint",
                json!({"owner_removed": true})
            ),
            stored(
                "oa-det",
                "detected",
                "balanced",
                false,
                "best-fit",
                json!({})
            ),
        ]),
    );
    let claude = Catalog::Host(Host::ClaudeCode);
    let open_ai = Catalog::Provider(Provider::OpenAi);
    let shown = listing(
        &[(open_ai, Some(&openai)), (claude, None)],
        Some(&state(9, Some(1))),
    );
    let alias = |name: &str| ListingRow {
        catalog: claude,
        name: name.into(),
        source: Source::Alias,
        tier: None,
        placed: None,
    };
    let expected = Listing {
        version: 9,
        rows: vec![
            alias("fable"),
            alias("haiku"),
            alias("opus"),
            alias("sonnet"),
            ListingRow {
                catalog: open_ai,
                name: "oa-det".into(),
                source: Source::Detected,
                tier: Some(Tier::Balanced),
                placed: Some(Placement::BestFit),
            },
        ],
    };
    assert_eq!(shown, expected);
}

#[test]
fn a_recorded_hint_version_equal_to_the_compiled_one_is_not_seeded_again() {
    assert!(!seed_due(4, Some(4)));
}

#[test]
fn a_higher_lower_or_missing_recorded_hint_version_is_seeded() {
    assert!(seed_due(4, Some(5)));
    assert!(seed_due(4, Some(3)));
    assert!(seed_due(4, None));
}

#[test]
fn the_seed_payload_carries_every_exact_row_once_and_no_prefix_row() {
    let payload = seed_payload(11);
    assert_eq!(payload["hint_version"], HINT_VERSION);
    assert_eq!(payload["catalog_version"], 11);
    let rows = payload["rows"].as_array().unwrap();
    assert_eq!(rows.len(), EXACT_HINTS.len());
    for row in EXACT_HINTS {
        let expected = json!({
            "provider": row.provider.name(), "name": row.id,
            "tier": row.tier.name(), "high_effort": row.high_effort,
        });
        let times = rows.iter().filter(|seeded| **seeded == expected).count();
        assert_eq!(times, 1, "{row:?}");
    }
}

#[test]
fn removing_a_claude_code_alias_is_refused_before_it_reaches_the_ledger() {
    let refusal = judge_alias_removal(Catalog::Host(Host::ClaudeCode), "opus").unwrap_err();
    assert_eq!(refusal.code(), "alias-not-removable");
    assert_eq!(
        refusal.to_string(),
        "alias-not-removable: \"opus\" is a claude-code alias; \
         the binary owns its aliases and the host resolves them"
    );
}

#[test]
fn adding_a_claude_code_alias_is_refused_before_it_reaches_the_ledger() {
    let refusal = judge_alias_addition(Catalog::Host(Host::ClaudeCode), "sonnet").unwrap_err();
    assert_eq!(refusal.code(), "alias-not-addable");
    assert_eq!(
        refusal.to_string(),
        "alias-not-addable: \"sonnet\" is a claude-code alias; \
         the binary owns its aliases and the host resolves them"
    );
    assert_eq!(
        judge_alias_addition(Catalog::Host(Host::Codex), "sonnet"),
        Ok(())
    );
    assert_eq!(
        judge_alias_addition(Catalog::Provider(Provider::OpenAi), "sonnet"),
        Ok(())
    );
}

#[test]
fn removing_opus_from_codex_is_no_alias_removal_but_a_name_codex_does_not_hold() {
    let codex = Catalog::Host(Host::Codex);
    assert_eq!(judge_alias_removal(codex, "opus"), Ok(()));
    let refusal = judge_held_removal(codex, "opus", None).unwrap_err();
    assert_eq!(refusal.code(), "unknown-model");
}

#[test]
fn removing_a_seeded_provider_id_is_allowed() {
    let openai = doc(
        "openai",
        json!([stored("oa-seed", "seed", "cheap", false, "hint", json!({}))]),
    );
    let catalog = Catalog::Provider(Provider::OpenAi);
    assert_eq!(
        judge_held_removal(catalog, "oa-seed", Some(&openai)),
        Ok(())
    );
}

#[test]
fn removing_an_id_the_owner_already_removed_is_refused() {
    let hidden = json!({"owner_removed": true});
    let openai = doc(
        "openai",
        json!([stored("oa-gone", "seed", "cheap", false, "hint", hidden)]),
    );
    let catalog = Catalog::Provider(Provider::OpenAi);
    let refusal = judge_held_removal(catalog, "oa-gone", Some(&openai)).unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "unknown-model: the openai catalog does not hold \"oa-gone\""
    );
}

#[test]
fn removing_a_hosts_owner_entry_is_allowed() {
    let codex = doc(
        "codex",
        json!([{
            "id": "o-own", "source": "owner", "high_effort": false, "placed": "owner",
            "first_seen": "2026-09-29T12:00:00Z", "accepted_seq": 3, "owner_removed": false,
        }]),
    );
    let catalog = Catalog::Host(Host::Codex);
    assert_eq!(judge_alias_removal(catalog, "o-own"), Ok(()));
    assert_eq!(judge_held_removal(catalog, "o-own", Some(&codex)), Ok(()));
}
