//! The port's traits (design 0001, The storage port, Figure 4).
//!
//! Shaped around Baley's access patterns, not a generic repository: one
//! command's decision inside one write transaction, a stream's events, a
//! project's history, view reads by key or declared index, payloads by hash,
//! and the owner's whole-store operations.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::chain::{Anchor, ChainReport, Head};
use crate::command::{Command, Decision, EventMatch, NewEvent, Recorded, StreamName};
use crate::error::StoreError;
use crate::event::{Event, Hash, ProjectId};
use crate::payload::{PayloadRef, RetentionClass};
use crate::view::{DocKey, Document, IndexQuery, Page, PageRequest};

/// A command's decision: runs once inside the write transaction, reads its
/// inputs there, appends events, and returns the outcome with the inputs
/// the caller's slow work observed.
pub type Decide<'a> = dyn FnMut(&mut dyn Transaction) -> Result<Decision, StoreError> + 'a;

/// The ledger: commands in, events and chain reports out.
pub trait Ledger {
    /// Runs one command: takes the writer queue, opens the transaction,
    /// checks the epoch and the request, calls `decide`, re-checks what it
    /// observed, runs the projectors, records `command.completed` and
    /// commits, or records nothing (EVD-R5, R6, R7).
    fn transact(&self, command: &Command, decide: &mut Decide<'_>) -> Result<Recorded, StoreError>;

    /// A stream's events from `from_version` on, in stream order.
    fn stream(
        &self,
        project: &ProjectId,
        stream: &StreamName,
        from_version: u64,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError>;

    /// A project's events in `range` that pass `filter`, in sequence order.
    fn history(
        &self,
        project: &ProjectId,
        range: RangeInclusive<u64>,
        filter: &HistoryFilter,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError>;

    /// The project's last event, or `None` for an empty chain.
    fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError>;

    /// Verifies the project's chain against `anchor`, which the caller
    /// fetched from the forge (the store never runs git), then hashes every
    /// present payload body and retained excerpt the project references.
    fn verify(
        &self,
        project: &ProjectId,
        anchor: Option<&Anchor>,
    ) -> Result<VerifyReport, StoreError>;
}

/// Which events `history` returns. Empty means every event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    pub types: Vec<String>,
    /// Events that recorded this commit, for `why`.
    pub git_commit: Option<String>,
}

/// The chain's verdict and every payload that failed its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub chain: ChainReport,
    pub payloads: Vec<PayloadFault>,
    /// Bodies and excerpts opened and hashed.
    pub bodies_checked: u64,
}

/// A referenced body that is not what its hash says. A reduced or purged
/// body is a valid tombstone, never a fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadFault {
    Missing(Hash),
    Corrupt(Hash),
}

/// What a decision can do inside the write transaction. Every read sees the
/// transaction's own writes so far.
pub trait Transaction {
    fn get(&mut self, view: &str, key: &DocKey) -> Result<Option<Document>, StoreError>;

    fn find(&mut self, view: &str, query: &IndexQuery) -> Result<Page<Document>, StoreError>;

    /// Whether a matching event exists in the command's project. Decisions
    /// that grant authority confirm their deciding facts here, never from a
    /// view (EVD-R27).
    fn event_exists(&mut self, matching: &EventMatch) -> Result<bool, StoreError>;

    /// Refuses the command as stale unless `stream` is at `version`, so two
    /// contested decisions are ordered by one counter.
    fn expect(&mut self, stream: &StreamName, version: u64) -> Result<(), StoreError>;

    /// Appends an event to the command's project and returns its sequence.
    fn append(&mut self, event: NewEvent) -> Result<u64, StoreError>;

    /// Stores a body once by hash and returns the reference an event will
    /// carry.
    fn put_payload(
        &mut self,
        bytes: &[u8],
        class: RetentionClass,
    ) -> Result<PayloadRef, StoreError>;
}

/// View reads outside a transaction. One query runs against one snapshot.
pub trait Views {
    fn get(
        &self,
        project: &ProjectId,
        view: &str,
        key: &DocKey,
    ) -> Result<Option<Document>, StoreError>;

    fn find(
        &self,
        project: &ProjectId,
        view: &str,
        query: &IndexQuery,
    ) -> Result<Page<Document>, StoreError>;
}

/// What the owner does to the store as a whole.
pub trait Admin {
    /// Creates an empty project. Slice 1 creates projects only this way;
    /// `baley init` arrives with slice 2.
    fn create_project(&self, project: &ProjectId, name: &str) -> Result<(), StoreError>;

    /// Copies the store into `dir` after the integrity check, chain
    /// verification and view verification all pass.
    fn backup(&self, dir: &Path) -> Result<BackupReport, StoreError>;

    /// Writes a standalone store holding one project's events, views,
    /// payloads and anchors, which verifies on its own (EVD-R15).
    fn export(&self, project: &ProjectId, target: &Path) -> Result<(), StoreError>;

    /// Keeps the first and last 64 KiB of a body as a new payload, records
    /// `payload.reduced` and tombstones the original.
    fn reduce(&self, command: &Command, hash: &Hash) -> Result<PayloadRef, StoreError>;

    /// Removes the bodies no remaining reference requires, and every copy
    /// the store manages, records `payload.purged` in every affected
    /// project, then scrubs the file.
    fn purge(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
    ) -> Result<PurgeReport, StoreError>;

    /// Replays the project's events into a new generation of every view
    /// and makes it live at once.
    fn rebuild(&self, project: &ProjectId) -> Result<RebuildReport, StoreError>;

    /// Rebuilds the project's views into a scratch generation and reports
    /// every document that differs from the live one.
    fn verify_views(&self, project: &ProjectId) -> Result<ViewsReport, StoreError>;

    /// The store's health as of `at`, a supplied UTC time.
    fn doctor(&self, at: &str) -> Result<Health, StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeReport {
    /// Bodies removed.
    pub purged: Vec<Hash>,
    /// Bodies kept because another project still requires them.
    pub shared: Vec<Hash>,
    /// The `payload.purged` event recorded in each affected project.
    pub recorded: Vec<(ProjectId, u64)>,
    /// Exports outside the home the purge cannot reach.
    pub unreachable: Vec<PathBuf>,
    /// Whether the scrub's last truncating checkpoint left no copy behind.
    pub scrubbed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildReport {
    pub generation: u64,
    pub events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewsReport {
    /// (view, key) of every document that differs from its rebuild.
    pub differing: Vec<(String, DocKey)>,
}

/// What `doctor` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub integrity_ok: bool,
    pub epoch: u32,
    pub database_bytes: u64,
    pub log_bytes: u64,
    /// Bytes by record family, then by retention class.
    pub bytes_by_family: BTreeMap<String, u64>,
    pub bytes_by_class: BTreeMap<String, u64>,
    pub backups: Vec<PathBuf>,
    pub projects: Vec<ProjectHealth>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectHealth {
    pub project: ProjectId,
    pub chain: ChainReport,
    /// (view, stored projector version, events behind the head).
    pub views: Vec<(String, u32, u64)>,
    pub active_claims: u64,
    pub interrupted_claims: u64,
    /// When the oldest unanchored event was recorded, if any.
    pub unanchored_since: Option<String>,
    /// Set when the unanchored range is more than a day old at `at`.
    pub unanchored_warning: bool,
}
