//! Commands, their decisions and their recorded outcomes (design 0001,
//! Commands; EVD-R5, R6, R7).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::chain::Head;
use crate::event::{Actor, GitFacts, Hash, ProjectId, RequestId};
use crate::payload::PayloadRef;
use crate::view::{DocKey, KeyValue};

/// A stream name such as `plan/5-2`: the events of one thing that changes
/// over time, with its own version counter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamName(pub String);

/// The kind of a command, such as `plan.approve`. Request ids are scoped to
/// (project, kind), so two kinds never share an answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandKind(pub String);

/// One request from a caller that may record events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub project: ProjectId,
    pub kind: CommandKind,
    /// The caller's fresh UUID.
    pub request_id: RequestId,
    /// SHA-256 of the canonical form of the kind and every field that
    /// carries authority. The same id with another digest is refused.
    pub digest: Hash,
    /// Stamped on every event the command appends.
    pub policy_version: u64,
    /// The supplied time every event of the command is recorded at: UTC,
    /// RFC 3339. Never read from a live clock inside the store.
    pub recorded_at: String,
    pub actor: Actor,
}

/// An event as a decision appends it. The transaction adds the rest from
/// the command and the ledger: sequence, stream version, request id,
/// policy version, time and the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub stream: StreamName,
    pub type_name: String,
    pub type_version: u32,
    pub git: Option<GitFacts>,
    /// Canonical-JSON-able: integers within ±(2^53 − 1), no floats.
    pub payload: Value,
    /// Every payload this event references. Each also appears in `payload`
    /// as its reference object; the store records one reference per entry.
    pub attachments: Vec<PayloadRef>,
}

/// A match against recorded events, for authority checks and absence
/// assertions: the type, optionally the stream, and top-level payload
/// fields that must be equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventMatch {
    pub type_name: String,
    pub stream: Option<StreamName>,
    pub fields: BTreeMap<String, Value>,
}

/// Something the caller relied on not existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Absence {
    /// No recorded event matches.
    Event(EventMatch),
    /// No document of `view` has these values for a leading prefix of the
    /// declared index's fields, such as "no active dispatch for phase 3".
    Documents {
        view: String,
        index: String,
        equals: Vec<KeyValue>,
    },
}

/// A view document as the caller saw it during its slow work, before the
/// transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedDocument {
    pub view: String,
    pub key: DocKey,
    /// The `produced_seq` it had, or `None` if it was absent.
    pub produced_seq: Option<u64>,
}

/// The cheap git facts of one checkout (EVD-R4, R7): what the caller's slow
/// work saw, and what the decision re-read through the core's git seam
/// inside the transaction. The store never runs git; it compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitObservation {
    pub checkout: String,
    pub head_seen: String,
    pub head_now: String,
    /// A fingerprint of the checkout's index, so staged changes under an
    /// unchanged HEAD are caught.
    pub index_seen: String,
    pub index_now: String,
}

/// Every input the caller's slow work depended on. The store re-checks
/// each one inside the transaction and refuses the command as stale if any
/// moved (EVD-R7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    pub documents: Vec<ObservedDocument>,
    pub absences: Vec<Absence>,
    pub git: Vec<GitObservation>,
}

/// Whether the command did what it was asked or refused on the merits.
/// Both are domain outcomes and both are recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Done,
    Refused,
}

/// What a decision returns: the outcome, the answer the caller receives,
/// and the inputs to re-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub kind: OutcomeKind,
    pub answer: Value,
    /// Sensitive answers are stored as a payload at any size.
    pub sensitive: bool,
    pub observed: Observed,
}

/// An answer as `command.completed` holds it: inline when small and not
/// sensitive, otherwise a `record` payload reference. A retry after a purge
/// finds the reference and the payload's tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Inline(Value),
    Stored(PayloadRef),
}

/// A recorded outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub kind: OutcomeKind,
    pub answer: Answer,
}

/// What `transact` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// The decision ran and committed; `head` is the project's new head.
    New { outcome: Outcome, head: Head },
    /// The request id was answered before with the same digest. Nothing was
    /// recorded; this is the original outcome.
    Replayed { outcome: Outcome },
}
