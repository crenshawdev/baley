//! When the hint table is recorded, and what is recorded (design 0003,
//! CFG-R19). The caller reads the latest recorded hint version from the
//! state document, once outside any transaction and again inside it, since
//! `Transaction::event_exists` cannot say which event is the latest.

use serde_json::Value;

use super::events::seeded_payload;
use super::tables::{EXACT_HINTS, HINT_VERSION};

/// Whether the hint table is due to be recorded: when no version is
/// recorded, and on any difference from the compiled one, a downgrade
/// included, so the catalog always holds the running binary's rows.
pub fn seed_due(compiled: u64, recorded: Option<u64>) -> bool {
    recorded != Some(compiled)
}

/// The `models.seeded` payload for the compiled table: its version, every
/// exact-id row and the catalog version before the event. Prefix rows are
/// never seeded; detection tags ids with them.
pub fn seed_payload(catalog_version: u64) -> Value {
    seeded_payload(HINT_VERSION, catalog_version, EXACT_HINTS)
}
