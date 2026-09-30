//! `baley models` and the seeding step every catalog use runs first (design
//! 0003, CFG-R19, CFG-R22). The catalog is the `model_catalog` view of the
//! reserved `user` project; the judges live in `baley_core::catalog`.

use std::fmt;
use std::process::ExitCode;

use baley_core::catalog::{
    Catalog, HINT_VERSION, MODEL_CATALOG_VIEW, MODELS_OWNER_CHANGED, MODELS_OWNER_CHANGED_VERSION,
    MODELS_SEEDED, MODELS_SEEDED_VERSION, MODELS_STREAM, OwnerChange, Placement, Tier,
    USER_PROJECT, catalog_key, judge_alias_addition, judge_alias_removal, judge_held_removal,
    listing, owner_changed_payload, read_state, seed_due, seed_payload, state_key,
};
use baley_store::{
    Actor, Admin, Answer, Command, CommandKind, Decision, DocKey, Ledger, NewEvent, Observed,
    OutcomeKind, ProjectId, Recorded, Refusal, RequestId, StoreError, StreamName, Views,
    request_digest,
};
use baley_store_sqlite::SqliteStore;
use serde_json::{Value, json};

use crate::folders::{Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;

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

/// The request-digest input of an owner change (D-15). `tier` is in it only
/// when `--tier` was given, so two additions that differ only in `--tier`
/// never share an identity.
pub fn owner_request(catalog: Catalog, name: &str, change: OwnerChange) -> Value {
    let kind = match change {
        OwnerChange::Added(_) => ADD_COMMAND,
        OwnerChange::Removed => REMOVE_COMMAND,
    };
    let mut input = json!({
        "kind": kind,
        "project": USER_PROJECT,
        "actor": Actor::Owner.as_str(),
        "policy_version": 0,
        "catalog": catalog.name(),
        "name": name,
        "scope": [],
    });
    if let OwnerChange::Added(Some(tier)) = change {
        input["tier"] = tier.name().into();
    }
    input
}

/// What an owner change came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerOutcome {
    /// Recorded; the catalog version after it.
    Changed {
        /// Read once the command committed.
        version: u64,
    },
    /// Refused on the merits, with the refusal's `<code>: ...` text. Only
    /// its `command.completed` was recorded.
    Refused(String),
}

/// Records one owner addition or removal of `name` in `catalog`. A removal
/// of a name the catalog does not hold is refused inside the transaction,
/// on the document read there. It assumes `user` exists: the seeding step
/// runs first.
pub fn change(
    store: &(impl Ledger + Views),
    catalog: Catalog,
    name: &str,
    change: OwnerChange,
    request_id: RequestId,
    at: &str,
) -> Result<OwnerOutcome, StoreError> {
    let kind = match change {
        OwnerChange::Added(_) => ADD_COMMAND,
        OwnerChange::Removed => REMOVE_COMMAND,
    };
    let command = Command {
        project: user(),
        kind: CommandKind(kind.into()),
        request_id,
        digest: request_digest(&owner_request(catalog, name, change)).map_err(invalid)?,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor: Actor::Owner,
    };
    let recorded = store.transact(&command, &mut |tx| {
        if change == OwnerChange::Removed {
            let stored = tx.get(MODEL_CATALOG_VIEW, &catalog_key(catalog))?;
            let document = stored.as_ref().map(|document| &document.body);
            if let Err(refusal) = judge_held_removal(catalog, name, document) {
                return Ok(Decision {
                    kind: OutcomeKind::Refused,
                    answer: json!({ "code": refusal.code(), "refusal": refusal.to_string() }),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                });
            }
        }
        let stored = tx.get(MODEL_CATALOG_VIEW, &state_key())?;
        let version = read_state(stored.as_ref().map(|document| &document.body)).catalog_version;
        tx.append(NewEvent {
            stream: StreamName(MODELS_STREAM.into()),
            type_name: MODELS_OWNER_CHANGED.into(),
            type_version: MODELS_OWNER_CHANGED_VERSION,
            git: None,
            payload: owner_changed_payload(catalog, name, change, version),
            attachments: vec![],
        })?;
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": true }),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    let outcome = match recorded {
        Recorded::New { outcome, .. } | Recorded::Replayed { outcome } => outcome,
    };
    if outcome.kind == OutcomeKind::Refused {
        // Owner answers are small and never sensitive, so they are inline.
        let text = match &outcome.answer {
            Answer::Inline(answer) => answer["refusal"].as_str(),
            _ => None,
        };
        let text = text.unwrap_or("the removal was refused; its answer cannot be read");
        return Ok(OwnerOutcome::Refused(text.into()));
    }
    let state = store.get(&user(), MODEL_CATALOG_VIEW, &state_key())?;
    let version = read_state(state.as_ref().map(|document| &document.body)).catalog_version;
    Ok(OwnerOutcome::Changed { version })
}

/// Arguments for `baley models`.
#[derive(clap::Args, Debug, Clone)]
pub struct ModelsArgs {
    #[command(subcommand)]
    command: ModelsCommand,
}

#[derive(clap::Subcommand, Debug, Clone)]
enum ModelsCommand {
    /// List every accepted name with its source, tier and placement, and
    /// the catalog version.
    List {
        /// One catalog: claude-code, codex, openai, gemini or deepseek.
        /// Every catalog when absent.
        #[arg(value_name = "CATALOG")]
        catalog: Option<String>,
    },
    /// Accept a name in a catalog, placed at --tier when given.
    Add {
        /// claude-code, codex, openai, gemini or deepseek.
        #[arg(value_name = "CATALOG")]
        catalog: String,
        /// The model name.
        #[arg(value_name = "NAME", value_parser = model_name)]
        name: String,
        /// flagship, balanced or cheap.
        #[arg(long, value_parser = tier)]
        tier: Option<Tier>,
    },
    /// Stop accepting a seeded, detected or owner name. A host's compiled
    /// aliases cannot be removed.
    Remove {
        /// claude-code, codex, openai, gemini or deepseek.
        #[arg(value_name = "CATALOG")]
        catalog: String,
        /// The model name.
        #[arg(value_name = "NAME", value_parser = model_name)]
        name: String,
    },
}

/// Runs one `baley models` command and prints what it did.
pub fn run(args: ModelsArgs) -> ExitCode {
    let started_at = SystemClock::now();
    let result = match args.command {
        ModelsCommand::List { catalog } => list(catalog.as_deref(), &started_at),
        ModelsCommand::Add {
            catalog,
            name,
            tier,
        } => owner(&catalog, &name, OwnerChange::Added(tier), &started_at),
        ModelsCommand::Remove { catalog, name } => {
            owner(&catalog, &name, OwnerChange::Removed, &started_at)
        }
    }
    .unwrap_or_else(|e| e);
    for line in result.lines {
        if result.error {
            eprintln!("baley: {line}");
        } else {
            println!("{line}");
        }
    }
    ExitCode::from(result.code)
}

fn refuse(refusal: &dyn fmt::Display) -> Render {
    Render::refusal(refusal.to_string())
}

fn failed(error: StoreError) -> Render {
    display::store_error(&error, Some(USER_PROJECT))
}

/// Resolves the folders, opens the store and seeds. Every refusal that
/// needs no store comes before this, so it leaves no ledger home behind.
fn open_seeded(
    folders: &Folders,
    started_at: &str,
    lines: &mut Vec<String>,
) -> Result<SqliteStore, Render> {
    let store = open::store(&folders.home, started_at, open::options())
        .map_err(|e| display::store_error(&e, None))?;
    if seed(&store, new_request_id(), started_at).map_err(failed)? {
        lines.push(format!(
            "seeded the model catalog with hint table version {HINT_VERSION}"
        ));
    }
    Ok(store)
}

fn folders() -> Result<Folders, Render> {
    Folders::resolve(Platform::current(), &Environment::read()).map_err(|e| refuse(&e))
}

fn list(catalog: Option<&str>, started_at: &str) -> Result<Render, Render> {
    let folders = folders()?;
    let catalogs = match catalog {
        Some(name) => vec![Catalog::parse(name).map_err(|e| refuse(&e))?],
        None => Catalog::ALL.to_vec(),
    };
    let mut lines = Vec::new();
    let store = open_seeded(&folders, started_at, &mut lines)?;
    // One snapshot, so the printed version describes the rows beside it.
    let keys: Vec<DocKey> = std::iter::once(state_key())
        .chain(catalogs.iter().map(|catalog| catalog_key(*catalog)))
        .collect();
    let mut bodies = store
        .get_many(&user(), MODEL_CATALOG_VIEW, &keys)
        .map_err(failed)?
        .into_iter()
        .map(|document| document.map(|document| document.body));
    let state = bodies.next().flatten();
    let bodies: Vec<Option<Value>> = bodies.collect();
    let documents: Vec<(Catalog, Option<&Value>)> = catalogs
        .iter()
        .copied()
        .zip(bodies.iter().map(Option::as_ref))
        .collect();
    let listing = listing(&documents, state.as_ref());
    lines.push(format!("catalog version {}", listing.version));
    let table: Vec<[&str; 5]> = listing
        .rows
        .iter()
        .map(|row| {
            [
                row.catalog.name(),
                row.name.as_str(),
                row.source.name(),
                row.tier.map_or("-", Tier::name),
                row.placed.map_or("-", Placement::name),
            ]
        })
        .collect();
    lines.extend(columns(
        ["CATALOG", "NAME", "SOURCE", "TIER", "PLACED"],
        &table,
    ));
    Ok(Render {
        lines,
        code: 0,
        error: false,
    })
}

// An empty entry could never match a setting's model name.
fn model_name(text: &str) -> Result<String, String> {
    if text.is_empty() {
        Err("a model name is never empty".into())
    } else {
        Ok(text.into())
    }
}

fn tier(text: &str) -> Result<Tier, String> {
    Tier::parse(text).ok_or_else(|| {
        let tiers: Vec<&str> = Tier::ALL.iter().map(|tier| tier.name()).collect();
        format!("tiers: {}", tiers.join(", "))
    })
}

fn owner(
    catalog: &str,
    name: &str,
    change: OwnerChange,
    started_at: &str,
) -> Result<Render, Render> {
    let folders = folders()?;
    let catalog = Catalog::parse(catalog).map_err(|e| refuse(&e))?;
    match change {
        OwnerChange::Added(_) => judge_alias_addition(catalog, name).map_err(|e| refuse(&e))?,
        OwnerChange::Removed => judge_alias_removal(catalog, name).map_err(|e| refuse(&e))?,
    }
    let mut lines = Vec::new();
    let store = open_seeded(&folders, started_at, &mut lines)?;
    let outcome = self::change(&store, catalog, name, change, new_request_id(), started_at)
        .map_err(failed)?;
    let version = match outcome {
        OwnerOutcome::Refused(text) => return Err(refuse(&text)),
        OwnerOutcome::Changed { version } => version,
    };
    // Debug quoting escapes control bytes from the command line.
    let done = match change {
        OwnerChange::Added(Some(tier)) => format!(
            "added {name:?} to {} at tier {}",
            catalog.name(),
            tier.name()
        ),
        OwnerChange::Added(None) => format!("added {name:?} to {}", catalog.name()),
        OwnerChange::Removed => format!("removed {name:?} from {}", catalog.name()),
    };
    lines.push(format!("{done}; catalog version {version}"));
    Ok(Render {
        lines,
        code: 0,
        error: false,
    })
}

// Pads every column but the last to its widest cell.
fn columns(header: [&str; 5], rows: &[[&str; 5]]) -> Vec<String> {
    let mut widths = header.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    std::iter::once(&header)
        .chain(rows)
        .map(|row| {
            let mut line = String::new();
            for (index, cell) in row.iter().enumerate() {
                if index + 1 < row.len() {
                    line.push_str(&format!("{cell:<width$}  ", width = widths[index]));
                } else {
                    line.push_str(cell);
                }
            }
            line
        })
        .collect()
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

    use super::{
        HINT_VERSION, OwnerChange, OwnerOutcome, SEED_COMMAND, USER_PROJECT_NAME, change,
        owner_request, record_seed, seed,
    };

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

    fn digest(change: OwnerChange) -> baley_store::Hash {
        request_digest(&owner_request(OPENAI, "gpt-test", change)).unwrap()
    }

    #[test]
    fn an_owner_change_identity_covers_its_kind_and_tier_and_nothing_else() {
        let cheap = digest(OwnerChange::Added(Some(Tier::Cheap)));
        let flagship = digest(OwnerChange::Added(Some(Tier::Flagship)));
        let untiered = digest(OwnerChange::Added(None));
        let removed = digest(OwnerChange::Removed);
        assert_ne!(cheap, flagship);
        assert_ne!(cheap, untiered);
        assert_ne!(flagship, untiered);
        assert_ne!(untiered, removed);

        let expected = json!({
            "kind": "models.add",
            "project": "user",
            "actor": "owner",
            "policy_version": 0,
            "catalog": "openai",
            "name": "gpt-test",
            "tier": "cheap",
            "scope": [],
        });
        assert_eq!(cheap, request_digest(&expected).unwrap());
    }

    #[test]
    fn an_addition_records_the_version_before_it_and_returns_the_one_after() {
        let (_dir, store) = open();
        seed(&store, request(1), &at(1)).unwrap();
        let seeded = events(&store, "models")[0].seq;

        let added = OwnerChange::Added(Some(Tier::Cheap));
        let outcome = change(&store, OPENAI, "gpt-test", added, request(2), &at(2)).unwrap();

        let recorded = events(&store, "models");
        assert_eq!(recorded.len(), 2);
        let event = &recorded[1];
        assert_eq!(event.type_name, "models.owner_changed");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.policy_version, 0);
        assert_eq!(
            event.payload,
            json!({
                "catalog": "openai",
                "name": "gpt-test",
                "change": "added",
                "tier": "cheap",
                "catalog_version": seeded,
            })
        );
        let completed = events(&store, "command/models.add");
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].type_name, "command.completed");
        assert_eq!(outcome, OwnerOutcome::Changed { version: event.seq });
    }

    #[test]
    fn a_removal_of_a_name_nobody_held_is_refused_and_records_no_model_event() {
        let (_dir, store) = open();
        seed(&store, request(1), &at(1)).unwrap();

        let outcome = change(
            &store,
            OPENAI,
            "no-such-model",
            OwnerChange::Removed,
            request(2),
            &at(2),
        )
        .unwrap();

        let OwnerOutcome::Refused(text) = outcome else {
            panic!("removal not refused: {outcome:?}")
        };
        assert_eq!(
            text,
            "unknown-model: the openai catalog does not hold \"no-such-model\""
        );
        assert_eq!(events(&store, "models").len(), 1);
    }

    #[test]
    fn a_removal_of_a_seeded_id_is_applied_to_the_catalog() {
        let (_dir, store) = open();
        seed(&store, request(1), &at(1)).unwrap();
        let hinted = EXACT_HINTS[0];
        assert_eq!(hinted.provider, Provider::OpenAi);
        assert!(accepts(&store, hinted.id));

        let outcome = change(
            &store,
            OPENAI,
            hinted.id,
            OwnerChange::Removed,
            request(2),
            &at(2),
        )
        .unwrap();

        let removal = events(&store, "models")[1].seq;
        assert_eq!(outcome, OwnerOutcome::Changed { version: removal });
        assert!(!accepts(&store, hinted.id));
    }

    #[test]
    fn a_snapshot_read_returns_each_document_at_its_own_key_in_the_order_asked() {
        let (_dir, store) = store();
        let rows = json!([
            {"provider": "openai", "name": "gpt-a", "tier": "flagship", "high_effort": true},
            {"provider": "deepseek", "name": "ds-a", "tier": "cheap", "high_effort": false},
        ]);
        let seq = append(&store, 1, &[seeded(rows)])[0];
        let deepseek = catalog_key(Catalog::Provider(Provider::DeepSeek));
        let gemini = catalog_key(Catalog::Provider(Provider::Gemini));
        let keys = [deepseek.clone(), state_key(), gemini, catalog_key(OPENAI)];

        let documents = store.get_many(&user(), MODEL_CATALOG_VIEW, &keys).unwrap();

        assert_eq!(documents.len(), 4);
        let [deepseek_doc, state, absent, openai] = [0, 1, 2, 3].map(|i| documents[i].as_ref());
        assert_eq!(deepseek_doc.unwrap().key, deepseek);
        assert_eq!(state.unwrap().key, state_key());
        assert!(absent.is_none(), "gemini holds no entry");
        assert_eq!(openai.unwrap().key, catalog_key(OPENAI));
        let state = state.map(|document| &document.body);
        let names = |catalog: &str, document: Option<&baley_store::Document>| {
            accepted_names(catalog, document.map(|document| &document.body), state).unwrap()
        };
        let deepseek_names = names("deepseek", deepseek_doc);
        assert_eq!(deepseek_names.names, ["ds-a".to_string()].into());
        assert_eq!(deepseek_names.version, seq);
        assert_eq!(names("openai", openai).names, ["gpt-a".to_string()].into());
    }
}
