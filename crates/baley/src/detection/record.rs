//! One provider's detection recorded as a command of its own on `user`, and
//! the entries a keyless provider can no longer verify (design 0003
//! section 6, CFG-R20, CFG-R21).

use baley_core::catalog::detection::{
    Category, IdChange, Observation, Outcome, choose_event, classify,
};
use baley_core::catalog::{
    Catalog, ListingRow, MODEL_CATALOG_VIEW, MODELS_STREAM, Provider, Source, USER_PROJECT,
    catalog_key, listing, read_state, state_key,
};
use baley_store::{
    Admin, Command, CommandKind, Decision, Ledger, NewEvent, Observed, OutcomeKind, ProjectId,
    Refusal, RequestId, StoreError, StreamName, Views, request_digest,
};
use serde_json::{Value, json};

use super::trigger::Trigger;

fn user() -> ProjectId {
    ProjectId(USER_PROJECT.into())
}

/// The request-digest input of one provider's record. It names the
/// provider and never a key or anything a provider sent.
pub fn detection_request(provider: Provider, trigger: &Trigger) -> Value {
    json!({
        "kind": trigger.kind(),
        "project": USER_PROJECT,
        "actor": trigger.actor().as_str(),
        "policy_version": 0,
        "provider": provider.name(),
        "scope": [],
    })
}

/// What one provider's record came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// The report of a listing, or the category of a failure.
    pub outcome: Outcome,
    /// The catalog version once the command committed.
    pub version: u64,
}

/// Records `provider`'s outcome: what the lister saw, or the category the
/// judge gave without listing. The diff runs inside the transaction on the
/// documents read there, so an owner change committed since the listing is
/// never overwritten. It assumes `user` exists: the run seeds first.
pub fn record(
    store: &(impl Ledger + Views),
    provider: Provider,
    trigger: &Trigger,
    seen: Result<&Observation, Category>,
    request_id: RequestId,
    at: &str,
) -> Result<Recording, StoreError> {
    let classified = seen.and_then(|observation| classify(provider, observation));
    let digest = request_digest(&detection_request(provider, trigger))
        .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    let command = Command {
        project: user(),
        kind: CommandKind(trigger.kind().into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor: trigger.actor(),
    };
    let mut outcome = None;
    store.transact(&command, &mut |tx| {
        let state = tx.get(MODEL_CATALOG_VIEW, &state_key())?;
        let version = read_state(state.as_ref().map(|document| &document.body)).catalog_version;
        let document = tx.get(
            MODEL_CATALOG_VIEW,
            &catalog_key(Catalog::Provider(provider)),
        )?;
        let chosen = choose_event(
            provider,
            classified.clone(),
            document.as_ref().map(|document| &document.body),
            version,
        );
        tx.append(NewEvent {
            stream: StreamName(MODELS_STREAM.into()),
            type_name: chosen.type_name.into(),
            type_version: chosen.type_version,
            git: None,
            payload: chosen.payload,
            attachments: vec![],
        })?;
        let answer = answer(chosen.type_name, &chosen.outcome);
        outcome = Some(chosen.outcome);
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    // A fresh request id is never answered before, so the decision ran.
    let outcome = outcome.ok_or_else(|| {
        StoreError::Unavailable("a fresh detection request was answered before".into())
    })?;
    let state = store.get(&user(), MODEL_CATALOG_VIEW, &state_key())?;
    let version = read_state(state.as_ref().map(|document| &document.body)).catalog_version;
    Ok(Recording { outcome, version })
}

// Small on purpose: counts or a category name, never an id or a body.
fn answer(event: &str, outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Detected(report) => json!({
            "event": event,
            "added": report.count(IdChange::New),
            "removed": report.count(IdChange::Removed),
            "unchanged": report.count(IdChange::Unchanged),
        }),
        Outcome::Failed(category) => json!({
            "event": event,
            "category": category.name(),
        }),
    }
}

/// `provider`'s accepted detected entries, which a run without its key
/// cannot verify. An absent `user` holds none and is not created. Nothing
/// is recorded, so each entry keeps its last-verified time.
pub fn unverifiable(
    store: &(impl Admin + Views),
    provider: Provider,
) -> Result<Vec<ListingRow>, StoreError> {
    if !store.projects()?.iter().any(|(known, _)| *known == user()) {
        return Ok(vec![]);
    }
    let catalog = Catalog::Provider(provider);
    let document = store.get(&user(), MODEL_CATALOG_VIEW, &catalog_key(catalog))?;
    let rows = listing(
        &[(catalog, document.as_ref().map(|document| &document.body))],
        None,
    )
    .rows;
    Ok(rows
        .into_iter()
        .filter(|row| row.source == Source::Detected)
        .collect())
}

#[cfg(test)]
mod tests {
    use baley_core::catalog::detection::ObservedResponse;
    use baley_core::catalog::{HINT_VERSION, OwnerChange, Placement, Tier, accepted_names};
    use baley_store::{Actor, Event, PageRequest};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::models::{change, seed};

    const T0: &str = "2026-09-30T10:00:00Z";
    const SENTINEL: &str = "sk-SENTINEL-2b8e";
    const OPENAI: Catalog = Catalog::Provider(Provider::OpenAi);

    fn at(n: u8) -> String {
        format!("2026-09-30T10:00:{n:02}Z")
    }

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    /// A real store in a private home under a fresh temporary directory.
    fn open() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        (dir, store)
    }

    /// A fresh store with the compiled hint table seeded.
    fn seeded() -> (tempfile::TempDir, SqliteStore) {
        let (dir, store) = open();
        assert!(seed(&store, request(1), &at(1)).unwrap());
        (dir, store)
    }

    fn answered(status: u16, body: &str) -> Observation {
        Observation {
            responses: vec![ObservedResponse {
                status,
                body: body.as_bytes().to_vec(),
                cut_short: false,
            }],
            ..Observation::default()
        }
    }

    /// An OpenAI 200 page listing `ids`.
    fn openai_page(ids: &[&str]) -> Observation {
        let data: Vec<Value> = ids
            .iter()
            .map(|id| json!({"id": id, "object": "model", "created": 1_780_000_000, "owned_by": "openai"}))
            .collect();
        answered(200, &json!({"object": "list", "data": data}).to_string())
    }

    fn events(store: &SqliteStore, stream: &str) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(&user(), &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
    }

    fn state_version(store: &SqliteStore) -> u64 {
        let state = store
            .get(&user(), MODEL_CATALOG_VIEW, &state_key())
            .unwrap();
        read_state(state.as_ref().map(|document| &document.body)).catalog_version
    }

    fn accepted(store: &SqliteStore) -> Vec<String> {
        let document = store
            .get(&user(), MODEL_CATALOG_VIEW, &catalog_key(OPENAI))
            .unwrap();
        let names = accepted_names(
            "openai",
            document.as_ref().map(|document| &document.body),
            None,
        );
        names.unwrap().names.into_iter().collect()
    }

    fn provider_of(event: &Event) -> &'static str {
        match event.payload["provider"].as_str() {
            Some("openai") => "openai",
            Some("deepseek") => "deepseek",
            other => panic!("unexpected provider {other:?}"),
        }
    }

    /// Appends one hand-written `models.*` event in a command of its own.
    fn append(store: &SqliteStore, n: u8, type_name: &str, payload: Value) {
        let command = Command {
            project: user(),
            kind: CommandKind("test.append".into()),
            request_id: request(n),
            digest: request_digest(&json!({ "request": n })).unwrap(),
            scope: vec![],
            policy_version: 0,
            recorded_at: at(n),
            actor: Actor::Owner,
        };
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(MODELS_STREAM.into()),
                    type_name: type_name.into(),
                    type_version: 1,
                    git: None,
                    payload: payload.clone(),
                    attachments: vec![],
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!(null),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }

    fn added_ids(event: &Event) -> Vec<&str> {
        event.payload["added"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_listing_records_detected_at_policy_zero_with_the_prior_catalog_and_compiled_hint_versions()
    {
        for (trigger, kind, actor) in [
            (Trigger::Owner(vec![]), "models.update", Actor::Owner),
            (Trigger::Automatic, "models.detect", Actor::Baley),
        ] {
            let (_dir, store) = seeded();
            let before = state_version(&store);
            assert!(before > 0, "seeding moved the version");
            let page = openai_page(&["gpt-6-astra", "gpt-6-luna-2026-09-01", "whisper-9"]);

            record(
                &store,
                Provider::OpenAi,
                &trigger,
                Ok(&page),
                request(2),
                &at(2),
            )
            .unwrap();

            let recorded = events(&store, "models");
            assert_eq!(recorded.len(), 2, "the seed and one detection");
            let event = &recorded[1];
            assert_eq!(event.type_name, "models.detected");
            assert_eq!(event.policy_version, 0);
            assert_eq!(event.actor, actor);
            assert_eq!(event.request_id, request(2));
            assert_eq!(event.payload["provider"], "openai");
            assert_eq!(event.payload["catalog_version"], before);
            assert_eq!(event.payload["hint_version"], HINT_VERSION);
            let completed = events(&store, &format!("command/{kind}"));
            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].payload["kind"], kind);
            assert_eq!(completed[0].actor, actor);
        }
    }

    #[test]
    fn two_providers_record_in_two_commands_each_with_its_own_request_and_event() {
        let (_dir, store) = seeded();
        let owner = Trigger::Owner(vec![]);
        let deepseek = answered(
            200,
            r#"{"object": "list", "data": [{"id": "deepseek-v4-pro", "object": "model"}]}"#,
        );

        let openai = openai_page(&["gpt-6-astra"]);
        record(
            &store,
            Provider::OpenAi,
            &owner,
            Ok(&openai),
            request(2),
            &at(2),
        )
        .unwrap();
        record(
            &store,
            Provider::DeepSeek,
            &owner,
            Ok(&deepseek),
            request(3),
            &at(3),
        )
        .unwrap();

        let completed = events(&store, "command/models.update");
        let requests: Vec<&str> = completed
            .iter()
            .map(|event| event.payload["request_id"].as_str().unwrap())
            .collect();
        assert_eq!(requests, vec![request(2).0.as_str(), request(3).0.as_str()]);
        let detected: Vec<(&str, RequestId)> = events(&store, "models")
            .into_iter()
            .filter(|event| event.type_name == "models.detected")
            .map(|event| (provider_of(&event), event.request_id))
            .collect();
        assert_eq!(
            detected,
            vec![("openai", request(2)), ("deepseek", request(3))]
        );
    }

    #[test]
    fn a_failure_of_any_category_after_a_detection_keeps_every_accepted_id_and_the_version() {
        let (_dir, store) = seeded();
        let page = openai_page(&["gpt-6-astra", "gpt-6-luna", "whisper-9"]);
        record(
            &store,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&page),
            request(2),
            &at(2),
        )
        .unwrap();
        let ids = accepted(&store);
        assert_eq!(ids, vec!["gpt-6-astra", "gpt-6-luna", "whisper-9"]);
        let version = state_version(&store);

        let categories = [
            Category::Offline,
            Category::Incomplete,
            Category::Malformed,
            Category::Unauthorized,
            Category::RateLimited,
            Category::Http(503),
            Category::KeysFileExposed,
            Category::KeysFileInvalid,
            Category::KeysFileUnreadable,
        ];
        for (n, category) in (3..).zip(categories) {
            let recording = record(
                &store,
                Provider::OpenAi,
                &Trigger::Automatic,
                Err(category),
                request(n),
                &at(n),
            )
            .unwrap();
            assert_eq!(recording.outcome, Outcome::Failed(category));
            assert_eq!(recording.version, version, "{}", category.name());
            assert_eq!(accepted(&store), ids, "{}", category.name());
        }
        let failed = events(&store, "models")
            .into_iter()
            .filter(|event| event.type_name == "models.detection_failed")
            .count();
        assert_eq!(failed, categories.len());
    }

    #[test]
    fn an_owner_removal_committed_after_listing_is_not_undone_by_the_recorded_diff() {
        let (_dir, store) = seeded();
        let page = openai_page(&["gpt-6-astra", "gpt-6-luna"]);
        change(
            &store,
            OPENAI,
            "gpt-6-luna",
            OwnerChange::Removed,
            request(2),
            &at(2),
        )
        .unwrap();

        record(
            &store,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&page),
            request(3),
            &at(3),
        )
        .unwrap();

        let recorded = events(&store, "models");
        let detected = recorded.last().unwrap();
        assert_eq!(detected.type_name, "models.detected");
        assert_eq!(added_ids(detected), vec!["gpt-6-astra"]);
        assert!(!accepted(&store).contains(&"gpt-6-luna".to_string()));
    }

    #[test]
    fn a_body_carrying_the_key_never_reaches_an_event_an_answer_or_the_digest_input() {
        let (_dir, store) = seeded();
        let trigger = Trigger::Owner(vec![]);
        let listed = answered(
            200,
            &json!({"object": "list", "data": [
                {"id": "gpt-6-astra", "object": SENTINEL, "created": 1, "owned_by": SENTINEL},
                {"id": "whisper-9", "object": "model", "owned_by": format!("Bearer {SENTINEL}")},
            ]})
            .to_string(),
        );
        let refused = answered(
            401,
            &json!({"error": {"message": format!("Incorrect API key provided: {SENTINEL}"), "code": "invalid_api_key"}})
                .to_string(),
        );

        let detected = record(
            &store,
            Provider::OpenAi,
            &trigger,
            Ok(&listed),
            request(2),
            &at(2),
        )
        .unwrap();
        let failed = record(
            &store,
            Provider::OpenAi,
            &trigger,
            Ok(&refused),
            request(3),
            &at(3),
        )
        .unwrap();

        assert!(matches!(detected.outcome, Outcome::Detected(_)));
        assert_eq!(failed.outcome, Outcome::Failed(Category::Unauthorized));
        let mut recorded = events(&store, "models");
        recorded.extend(events(&store, "command/models.update"));
        assert_eq!(
            recorded.len(),
            5,
            "seed, detected, failed and two completions"
        );
        for event in &recorded {
            assert!(
                !event.payload.to_string().contains(SENTINEL),
                "{}",
                event.type_name
            );
        }
        let answers: Vec<&Value> = recorded
            .iter()
            .filter(|event| event.type_name == "command.completed")
            .map(|event| &event.payload["answer"]["inline"])
            .collect();
        assert_eq!(
            answers,
            vec![
                &json!({"event": "models.detected", "added": 1, "removed": 2, "unchanged": 1}),
                &json!({"event": "models.detection_failed", "category": "unauthorized"}),
            ]
        );
        let digest = detection_request(Provider::OpenAi, &trigger).to_string();
        assert!(!digest.contains(SENTINEL));
    }

    #[test]
    fn the_unverifiable_read_neither_creates_user_nor_counts_seed_owner_or_removed_entries() {
        let (_dir, store) = open();
        assert_eq!(unverifiable(&store, Provider::OpenAi).unwrap(), vec![]);
        assert_eq!(store.projects().unwrap(), vec![]);

        assert!(seed(&store, request(1), &at(1)).unwrap());
        let page = openai_page(&["gpt-6-astra", "whisper-9", "dall-e-9"]);
        record(
            &store,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&page),
            request(2),
            &at(2),
        )
        .unwrap();
        change(
            &store,
            OPENAI,
            "dall-e-9",
            OwnerChange::Removed,
            request(3),
            &at(3),
        )
        .unwrap();
        let tier = Some(Tier::Cheap);
        change(
            &store,
            OPENAI,
            "gpt-mine",
            OwnerChange::Added(tier),
            request(4),
            &at(4),
        )
        .unwrap();
        let row = json!({"provider": "openai", "name": "gpt-seed-only", "tier": "cheap", "high_effort": false});
        let seeded = json!({"hint_version": HINT_VERSION, "catalog_version": 0, "rows": [row]});
        append(&store, 5, "models.seeded", seeded);
        let held = accepted(&store);
        assert_eq!(
            held,
            vec!["gpt-6-astra", "gpt-mine", "gpt-seed-only", "whisper-9"]
        );

        let rows = unverifiable(&store, Provider::OpenAi).unwrap();

        let names: Vec<(&str, Source, Option<Tier>, Option<Placement>)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.source, row.tier, row.placed))
            .collect();
        assert_eq!(
            names,
            vec![
                (
                    "gpt-6-astra",
                    Source::Detected,
                    Some(Tier::Flagship),
                    Some(Placement::Hint)
                ),
                (
                    "whisper-9",
                    Source::Detected,
                    Some(Tier::Balanced),
                    Some(Placement::BestFit)
                ),
            ]
        );
    }
}
