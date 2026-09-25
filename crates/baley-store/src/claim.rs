//! Commands with an external effect: claim, act, record (design 0001,
//! Commands; EVD-R26).
//!
//! A claim is the intent event recorded before the effect runs. Its lease is
//! liveness, not evidence: it lives outside the chain, and renewing it
//! records nothing. Whether a claim is active or interrupted, and whether it
//! blocks a command, are the core's decisions over the facts here and a
//! supplied time.

use serde_json::Value;

use crate::command::{CommandKind, Decision, Observed, Outcome};
use crate::event::RequestId;

/// Who holds a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOwner {
    pub process: String,
    pub host_session: String,
    /// UTC, RFC 3339.
    pub started_at: String,
}

/// A claim's identity within its project: the command kind and request id
/// that took it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimId {
    pub kind: CommandKind,
    pub request_id: RequestId,
}

/// A claim not yet completed, active or interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: ClaimId,
    /// The sequence of its `command.claimed` event.
    pub seq: u64,
    /// The command's inputs and intended effect.
    pub intent: Value,
    /// What it blocks while open, for example `anchor` or `phase/3`.
    pub scope: Vec<String>,
    pub owner: ClaimOwner,
    /// When the lease was last renewed. Expired 60 s after this.
    pub lease_renewed_at: String,
}

/// What a claim decision returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDecision {
    /// Record `command.claimed` and take the lease.
    Claim {
        intent: Value,
        scope: Vec<String>,
        owner: ClaimOwner,
        observed: Observed,
    },
    /// Refuse on the merits before any effect: recorded as
    /// `command.completed`, like any other outcome.
    Refuse(Decision),
}

/// What `claim` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claimed {
    /// The claim committed; the caller may act.
    New { seq: u64 },
    /// The same request holds an open claim: the effect is under way or
    /// was interrupted. Nothing was recorded, and the effect must not run
    /// again.
    InProgress(Claim),
    /// The request was completed before, with the same digest. Nothing was
    /// recorded; this is its outcome.
    Replayed(Outcome),
}

/// What reconciling an interrupted claim found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The real state was read. The claim's request completes with this
    /// decision, which may record result events.
    Resolved(Box<Decision>),
    /// The real state is ambiguous, as with a half-applied revert. The claim
    /// stays open and waits for the owner.
    AwaitingOwner,
}

/// What a reconciliation decision returns. `command.reconciled` records
/// `finding` either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub finding: Value,
    pub resolution: Resolution,
    pub observed: Observed,
}
