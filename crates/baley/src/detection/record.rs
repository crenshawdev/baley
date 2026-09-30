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
    /// The catalog version this command committed, read inside its own
    /// transaction.
    pub version: u64,
}

/// Records `provider`'s outcome: what the lister saw, or the category the
/// judge gave without listing. The diff runs inside the transaction on the
/// documents read there, so an owner change committed since the listing is
/// never overwritten. The version it returns is read inside the same
/// transaction, after the event, so a command another process commits
/// later is never paired with this report. It assumes `user` exists: the
/// run seeds first.
pub fn record(
    store: &impl Ledger,
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
    let mut recording = None;
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
        // Reads see this transaction's own projected writes, so this is the
        // version the command commits, whatever lands after it.
        let after = tx.get(MODEL_CATALOG_VIEW, &state_key())?;
        let answer = answer(chosen.type_name, &chosen.outcome);
        recording = Some(Recording {
            outcome: chosen.outcome,
            version: read_state(after.as_ref().map(|document| &document.body)).catalog_version,
        });
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    // A fresh request id is never answered before, so the decision ran.
    recording.ok_or_else(|| {
        StoreError::Unavailable("a fresh detection request was answered before".into())
    })
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
    use std::ops::RangeInclusive;

    use baley_store::{
        Actor, Anchor, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim, DecideReconcile,
        Event, Head, HistoryFilter, Page, PageRequest, ReconcileAuthority, Recorded, VerifyReport,
    };
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
            response: Some(ObservedResponse {
                status,
                body: body.as_bytes().to_vec(),
                cut_short: false,
            }),
            ..Observation::default()
        }
    }

    /// An OpenAI 200 body listing `ids`.
    fn openai_body(ids: &[&str]) -> Observation {
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
            let listed = openai_body(&["gpt-6-astra", "gpt-6-luna-2026-09-01", "whisper-9"]);

            record(
                &store,
                Provider::OpenAi,
                &trigger,
                Ok(&listed),
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

        let openai = openai_body(&["gpt-6-astra"]);
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
        let listed = openai_body(&["gpt-6-astra", "gpt-6-luna", "whisper-9"]);
        record(
            &store,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&listed),
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
        let listed = openai_body(&["gpt-6-astra", "gpt-6-luna"]);
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
            Ok(&listed),
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
        let listed = openai_body(&["gpt-6-astra", "whisper-9", "dall-e-9"]);
        record(
            &store,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&listed),
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

    /// The real store, with an owner removal committed right after each
    /// command commits and before the caller runs again: the window in
    /// which another process can commit.
    struct CommitsAfter<'a> {
        store: &'a SqliteStore,
        removes: &'a str,
    }

    impl Ledger for CommitsAfter<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            let recorded = Ledger::transact(self.store, command, decide)?;
            let removal = OwnerChange::Removed;
            change(
                self.store,
                OPENAI,
                self.removes,
                removal,
                request(9),
                &at(9),
            )?;
            Ok(recorded)
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut DecideClaim<'_>,
        ) -> Result<Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &ClaimId,
            owner: &ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &ClaimOwner,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &ClaimId,
            authority: ReconcileAuthority,
            decide: &mut DecideReconcile<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: RangeInclusive<u64>,
            filter: &HistoryFilter,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::history(self.store, project, range, filter, page)
        }

        fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&Anchor>,
        ) -> Result<VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    #[test]
    fn a_catalog_change_committed_after_the_detection_is_not_reported_as_its_version() {
        let (_dir, store) = seeded();
        let listed = openai_body(&["gpt-6-astra", "whisper-9"]);
        let racing = CommitsAfter {
            store: &store,
            removes: "whisper-9",
        };

        let recording = record(
            &racing,
            Provider::OpenAi,
            &Trigger::Automatic,
            Ok(&listed),
            request(2),
            &at(2),
        )
        .unwrap();

        // The catalog version is the sequence of the event that last changed
        // the accepted set: the detection's for its report, the removal's now.
        let recorded = events(&store, "models");
        let detected = &recorded[1];
        let removed = &recorded[2];
        assert_eq!(detected.type_name, "models.detected");
        assert_eq!(removed.type_name, "models.owner_changed");
        assert_eq!(state_version(&store), removed.seq, "the removal landed");
        assert_eq!(recording.version, detected.seq);
    }
}
