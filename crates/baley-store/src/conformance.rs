//! What an adapter's test harness gives the conformance suite (design 0001,
//! Testing). The suite itself, `run(factory)`, lands with Build 1's
//! thirteenth task; it owns the scenarios and assertions, and each adapter
//! runs it against a fresh temporary directory per check.

use crate::error::StoreError;
use crate::event::{Event, Hash, ProjectId};
use crate::ledger::{Admin, EventSchema, Ledger, Views};
use crate::payload::Payloads;
use crate::view::{DocKey, Projector};

/// Opens stores for the suite and damages them the way an attacker or an
/// accident would. Every operation stays inside the store's own temporary
/// directory.
pub trait StoreFactory {
    type Store: Ledger + Views + Payloads + Admin;
    /// A copy of a store's files, taken to restore later.
    type Snapshot;

    /// A fresh, empty store in a new temporary directory it owns, with
    /// these projectors and this event schema registered.
    fn create(
        &self,
        projectors: Vec<Box<dyn Projector>>,
        schema: Box<dyn EventSchema>,
    ) -> Result<Self::Store, StoreError>;

    /// A second, independent connection to the same store.
    fn reopen(&self, store: &Self::Store) -> Result<Self::Store, StoreError>;

    /// Damages the store behind the port's back.
    fn corrupt(
        &self,
        store: &Self::Store,
        project: &ProjectId,
        damage: Corruption,
    ) -> Result<(), StoreError>;

    fn snapshot(&self, store: &Self::Store) -> Result<Self::Snapshot, StoreError>;

    /// Puts an older copy back in place: the rollback an anchor catches.
    fn restore(&self, store: &Self::Store, snapshot: &Self::Snapshot) -> Result<(), StoreError>;

    /// Starts a rebuild of the project's views and stops it after
    /// `batches` replay batches as a crash would, leaving the unfinished
    /// generation behind.
    fn crash_rebuild(
        &self,
        store: &Self::Store,
        project: &ProjectId,
        batches: u32,
    ) -> Result<(), StoreError>;
}

/// The damage the suite asks for, each a row operation on the stored events
/// bodies or view documents.
#[derive(Debug, Clone, PartialEq)]
pub enum Corruption {
    /// Replaces a stored event's payload, leaving its hashes as they were.
    AlterPayload {
        seq: u64,
        payload: serde_json::Value,
    },
    /// Inserts a whole stored event row as given, hashes and all.
    Insert(Box<Event>),
    Delete {
        seq: u64,
    },
    /// Swaps the sequences of two stored events.
    Reorder {
        first: u64,
        second: u64,
    },
    /// Replaces a payload and recomputes every hash from that event to the
    /// head, so the local chain is consistent and only an anchor can tell.
    RecomputeAfterEdit {
        seq: u64,
        payload: serde_json::Value,
    },
    /// Deletes every event after `keep_through`.
    Truncate {
        keep_through: u64,
    },
    /// Flips bytes in a stored body, leaving its events untouched.
    CorruptBody(Hash),
    /// Replaces a live view document's body, leaving the events untouched,
    /// so a rebuild that copies live rows is caught.
    AlterDocument {
        view: String,
        key: DocKey,
        body: serde_json::Value,
    },
}
