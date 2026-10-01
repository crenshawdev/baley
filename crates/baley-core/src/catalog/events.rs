//! The four `models.*` event types and their version-1 payloads (design
//! 0003 section 6). Every one goes on stream `models` of the `user` project
//! with policy version 0.
//!
//! `catalog_version` in each payload is the version before the event, read in
//! the recording transaction: 0 when nothing is recorded or `user` is absent.
//! First-seen and last-verified times come from the envelope's `recorded_at`,
//! never from a payload field.

use serde_json::{Map, Value, json};

use super::detection::{Added, Category};
use super::tables::HintRow;
use super::{Catalog, Provider, Tier};
use crate::registry::{Registry, RegistryError};

/// The stream every catalog event is on.
pub const MODELS_STREAM: &str = "models";

/// The hint table recorded, actor `baley`:
/// `{hint_version, catalog_version, rows: [{provider, name, tier, high_effort}]}`.
pub const MODELS_SEEDED: &str = "models.seeded";
/// The current `models.seeded` payload version.
pub const MODELS_SEEDED_VERSION: u32 = 1;

/// The owner added or removed one name, actor `owner`:
/// `{catalog, name, change, tier, catalog_version}`. `change` is `added` or
/// `removed`. `tier` is absent when `--tier` was not given, and a removal
/// never carries it.
pub const MODELS_OWNER_CHANGED: &str = "models.owner_changed";
/// The current `models.owner_changed` payload version.
pub const MODELS_OWNER_CHANGED_VERSION: u32 = 1;

/// One provider's list endpoint answered:
/// `{provider, added: [{id, tier, high_effort, placed}], removed: [id, ...],
/// catalog_version, hint_version}`. `placed` is `hint`, `prefix` or
/// `best-fit`. Built by [`detected_payload`].
pub const MODELS_DETECTED: &str = "models.detected";
/// The current `models.detected` payload version.
pub const MODELS_DETECTED_VERSION: u32 = 1;

/// One provider's detection failed and its catalog was left as it was:
/// `{provider, category, catalog_version}`. Built by
/// [`detection_failed_payload`].
pub const MODELS_DETECTION_FAILED: &str = "models.detection_failed";
/// The current `models.detection_failed` payload version.
pub const MODELS_DETECTION_FAILED_VERSION: u32 = 1;

/// Registers the four `models.*` types at version 1, with no upcasters.
pub fn register_model_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(MODELS_SEEDED, MODELS_SEEDED_VERSION, [])?;
    registry.register(MODELS_OWNER_CHANGED, MODELS_OWNER_CHANGED_VERSION, [])?;
    registry.register(MODELS_DETECTED, MODELS_DETECTED_VERSION, [])?;
    registry.register(MODELS_DETECTION_FAILED, MODELS_DETECTION_FAILED_VERSION, [])
}

/// What the owner did to one name. A removal holds no tier, so no removal
/// payload can carry one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerChange {
    /// Added, with the `--tier` given, if any.
    Added(Option<Tier>),
    /// Removed.
    Removed,
}

/// The `models.seeded` payload for `rows`.
pub fn seeded_payload(hint_version: u64, catalog_version: u64, rows: &[HintRow]) -> Value {
    let rows: Vec<Value> = rows
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
    json!({
        "hint_version": hint_version,
        "catalog_version": catalog_version,
        "rows": rows,
    })
}

/// The `models.owner_changed` payload for one change to `name` in `catalog`.
pub fn owner_changed_payload(
    catalog: Catalog,
    name: &str,
    change: OwnerChange,
    catalog_version: u64,
) -> Value {
    let mut payload = Map::new();
    payload.insert("catalog".into(), catalog.name().into());
    payload.insert("name".into(), name.into());
    let change = match change {
        OwnerChange::Added(tier) => {
            if let Some(tier) = tier {
                payload.insert("tier".into(), tier.name().into());
            }
            "added"
        }
        OwnerChange::Removed => "removed",
    };
    payload.insert("change".into(), change.into());
    payload.insert("catalog_version".into(), catalog_version.into());
    Value::Object(payload)
}

/// The `models.detected` payload for one provider's listing: each id in
/// `added` with its tag, the ids in `removed`, the catalog version before
/// the event and the hint table's version.
pub fn detected_payload(
    provider: Provider,
    added: &[Added],
    removed: &[String],
    catalog_version: u64,
    hint_version: u64,
) -> Value {
    let added: Vec<Value> = added
        .iter()
        .map(|added| {
            json!({
                "id": added.id,
                "tier": added.tag.tier.name(),
                "high_effort": added.tag.high_effort,
                "placed": added.tag.placed.name(),
            })
        })
        .collect();
    json!({
        "provider": provider.name(),
        "added": added,
        "removed": removed,
        "catalog_version": catalog_version,
        "hint_version": hint_version,
    })
}

/// The `models.detection_failed` payload for one provider. It names the
/// category only, and holds no `removed`: a failed listing removes nothing.
pub fn detection_failed_payload(
    provider: Provider,
    category: Category,
    catalog_version: u64,
) -> Value {
    json!({
        "provider": provider.name(),
        "category": category.name(),
        "catalog_version": catalog_version,
    })
}
