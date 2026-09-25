//! What can go wrong at the port, and what the caller does about it.
//!
//! None of these is a domain outcome. A domain outcome, success or a real
//! refusal, is recorded as `command.completed` and comes back as `Ok`
//! (design 0001, Commands). Apart from `CleanupFailed`, these record
//! nothing: the caller re-reads and retries with the same request id,
//! waits, or stops.

use std::fmt;

use crate::claim::ClaimId;
use crate::command::{Absence, StreamName};
use crate::event::{Hash, ProjectId, RequestId};
use crate::payload::PayloadReference;
use crate::view::DocKey;

/// Why a port operation did not return its result. Every variant but
/// `CleanupFailed` means the operation recorded nothing; `CleanupFailed`
/// comes after a rebuild whose new generation is already live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Something the decision depended on changed between the caller's slow
    /// work and the transaction (EVD-R7). Re-read and retry with the same
    /// request id.
    Stale(StaleInput),
    /// The writer queue or the database lock was not free within the
    /// backstop timeout. Retry later.
    Busy,
    /// The store's compatibility epoch is newer than this binary's. Reads
    /// still work; writes need a binary at `needed_epoch` or later.
    ReadOnly { needed_epoch: u32 },
    /// The port refused the request itself.
    Refused(Refusal),
    /// A projector could not apply an event, so the whole command was
    /// rolled back (EVD-R5).
    Projector {
        view: String,
        seq: u64,
        message: String,
    },
    /// The engine failed: I/O, a full disk, a corrupt file.
    Unavailable(String),
    /// A rebuild made `live_generation` live, and then removing the old
    /// generation failed with `cause`. The flip stands: reads and commands
    /// use the new generation, and the next rebuild removes what is left.
    CleanupFailed {
        project: ProjectId,
        live_generation: u64,
        cause: Box<StoreError>,
    },
    /// A rebuild or view verification that never finished, as after a
    /// crash, left `generation` behind. Views cannot be verified until a
    /// rebuild removes it. Nothing live was changed.
    UnfinishedGeneration { project: ProjectId, generation: u64 },
}

/// The input that moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleInput {
    /// A view document is not at the sequence the caller saw. `None` on
    /// either side means absent.
    Document {
        view: String,
        key: DocKey,
        seen: Option<u64>,
        now: Option<u64>,
    },
    /// Something the caller saw absent now exists.
    Absence(Absence),
    /// A git fact of a checkout moved.
    Git {
        checkout: String,
        fact: GitFact,
        seen: String,
        now: String,
    },
    /// The claim being completed or reconciled is no longer open: another
    /// process completed or reconciled it. Its outcome is in the request.
    Claim(ClaimId),
    /// `expect` named a stream version the stream has moved past.
    StreamVersion {
        stream: StreamName,
        expected: u64,
        actual: u64,
    },
}

/// Why the port refused a request outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No such project in this store.
    UnknownProject(ProjectId),
    /// The project exists already.
    ProjectExists(ProjectId),
    /// This binary must not write to the project: it holds an event type or
    /// version this binary cannot read, or its live views were built by a
    /// newer projector or view set, which refuses this binary's reads of
    /// those views as well (EVD-R19). History and payload reads still work.
    ProjectReadOnly { project: ProjectId, reason: String },
    /// The request id was used before for this project and command kind
    /// with a different digest (EVD-R6).
    RequestDigestMismatch { request_id: RequestId },
    /// No view of that name is registered.
    UnknownView(String),
    /// The view declares no index of that name; `find` never scans.
    UndeclaredIndex { view: String, index: String },
    /// A key or index value does not fit the declared fields. At open, also
    /// a view declared badly, or a view spec or view set changed under a
    /// version already recorded: `view` then names the view whose spec
    /// changed, or one that is in only one of the two sets.
    MalformedKey { view: String, reason: String },
    /// No payload with that hash is stored.
    UnknownPayload(Hash),
    /// Bytes whose hash was reduced or purged cannot be stored again: the
    /// new reference would point at a body that is gone, and storing it anew
    /// would bring the purged content back for the old references.
    PayloadTombstoned(Hash),
    /// The reference is outside the command's project, absent, not `output`,
    /// or released; or its body is absent or at most 128 KiB.
    NotReducible(PayloadReference),
    /// The project has no reference to this hash, or has released all of
    /// them and every excerpt its own reductions attached.
    NothingToPurge(Hash),
    /// A cursor not issued for this query, project, generation and view
    /// version, or not a cursor at all.
    InvalidCursor,
    /// The event cannot be sealed: see the reason.
    InvalidEvent(String),
    /// A decision appended an event type or version this binary cannot
    /// read, so it would fence its own project.
    UnreadableType { type_name: String, version: u32 },
    /// A decision stored a payload that no event of its command attaches.
    /// The body would sit outside the chain, where no reference names it
    /// and no purge reaches it.
    UnattachedPayload(Hash),
    /// No claim with that id was ever taken in the project.
    UnknownClaim(ClaimId),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(input) => write!(f, "stale input, re-read and retry: {input:?}"),
            Self::Busy => f.write_str("store busy, retry"),
            Self::ReadOnly { needed_epoch } => {
                write!(
                    f,
                    "store is read-only for this binary; epoch {needed_epoch} is needed"
                )
            }
            Self::Refused(refusal) => write!(f, "refused: {refusal:?}"),
            Self::Projector { view, seq, message } => {
                write!(f, "projector for {view} failed at event {seq}: {message}")
            }
            Self::Unavailable(reason) => write!(f, "store unavailable: {reason}"),
            Self::CleanupFailed {
                project,
                live_generation,
                cause,
            } => write!(
                f,
                "project {} now reads generation {live_generation}; removing the old generation failed and is left for the next rebuild: {cause}",
                project.0
            ),
            Self::UnfinishedGeneration {
                project,
                generation,
            } => write!(
                f,
                "project {} holds generation {generation} of a rebuild or view verification that never finished; rebuild the project first",
                project.0
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// Which git fact moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFact {
    Head,
    Index,
}
