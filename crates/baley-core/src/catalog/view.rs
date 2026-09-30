//! The `model_catalog` view of the `user` project: one document per catalog
//! holding its entries, and one state document holding the catalog version
//! and the latest seeded hint version (design 0003 section 6).
//!
//! The projector reads only its events and documents, never a compiled
//! table, so a rebuild gives the same catalog under any later binary. Host
//! aliases are never entries.

use std::collections::{BTreeMap, BTreeSet};

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::events::{MODELS_OWNER_CHANGED, MODELS_SEEDED};
use super::{Catalog, Provider, Tier};

/// The view's name.
pub const MODEL_CATALOG_VIEW: &str = "model_catalog";

/// The key field every document holds.
const KEY_FIELD: &str = "catalog";

/// The state document's key text, which is no catalog's name.
const STATE_KEY: &str = "state";

/// The view's declaration: one text key naming a catalog, no indexes, and
/// page bound 1, since every read is a `get` by key.
pub fn model_catalog_spec() -> ViewSpec {
    ViewSpec {
        name: MODEL_CATALOG_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: KEY_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of one catalog's document.
pub fn catalog_key(catalog: Catalog) -> DocKey {
    DocKey(vec![KeyValue::Text(catalog.name().into())])
}

/// The key of the catalog-state document.
pub fn state_key() -> DocKey {
    DocKey(vec![KeyValue::Text(STATE_KEY.into())])
}

/// Where an accepted name came from. The order is the listing's within one
/// name: an alias row first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A host's compiled alias. Never stored.
    Alias,
    /// The hint table, recorded by `models.seeded`.
    Seed,
    /// A provider's list endpoint, recorded by `models.detected`.
    Detected,
    /// The owner, recorded by `models.owner_changed`.
    Owner,
}
impl Source {
    /// The name documents and listings use.
    pub fn name(self) -> &'static str {
        match self {
            Source::Alias => "alias",
            Source::Seed => "seed",
            Source::Detected => "detected",
            Source::Owner => "owner",
        }
    }

    // A stored entry is never an alias.
    fn parse_stored(name: &str) -> Option<Source> {
        [Source::Seed, Source::Detected, Source::Owner]
            .into_iter()
            .find(|source| source.name() == name)
    }
}

/// How an entry got its tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Placement {
    /// An exact hint-table row.
    Hint,
    /// A hint-table prefix row.
    Prefix,
    /// Detection's best fit for an id no row names.
    BestFit,
    /// The owner's `--tier`.
    Owner,
}
impl Placement {
    /// The name documents, events and listings use.
    pub fn name(self) -> &'static str {
        match self {
            Placement::Hint => "hint",
            Placement::Prefix => "prefix",
            Placement::BestFit => "best-fit",
            Placement::Owner => "owner",
        }
    }

    fn parse(name: &str) -> Option<Placement> {
        [
            Placement::Hint,
            Placement::Prefix,
            Placement::BestFit,
            Placement::Owner,
        ]
        .into_iter()
        .find(|placed| placed.name() == name)
    }
}

/// One id in a catalog document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) id: String,
    pub(super) source: Source,
    /// Absent only for an owner entry that never had one.
    pub(super) tier: Option<Tier>,
    pub(super) high_effort: bool,
    pub(super) placed: Placement,
    /// The `recorded_at` of the event that first put the id here.
    pub(super) first_seen: String,
    /// Absent until a detection sets it.
    pub(super) last_verified: Option<String>,
    /// The sequence of the event that last made the id accepted; absent for
    /// an id the owner removed before anything accepted it.
    pub(super) accepted_seq: Option<u64>,
    /// The owner removed it: kept so no seed or detection brings it back.
    pub(super) owner_removed: bool,
}
impl Entry {
    fn to_value(&self) -> Value {
        let mut entry = Map::new();
        entry.insert("id".into(), self.id.clone().into());
        entry.insert("source".into(), self.source.name().into());
        if let Some(tier) = self.tier {
            entry.insert("tier".into(), tier.name().into());
        }
        entry.insert("high_effort".into(), self.high_effort.into());
        entry.insert("placed".into(), self.placed.name().into());
        entry.insert("first_seen".into(), self.first_seen.clone().into());
        if let Some(at) = &self.last_verified {
            entry.insert("last_verified".into(), at.clone().into());
        }
        if let Some(seq) = self.accepted_seq {
            entry.insert("accepted_seq".into(), seq.into());
        }
        entry.insert("owner_removed".into(), self.owner_removed.into());
        Value::Object(entry)
    }

    fn from_value(value: &Value) -> Result<Entry, String> {
        let source = text(value, "source")?;
        let placed = text(value, "placed")?;
        Ok(Entry {
            id: text(value, "id")?.to_owned(),
            source: Source::parse_stored(source).ok_or(format!("unknown source {source:?}"))?,
            tier: optional_tier(value)?,
            high_effort: flag(value, "high_effort")?,
            placed: Placement::parse(placed).ok_or(format!("unknown placement {placed:?}"))?,
            first_seen: text(value, "first_seen")?.to_owned(),
            last_verified: optional_text(value, "last_verified")?,
            accepted_seq: optional_number(value, "accepted_seq")?,
            owner_removed: flag(value, "owner_removed")?,
        })
    }

    fn accepted(&self) -> bool {
        !self.owner_removed
    }
}

/// A catalog document's entries by id, or none for an absent document.
pub(super) fn read_entries(body: Option<&Value>) -> Result<BTreeMap<String, Entry>, String> {
    let Some(body) = body else {
        return Ok(BTreeMap::new());
    };
    let entries = body
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("a catalog document without entries")?;
    entries
        .iter()
        .map(|value| Entry::from_value(value).map(|entry| (entry.id.clone(), entry)))
        .collect()
}

/// The ids a catalog document accepts: every entry the owner did not remove.
fn accepted_ids(entries: &BTreeMap<String, Entry>) -> BTreeSet<&str> {
    entries
        .values()
        .filter(|entry| entry.accepted())
        .map(|entry| entry.id.as_str())
        .collect()
}

// Entries in id order, so a rebuild writes byte-identical documents.
fn catalog_body(catalog: Catalog, entries: &BTreeMap<String, Entry>) -> Value {
    let entries: Vec<Value> = entries.values().map(Entry::to_value).collect();
    json!({KEY_FIELD: catalog.name(), "entries": entries})
}

/// What the state document holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogState {
    /// The sequence of the last event that changed some catalog's accepted
    /// ids, 0 before any.
    pub catalog_version: u64,
    /// The latest seeded hint-table version, `None` before any seed.
    pub hint_version: Option<u64>,
}

fn parse_state(body: Option<&Value>) -> Result<CatalogState, String> {
    let Some(body) = body else {
        return Ok(CatalogState::default());
    };
    Ok(CatalogState {
        catalog_version: number(body, "catalog_version")?,
        hint_version: optional_number(body, "hint_version")?,
    })
}

/// The catalog version and latest seeded hint version from the state
/// document's body; version 0 and no hint version when it is absent. The
/// document is only ever written by [`ModelCatalogProjector`], so one that
/// does not read is taken as absent: a lower version refuses more, never
/// less, and a rebuild restores it.
pub fn read_state(body: Option<&Value>) -> CatalogState {
    parse_state(body).unwrap_or_default()
}

fn state_body(state: CatalogState) -> Value {
    let mut body = Map::new();
    body.insert(KEY_FIELD.into(), STATE_KEY.into());
    body.insert("catalog_version".into(), state.catalog_version.into());
    if let Some(hint) = state.hint_version {
        body.insert("hint_version".into(), hint.into());
    }
    Value::Object(body)
}

fn text<'a>(object: &'a Value, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(format!("{field} is missing or not text"))
}

fn optional_text(object: &Value, field: &str) -> Result<Option<String>, String> {
    match object.get(field) {
        None => Ok(None),
        Some(_) => text(object, field).map(|text| Some(text.to_owned())),
    }
}

fn flag(object: &Value, field: &str) -> Result<bool, String> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(format!("{field} is missing or not true or false"))
}

fn number(object: &Value, field: &str) -> Result<u64, String> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(format!("{field} is missing or not a whole number"))
}

fn optional_number(object: &Value, field: &str) -> Result<Option<u64>, String> {
    match object.get(field) {
        None => Ok(None),
        Some(_) => number(object, field).map(Some),
    }
}

fn tier(object: &Value) -> Result<Tier, String> {
    let name = text(object, "tier")?;
    Tier::parse(name).ok_or(format!("tier {name:?} is no tier"))
}

fn optional_tier(object: &Value) -> Result<Option<Tier>, String> {
    match object.get("tier") {
        None => Ok(None),
        Some(_) => tier(object).map(Some),
    }
}

fn provider(object: &Value) -> Result<Provider, String> {
    let name = text(object, "provider")?;
    Provider::parse(name).ok_or(format!("provider {name:?} is no provider"))
}

fn catalog(object: &Value) -> Result<Catalog, String> {
    let name = text(object, "catalog")?;
    Catalog::parse(name).map_err(|_| format!("catalog {name:?} is no catalog"))
}

/// One catalog document an event touches, before and after.
struct Book<'a> {
    catalog: Catalog,
    stored: Option<&'a Value>,
    entries: BTreeMap<String, Entry>,
}

/// Keeps the `model_catalog` view current from the `models.*` events.
pub struct ModelCatalogProjector {
    spec: ViewSpec,
}

impl ModelCatalogProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: model_catalog_spec(),
        }
    }
}

impl Default for ModelCatalogProjector {
    fn default() -> Self {
        Self::new()
    }
}

/// The catalogs whose documents an event touches, or why it names none.
fn touched(event: &Event) -> Result<Vec<Catalog>, String> {
    match event.type_name.as_str() {
        MODELS_SEEDED => Ok(Provider::ALL.map(Catalog::Provider).to_vec()),
        MODELS_OWNER_CHANGED => Ok(vec![catalog(&event.payload)?]),
        other => Err(format!("{MODEL_CATALOG_VIEW} does not apply {other}")),
    }
}

impl Projector for ModelCatalogProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[MODELS_SEEDED, MODELS_OWNER_CHANGED]
    }

    // A payload too broken to name its catalog still names the state
    // document; `apply` refuses it.
    fn keys(&self, event: &Event) -> Vec<DocKey> {
        let mut keys = vec![state_key()];
        keys.extend(
            touched(event)
                .unwrap_or_default()
                .into_iter()
                .map(catalog_key),
        );
        keys
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: String| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let stored = |key: &DocKey| {
            documents
                .iter()
                .find(|(found, _)| found == key)
                .map(|(_, body)| body)
        };
        let mut books = Vec::new();
        for catalog in touched(event).map_err(refuse)? {
            let body = stored(&catalog_key(catalog));
            books.push(Book {
                catalog,
                stored: body,
                entries: read_entries(body).map_err(refuse)?,
            });
        }
        let before: Vec<BTreeSet<String>> = books
            .iter()
            .map(|book| {
                accepted_ids(&book.entries)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        let stored_state = stored(&state_key());
        let mut state = parse_state(stored_state).map_err(refuse)?;
        match event.type_name.as_str() {
            MODELS_SEEDED => {
                state.hint_version = Some(seed(event, &mut books).map_err(refuse)?);
            }
            _ => owner_change(event, &mut books[0].entries).map_err(refuse)?,
        }

        let mut changes = Vec::new();
        let mut moved = false;
        for (book, before) in books.iter().zip(&before) {
            let after = accepted_ids(&book.entries);
            moved |= after.len() != before.len() || after.iter().any(|id| !before.contains(*id));
            let body = catalog_body(book.catalog, &book.entries);
            let unchanged = match book.stored {
                Some(stored) => *stored == body,
                None => book.entries.is_empty(),
            };
            if !unchanged {
                let key = catalog_key(book.catalog);
                changes.push(Change::Put { key, body });
            }
        }
        if moved {
            state.catalog_version = event.seq;
        }
        let body = state_body(state);
        let unchanged = match stored_state {
            Some(stored) => *stored == body,
            None => state == CatalogState::default(),
        };
        if !unchanged {
            changes.push(Change::Put {
                key: state_key(),
                body,
            });
        }
        Ok(changes)
    }
}

/// A seed makes each provider's seed-sourced entries exactly its rows.
/// Detected and owner entries, and ids the owner removed, are left as they
/// are: the owner wins over every other source. Returns the seeded hint
/// version.
fn seed(event: &Event, books: &mut [Book<'_>]) -> Result<u64, String> {
    let payload = &event.payload;
    let hint_version = number(payload, "hint_version")?;
    let rows = payload
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("rows is missing or not a list")?;
    let mut named: BTreeMap<Provider, BTreeMap<&str, (Tier, bool)>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let at = |message: String| format!("rows[{index}]: {message}");
        let provider = provider(row).map_err(at)?;
        let name = text(row, "name").map_err(at)?;
        let hint = (
            tier(row).map_err(at)?,
            flag(row, "high_effort").map_err(at)?,
        );
        named.entry(provider).or_default().insert(name, hint);
    }
    for book in books {
        let Catalog::Provider(provider) = book.catalog else {
            continue;
        };
        let rows = named.remove(&provider).unwrap_or_default();
        book.entries.retain(|id, entry| {
            entry.owner_removed || entry.source != Source::Seed || rows.contains_key(id.as_str())
        });
        for (id, (tier, high_effort)) in rows {
            match book.entries.get_mut(id) {
                None => {
                    let entry = Entry {
                        id: id.to_owned(),
                        source: Source::Seed,
                        tier: Some(tier),
                        high_effort,
                        placed: Placement::Hint,
                        first_seen: event.recorded_at.clone(),
                        last_verified: None,
                        accepted_seq: Some(event.seq),
                        owner_removed: false,
                    };
                    book.entries.insert(id.to_owned(), entry);
                }
                Some(entry) if entry.source == Source::Seed && !entry.owner_removed => {
                    entry.tier = Some(tier);
                    entry.high_effort = high_effort;
                }
                Some(_) => {}
            }
        }
    }
    Ok(hint_version)
}

/// An owner removal marks the id and keeps its fields; an addition makes it
/// an owner entry, keeping its high-effort flag, since `--tier` says nothing
/// about effort.
fn owner_change(event: &Event, entries: &mut BTreeMap<String, Entry>) -> Result<(), String> {
    let payload = &event.payload;
    let name = text(payload, "name")?;
    let change = text(payload, "change")?;
    let given = optional_tier(payload)?;
    match change {
        "removed" => {
            let entry = entries.entry(name.to_owned()).or_insert_with(|| Entry {
                id: name.to_owned(),
                source: Source::Owner,
                tier: None,
                high_effort: false,
                placed: Placement::Owner,
                first_seen: event.recorded_at.clone(),
                last_verified: None,
                accepted_seq: None,
                owner_removed: true,
            });
            entry.owner_removed = true;
        }
        "added" => match entries.get_mut(name) {
            Some(entry) => {
                if entry.owner_removed {
                    entry.owner_removed = false;
                    entry.accepted_seq = Some(event.seq);
                }
                entry.source = Source::Owner;
                if let Some(tier) = given {
                    entry.tier = Some(tier);
                    entry.placed = Placement::Owner;
                }
            }
            None => {
                let entry = Entry {
                    id: name.to_owned(),
                    source: Source::Owner,
                    tier: given,
                    high_effort: false,
                    placed: Placement::Owner,
                    first_seen: event.recorded_at.clone(),
                    last_verified: None,
                    accepted_seq: Some(event.seq),
                    owner_removed: false,
                };
                entries.insert(name.to_owned(), entry);
            }
        },
        other => return Err(format!("change {other:?} is neither added nor removed")),
    }
    Ok(())
}
