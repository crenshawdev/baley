//! The anchor command and verification against the forge (design 0001,
//! Commands and The hash chain and anchors; 0011 LND-R18; EVD-R3, R26).
//!
//! An anchor push is a command with an external effect. The claim reads the
//! pre-claim head inside its transaction and records it as the intent. With
//! the claim held and its lease renewed, the command fetches the latest
//! remote anchor and verifies the local chain against it, so a rolled-back
//! or rewritten copy is never anchored. Only then does it push the tag, and
//! it records what it observed: `anchor.pushed` and the anchor row, or
//! `anchor.failed`. Every step is callable on its own over supplied
//! observations; `anchor_command` runs them in order over the seams the
//! binary supplies. Nothing here reads a clock, git, configuration or the
//! network: times come from the caller's `now`, and the forge is a seam.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use baley_store::{
    ANCHOR_FAILED, ANCHOR_FAILED_VERSION, ANCHOR_PUSH, ANCHOR_PUSHED, ANCHOR_PUSHED_VERSION,
    ANCHOR_RECONCILE, ANCHOR_SCOPE, ANCHOR_STREAM, Actor, Anchor, AnchorCheck, AnchorPushedPayload,
    AnchorVerdict, Block, ChainReport, Claim, ClaimDecision, ClaimId, ClaimOwner, ClaimState,
    Claimed, Command, CommandKind, Decision, Hash, Head, LEASE_RENEWAL_SECONDS, Ledger, NewEvent,
    Observed, Outcome, OutcomeKind, PayloadFault, ProjectId, ReconcileAuthority, Recorded, Refusal,
    RequestId, StaleInput, StoreError, StreamName, VerifyReport, anchor_tag, canonical_json,
    request_digest,
};
use serde_json::{Value, json};

use crate::forge::{
    FetchObservation, Forge, PushObservation, TagQuery, anchor_annotation, parse_tag_anchor,
};
use crate::reconcile::{
    AnchorFinding, AnchorIntent, RemoteTag, anchor_reconciliation, judge_anchor,
};

/// One anchor request as the binary supplies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRequest {
    /// The project to anchor.
    pub project: ProjectId,
    /// The caller's fresh request id for the push.
    pub request_id: RequestId,
    /// A second fresh request id, used only if an interrupted holder must
    /// be reconciled first.
    pub reconcile_request_id: RequestId,
    /// Who asked: the owner, or Baley at an anchor point.
    pub actor: Actor,
    /// The process that acts; recorded in the claim.
    pub owner: ClaimOwner,
    /// The configured remote's name, never a URL; `None` when the project
    /// has no remote configured.
    pub remote: Option<String>,
    /// The effective policy the command runs under.
    pub policy_version: u64,
}

impl AnchorRequest {
    /// The request digest: the kind, project, actor, policy version,
    /// configured remote name and scope. No head, sequence or tag enters it,
    /// so a retry of the same request keeps its digest while the head moves.
    pub fn digest(&self) -> Result<Hash, StoreError> {
        request_digest(&json!({
            "kind": ANCHOR_PUSH,
            "project": self.project.0,
            "actor": self.actor.as_str(),
            "policy_version": self.policy_version,
            "remote": self.remote,
            "scope": [ANCHOR_SCOPE],
        }))
        .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))
    }

    /// The push command at a supplied UTC time.
    pub fn command(&self, recorded_at: &str) -> Result<Command, StoreError> {
        Ok(Command {
            project: self.project.clone(),
            kind: CommandKind(ANCHOR_PUSH.into()),
            request_id: self.request_id.clone(),
            digest: self.digest()?,
            scope: vec![ANCHOR_SCOPE.into()],
            policy_version: self.policy_version,
            recorded_at: recorded_at.into(),
            actor: self.actor.clone(),
        })
    }
}

/// What one anchor points at: the pre-claim head, its tag and the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorTarget {
    /// The pre-claim head.
    pub intent: AnchorIntent,
    /// `baley-anchor/<project_id>/<intent.seq>`.
    pub tag: String,
    /// The configured remote's name.
    pub remote: String,
}

impl AnchorTarget {
    fn new(project: &ProjectId, intent: AnchorIntent, remote: String) -> Self {
        Self {
            tag: anchor_tag(project, intent.seq),
            intent,
            remote,
        }
    }

    fn anchor(&self) -> Anchor {
        Anchor {
            seq: self.intent.seq,
            hash: self.intent.head,
        }
    }

    fn pushed(&self, observed_at: &str) -> AnchorPushedPayload {
        AnchorPushedPayload {
            tag: self.tag.clone(),
            seq: self.intent.seq,
            head: self.intent.head,
            remote: self.remote.clone(),
            observed_at: observed_at.into(),
        }
    }
}

/// The claim decision over the head read once inside the claim transaction.
/// With no configured remote or an empty chain it refuses on the merits;
/// otherwise it claims with the head as the intent. The intent is returned
/// so the caller knows what it claimed.
pub fn anchor_claim_decision(
    head: Option<Head>,
    request: &AnchorRequest,
) -> (ClaimDecision, Option<AnchorIntent>) {
    let refuse = |code: &str| {
        (
            ClaimDecision::Refuse(Decision {
                kind: OutcomeKind::Refused,
                answer: json!({"refused": code}),
                sensitive: false,
                observed: Observed::default(),
                git: None,
            }),
            None,
        )
    };
    let Some(remote) = &request.remote else {
        return refuse("no-remote");
    };
    let Some(head) = head else {
        return refuse("empty-chain");
    };
    let intent = AnchorIntent {
        seq: head.seq,
        head: head.hash,
    };
    let mut value = intent.to_value();
    value["remote"] = json!(remote);
    (
        ClaimDecision::Claim {
            intent: value,
            owner: request.owner.clone(),
            git: None,
            observed: Observed::default(),
        },
        Some(intent),
    )
}

/// What to do after the claim step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStep {
    /// The claim committed: renew, check and push this tag.
    Act(AnchorAct),
    /// The request was completed before; nothing was written.
    Replayed(Outcome),
    /// The request holds an open claim; the effect must not run again.
    InProgress(Claim),
    /// The claim decision refused before any effect and recorded it.
    Refused {
        /// The recorded refusal.
        outcome: Outcome,
        /// The head after the refusal was recorded.
        head: Head,
    },
    /// Another claim holds the anchor scope.
    Blocked(Block),
}

/// A claimed anchor, ready to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorAct {
    /// The `command.claimed` event's sequence: the lease, not the tag.
    pub claim_seq: u64,
    /// What the claim intends to anchor.
    pub target: AnchorTarget,
    /// The tag's annotation.
    pub annotation: String,
}

/// Takes the claim at a supplied time. The tag uses the pre-claim sequence
/// the claim decision read, never the claim event's own.
pub fn claim_step(
    ledger: &dyn Ledger,
    request: &AnchorRequest,
    recorded_at: &str,
) -> Result<ClaimStep, StoreError> {
    let command = request.command(recorded_at)?;
    let mut claimed: Option<AnchorIntent> = None;
    let result = ledger.claim(&command, &mut |tx| {
        let (decision, intent) = anchor_claim_decision(tx.head()?, request);
        claimed = intent;
        Ok(decision)
    });
    match result {
        Ok(Claimed::New { seq }) => {
            let (Some(intent), Some(remote)) = (claimed, request.remote.clone()) else {
                return Err(StoreError::Unavailable(
                    "an anchor claim committed without its intent".into(),
                ));
            };
            let target = AnchorTarget::new(&request.project, intent, remote);
            Ok(ClaimStep::Act(AnchorAct {
                claim_seq: seq,
                annotation: anchor_annotation(&target.anchor()),
                target,
            }))
        }
        Ok(Claimed::Replayed(outcome)) => Ok(ClaimStep::Replayed(outcome)),
        Ok(Claimed::InProgress(claim)) => Ok(ClaimStep::InProgress(claim)),
        Ok(Claimed::Refused { outcome, head }) => Ok(ClaimStep::Refused { outcome, head }),
        Err(StoreError::Blocked(block)) => Ok(ClaimStep::Blocked(block)),
        Err(error) => Err(error),
    }
}

/// Whether the push may go ahead after the latest-anchor check. Payload
/// faults are carried either way: damaged body bytes do not stop an anchor
/// of the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrePushCheck {
    /// The chain agrees with the remote: push.
    Push {
        /// Bodies that failed their hash.
        payload_faults: Vec<PayloadFault>,
    },
    /// Record `anchor.failed` and push nothing.
    Refuse {
        /// The mismatch or remote failure, for the recorded outcome.
        reason: String,
        /// Bodies that failed their hash.
        payload_faults: Vec<PayloadFault>,
    },
}

/// Why a chain report forbids anchoring the local chain, or `None` when
/// every event was accepted and the anchor, if any, matches.
pub fn chain_mismatch(chain: &ChainReport) -> Option<String> {
    let broken = chain.first_break.as_ref().map(|at| at.seq);
    match (&chain.anchor, broken) {
        (AnchorVerdict::Truncated { anchored, head }, _) => Some(format!(
            "the local chain ends at sequence {head}, before the remote anchor at {anchored}"
        )),
        (AnchorVerdict::Rewritten { seq, .. }, _) => Some(format!(
            "the local chain differs from the remote anchor at sequence {seq}"
        )),
        (AnchorVerdict::Unchecked { anchored }, broken) => Some(format!(
            "the local chain breaks at sequence {}, before the remote anchor at {anchored}",
            broken.unwrap_or(0)
        )),
        (_, Some(seq)) => Some(format!("the local chain breaks at sequence {seq}")),
        (
            AnchorVerdict::NoAnchor | AnchorVerdict::Matches | AnchorVerdict::Acknowledged { .. },
            None,
        ) => None,
    }
}

/// The pre-push verdict over a verification of the local chain.
pub fn pre_push_verdict(report: VerifyReport) -> PrePushCheck {
    match chain_mismatch(&report.chain) {
        Some(reason) => PrePushCheck::Refuse {
            reason,
            payload_faults: report.payloads,
        },
        None => PrePushCheck::Push {
            payload_faults: report.payloads,
        },
    }
}

/// Checks the local chain against the latest remote anchor the caller
/// fetched. A present anchor is the witness; a confirmed absence allows a
/// first anchor once the local chain verifies. A malformed latest tag, an
/// unreachable remote or no remote refuses, as a failed push does. The
/// local anchor row is never used in place of the remote.
pub fn pre_push_check(
    ledger: &dyn Ledger,
    project: &ProjectId,
    fetched: &FetchObservation,
) -> Result<PrePushCheck, StoreError> {
    let refuse = |reason: String| PrePushCheck::Refuse {
        reason,
        payload_faults: Vec::new(),
    };
    Ok(match anchor_status(project, fetched) {
        AnchorCheck::Remote(anchor) => pre_push_verdict(ledger.verify(project, Some(&anchor))?),
        AnchorCheck::RemoteAbsent => pre_push_verdict(ledger.verify(project, None)?),
        AnchorCheck::RemoteMalformed(tag) => {
            refuse(format!("the latest remote tag {tag} is not a valid anchor"))
        }
        AnchorCheck::RemoteUnreachable => {
            refuse("the remote was unreachable at the latest-anchor fetch".into())
        }
        AnchorCheck::LocalOnly => refuse("no remote at the latest-anchor fetch".into()),
    })
}

/// A record step's writes: the result event, the anchor row when the tag is
/// confirmed, and the claim's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDecision {
    /// `anchor.pushed` or `anchor.failed` on the `project` stream.
    pub event: NewEvent,
    /// The anchor row, for a confirmed tag only.
    pub row: Option<AnchorRow>,
    /// The claim's outcome.
    pub decision: Decision,
}

/// The local anchor row a confirmed tag writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRow {
    /// The anchored sequence and head.
    pub anchor: Anchor,
    /// The confirmed tag.
    pub tag: String,
    /// The configured remote's name.
    pub remote: String,
    /// Becomes the row's `pushed_at`.
    pub observed_at: String,
}

/// `anchor.pushed`, its row and a done outcome for a confirmed tag.
pub fn pushed_record_decision(target: &AnchorTarget, observed_at: &str) -> RecordDecision {
    let payload = target.pushed(observed_at).to_value();
    RecordDecision {
        event: result_event(ANCHOR_PUSHED, ANCHOR_PUSHED_VERSION, payload.clone()),
        row: Some(AnchorRow {
            anchor: target.anchor(),
            tag: target.tag.clone(),
            remote: target.remote.clone(),
            observed_at: observed_at.into(),
        }),
        decision: outcome_decision(OutcomeKind::Done, json!({"anchored": payload})),
    }
}

/// `anchor.failed` and a refused outcome naming `reason`, with no row.
pub fn failed_record_decision(
    target: &AnchorTarget,
    reason: &str,
    observed_at: &str,
) -> RecordDecision {
    let mut payload = target.pushed(observed_at).to_value();
    payload["reason"] = json!(reason);
    RecordDecision {
        event: result_event(ANCHOR_FAILED, ANCHOR_FAILED_VERSION, payload.clone()),
        row: None,
        decision: outcome_decision(OutcomeKind::Refused, json!({"failed": payload})),
    }
}

/// What a push observation records: a confirmed tag is done; a refusal, no
/// remote or an unreachable remote is a clean failure that completes the
/// claim as refused.
pub fn push_observation_to_record_decision(
    observation: &PushObservation,
    target: &AnchorTarget,
    observed_at: &str,
) -> RecordDecision {
    let reason = match observation {
        PushObservation::Pushed => return pushed_record_decision(target, observed_at),
        PushObservation::Refused { reason } => format!("the remote refused the push: {reason}"),
        PushObservation::NoRemote => "no remote to push to".into(),
        PushObservation::Unreachable => "the remote was unreachable".into(),
    };
    failed_record_decision(target, &reason, observed_at)
}

fn result_event(type_name: &str, type_version: u32, payload: Value) -> NewEvent {
    NewEvent {
        stream: StreamName(ANCHOR_STREAM.into()),
        type_name: type_name.into(),
        type_version,
        git: None,
        payload,
        attachments: Vec::new(),
    }
}

fn outcome_decision(kind: OutcomeKind, answer: Value) -> Decision {
    Decision {
        kind,
        answer,
        sensitive: false,
        observed: Observed::default(),
        git: None,
    }
}

/// The record step for a supplied push observation, at a supplied result
/// time. A `Recorded::Replayed` answer is the outcome reconciliation already
/// recorded for this claim, which may say not pushed although this push
/// landed after its check.
pub fn record_step(
    ledger: &dyn Ledger,
    request: &AnchorRequest,
    act: &AnchorAct,
    observation: &PushObservation,
    observed_at: &str,
    recorded_at: &str,
) -> Result<Recorded, StoreError> {
    let plan = push_observation_to_record_decision(observation, &act.target, observed_at);
    record(ledger, request, plan, recorded_at)
}

/// The record step for a pre-push refusal: `anchor.failed` naming the
/// reason, nothing pushed.
pub fn record_refusal(
    ledger: &dyn Ledger,
    request: &AnchorRequest,
    act: &AnchorAct,
    reason: &str,
    observed_at: &str,
    recorded_at: &str,
) -> Result<Recorded, StoreError> {
    let plan = failed_record_decision(&act.target, reason, observed_at);
    record(ledger, request, plan, recorded_at)
}

/// Completes the claim with the original request, its digest and scope.
fn record(
    ledger: &dyn Ledger,
    request: &AnchorRequest,
    plan: RecordDecision,
    recorded_at: &str,
) -> Result<Recorded, StoreError> {
    let command = request.command(recorded_at)?;
    ledger.complete(&command, &request.owner, &mut |tx| {
        tx.append(plan.event.clone())?;
        if let Some(row) = &plan.row {
            tx.record_anchor(&row.anchor, &row.tag, &row.remote, &row.observed_at)?;
        }
        Ok(plan.decision.clone())
    })
}

/// Schedules a callback every `interval_seconds`, handing it a supplied UTC
/// time. The binary's ticker reads the clock; a test's delivers ticks by
/// hand.
pub trait Ticker {
    /// Starts calling `tick` every `interval_seconds` with the current UTC
    /// time, until the returned guard stops it.
    fn start(
        &mut self,
        interval_seconds: u64,
        tick: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Box<dyn TickGuard>;
}

/// A running ticker.
pub trait TickGuard {
    /// Stops the ticker and waits for a tick in progress to end. No tick
    /// runs after it returns.
    fn stop(self: Box<Self>);
}

/// Runs `work` under the claim's heartbeat: renews once at `first_at`,
/// starts the ticker at once, renews on every tick, and stops the ticker
/// before returning. A failed renewal never cancels the work; every renewal
/// error comes back beside its result.
pub fn with_heartbeat<T>(
    renew: impl FnMut(&str) -> Result<(), StoreError> + Send + 'static,
    first_at: &str,
    ticker: &mut dyn Ticker,
    work: impl FnOnce() -> T,
) -> (T, Vec<StoreError>) {
    let renew = Arc::new(Mutex::new(renew));
    let errors: Arc<Mutex<Vec<StoreError>>> = Arc::new(Mutex::new(Vec::new()));
    let stopped = Arc::new(AtomicBool::new(false));
    let renew_at = {
        let renew = Arc::clone(&renew);
        let errors = Arc::clone(&errors);
        move |at: &str| {
            let result = (renew.lock().unwrap_or_else(PoisonError::into_inner))(at);
            if let Err(error) = result {
                errors
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(error);
            }
        }
    };
    renew_at(first_at);
    let guard = ticker.start(LEASE_RENEWAL_SECONDS, {
        let stopped = Arc::clone(&stopped);
        let renew_at = renew_at.clone();
        Box::new(move |at: String| {
            // A ticker that fires after its guard stopped renews nothing.
            if !stopped.load(Ordering::SeqCst) {
                renew_at(&at);
            }
        })
    });
    let result = work();
    stopped.store(true, Ordering::SeqCst);
    guard.stop();
    let errors = std::mem::take(&mut *errors.lock().unwrap_or_else(PoisonError::into_inner));
    (result, errors)
}

/// A diagnostic row outside the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRecord {
    /// A supplied UTC time.
    pub at: String,
    /// The project the row concerns, if any.
    pub project: Option<ProjectId>,
    /// The payload the row derives from, if any.
    pub payload: Option<Hash>,
    /// What the row records, such as `claim.unreachable`.
    pub kind: String,
    /// The facts, as canonical JSON text.
    pub data: String,
}

/// Where the core writes diagnostics. The binary bridges it to the store's
/// trace table.
pub trait TraceSink {
    /// Writes one row.
    fn record(&self, record: TraceRecord) -> Result<(), StoreError>;
}

/// An open anchor claim that blocks a request, read from `open_claims`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldAnchor {
    /// The open claim.
    pub claim: Claim,
    /// What it intended to anchor.
    pub target: AnchorTarget,
}

impl HeldAnchor {
    /// The anchor an open claim of `project` intended, or `None` when the
    /// claim is not an anchor push or its intent cannot be read.
    pub fn from_claim(project: &ProjectId, claim: &Claim) -> Option<Self> {
        if claim.id.kind.0 != ANCHOR_PUSH {
            return None;
        }
        let intent = AnchorIntent::from_value(&claim.intent)?;
        let remote = claim.intent.get("remote")?.as_str()?.to_owned();
        Some(Self {
            claim: claim.clone(),
            target: AnchorTarget::new(project, intent, remote),
        })
    }
}

/// What a fetch of the held claim's exact tag says about it. A tag that
/// exists but is not a well-formed anchor of this tag is present without an
/// anchor, never absence. Without a remote the state is unknown, since an
/// earlier remote may have received the push.
pub fn fetch_observation_to_remote_tag(
    project: &ProjectId,
    tag: &str,
    observation: &FetchObservation,
) -> RemoteTag {
    match observation {
        FetchObservation::Present {
            tag: found,
            annotation,
        } => RemoteTag::Present(
            (found == tag)
                .then(|| parse_tag_anchor(project, found, annotation))
                .flatten(),
        ),
        FetchObservation::Absent => RemoteTag::Absent,
        FetchObservation::NoRemote | FetchObservation::Unreachable => RemoteTag::Unreachable,
    }
}

/// What the command does about a blocking claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedAction {
    /// Leave it: it is active or held for the owner.
    Stop,
    /// Fetch the holder's tag.
    Fetch,
    /// Record the finding and retry once.
    Reconcile,
    /// Write the unreachable trace row and leave the claim interrupted.
    Trace,
}

/// The next action for a blocking claim in `state`, given the holder's tag
/// fetch once there is one.
pub fn blocked_action(state: ClaimState, observation: Option<&FetchObservation>) -> BlockedAction {
    match (state, observation) {
        (ClaimState::Active | ClaimState::AwaitingOwner, _) => BlockedAction::Stop,
        (ClaimState::Interrupted, None) => BlockedAction::Fetch,
        (
            ClaimState::Interrupted,
            Some(FetchObservation::Present { .. } | FetchObservation::Absent),
        ) => BlockedAction::Reconcile,
        (
            ClaimState::Interrupted,
            Some(FetchObservation::NoRemote | FetchObservation::Unreachable),
        ) => BlockedAction::Trace,
    }
}

/// What reconciling a held claim did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileStep {
    /// The finding was recorded and the claim completed; retry the
    /// original request once.
    Retry(Recorded),
    /// The remote state is unknown: one trace row was written, nothing in
    /// the chain, and the claim stays interrupted.
    Unknown,
}

/// The canonical text of a fetch observation, for the reconciler's digest.
fn observation_value(observation: &FetchObservation) -> Value {
    match observation {
        FetchObservation::Present { tag, annotation } => {
            json!({"present": {"tag": tag, "annotation": annotation}})
        }
        FetchObservation::Absent => json!("absent"),
        FetchObservation::NoRemote => json!("no-remote"),
        FetchObservation::Unreachable => json!("unreachable"),
    }
}

/// Reconciles the held claim from a fetch of its own tag, checked at
/// `checked_at`. A matching tag records `anchor.pushed` and the row at
/// `checked_at`; a conflicting or confirmed-absent tag records
/// `anchor.failed`; both complete the claim under a separate
/// `anchor.reconcile` command attributed to Baley. An unknown state writes
/// one `claim.unreachable` trace row and records nothing.
pub fn reconcile_from_observation(
    ledger: &dyn Ledger,
    trace: &dyn TraceSink,
    request: &AnchorRequest,
    held: &HeldAnchor,
    observation: &FetchObservation,
    checked_at: &str,
) -> Result<ReconcileStep, StoreError> {
    let target = &held.target;
    let remote = fetch_observation_to_remote_tag(&request.project, &target.tag, observation);
    let finding = judge_anchor(&target.intent, remote);
    let plan = match &finding {
        AnchorFinding::Anchored => pushed_record_decision(target, checked_at),
        AnchorFinding::Conflicting { .. } => failed_record_decision(
            target,
            &format!(
                "the remote tag {} does not name the claimed head",
                target.tag
            ),
            checked_at,
        ),
        AnchorFinding::NotPushed => failed_record_decision(
            target,
            &format!("the remote confirmed {} absent", target.tag),
            checked_at,
        ),
        AnchorFinding::Unknown => {
            record_unreachable(trace, request, held, observation, checked_at)?;
            return Ok(ReconcileStep::Unknown);
        }
    };
    let Some(reconciliation) = anchor_reconciliation(&target.intent, finding, checked_at) else {
        record_unreachable(trace, request, held, observation, checked_at)?;
        return Ok(ReconcileStep::Unknown);
    };
    let command = Command {
        project: request.project.clone(),
        kind: CommandKind(ANCHOR_RECONCILE.into()),
        request_id: request.reconcile_request_id.clone(),
        digest: request_digest(&json!({
            "kind": ANCHOR_RECONCILE,
            "project": request.project.0,
            "claim": {"kind": held.claim.id.kind.0, "request_id": held.claim.id.request_id.0,
                "seq": held.claim.seq},
            "observation": observation_value(observation),
            "checked_at": checked_at,
        }))
        .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?,
        scope: Vec::new(),
        policy_version: request.policy_version,
        recorded_at: checked_at.into(),
        actor: Actor::Baley,
    };
    let recorded = ledger.reconcile(
        &command,
        &held.claim.id,
        ReconcileAuthority::Automatic,
        &mut |tx, claim| {
            if claim.seq != held.claim.seq {
                return Err(StoreError::Stale(StaleInput::Claim(claim.id.clone())));
            }
            tx.append(plan.event.clone())?;
            if let Some(row) = &plan.row {
                tx.record_anchor(&row.anchor, &row.tag, &row.remote, &row.observed_at)?;
            }
            Ok(reconciliation.clone())
        },
    )?;
    Ok(ReconcileStep::Retry(recorded))
}

/// The one trace row an unknown remote state leaves.
fn record_unreachable(
    trace: &dyn TraceSink,
    request: &AnchorRequest,
    held: &HeldAnchor,
    observation: &FetchObservation,
    checked_at: &str,
) -> Result<(), StoreError> {
    let remote = match observation {
        FetchObservation::NoRemote => "no-remote",
        _ => "unreachable",
    };
    let data = canonical_json(&json!({
        "claim_seq": held.claim.seq,
        "kind": held.claim.id.kind.0,
        "remote": remote,
        "request_id": held.claim.id.request_id.0,
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    trace.record(TraceRecord {
        at: checked_at.into(),
        project: Some(request.project.clone()),
        payload: None,
        kind: "claim.unreachable".into(),
        data: String::from_utf8(data)
            .map_err(|_| StoreError::Unavailable("canonical JSON that is not UTF-8".into()))?,
    })
}

/// The status a latest-anchor fetch gives.
pub fn anchor_status(project: &ProjectId, observation: &FetchObservation) -> AnchorCheck {
    match observation {
        FetchObservation::Present { tag, annotation } => {
            match parse_tag_anchor(project, tag, annotation) {
                Some(anchor) => AnchorCheck::Remote(anchor),
                None => AnchorCheck::RemoteMalformed(tag.clone()),
            }
        }
        FetchObservation::Absent => AnchorCheck::RemoteAbsent,
        FetchObservation::Unreachable => AnchorCheck::RemoteUnreachable,
        FetchObservation::NoRemote => AnchorCheck::LocalOnly,
    }
}

/// Fetches the latest anchor for doctor, or reports a local-only project.
pub fn anchor_check(
    forge: &mut dyn Forge,
    project: &ProjectId,
    remote: Option<&str>,
) -> AnchorCheck {
    match remote {
        Some(remote) => anchor_status(
            project,
            &forge.fetch_tag(project, remote, &TagQuery::LatestAnchor),
        ),
        None => AnchorCheck::LocalOnly,
    }
}

/// A verification and the fetch status it was made under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// What the latest-anchor fetch found.
    pub status: AnchorCheck,
    /// The store's verification against the remote anchor, if any.
    pub report: VerifyReport,
    /// When the remote was checked, a supplied UTC time.
    pub checked_at: String,
}

/// Verifies against a supplied latest-anchor observation. Only a
/// well-formed remote anchor is handed to the store; otherwise the chain is
/// checked locally and the status says why.
pub fn verify_observed(
    ledger: &dyn Ledger,
    project: &ProjectId,
    observation: &FetchObservation,
    checked_at: &str,
) -> Result<Verification, StoreError> {
    let status = anchor_status(project, observation);
    let witness = match &status {
        AnchorCheck::Remote(anchor) => Some(anchor),
        _ => None,
    };
    let report = ledger.verify(project, witness)?;
    Ok(Verification {
        status,
        report,
        checked_at: checked_at.into(),
    })
}

/// Fetches the latest anchor from the configured remote, takes the check
/// time from `now` after the fetch, and verifies. A project with no
/// configured remote is verified locally, without a fetch.
pub fn verify_project(
    ledger: &dyn Ledger,
    forge: &mut dyn Forge,
    project: &ProjectId,
    remote: Option<&str>,
    now: &mut dyn FnMut() -> String,
) -> Result<Verification, StoreError> {
    let observation = match remote {
        Some(remote) => forge.fetch_tag(project, remote, &TagQuery::LatestAnchor),
        None => FetchObservation::NoRemote,
    };
    let checked_at = now();
    verify_observed(ledger, project, &observation, &checked_at)
}

/// The seams the binary supplies to one anchor command.
pub struct AnchorSeams<'a> {
    /// The store, shared with the heartbeat.
    pub ledger: Arc<dyn Ledger + Send + Sync>,
    /// The forge the tags are fetched from and pushed to.
    pub forge: &'a mut dyn Forge,
    /// Drives lease renewals during the check and push.
    pub ticker: &'a mut dyn Ticker,
    /// Where an unreachable reconciliation is recorded.
    pub trace: &'a dyn TraceSink,
    /// The binary's clock: a UTC time string each call.
    pub now: &'a mut dyn FnMut() -> String,
}

/// How an anchor command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorOutcome {
    /// The record step committed: done with the row, or refused with
    /// `anchor.failed`.
    Recorded {
        /// The recorded outcome.
        outcome: Outcome,
        /// The head after it.
        head: Head,
    },
    /// Reconciliation closed the claim before this record step; its outcome
    /// stands. `pushed` says this run's push reported success, so the tag
    /// may have landed after a check that found it absent.
    LateReplay {
        /// The reconciliation's outcome.
        outcome: Outcome,
        /// Whether this run's push reported success.
        pushed: bool,
    },
    /// The request was completed before.
    Replayed(Outcome),
    /// The request holds an open claim.
    InProgress(Claim),
    /// The claim decision refused: no remote, or nothing to anchor.
    Refused {
        /// The recorded refusal.
        outcome: Outcome,
        /// The head after it.
        head: Head,
    },
    /// Another claim holds the anchor scope and was left as it is.
    Blocked(Block),
    /// The blocking claim's remote state is unknown: a trace row was
    /// written and the claim stays interrupted.
    RemoteUnknown(ClaimId),
    /// Reconciliation lost a race to a renewal or completion.
    ReconcileLost(StoreError),
}

/// An anchor command's result, with the renewal errors collected on the way
/// and any payload faults the pre-push verification found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorReport {
    /// How the command ended.
    pub outcome: AnchorOutcome,
    /// Every failed lease renewal, none of which stopped the command.
    pub renewal_errors: Vec<StoreError>,
    /// Bodies the pre-push verification found damaged.
    pub payload_faults: Vec<PayloadFault>,
}

impl AnchorReport {
    fn only(outcome: AnchorOutcome) -> Self {
        Self {
            outcome,
            renewal_errors: Vec::new(),
            payload_faults: Vec::new(),
        }
    }
}

/// What acting on a claim observed.
enum Acted {
    Pushed(PushObservation),
    Refused(String),
}

/// Runs one anchor command: claim, then under the heartbeat fetch the
/// latest anchor, verify, and push only if the chain agrees, then record.
/// A blocking interrupted anchor claim is reconciled from its own tag and
/// the request is retried once.
pub fn anchor_command(
    request: &AnchorRequest,
    seams: AnchorSeams<'_>,
) -> Result<AnchorReport, StoreError> {
    let AnchorSeams {
        ledger,
        forge,
        ticker,
        trace,
        now,
    } = seams;
    let mut retried = false;
    loop {
        let recorded_at = now();
        let block = match claim_step(ledger.as_ref(), request, &recorded_at)? {
            ClaimStep::Act(act) => {
                return act_and_record(request, &act, &ledger, forge, ticker, now);
            }
            ClaimStep::Replayed(outcome) => {
                return Ok(AnchorReport::only(AnchorOutcome::Replayed(outcome)));
            }
            ClaimStep::InProgress(claim) => {
                return Ok(AnchorReport::only(AnchorOutcome::InProgress(claim)));
            }
            ClaimStep::Refused { outcome, head } => {
                return Ok(AnchorReport::only(AnchorOutcome::Refused { outcome, head }));
            }
            ClaimStep::Blocked(block) => block,
        };
        if retried || blocked_action(block.state, None) != BlockedAction::Fetch {
            return Ok(AnchorReport::only(AnchorOutcome::Blocked(block)));
        }
        let holder = ledger
            .open_claims(&request.project)?
            .into_iter()
            .find(|claim| claim.id == block.claim);
        let Some(holder) = holder else {
            return Ok(AnchorReport::only(AnchorOutcome::ReconcileLost(
                StoreError::Stale(StaleInput::Claim(block.claim)),
            )));
        };
        let Some(held) = HeldAnchor::from_claim(&request.project, &holder) else {
            return Ok(AnchorReport::only(AnchorOutcome::Blocked(block)));
        };
        let observation = forge.fetch_tag(
            &request.project,
            &held.target.remote,
            &TagQuery::Exact(held.target.tag.clone()),
        );
        let checked_at = now();
        match reconcile_from_observation(
            ledger.as_ref(),
            trace,
            request,
            &held,
            &observation,
            &checked_at,
        ) {
            Ok(ReconcileStep::Retry(_)) => retried = true,
            Ok(ReconcileStep::Unknown) => {
                return Ok(AnchorReport::only(AnchorOutcome::RemoteUnknown(
                    held.claim.id,
                )));
            }
            Err(
                error @ (StoreError::Stale(_) | StoreError::Refused(Refusal::AwaitingOwner(_))),
            ) => {
                return Ok(AnchorReport::only(AnchorOutcome::ReconcileLost(error)));
            }
            Err(error) => return Err(error),
        }
    }
}

/// The act and record steps for a new claim.
fn act_and_record(
    request: &AnchorRequest,
    act: &AnchorAct,
    ledger: &Arc<dyn Ledger + Send + Sync>,
    forge: &mut dyn Forge,
    ticker: &mut dyn Ticker,
    now: &mut dyn FnMut() -> String,
) -> Result<AnchorReport, StoreError> {
    let renew = {
        let ledger = Arc::clone(ledger);
        let project = request.project.clone();
        let claim = ClaimId {
            kind: CommandKind(ANCHOR_PUSH.into()),
            request_id: request.request_id.clone(),
        };
        let owner = request.owner.clone();
        move |at: &str| ledger.renew_lease(&project, &claim, &owner, at)
    };
    let first_at = now();
    let (acted, renewal_errors) = with_heartbeat(renew, &first_at, ticker, || {
        let fetched = forge.fetch_tag(
            &request.project,
            &act.target.remote,
            &TagQuery::LatestAnchor,
        );
        let fetched_at = now();
        match pre_push_check(ledger.as_ref(), &request.project, &fetched)? {
            PrePushCheck::Push { payload_faults } => {
                let pushed = forge.push_tag(
                    &request.project,
                    &act.target.remote,
                    &act.target.tag,
                    &act.annotation,
                );
                Ok((Acted::Pushed(pushed), now(), payload_faults))
            }
            PrePushCheck::Refuse {
                reason,
                payload_faults,
            } => Ok((Acted::Refused(reason), fetched_at, payload_faults)),
        }
    });
    let (acted, observed_at, payload_faults) = acted?;
    let recorded_at = now();
    let recorded = match &acted {
        Acted::Pushed(observation) => record_step(
            ledger.as_ref(),
            request,
            act,
            observation,
            &observed_at,
            &recorded_at,
        )?,
        Acted::Refused(reason) => record_refusal(
            ledger.as_ref(),
            request,
            act,
            reason,
            &observed_at,
            &recorded_at,
        )?,
    };
    let outcome = match recorded {
        Recorded::New { outcome, head } => AnchorOutcome::Recorded { outcome, head },
        Recorded::Replayed { outcome } => AnchorOutcome::LateReplay {
            outcome,
            pushed: matches!(acted, Acted::Pushed(PushObservation::Pushed)),
        },
    };
    Ok(AnchorReport {
        outcome,
        renewal_errors,
        payload_faults,
    })
}

#[cfg(test)]
mod tests {
    use baley_store::{Break, BreakKind, StoredAnchorComparison};

    use super::*;

    const PROJECT: &str = "3f2b1a9c-6d4e-4f0a-9b8c-7d6e5f4a3b2c";
    const AT: &str = "2026-09-25T18:00:00Z";

    fn project() -> ProjectId {
        ProjectId(PROJECT.into())
    }

    fn request(remote: Option<&str>) -> AnchorRequest {
        AnchorRequest {
            project: project(),
            request_id: RequestId("r1".into()),
            reconcile_request_id: RequestId("r2".into()),
            actor: Actor::Owner,
            owner: ClaimOwner {
                process: "p".into(),
                host_session: "h".into(),
                started_at: AT.into(),
            },
            remote: remote.map(str::to_owned),
            policy_version: 1,
        }
    }

    fn head(seq: u64) -> Head {
        Head {
            seq,
            hash: Hash([0x42; 32]),
        }
    }

    fn target() -> AnchorTarget {
        AnchorTarget::new(
            &project(),
            AnchorIntent {
                seq: 7,
                head: Hash([0x42; 32]),
            },
            "origin".into(),
        )
    }

    fn report(chain: ChainReport, payloads: Vec<PayloadFault>) -> VerifyReport {
        VerifyReport {
            chain,
            payloads,
            bodies_checked: 0,
            tombstones_checked: 0,
            stored_anchor: None,
            stored_anchor_comparison: StoredAnchorComparison::NotCompared,
        }
    }

    fn chain(anchor: AnchorVerdict, first_break: Option<Break>) -> ChainReport {
        ChainReport {
            head: Some(head(9)),
            first_break,
            anchor,
            unanchored: None,
            acknowledged_restores: Vec::new(),
            age_unanchored_since: None,
        }
    }

    // Catches the accepted restore gap still blocking future anchor pushes.
    #[test]
    fn acknowledged_restore_allows_a_new_anchor_push() {
        assert_eq!(
            chain_mismatch(&chain(
                AnchorVerdict::Acknowledged {
                    anchored: 12,
                    restored: Some(head(9)),
                    acknowledged_seq: 10,
                },
                None
            )),
            None
        );
    }

    // The claim over head 7 intends 7, and the tag an open claim at event 8
    // names is the one for 7. Catches anchoring the claim event itself.
    #[test]
    fn the_tag_uses_the_pre_claim_sequence() {
        let (decision, intent) = anchor_claim_decision(Some(head(7)), &request(Some("origin")));
        let ClaimDecision::Claim { intent: value, .. } = decision else {
            panic!("expected a claim");
        };
        assert_eq!(intent.map(|intent| intent.seq), Some(7));
        let claim = Claim {
            id: ClaimId {
                kind: CommandKind(ANCHOR_PUSH.into()),
                request_id: RequestId("r1".into()),
            },
            seq: 8,
            claimed_at: AT.into(),
            intent: value,
            scope: vec![ANCHOR_SCOPE.into()],
            owner: request(None).owner,
            lease_renewed_at: None,
            awaiting_owner: None,
        };
        let held = HeldAnchor::from_claim(&project(), &claim).expect("an anchor claim");
        assert_eq!(held.target.tag, format!("baley-anchor/{PROJECT}/7"));
        assert_eq!(held.target.remote, "origin");
    }

    // No configured remote, or nothing to anchor, refuses with its reason
    // and claims nothing. Catches a lease taken for an impossible push.
    #[test]
    fn no_remote_or_an_empty_chain_refuses_the_claim() {
        let (no_remote, intent) = anchor_claim_decision(Some(head(7)), &request(None));
        assert_eq!(intent, None);
        assert!(matches!(no_remote, ClaimDecision::Refuse(decision)
            if decision.kind == OutcomeKind::Refused && decision.answer == json!({"refused": "no-remote"})));
        let (empty, _) = anchor_claim_decision(None, &request(Some("origin")));
        assert!(matches!(empty, ClaimDecision::Refuse(decision)
            if decision.answer == json!({"refused": "empty-chain"})));
    }

    // A confirmed push records anchor.pushed with exactly its five fields,
    // the configured remote's name and the row, and is done. Catches a
    // success recorded without its result event or row.
    #[test]
    fn a_pushed_observation_records_anchor_pushed_and_done() {
        let plan = push_observation_to_record_decision(&PushObservation::Pushed, &target(), AT);
        assert_eq!(plan.event.type_name, ANCHOR_PUSHED);
        assert_eq!(plan.event.type_version, 1);
        assert_eq!(plan.event.stream, StreamName("project".into()));
        assert_eq!(
            plan.event.payload,
            json!({"tag": format!("baley-anchor/{PROJECT}/7"), "seq": 7, "head": "42".repeat(32),
                "remote": "origin", "observed_at": AT})
        );
        assert_eq!(plan.decision.kind, OutcomeKind::Done);
        assert_eq!(
            plan.row,
            Some(AnchorRow {
                anchor: Anchor {
                    seq: 7,
                    hash: Hash([0x42; 32])
                },
                tag: format!("baley-anchor/{PROJECT}/7"),
                remote: "origin".into(),
                observed_at: AT.into(),
            })
        );
    }

    // A refused, unreachable or remote-less push records anchor.failed
    // with the reason, completes the claim as refused and writes no row.
    // Catches a clean failure left open or recorded as an anchor.
    #[test]
    fn a_failed_push_records_anchor_failed_and_refused() {
        for (observation, reason) in [
            (
                PushObservation::Refused {
                    reason: "ruleset".into(),
                },
                "the remote refused the push: ruleset",
            ),
            (PushObservation::Unreachable, "the remote was unreachable"),
            (PushObservation::NoRemote, "no remote to push to"),
        ] {
            let plan = push_observation_to_record_decision(&observation, &target(), AT);
            assert_eq!(plan.event.type_name, ANCHOR_FAILED);
            assert_eq!(
                plan.event.payload,
                json!({"tag": format!("baley-anchor/{PROJECT}/7"), "seq": 7, "head": "42".repeat(32),
                    "remote": "origin", "observed_at": AT, "reason": reason})
            );
            assert_eq!(plan.decision.kind, OutcomeKind::Refused);
            assert_eq!(plan.decision.answer["failed"]["reason"], json!(reason));
            assert_eq!(plan.row, None);
        }
    }

    // Present-and-matching, present-but-malformed, a sequence mismatch,
    // absent and unreachable are all told apart; no remote is unknown, not
    // absent. Catches existence taken for agreement and an unreachable
    // reconciliation fetch taken for absence.
    #[test]
    fn fetch_observations_map_to_distinct_remote_tags() {
        let tag = format!("baley-anchor/{PROJECT}/7");
        let good = anchor_annotation(&Anchor {
            seq: 7,
            hash: Hash([0x42; 32]),
        });
        let other_seq = anchor_annotation(&Anchor {
            seq: 8,
            hash: Hash([0x42; 32]),
        });
        let present = |annotation: &str| FetchObservation::Present {
            tag: tag.clone(),
            annotation: annotation.into(),
        };
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &present(&good)),
            RemoteTag::Present(Some(Anchor {
                seq: 7,
                hash: Hash([0x42; 32])
            }))
        );
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &present("not an anchor")),
            RemoteTag::Present(None)
        );
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &present(&other_seq)),
            RemoteTag::Present(None)
        );
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &FetchObservation::Absent),
            RemoteTag::Absent
        );
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &FetchObservation::Unreachable),
            RemoteTag::Unreachable
        );
        assert_eq!(
            fetch_observation_to_remote_tag(&project(), &tag, &FetchObservation::NoRemote),
            RemoteTag::Unreachable
        );
    }

    // An interrupted holder is fetched first, then reconciled from a read
    // remote or traced from an unread one; an active or held claim is left.
    // Catches a blocked path that skips the trace or acts twice.
    #[test]
    fn a_blocked_claim_chooses_fetch_reconcile_trace_or_stop() {
        let present = FetchObservation::Present {
            tag: "t".into(),
            annotation: "a".into(),
        };
        assert_eq!(
            blocked_action(ClaimState::Interrupted, None),
            BlockedAction::Fetch
        );
        assert_eq!(
            blocked_action(ClaimState::Interrupted, Some(&present)),
            BlockedAction::Reconcile
        );
        assert_eq!(
            blocked_action(ClaimState::Interrupted, Some(&FetchObservation::Absent)),
            BlockedAction::Reconcile
        );
        assert_eq!(
            blocked_action(
                ClaimState::Interrupted,
                Some(&FetchObservation::Unreachable)
            ),
            BlockedAction::Trace
        );
        assert_eq!(
            blocked_action(ClaimState::Interrupted, Some(&FetchObservation::NoRemote)),
            BlockedAction::Trace
        );
        assert_eq!(
            blocked_action(ClaimState::Active, None),
            BlockedAction::Stop
        );
        assert_eq!(
            blocked_action(ClaimState::AwaitingOwner, Some(&present)),
            BlockedAction::Stop
        );
    }

    // Truncation, a rewrite, an unchecked anchor and a break each refuse
    // the push, naming the mismatch. Catches re-anchoring a rolled-back or
    // rewritten chain.
    #[test]
    fn a_chain_that_disagrees_with_the_remote_refuses_the_push() {
        let broken = Some(Break {
            seq: 4,
            kind: BreakKind::Sequence { found: 5 },
        });
        for (chain, reason) in [
            (
                chain(
                    AnchorVerdict::Truncated {
                        anchored: 60,
                        head: 50,
                    },
                    None,
                ),
                "the local chain ends at sequence 50, before the remote anchor at 60",
            ),
            (
                chain(
                    AnchorVerdict::Rewritten {
                        seq: 60,
                        anchored: Hash([1; 32]),
                        found: Hash([2; 32]),
                    },
                    None,
                ),
                "the local chain differs from the remote anchor at sequence 60",
            ),
            (
                chain(AnchorVerdict::Unchecked { anchored: 60 }, broken.clone()),
                "the local chain breaks at sequence 4, before the remote anchor at 60",
            ),
            (
                chain(AnchorVerdict::NoAnchor, broken),
                "the local chain breaks at sequence 4",
            ),
        ] {
            assert_eq!(
                pre_push_verdict(report(chain, Vec::new())),
                PrePushCheck::Refuse {
                    reason: reason.into(),
                    payload_faults: Vec::new()
                }
            );
        }
    }

    // A matching chain with damaged bodies still pushes and carries the
    // faults. Catches refusing an anchor over body bytes alone.
    #[test]
    fn payload_faults_alone_permit_the_push() {
        let faults = vec![PayloadFault::Corrupt(Hash([9; 32]))];
        assert_eq!(
            pre_push_verdict(report(chain(AnchorVerdict::Matches, None), faults.clone())),
            PrePushCheck::Push {
                payload_faults: faults
            }
        );
    }

    // A well-formed latest tag is the witness; a malformed one, an absent
    // one, an unreachable remote and no remote are each their own status.
    // Catches an unchecked remote reported as verified.
    #[test]
    fn the_latest_fetch_gives_its_status() {
        let anchor = Anchor {
            seq: 7,
            hash: Hash([0x42; 32]),
        };
        let tag = format!("baley-anchor/{PROJECT}/7");
        assert_eq!(
            anchor_status(
                &project(),
                &FetchObservation::Present {
                    tag: tag.clone(),
                    annotation: anchor_annotation(&anchor)
                }
            ),
            AnchorCheck::Remote(anchor)
        );
        assert_eq!(
            anchor_status(
                &project(),
                &FetchObservation::Present {
                    tag: tag.clone(),
                    annotation: "{}".into()
                }
            ),
            AnchorCheck::RemoteMalformed(tag)
        );
        assert_eq!(
            anchor_status(&project(), &FetchObservation::Absent),
            AnchorCheck::RemoteAbsent
        );
        assert_eq!(
            anchor_status(&project(), &FetchObservation::Unreachable),
            AnchorCheck::RemoteUnreachable
        );
        assert_eq!(
            anchor_status(&project(), &FetchObservation::NoRemote),
            AnchorCheck::LocalOnly
        );
    }

    /// A ticker a test drives by hand: it keeps the callback and delivers
    /// ticks when told, until its guard stops it.
    #[derive(Clone, Default)]
    struct ScriptedTicker {
        state: Arc<Mutex<TickerState>>,
        log: Log,
    }

    #[derive(Default)]
    struct TickerState {
        tick: Option<Box<dyn FnMut(String) + Send>>,
    }

    type Log = Arc<Mutex<Vec<String>>>;

    impl ScriptedTicker {
        fn with_log(log: Log) -> Self {
            Self {
                state: Arc::default(),
                log,
            }
        }

        /// Delivers one tick; false when the ticker is stopped.
        fn tick(&self, at: &str) -> bool {
            let mut state = self.state.lock().expect("ticker");
            match state.tick.as_mut() {
                Some(tick) => {
                    tick(at.into());
                    true
                }
                None => false,
            }
        }
    }

    struct ScriptedGuard {
        state: Arc<Mutex<TickerState>>,
        log: Log,
    }

    impl Ticker for ScriptedTicker {
        fn start(
            &mut self,
            interval_seconds: u64,
            tick: Box<dyn FnMut(String) + Send + 'static>,
        ) -> Box<dyn TickGuard> {
            self.log
                .lock()
                .expect("log")
                .push(format!("start {interval_seconds}"));
            self.state.lock().expect("ticker").tick = Some(tick);
            Box::new(ScriptedGuard {
                state: Arc::clone(&self.state),
                log: Arc::clone(&self.log),
            })
        }
    }

    impl TickGuard for ScriptedGuard {
        fn stop(self: Box<Self>) {
            self.state.lock().expect("ticker").tick = None;
            self.log.lock().expect("log").push("stop".into());
        }
    }

    fn recording_renew(
        log: &Log,
        fail_at: Option<&'static str>,
    ) -> impl FnMut(&str) -> Result<(), StoreError> + Send + 'static {
        let log = Arc::clone(log);
        move |at: &str| {
            log.lock().expect("log").push(format!("renew {at}"));
            if fail_at == Some(at) {
                Err(StoreError::Busy)
            } else {
                Ok(())
            }
        }
    }

    const TEN: &str = "2026-09-25T18:00:10Z";

    // The heartbeat renews once, starts the ticker before the work runs,
    // takes a ten-second tick during it, and stops before returning,
    // whether the work pushes or refuses; a tick after that renews
    // nothing. Catches a missing first renewal, a ticker started late, and
    // a renewal after the record step.
    #[test]
    fn the_heartbeat_renews_then_ticks_through_the_work_and_stops() {
        for outcome in [
            PushObservation::Pushed,
            PushObservation::Refused { reason: "x".into() },
        ] {
            let log: Log = Arc::default();
            let mut ticker = ScriptedTicker::with_log(Arc::clone(&log));
            let handle = ticker.clone();
            let work_log = Arc::clone(&log);
            let (result, errors) =
                with_heartbeat(recording_renew(&log, None), AT, &mut ticker, || {
                    work_log.lock().expect("log").push("work".into());
                    assert!(handle.tick(TEN));
                    outcome.clone()
                });
            assert_eq!(result, outcome);
            assert!(errors.is_empty());
            assert!(!ticker.tick("2026-09-25T18:00:20Z"));
            assert_eq!(
                *log.lock().expect("log"),
                [
                    format!("renew {AT}"),
                    "start 10".into(),
                    "work".into(),
                    format!("renew {TEN}"),
                    "stop".into()
                ]
            );
        }
    }

    // A failed first renewal and a failed tick renewal are both returned
    // beside the work's result, and the work still runs to its one push.
    // Catches a renewal failure that cancels the act.
    #[test]
    fn failed_renewals_do_not_cancel_the_work() {
        for fail_at in [AT, TEN] {
            let log: Log = Arc::default();
            let mut ticker = ScriptedTicker::with_log(Arc::clone(&log));
            let handle = ticker.clone();
            let mut pushes = 0;
            let (result, errors) = with_heartbeat(
                recording_renew(&log, Some(fail_at)),
                AT,
                &mut ticker,
                || {
                    handle.tick(TEN);
                    pushes += 1;
                    PushObservation::Pushed
                },
            );
            assert_eq!(result, PushObservation::Pushed);
            assert_eq!(pushes, 1);
            assert_eq!(errors, [StoreError::Busy]);
        }
    }
}
