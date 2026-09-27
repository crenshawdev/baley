//! The port's traits (design 0001, The storage port, Figure 4).
//!
//! Shaped around Baley's access patterns, not a generic repository: one
//! command's decision inside one write transaction, a stream's events, a
//! project's history, view reads by key or declared index, payloads by hash,
//! and the owner's whole-store operations.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::anchor::anchor_tag;
use crate::chain::{Anchor, ChainReport, Head};
use crate::claim::{
    Claim, ClaimDecision, ClaimId, ClaimOwner, Claimed, ReconcileAuthority, Reconciliation,
};
use crate::command::{Command, Decision, EventMatch, NewEvent, Recorded, StreamName};
use crate::error::StoreError;
use crate::event::{Event, Hash, ProjectId};
use crate::payload::{PayloadRef, PayloadReference, RetentionClass};
use crate::view::{DocKey, Document, IndexQuery, Page, PageRequest};

/// A command's decision: runs once inside the write transaction, reads its
/// inputs there, appends events, and returns the outcome with the inputs
/// the caller's slow work observed.
pub type Decide<'a> = dyn FnMut(&mut dyn Transaction) -> Result<Decision, StoreError> + 'a;

/// The decision that takes a claim, or refuses on the merits.
pub type DecideClaim<'a> =
    dyn FnMut(&mut dyn Transaction) -> Result<ClaimDecision, StoreError> + 'a;

/// The decision that reconciles an interrupted claim from the real state
/// the caller read.
pub type DecideReconcile<'a> =
    dyn FnMut(&mut dyn Transaction, &Claim) -> Result<Reconciliation, StoreError> + 'a;

/// Which event types and versions this binary can read. Implemented by the
/// core's registry and handed to the store when it opens, so the store can
/// fence a project before its next write (EVD-R19).
pub trait EventSchema: Send + Sync {
    fn reads(&self, type_name: &str, version: u32) -> bool;

    /// The version and payload a projector sees for a stored event: the
    /// type's current version and the payload upcast to it. Ordinary
    /// projection and replay both hand projectors a copy of the event with
    /// these in place; the stored event, its hash and its references never
    /// change. The default reads an event only at its own version, and only
    /// when `reads` holds; a schema with upcasters overrides it, and must
    /// still refuse every event `reads` refuses, since replay relies on this
    /// error to stop at an event the binary cannot read. The error says why
    /// the event cannot be read.
    fn projection_payload(&self, event: &Event) -> Result<(u32, Value), String> {
        if self.reads(&event.type_name, event.type_version) {
            Ok((event.type_version, event.payload.clone()))
        } else {
            Err(format!(
                "{} version {} is not readable by this binary",
                event.type_name, event.type_version
            ))
        }
    }
}

/// The ledger: commands in, events and chain reports out.
pub trait Ledger {
    /// Runs one database-only command: takes the writer queue, opens the
    /// transaction, checks the epoch, the project's readability and the
    /// request, calls `decide`, re-checks what it observed, runs the
    /// projectors, records `command.completed` and commits, or records
    /// nothing (EVD-R5, R6, R7). Commands with an external effect use
    /// `claim`, `complete` and `reconcile` instead.
    fn transact(&self, command: &Command, decide: &mut Decide<'_>) -> Result<Recorded, StoreError>;

    /// The claim step of a command with an external effect (EVD-R26):
    /// checks the request and scope, then records `command.claimed` and
    /// takes the lease, or completes a refusal. A retry gets `InProgress`
    /// or its recorded outcome without writing.
    fn claim(&self, command: &Command, decide: &mut DecideClaim<'_>)
    -> Result<Claimed, StoreError>;

    /// Renews a claimed request's lease at a supplied UTC time without an
    /// event. The owner must match and the time cannot precede the matching
    /// row's time, or the claim time when no row matches.
    fn renew_lease(
        &self,
        project: &ProjectId,
        claim: &ClaimId,
        owner: &ClaimOwner,
        at: &str,
    ) -> Result<(), StoreError>;

    /// The acting owner's record step: checks the claimed request, digest,
    /// owner and scope, then records the result and closes the claim. An
    /// expired lease does not prevent its owner from recording first.
    fn complete(
        &self,
        command: &Command,
        owner: &ClaimOwner,
        decide: &mut Decide<'_>,
    ) -> Result<Recorded, StoreError>;

    /// Reconciles an interrupted claim under a separate command. It records
    /// the finding and the reconciler's receipt, and either completes the
    /// claim or holds it for the owner. Only an owner actor with owner
    /// authority may resolve a held claim.
    fn reconcile(
        &self,
        command: &Command,
        claim: &ClaimId,
        authority: ReconcileAuthority,
        decide: &mut DecideReconcile<'_>,
    ) -> Result<Recorded, StoreError>;

    /// The project's claimed and awaiting-owner requests, oldest first,
    /// joined with matching lease rows for caller-driven reconciliation.
    fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError>;

    /// A stream's events from `from_version` on, in stream order, at most
    /// `min(limit, EVENT_PAGE_BOUND)` per page with a cursor bound to this query. Events
    /// come back exactly as stored, never upcast.
    fn stream(
        &self,
        project: &ProjectId,
        stream: &StreamName,
        from_version: u64,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError>;

    /// A project's events in `range` that pass `filter`, in sequence order,
    /// paged and returned as `stream` returns them.
    fn history(
        &self,
        project: &ProjectId,
        range: RangeInclusive<u64>,
        filter: &HistoryFilter,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError>;

    /// The project's last event, or `None` for an empty chain. An absent
    /// project is `UnknownProject`.
    fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError>;

    /// Verifies the project's whole chain against `anchor`, which the caller
    /// fetched from the forge (the store never runs git), then hashes every
    /// present payload body and retained excerpt the project references,
    /// counting reduced and purged bodies as tombstones. Reports the latest
    /// local anchor row beside the supplied anchor without trusting it, and
    /// the time the unanchored age runs from in the chain report. Reads one snapshot, holding
    /// no more than one page of rows or one chunk of a body at a time.
    fn verify(
        &self,
        project: &ProjectId,
        anchor: Option<&Anchor>,
    ) -> Result<VerifyReport, StoreError>;
}

/// The most events one `stream` or `history` page holds.
pub const EVENT_PAGE_BOUND: u32 = 100;

/// Which events `history` returns: any of `types` (empty means every
/// type), and of those only the ones that recorded `git_commit` when it is
/// set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    /// Event types, any of which passes.
    pub types: Vec<String>,
    /// Events that recorded this commit, for `why`.
    pub git_commit: Option<String>,
}

/// What `verify` found: the chain's verdict against the supplied anchor,
/// every referenced body that failed its hash, and how the latest local
/// anchor row compares with the supplied anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// The chain's verdict against the supplied anchor.
    pub chain: ChainReport,
    /// Every referenced body or excerpt that is missing or corrupt.
    pub payloads: Vec<PayloadFault>,
    /// Bodies and excerpts opened and hashed.
    pub bodies_checked: u64,
    /// Reduced or purged bodies found as valid tombstones.
    pub tombstones_checked: u64,
    /// The project's latest local anchor row, if any. A cache of what the
    /// record step confirmed, never the outside witness.
    pub stored_anchor: Option<StoredAnchor>,
    /// How that row compares with the supplied anchor.
    pub stored_anchor_comparison: StoredAnchorComparison,
}

/// The caller's observation of the configured remote anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorCheck {
    /// The remote's latest valid anchor.
    Remote(Anchor),
    /// The remote confirmed no anchor.
    RemoteAbsent,
    /// The remote could not be reached.
    RemoteUnreachable,
    /// The remote returned malformed anchor data.
    RemoteMalformed(String),
    /// This project has no configured remote.
    LocalOnly,
}

/// A local anchor row: the anchor, its tag and when Baley confirmed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAnchor {
    /// The anchored sequence and head.
    pub anchor: Anchor,
    /// The confirmed tag.
    pub tag: String,
    /// When Baley confirmed it, a UTC time.
    pub pushed_at: String,
}

/// How the latest local anchor row compares with the anchor fetched from
/// the remote. The remote is the witness; a difference is reported, never
/// repaired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredAnchorComparison {
    /// No anchor was supplied.
    NotCompared,
    /// The row names the supplied anchor's sequence, hash and tag.
    Matches,
    /// No local row, although the remote holds an anchor.
    MissingLocal,
    /// The row is older than the remote anchor, as when a push landed
    /// before its record step.
    LocalBehind,
    /// The row is newer than the remote anchor.
    LocalAhead,
    /// The row names the same sequence with another hash or tag.
    Conflict,
}

/// Compares the latest local row with the supplied anchor of `project`.
/// The expected tag comes from the supplied anchor's sequence.
pub fn compare_stored_anchor(
    project: &ProjectId,
    supplied: Option<&Anchor>,
    stored: Option<&StoredAnchor>,
) -> StoredAnchorComparison {
    let Some(supplied) = supplied else {
        return StoredAnchorComparison::NotCompared;
    };
    let Some(stored) = stored else {
        return StoredAnchorComparison::MissingLocal;
    };
    match stored.anchor.seq.cmp(&supplied.seq) {
        std::cmp::Ordering::Less => StoredAnchorComparison::LocalBehind,
        std::cmp::Ordering::Greater => StoredAnchorComparison::LocalAhead,
        std::cmp::Ordering::Equal
            if stored.anchor.hash == supplied.hash
                && stored.tag == anchor_tag(project, supplied.seq) =>
        {
            StoredAnchorComparison::Matches
        }
        std::cmp::Ordering::Equal => StoredAnchorComparison::Conflict,
    }
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

    /// The project's open claims with their matching leases, including
    /// documents staged earlier in this transaction.
    fn open_claims(&mut self) -> Result<Vec<Claim>, StoreError>;

    /// The project's head after events appended in this transaction, or
    /// `None` when it has no events. A claim decision sees the pre-claim head.
    fn head(&mut self) -> Result<Option<Head>, StoreError>;

    /// Writes the project's anchor row for `anchor` in this transaction, so
    /// it commits or rolls back with the result event. An `anchor.pushed`
    /// event appended earlier in this transaction must carry exactly these
    /// values, and `tag` must be the anchor's tag in the command's project.
    /// `observed_at` becomes the row's `pushed_at`. A row already stored
    /// for the same sequence must agree in every value, or the command is
    /// refused and nothing is written.
    fn record_anchor(
        &mut self,
        anchor: &Anchor,
        tag: &str,
        remote: &str,
        observed_at: &str,
    ) -> Result<(), StoreError>;
}

/// View reads outside a transaction. One query runs against one snapshot,
/// in which the project's live views are also checked: built by a newer
/// binary, the read is refused as read-only; built by an older one, they
/// are rebuilt forward first.
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
    fn create_project(&self, project: &ProjectId, name: &str, at: &str) -> Result<(), StoreError>;

    /// Lists project ids and names in id order.
    fn projects(&self) -> Result<Vec<(ProjectId, String)>, StoreError>;

    /// Writes a standalone store holding one project's events, views,
    /// payloads and anchors, which verifies on its own (EVD-R15).
    fn export(
        &self,
        project: &ProjectId,
        target: &Path,
        at: &str,
    ) -> Result<ExportReport, StoreError>;

    /// Reduces one reference whose retention has ended: keeps the first and
    /// last 64 KiB of its body as a new payload and records
    /// `payload.reduced`. The original body is tombstoned only when no
    /// other reference still requires it whole.
    fn reduce(
        &self,
        command: &Command,
        reference: &PayloadReference,
    ) -> Result<PayloadRef, StoreError>;

    /// Removes the bodies no remaining reference requires, and every copy
    /// the store manages, records `payload.purged` in this project's chain,
    /// then scrubs the file.
    fn purge(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
    ) -> Result<PurgeReport, StoreError>;

    /// Repeats the idempotent scrub.
    fn scrub(&self) -> Result<ScrubReport, StoreError>;

    /// Replays the project's events into a new generation of every
    /// registered view, in short batches that yield the writer queue, then
    /// applies the remaining tail and makes the generation live for every
    /// view at once in one transaction. Commands keep writing the live
    /// generation meanwhile, and the tail catches them up. The old
    /// generation is removed before this returns; a failure there comes
    /// back as `StoreError::CleanupFailed`, naming the generation already
    /// live. Views only rebuild forward: a project whose live views a newer
    /// binary built is refused as read-only. A rebuild never removes the
    /// live generation: a building marker that names it is refused as
    /// `StoreError::LiveGenerationProtected`, with nothing removed.
    fn rebuild(&self, project: &ProjectId) -> Result<RebuildReport, StoreError>;

    /// Replays the project's events into a scratch generation that never
    /// becomes live, compares every registered view's stored rows with the
    /// live generation's at one head, then removes the scratch rows,
    /// including after an error. Views behind this binary's are first
    /// rebuilt forward, as a rebuild would; apart from that it changes no
    /// live row, event or payload. Returns
    /// `StoreError::UnfinishedGeneration` while a rebuild or verification
    /// that never finished has left its generation behind, and whenever
    /// removing its own scratch generation fails, naming that generation,
    /// whether or not the comparison failed too. Only when the scratch
    /// generation is removed does the comparison's report or error come
    /// back. A building marker that names the live generation is refused
    /// as `StoreError::LiveGenerationProtected`, as a rebuild refuses it,
    /// with nothing removed.
    fn verify_views(&self, project: &ProjectId) -> Result<ViewsReport, StoreError>;

    /// The store's health as of `at`, a supplied UTC time, with each
    /// project's chain verified against its supplied anchor check.
    fn doctor(
        &self,
        at: &str,
        checks: &BTreeMap<ProjectId, AnchorCheck>,
    ) -> Result<Health, StoreError>;
}

/// A verified standalone project home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// The directory created for the export.
    pub target: PathBuf,
    /// The exported project's verified head.
    pub head: Option<Head>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeReport {
    /// Bodies removed.
    pub purged: Vec<Hash>,
    /// Hashes released or requested whose body or excerpt is still required
    /// by another reference, in any project.
    pub shared: Vec<Hash>,
    /// The `payload.purged` event in the purging project.
    pub recorded: Vec<(ProjectId, u64)>,
    /// Exports that already received a purged or shared body, from the export
    /// records; a pending export is listed.
    pub unreachable: Vec<PathBuf>,
    /// Whether the main database scrub completed.
    pub scrubbed: bool,
}

/// The standalone scrub's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubReport {
    /// Whether the main database scrub completed.
    pub scrubbed: bool,
}

/// What a rebuild made live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildReport {
    /// The generation now live.
    pub generation: u64,
    /// Every event replayed into it, the final tail included: the head it
    /// flipped at, for a chain without gaps.
    pub events: u64,
}

/// What a view verification found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewsReport {
    /// The project head both generations were compared at.
    pub checked_seq: u64,
    /// (view, key) of every document that is missing, extra or unequal in
    /// the live generation against its replay, sorted by view and key.
    pub differing: Vec<(String, DocKey)>,
}

/// What `doctor` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// Compatibility epoch recorded in the database.
    pub epoch: u32,
    /// Time of a logical purge whose scrub has not finished.
    pub scrub_pending: Option<String>,
    /// Every row returned by SQLite's integrity check.
    pub integrity: Vec<String>,
    /// Main database file size.
    pub database_bytes: u64,
    /// Write-ahead log file size, or zero when absent.
    pub log_bytes: u64,
    /// Health of each project, in id order.
    pub projects: Vec<ProjectHealth>,
}

/// Store and remote findings for one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectHealth {
    /// Project id.
    pub project: ProjectId,
    /// Caller-supplied remote observation.
    pub check: AnchorCheck,
    /// Chain and body verification against that observation.
    pub verify: Result<VerifyReport, StoreError>,
    /// A local row despite confirmed remote absence.
    pub remote_absent_local_row: Option<StoredAnchor>,
    /// Age of unanchored work.
    pub unanchored: UnanchoredAge,
    /// Raw version of each registered view in the live generation.
    pub views: Vec<ViewHealth>,
    /// Live view-set version and this binary's version.
    pub view_set: (Option<u32>, u32),
    /// Unfinished building generation, if any.
    pub building: Option<Building>,
    /// Replay verification of the views.
    pub views_check: Result<ViewsReport, StoreError>,
    /// Open claims judged at the supplied time.
    pub claims: Result<ClaimCounts, StoreError>,
}

/// Whether the unanchored age could be checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnanchoredAge {
    /// No trustworthy remote observation was available.
    Unchecked,
    /// No unanchored work event was accepted.
    None,
    /// The first work event and whether it is more than one day old.
    Since { since: String, warning: bool },
}

/// One registered view's stored and current version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewHealth {
    /// View name.
    pub view: String,
    /// Version stamped on the live generation.
    pub live_version: Option<u32>,
    /// Version in this binary.
    pub binary_version: u32,
}

/// Progress of an unfinished generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Building {
    /// Building generation number.
    pub generation: u64,
    /// Last event applied in the building generation.
    pub applied_seq: u64,
    /// Events behind the project head.
    pub lag: u64,
}

/// Counts of claims at the supplied time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimCounts {
    /// Claims with a live lease.
    pub active: u64,
    /// Claims with an expired lease.
    pub interrupted: u64,
    /// Claims held for owner resolution.
    pub awaiting_owner: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectId {
        ProjectId("p".into())
    }

    fn anchor(seq: u64, byte: u8) -> Anchor {
        Anchor {
            seq,
            hash: Hash([byte; 32]),
        }
    }

    fn row(seq: u64, byte: u8, tag: &str) -> StoredAnchor {
        StoredAnchor {
            anchor: anchor(seq, byte),
            tag: tag.into(),
            pushed_at: "2026-09-25T18:00:00Z".into(),
        }
    }

    // With no supplied anchor there is nothing to compare, even when a row
    // exists. Catches a local row reported as agreeing with a witness that
    // was never fetched.
    #[test]
    fn no_supplied_anchor_is_not_compared() {
        let stored = row(3, 1, "baley-anchor/p/3");
        assert_eq!(
            compare_stored_anchor(&project(), None, Some(&stored)),
            StoredAnchorComparison::NotCompared
        );
    }

    // The same sequence, hash and tag match; a missing row is reported as
    // missing. Catches a match on sequence alone.
    #[test]
    fn a_row_matches_only_with_its_hash_and_tag() {
        let remote = anchor(3, 1);
        assert_eq!(
            compare_stored_anchor(
                &project(),
                Some(&remote),
                Some(&row(3, 1, "baley-anchor/p/3"))
            ),
            StoredAnchorComparison::Matches
        );
        assert_eq!(
            compare_stored_anchor(&project(), Some(&remote), None),
            StoredAnchorComparison::MissingLocal
        );
    }

    // A row at the remote's sequence with another hash, or with another
    // tag, is a conflict. Catches a row silently trusted.
    #[test]
    fn a_same_sequence_row_that_differs_conflicts() {
        let remote = anchor(3, 1);
        for stored in [row(3, 2, "baley-anchor/p/3"), row(3, 1, "baley-anchor/q/3")] {
            assert_eq!(
                compare_stored_anchor(&project(), Some(&remote), Some(&stored)),
                StoredAnchorComparison::Conflict
            );
        }
    }

    // A row older or newer than the remote is reported as behind or ahead.
    // Catches the local row taken for the witness.
    #[test]
    fn an_older_or_newer_row_is_behind_or_ahead() {
        let remote = anchor(5, 1);
        assert_eq!(
            compare_stored_anchor(
                &project(),
                Some(&remote),
                Some(&row(3, 1, "baley-anchor/p/3"))
            ),
            StoredAnchorComparison::LocalBehind
        );
        assert_eq!(
            compare_stored_anchor(
                &project(),
                Some(&remote),
                Some(&row(7, 1, "baley-anchor/p/7"))
            ),
            StoredAnchorComparison::LocalAhead
        );
    }
}
