//! What can go wrong at the port, and what the caller does about it.
//!
//! None of these is a domain outcome. A domain outcome, success or a real
//! refusal, is recorded as `command.completed` and comes back as `Ok`
//! (design 0001, Commands). These record nothing: the caller re-reads and
//! retries with the same request id, waits, or stops.

use std::fmt;

use crate::command::{Absence, StreamName};
use crate::event::{Hash, ProjectId, RequestId};
use crate::view::DocKey;

/// A port operation that recorded nothing.
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
    /// The checkout's HEAD moved.
    Head { seen: String, now: String },
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
    /// This binary must not write to the project: an event type or version
    /// it cannot read, or a view written by a newer projector (EVD-R19).
    ProjectReadOnly { project: ProjectId, reason: String },
    /// The request id was used before for this project and command kind
    /// with a different digest (EVD-R6).
    RequestDigestMismatch { request_id: RequestId },
    /// No view of that name is registered.
    UnknownView(String),
    /// The view declares no index of that name; `find` never scans.
    UndeclaredIndex { view: String, index: String },
    /// A key or index value does not fit the declared fields.
    MalformedKey { view: String, reason: String },
    /// No payload with that hash is stored.
    UnknownPayload(Hash),
    /// A cursor this store did not issue, or issued for another query.
    InvalidCursor,
    /// The event cannot be sealed: see the reason.
    InvalidEvent(String),
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
        }
    }
}

impl std::error::Error for StoreError {}
