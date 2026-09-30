//! `baley models` and the seeding step every catalog use runs first (design
//! 0003, CFG-R19, CFG-R22). The catalog is the `model_catalog` view of the
//! reserved `user` project; the judges live in `baley_core::catalog`.

use baley_core::catalog::{
    HINT_VERSION, MODEL_CATALOG_VIEW, MODELS_SEEDED, MODELS_SEEDED_VERSION, MODELS_STREAM,
    USER_PROJECT, read_state, seed_due, seed_payload, state_key,
};
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, Ledger, NewEvent, Observed, OutcomeKind,
    ProjectId, Refusal, RequestId, StoreError, StreamName, Views, request_digest,
};
use serde_json::json;

/// The command kind the seeding step records under.
pub const SEED_COMMAND: &str = "models.seed";
/// The command kind `baley models add` records under.
pub const ADD_COMMAND: &str = "models.add";
/// The command kind `baley models remove` records under.
pub const REMOVE_COMMAND: &str = "models.remove";
/// The name the `user` project is created with, as `doctor` and `export`
/// show it.
pub const USER_PROJECT_NAME: &str = "per-user records";

fn user() -> ProjectId {
    ProjectId(USER_PROJECT.into())
}

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Refused(Refusal::InvalidEvent(error.to_string()))
}

/// Creates the `user` project. True when this run created it; one already
/// there is present, not a failure.
pub fn create_user(store: &impl Admin, at: &str) -> Result<bool, StoreError> {
    match store.create_project(&user(), USER_PROJECT_NAME, at) {
        Ok(()) => Ok(true),
        Err(StoreError::Refused(Refusal::ProjectExists(_))) => Ok(false),
        Err(error) => Err(error),
    }
}

/// The latest recorded hint version, read outside any transaction: none
/// when `user` is absent or nothing was seeded. The project list is read
/// first, so an absent project is not read as an error.
pub fn observe_hint_version(store: &(impl Admin + Views)) -> Result<Option<u64>, StoreError> {
    let project = user();
    if !store.projects()?.iter().any(|(known, _)| *known == project) {
        return Ok(None);
    }
    let state = store.get(&project, MODEL_CATALOG_VIEW, &state_key())?;
    Ok(read_state(state.as_ref().map(|document| &document.body)).hint_version)
}

/// Records the compiled hint table as Baley's own command. True when this
/// run appended `models.seeded`. The judge runs again inside the
/// transaction, so a racing second run records only its `command.completed`.
pub fn record_seed(
    store: &impl Ledger,
    request_id: RequestId,
    at: &str,
) -> Result<bool, StoreError> {
    let actor = Actor::Baley;
    let digest = request_digest(&json!({
        "kind": SEED_COMMAND,
        "project": USER_PROJECT,
        "actor": actor.as_str(),
        "policy_version": 0,
        "hint_version": HINT_VERSION,
        "scope": [],
    }))
    .map_err(invalid)?;
    let command = Command {
        project: user(),
        kind: CommandKind(SEED_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
    };
    let mut appended = false;
    store.transact(&command, &mut |tx| {
        let stored = tx.get(MODEL_CATALOG_VIEW, &state_key())?;
        let state = read_state(stored.as_ref().map(|document| &document.body));
        appended = seed_due(HINT_VERSION, state.hint_version);
        if appended {
            tx.append(NewEvent {
                stream: StreamName(MODELS_STREAM.into()),
                type_name: MODELS_SEEDED.into(),
                type_version: MODELS_SEEDED_VERSION,
                git: None,
                payload: seed_payload(state.catalog_version),
                attachments: vec![],
            })?;
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": appended }),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    Ok(appended)
}

/// Seeds the catalog when the recorded hint version differs from the
/// compiled one, creating `user` first. True when this run recorded. A match
/// records nothing, not even `command.completed`. It runs as its own
/// command, before the owner's, never inside it.
pub fn seed(
    store: &(impl Admin + Views + Ledger),
    request_id: RequestId,
    at: &str,
) -> Result<bool, StoreError> {
    if !seed_due(HINT_VERSION, observe_hint_version(store)?) {
        return Ok(false);
    }
    create_user(store, at)?;
    record_seed(store, request_id, at)
}

#[cfg(test)]
mod tests {
    use baley_core::catalog::{
        Catalog, EXACT_HINTS, MODEL_CATALOG_VIEW, Placement, Provider, Source, Tier, USER_PROJECT,
        accepted_names, catalog_key, listing, read_state, state_key,
    };
    use baley_store::{
        Actor, Admin, Command, CommandKind, Decision, Ledger, NewEvent, Observed, OutcomeKind,
        ProjectId, RequestId, StreamName, Views, request_digest,
    };
    use serde_json::{Value, json};

    use super::{HINT_VERSION, SEED_COMMAND, USER_PROJECT_NAME, record_seed, seed};

    const T0: &str = "2026-09-29T10:00:00Z";

    fn at(n: u8) -> String {
        format!("2026-09-29T10:00:{n:02}Z")
    }

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    fn user() -> ProjectId {
        ProjectId(USER_PROJECT.into())
    }

    /// A real store in a private home under a fresh temporary directory.
    fn open() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        (dir, store)
    }

    /// A fresh store holding the project `user`.
    fn store() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let (dir, store) = open();
        store
            .create_project(&user(), "per-user records", T0)
            .unwrap();
        (dir, store)
    }

    fn events(store: &baley_store_sqlite::SqliteStore, stream: &str) -> Vec<baley_store::Event> {
        let page = baley_store::PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(&user(), &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
    }

    fn head(store: &baley_store_sqlite::SqliteStore) -> u64 {
        store.head(&user()).unwrap().unwrap().seq
    }

    /// Appends hand-written `models.*` events in one command recorded at
    /// `at(n)` under request `n`, and returns the sequence each got. The head
    /// is one higher than the last, since `command.completed` follows.
    fn append(
        store: &baley_store_sqlite::SqliteStore,
        n: u8,
        events: &[(&str, Value)],
    ) -> Vec<u64> {
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
        let mut seqs = Vec::new();
        store
            .transact(&command, &mut |tx| {
                seqs.clear();
                for (type_name, payload) in events {
                    seqs.push(tx.append(NewEvent {
                        stream: StreamName("models".into()),
                        type_name: (*type_name).into(),
                        type_version: 1,
                        git: None,
                        payload: payload.clone(),
                        attachments: vec![],
                    })?);
                }
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!(null),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
        seqs
    }

    fn document(
        store: &baley_store_sqlite::SqliteStore,
        key: &baley_store::DocKey,
    ) -> Option<Value> {
        store
            .get(&user(), MODEL_CATALOG_VIEW, key)
            .unwrap()
            .map(|document| document.body)
    }

    const OPENAI: Catalog = Catalog::Provider(Provider::OpenAi);

    /// Whether the `openai` catalog as stored accepts `name`.
    fn accepts(store: &baley_store_sqlite::SqliteStore, name: &str) -> bool {
        let catalog = document(store, &catalog_key(OPENAI));
        let state = document(store, &state_key());
        accepted_names("openai", catalog.as_ref(), state.as_ref())
            .unwrap()
            .names
            .contains(name)
    }

    fn catalog_version(store: &baley_store_sqlite::SqliteStore) -> u64 {
        read_state(document(store, &state_key()).as_ref()).catalog_version
    }

    fn seeded(rows: Value) -> (&'static str, Value) {
        let payload = json!({"hint_version": 1, "catalog_version": 0, "rows": rows});
        ("models.seeded", payload)
    }

    fn detected(added: Value, removed: Value) -> (&'static str, Value) {
        let payload = json!({
            "provider": "openai",
            "added": added,
            "removed": removed,
            "catalog_version": 0,
            "hint_version": 1,
        });
        ("models.detected", payload)
    }

    fn owner(change: &str, name: &str, tier: Option<&str>) -> (&'static str, Value) {
        let mut payload = json!({
            "catalog": "openai",
            "name": name,
            "change": change,
            "catalog_version": 0,
        });
        if let Some(tier) = tier {
            payload["tier"] = tier.into();
        }
        ("models.owner_changed", payload)
    }

    #[test]
    fn a_detection_removing_an_owner_entry_does_not_drop_it_in_the_store() {
        let (_dir, store) = store();
        let row = json!([{"provider": "openai", "name": "gpt-a", "tier": "flagship", "high_effort": true}]);
        append(&store, 1, &[seeded(row)]);
        append(&store, 2, &[owner("added", "gpt-mine", None)]);
        append(&store, 3, &[detected(json!([]), json!(["gpt-mine"]))]);

        assert!(accepts(&store, "gpt-mine"));
        assert!(accepts(&store, "gpt-a"));
    }

    #[test]
    fn an_owner_removal_is_not_lost_when_a_later_detection_names_the_id() {
        let (_dir, store) = store();
        let found =
            json!([{"id": "gpt-x", "tier": "cheap", "high_effort": false, "placed": "best-fit"}]);
        append(&store, 1, &[detected(found.clone(), json!([]))]);
        assert!(accepts(&store, "gpt-x"));
        append(&store, 2, &[owner("removed", "gpt-x", None)]);
        append(&store, 3, &[detected(found, json!([]))]);

        assert!(!accepts(&store, "gpt-x"));
    }

    #[test]
    fn a_failed_detection_does_not_move_the_catalog_version() {
        let (_dir, store) = store();
        let found =
            json!([{"id": "gpt-x", "tier": "cheap", "high_effort": false, "placed": "hint"}]);
        let detected_seq = append(&store, 1, &[detected(found, json!([]))])[0];
        let failed = json!({"provider": "openai", "category": "unreachable", "catalog_version": detected_seq});
        append(&store, 2, &[("models.detection_failed", failed)]);

        assert_eq!(catalog_version(&store), detected_seq);
        let head = store.head(&user()).unwrap().unwrap().seq;
        assert_ne!(head, detected_seq);
    }

    #[test]
    fn an_empty_detection_keeps_the_version_and_verifies_every_entry_of_its_provider() {
        let (_dir, store) = store();
        let row = json!([{"provider": "openai", "name": "gpt-a", "tier": "flagship", "high_effort": true}]);
        append(&store, 1, &[seeded(row)]);
        let found =
            json!([{"id": "gpt-x", "tier": "cheap", "high_effort": false, "placed": "hint"}]);
        let found_seq = append(&store, 2, &[detected(found, json!([]))])[0];
        append(&store, 3, &[owner("added", "gpt-mine", Some("balanced"))]);
        let version = catalog_version(&store);
        assert!(version > found_seq);

        append(&store, 4, &[detected(json!([]), json!([]))]);

        assert_eq!(catalog_version(&store), version);
        let body = document(&store, &catalog_key(OPENAI)).unwrap();
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        for entry in entries {
            assert_eq!(entry["last_verified"], json!(at(4)), "{entry}");
        }
    }

    #[test]
    fn a_detection_removing_a_detected_id_drops_it() {
        let (_dir, store) = store();
        let found =
            json!([{"id": "gpt-x", "tier": "cheap", "high_effort": false, "placed": "hint"}]);
        append(&store, 1, &[detected(found, json!([]))]);
        assert!(accepts(&store, "gpt-x"));

        append(&store, 2, &[detected(json!([]), json!(["gpt-x"]))]);

        assert!(!accepts(&store, "gpt-x"));
    }

    #[test]
    fn a_rebuild_keeps_recorded_tiers_and_flags_not_the_compiled_table() {
        let (_dir, store) = store();
        let hinted = EXACT_HINTS[0];
        assert_eq!(hinted.provider, Provider::OpenAi);
        assert_ne!(hinted.tier, Tier::Cheap);
        append(&store, 1, &[owner("added", "gpt-mine", Some("cheap"))]);
        let found = json!([
            {"id": hinted.id, "tier": "cheap", "high_effort": !hinted.high_effort, "placed": "hint"},
            {"id": "gpt-new", "tier": "balanced", "high_effort": true, "placed": "best-fit"},
        ]);
        append(&store, 2, &[detected(found, json!([]))]);
        append(&store, 3, &[owner("added", "gpt-new", Some("flagship"))]);

        store.rebuild(&user()).unwrap();

        let body = document(&store, &catalog_key(OPENAI));
        let state = document(&store, &state_key());
        let rows = listing(&[(OPENAI, body.as_ref())], state.as_ref()).rows;
        let row = |name: &str| rows.iter().find(|row| row.name == name).unwrap().clone();
        let high_effort = |name: &str| {
            let entries = body.as_ref().unwrap()["entries"].as_array().unwrap();
            let entry = entries.iter().find(|entry| entry["id"] == name).unwrap();
            entry["high_effort"].as_bool().unwrap()
        };

        let mine = row("gpt-mine");
        assert_eq!((mine.source, mine.tier), (Source::Owner, Some(Tier::Cheap)));
        let hint = row(hinted.id);
        assert_eq!(hint.tier, Some(Tier::Cheap));
        assert_eq!(high_effort(hinted.id), !hinted.high_effort);
        let new = row("gpt-new");
        assert_eq!(
            (new.source, new.tier, new.placed),
            (Source::Owner, Some(Tier::Flagship), Some(Placement::Owner))
        );
        assert!(high_effort("gpt-new"));
    }

    #[test]
    fn a_ledger_written_at_view_set_version_two_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let options = baley_store_sqlite::Options::default();
        drop(crate::ledger::open::store(&home, T0, options).unwrap());

        crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
    }

    #[test]
    fn a_first_seed_creates_user_and_records_the_hint_table_as_baley() {
        let (_dir, store) = open();

        assert!(seed(&store, request(1), &at(1)).unwrap());

        assert_eq!(
            store.projects().unwrap(),
            vec![(user(), USER_PROJECT_NAME.to_string())]
        );
        assert_eq!(USER_PROJECT_NAME, "per-user records");
        let recorded = events(&store, "models");
        assert_eq!(recorded.len(), 1);
        let event = &recorded[0];
        assert_eq!(event.type_name, "models.seeded");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.actor, Actor::Baley);
        assert_eq!(event.policy_version, 0);
        let rows: Vec<Value> = EXACT_HINTS
            .iter()
            .map(|row| {
                json!({
                    "provider": row.provider.name(),
                    "name": row.id,
                    "tier": row.tier.name(),
                    "high_effort": row.high_effort,
                })
            })
            .collect();
        assert_eq!(
            event.payload,
            json!({"hint_version": HINT_VERSION, "catalog_version": 0, "rows": rows})
        );
        let completed = events(&store, "command/models.seed");
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].type_name, "command.completed");
        assert_eq!(SEED_COMMAND, "models.seed");
    }

    #[test]
    fn a_second_seed_sees_the_first_and_records_nothing() {
        let (_dir, store) = open();
        assert!(seed(&store, request(1), &at(1)).unwrap());
        let before = head(&store);

        assert!(!seed(&store, request(2), &at(2)).unwrap());

        assert_eq!(head(&store), before);
        assert_eq!(events(&store, "models").len(), 1);
        assert_eq!(events(&store, "command/models.seed").len(), 1);
    }

    #[test]
    fn a_racing_seed_rechecks_inside_the_transaction() {
        let (_dir, store) = open();
        assert!(seed(&store, request(1), &at(1)).unwrap());
        let before = head(&store);

        assert!(!record_seed(&store, request(2), &at(2)).unwrap());

        assert_eq!(events(&store, "models").len(), 1);
        assert_eq!(head(&store), before + 1);
        let completed = events(&store, "command/models.seed");
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[1].seq, before + 1);
    }

    #[test]
    fn a_downgrade_seeds_again_carrying_the_version_before_it() {
        let (_dir, store) = store();
        let newer = json!({
            "hint_version": HINT_VERSION + 1,
            "catalog_version": 0,
            "rows": [{"provider": "openai", "name": "gpt-later", "tier": "cheap", "high_effort": false}],
        });
        let first = append(&store, 1, &[("models.seeded", newer)])[0];

        assert!(seed(&store, request(2), &at(2)).unwrap());

        let recorded = events(&store, "models");
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[1].payload["hint_version"], json!(HINT_VERSION));
        assert_eq!(recorded[1].payload["catalog_version"], json!(first));
    }
}
