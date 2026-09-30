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
