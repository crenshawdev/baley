//! Commands with an external effect: claim, act, record (design 0001,
//! Commands; EVD-R26).
//!
//! A claim is the intent event recorded before the effect runs. Its lease is
//! liveness, not evidence: it lives outside the chain, and renewing it
//! records nothing. Whether a claim is active or interrupted, and whether it
//! blocks a command, are port decisions over the facts here and a supplied
//! time.

use serde_json::{Value, json};

use crate::Event;
use crate::command::{CommandKind, Decision, Observed, Outcome};
use crate::event::{GitFacts, Hash, RequestId};
use crate::time::{TimeError, UtcInstant};
use crate::view::{
    Change, DocKey, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};

/// The event that takes a claim.
pub const COMMAND_CLAIMED: &str = "command.claimed";
/// The current claimed payload version.
pub const COMMAND_CLAIMED_VERSION: u32 = 1;
/// The event that records a reconciliation finding.
pub const COMMAND_RECONCILED: &str = "command.reconciled";
/// The current reconciled payload version.
pub const COMMAND_RECONCILED_VERSION: u32 = 1;
/// The heartbeat interval used by callers.
pub const LEASE_RENEWAL_SECONDS: u64 = 10;
/// The time after the last renewal at which a claim is interrupted.
pub const LEASE_EXPIRY_SECONDS: u64 = 60;
/// The view that maps a scope token to its holder.
pub const CLAIM_SCOPE_VIEW: &str = "claim_scope";

/// Who holds a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOwner {
    pub process: String,
    pub host_session: String,
    /// The store's UTC instant form.
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
    /// The claim event's recorded time.
    pub claimed_at: String,
    /// The command's inputs and intended effect.
    pub intent: Value,
    /// What it blocks while open, for example `anchor` or `phase/3`.
    pub scope: Vec<String>,
    pub owner: ClaimOwner,
    /// The matching row's renewal time, or `None` when no row matches.
    pub lease_renewed_at: Option<String>,
    /// The reconciliation sequence that held this claim for the owner.
    pub awaiting_owner: Option<u64>,
}

/// What a claim decision returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDecision {
    /// Record `command.claimed` and take the lease.
    Claim {
        intent: Value,
        owner: ClaimOwner,
        /// Git facts the claim event depends on.
        git: Option<GitFacts>,
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
    /// The claim decision refused before an effect and recorded the outcome.
    Refused {
        /// The refusal recorded for this request.
        outcome: Outcome,
        /// The committed head after the refusal.
        head: crate::Head,
    },
}

/// Whether a lease is live at a supplied time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    /// The expiry boundary has not arrived.
    Active,
    /// The row is absent or its expiry boundary has arrived.
    Interrupted,
}

/// Judges a lease from a matching row and a supplied time.
pub fn lease_state(renewed_at: Option<&str>, now: &str) -> Result<LeaseState, TimeError> {
    let Some(renewed_at) = renewed_at else {
        return Ok(LeaseState::Interrupted);
    };
    let renewed = UtcInstant::parse(renewed_at)?;
    let now = UtcInstant::parse(now)?;
    Ok(if now >= renewed.plus_seconds(LEASE_EXPIRY_SECONDS)? {
        LeaseState::Interrupted
    } else {
        LeaseState::Active
    })
}

/// An open claim's state at a supplied time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimState {
    /// The lease is live.
    Active,
    /// The lease is absent or expired.
    Interrupted,
    /// A reconciliation held the claim for the owner.
    AwaitingOwner,
}

/// Judges a claim, giving an owner hold priority over its lease.
pub fn claim_state(claim: &Claim, now: &str) -> Result<ClaimState, TimeError> {
    if claim.awaiting_owner.is_some() {
        return Ok(ClaimState::AwaitingOwner);
    }
    Ok(match lease_state(claim.lease_renewed_at.as_deref(), now)? {
        LeaseState::Active => ClaimState::Active,
        LeaseState::Interrupted => ClaimState::Interrupted,
    })
}

/// The claim that blocks a command and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The holder's identity.
    pub claim: ClaimId,
    /// The holder's state at the command's supplied time.
    pub state: ClaimState,
}

/// Returns the lowest-sequence holder sharing an exact scope token.
pub fn blocking(
    scope: &[String],
    holders: &[Claim],
    now: &str,
) -> Result<Option<Block>, TimeError> {
    holders
        .iter()
        .filter(|holder| holder.scope.iter().any(|token| scope.contains(token)))
        .min_by_key(|holder| holder.seq)
        .map(|holder| {
            Ok(Block {
                claim: holder.id.clone(),
                state: claim_state(holder, now)?,
            })
        })
        .transpose()
}

/// Who may resolve a claim held for the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileAuthority {
    /// Reconcile only an interrupted claim.
    Automatic,
    /// Also resolve a claim held for the owner.
    Owner,
}

/// The `command.claimed` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedPayload {
    /// The claimed command's kind.
    pub kind: CommandKind,
    /// The claimed command's request id.
    pub request_id: RequestId,
    /// The claimed command's digest.
    pub digest: Hash,
    /// The intended external effect.
    pub intent: Value,
    /// The tokens held by the claim.
    pub scope: Vec<String>,
    /// The process that acts.
    pub owner: ClaimOwner,
}

impl ClaimedPayload {
    /// Builds the event payload.
    pub fn to_value(&self) -> Value {
        json!({"kind": self.kind.0, "request_id": self.request_id.0, "digest": self.digest.to_hex(),
            "intent": self.intent, "scope": self.scope,
            "owner": {"process": self.owner.process, "host_session": self.owner.host_session, "started_at": self.owner.started_at}})
    }

    /// Reads the event payload.
    pub fn from_value(value: &Value) -> Option<Self> {
        let owner = value.get("owner")?;
        Some(Self {
            kind: CommandKind(value.get("kind")?.as_str()?.into()),
            request_id: RequestId(value.get("request_id")?.as_str()?.into()),
            digest: Hash::from_hex(value.get("digest")?.as_str()?)?,
            intent: value.get("intent")?.clone(),
            scope: value
                .get("scope")?
                .as_array()?
                .iter()
                .map(|token| token.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
            owner: ClaimOwner {
                process: owner.get("process")?.as_str()?.into(),
                host_session: owner.get("host_session")?.as_str()?.into(),
                started_at: owner.get("started_at")?.as_str()?.into(),
            },
        })
    }
}

/// The result recorded by `command.reconciled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciledResolution {
    /// The claim is completed in this transaction.
    Resolved,
    /// The claim waits for the owner.
    AwaitingOwner,
}

/// The `command.reconciled` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledPayload {
    /// The claim's identity.
    pub claim: ClaimId,
    /// The event sequence of the original claim.
    pub claim_seq: u64,
    /// The external finding.
    pub finding: Value,
    /// Whether the claim is closed or held.
    pub resolution: ReconciledResolution,
}

impl ReconciledPayload {
    /// Builds the event payload.
    pub fn to_value(&self) -> Value {
        json!({"kind": self.claim.kind.0, "request_id": self.claim.request_id.0,
            "claim_seq": self.claim_seq, "finding": self.finding,
            "resolution": match self.resolution { ReconciledResolution::Resolved => "resolved", ReconciledResolution::AwaitingOwner => "awaiting_owner" }})
    }

    /// Reads the event payload.
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            claim: ClaimId {
                kind: CommandKind(value.get("kind")?.as_str()?.into()),
                request_id: RequestId(value.get("request_id")?.as_str()?.into()),
            },
            claim_seq: value.get("claim_seq")?.as_u64()?,
            finding: value.get("finding")?.clone(),
            resolution: match value.get("resolution")?.as_str()? {
                "resolved" => ReconciledResolution::Resolved,
                "awaiting_owner" => ReconciledResolution::AwaitingOwner,
                _ => return None,
            },
        })
    }
}

/// The view's declared key and page bound.
pub fn claim_scope_spec() -> ViewSpec {
    ViewSpec {
        name: CLAIM_SCOPE_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: "scope".into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// Projects each open claim's exact tokens.
pub struct ClaimScopeProjector {
    spec: ViewSpec,
}

impl ClaimScopeProjector {
    /// Makes the store-owned projector.
    pub fn new() -> Self {
        Self {
            spec: claim_scope_spec(),
        }
    }
}

impl Default for ClaimScopeProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for ClaimScopeProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }
    fn handles(&self) -> &[&str] {
        &[COMMAND_CLAIMED, crate::request::COMMAND_COMPLETED]
    }
    fn keys(&self, event: &Event) -> Vec<DocKey> {
        event
            .payload
            .get("scope")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|token| DocKey(vec![KeyValue::Text(token.into())]))
            .collect()
    }
    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let tokens = event
            .payload
            .get("scope")
            .and_then(Value::as_array)
            .ok_or_else(|| ProjectorError("a command event without scope".into()))?;
        let kind = event
            .payload
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| ProjectorError("a command event without kind".into()))?;
        let request_id = event
            .payload
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProjectorError("a command event without request id".into()))?;
        let mut changes = Vec::new();
        for token in tokens {
            let token = token
                .as_str()
                .ok_or_else(|| ProjectorError("a scope token is not text".into()))?;
            let key = DocKey(vec![KeyValue::Text(token.into())]);
            if event.type_name == COMMAND_CLAIMED {
                changes.push(Change::Put { key, body: json!({"scope": token, "claim": {"seq": event.seq, "kind": kind, "request_id": request_id}}) });
            } else if documents.iter().any(|(found_key, body)| {
                found_key == &key
                    && body.pointer("/claim/kind").and_then(Value::as_str) == Some(kind)
                    && body.pointer("/claim/request_id").and_then(Value::as_str) == Some(request_id)
            }) {
                changes.push(Change::Delete { key });
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "2026-09-25T18:00:00Z";
    fn claim(scope: &[&str]) -> Claim {
        Claim {
            id: ClaimId {
                kind: CommandKind("anchor.push".into()),
                request_id: RequestId("r1".into()),
            },
            seq: 7,
            claimed_at: T.into(),
            intent: json!({}),
            scope: scope.iter().map(|s| (*s).into()).collect(),
            owner: ClaimOwner {
                process: "p".into(),
                host_session: "h".into(),
                started_at: T.into(),
            },
            lease_renewed_at: Some(T.into()),
            awaiting_owner: None,
        }
    }

    // Catches a scope check that blocks nothing.
    #[test]
    fn matching_token_names_its_holder() {
        let holder = claim(&["anchor"]);
        assert_eq!(
            blocking(&["anchor".into()], std::slice::from_ref(&holder), T),
            Ok(Some(Block {
                claim: holder.id,
                state: ClaimState::Active
            }))
        );
    }

    // Catches a scope check that blocks everything.
    #[test]
    fn unrelated_token_does_not_block() {
        assert_eq!(
            blocking(&["phase/3".into()], &[claim(&["anchor"])], T),
            Ok(None)
        );
    }

    // Catches a lease judged from a live clock instead of supplied time.
    #[test]
    fn lease_is_interrupted_after_sixty_one_seconds() {
        assert_eq!(
            lease_state(Some(T), "2026-09-25T18:01:01Z"),
            Ok(LeaseState::Interrupted)
        );
    }

    // Catches expiry only after the boundary.
    #[test]
    fn lease_expires_at_exactly_sixty_seconds() {
        assert_eq!(
            lease_state(Some(T), "2026-09-25T18:01:00Z"),
            Ok(LeaseState::Interrupted)
        );
    }

    // Catches whole-second truncation before the boundary.
    #[test]
    fn lease_is_active_just_before_the_boundary() {
        assert_eq!(
            lease_state(Some(T), "2026-09-25T18:00:59.999999999Z"),
            Ok(LeaseState::Active)
        );
    }

    // Catches using the claim time as a lease after row loss.
    #[test]
    fn missing_row_is_interrupted_at_once() {
        assert_eq!(
            lease_state(None, "2026-09-25T18:00:01Z"),
            Ok(LeaseState::Interrupted)
        );
    }

    // Catches a held claim reported as active and reconciled automatically.
    #[test]
    fn owner_hold_overrides_a_live_lease() {
        let mut holder = claim(&["anchor"]);
        holder.awaiting_owner = Some(8);
        assert_eq!(claim_state(&holder, T), Ok(ClaimState::AwaitingOwner));
    }
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
