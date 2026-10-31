//! The owner's approval of a start or change draft by its digest (design
//! 0004 section 5), in two steps that only `baley scope approve` calls: no
//! approval is served over MCP (ADR 0044).
//!
//! The first step reads and judges outside any transaction, before its
//! caller prepares the write, so a refusal records nothing at all: no
//! checkout admission, no policy step and no `command.completed`. A change
//! is judged twice there: against the newest draft of the same change, then
//! against accepted scope as it stands at approval (D-12). The second step
//! records the approval as the owner, confirming inside its transaction that
//! nothing the first step read has moved.
//!
//! A `phase-edit` draft whose phase's latest planning continuation names
//! its digest is a commitment (owner ruling 9, D-01): its first step also
//! rechecks the continuation's selection with the readiness `phase-plan`
//! judges and refuses beside a pending planner order of the phase, and its
//! second step, given the route, issues the continuation's planner order in
//! the same transaction as the stories. Every other draft is approved as
//! before, with no order.

use std::collections::BTreeMap;

use baley_core::plan::{CommitmentRefusal, PHASE_PLAN_VIEW, judge_readiness, phase_plan_key};
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host};
use baley_core::scope::{
    AcceptedScope, ApprovalFacts, ApprovalRefusal, ChangeApprovalFacts, ChangeApprovalRefusal,
    ChangeSubmission, DRAFT_NEWEST_INDEX, DRAFT_VIEW, NumbersExhausted, PHASE_DECLARED,
    PHASE_DETAIL_VIEW, PHASE_WITHDRAWN, PROJECT_STREAM, PriorApproval, ROADMAP_VIEW,
    SCOPE_APPROVED, SCOPE_SUBMITTED, START_KIND, START_TARGET, STORY_CORRECTED, STORY_DECLARED,
    STORY_DROPPED, STORY_REASSIGNED, ScopeChange, Submission, change_events, draft_key,
    judge_approval, judge_change, judge_change_approval, judge_scope_change, judge_submission,
    roadmap_key, start_events,
};
use baley_core::truth::STORY_TRUTHS;
use baley_store::{
    Absence, Actor, Command, Decision, DocKey, Document, EVENT_PAGE_BOUND, EventMatch, Hash,
    HistoryFilter, IndexQuery, KeyValue, Ledger, Observed, ObservedDocument, OutcomeKind,
    PageRequest, Payloads, ProjectId, Recorded, RetentionClass, StaleInput, StoreError, StreamName,
    Transaction, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, TEXT_PURGED, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::mcp::scope::change_kind;
use crate::planning::order::{self, PreparedOrder, pending_planner, planner_absences};
use crate::planning::phase_plan::{latest_continuation, read_readiness, readiness_refused};
use crate::scope::read::{
    self, Detail, Preview, PreviewError, ReadError, ScopeRead, preview, preview_of,
};
use crate::scope::submit::{
    NOT_RECORDED_BUSY, scope_refused, scope_unread, start_approval, start_approved,
};

/// The command kind every approval is recorded under.
pub const APPROVE_COMMAND: &str = "scope.approve";

/// What the first step read and judged, which the second step confirms
/// and records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The approved draft's digest.
    digest: Hash,
    /// The sequence of its latest submission.
    seq: u64,
    /// The newest draft of the same change's digest and sequence when it
    /// was read.
    newest: (Hash, u64),
    /// What is approved.
    approving: Approving,
}

/// The draft a first step judged, with what its second step needs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Approving {
    /// A start, as phase 3 approves it.
    Start(Box<Submission>),
    /// A change.
    Change(Box<ChangeDraft>),
}

/// A change draft the first step judged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChangeDraft {
    /// The accepted change.
    change: ChangeSubmission,
    /// Whether an approval of this digest was recorded before its latest
    /// submission.
    approved_before: bool,
    /// Accepted scope as the change was judged against it.
    scope: ScopeRead,
    /// The detail in force a `phase-edit` that gives none keeps.
    detail: Option<CarriedDetail>,
    /// The commitment a `phase-edit` its phase's continuation names makes.
    commitment: Option<Commitment>,
}

/// A commitment the first step rechecked (D-01): what its planner order is
/// issued for, and what the second step observes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Commitment {
    /// The phase it commits.
    phase: u32,
    /// The continuation's selection, in order.
    stories: Vec<u32>,
    /// The planner order the continuation names, issued under this id.
    work_order: String,
    /// The `phase_plan` document, then the readiness documents, each at the
    /// sequence that produced it.
    documents: Vec<ObservedDocument>,
    /// The active-index absence when no phase was active.
    absences: Vec<Absence>,
}

impl Judged {
    /// The id of the planner order the second step issues when the draft is
    /// a commitment, so its caller knows to route one; none otherwise.
    pub fn planner_order(&self) -> Option<&str> {
        self.commitment()
            .map(|commitment| commitment.work_order.as_str())
    }

    fn commitment(&self) -> Option<&Commitment> {
        match &self.approving {
            Approving::Change(draft) => draft.commitment.as_ref(),
            Approving::Start(_) => None,
        }
    }
}

/// A phase's latest continuation that names the draft being approved, with
/// the `phase_plan` document it was read from.
struct Named {
    phase: u32,
    stories: Vec<u32>,
    work_order: String,
    document: ObservedDocument,
}

/// The detail in force a `phase-edit` keeps, with the `phase_detail`
/// document it was read under, so the second step can confirm this project
/// has not purged it since.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CarriedDetail {
    /// The detail text.
    text: String,
    /// The `phase_detail` document's key.
    key: DocKey,
    /// Its `produced_seq` when read.
    seq: u64,
}

/// The first step: reads the draft with `digest` and its `scope.submitted`.
/// For a start it reads whether a start is approved and the newest start
/// draft; for a change, what `judge_change_draft` names. Then it judges the
/// approval. A story's truth draft is refused `unknown-draft`, its reason
/// naming `baley story truths approve <digest>`, which approves it (owner
/// ruling R2). It runs outside any transaction, since a transaction has no
/// payload read (D-02). A refusal is the `refused` answer, slot `digest`; a
/// body that is gone or no longer judges clean is a ledger fault, never
/// approved. Nothing is recorded. A commitment, a `phase-edit` its phase's
/// latest continuation names, is also rechecked here (D-01), and
/// [`Judged::planner_order`] then names the order its second step issues.
pub fn judge<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Judged, Value> {
    let target = match preview(store, project, digest) {
        Ok(read) if read.kind == STORY_TRUTHS => return Err(truth_draft(digest)),
        Ok(read) if read.kind != START_KIND => return judge_change_draft(store, project, &read),
        Ok(read) => Some(accept(&read)?),
        Err(PreviewError::UnknownDraft) => None,
        Err(error) => return Err(preview_failed(error)),
    };
    let recorded = match &target {
        Some((_, seq)) => submitted_at(store, project, digest, *seq).map_err(busy)?,
        None => false,
    };
    let started = start_approved(store, project).map_err(busy)?;
    let newest = match store
        .find(project, DRAFT_VIEW, &newest_query(START_KIND, START_TARGET))
        .map_err(busy)?
        .items
        .into_iter()
        .next()
    {
        Some(document) => Some(newest_of(store, &document)?),
        None => None,
    };
    let facts = ApprovalFacts {
        target: target.as_ref().map(|(submission, _)| submission),
        recorded,
        started,
        newest: newest.as_ref().map(|(submission, _)| submission),
    };
    if let Err(refusal) = judge_approval(&facts) {
        return Err(refused(&refusal));
    }
    match (target, newest) {
        (Some((submission, seq)), Some((newest, newest_seq))) => Ok(Judged {
            digest,
            seq,
            newest: (newest.digest(), newest_seq),
            approving: Approving::Start(Box::new(submission)),
        }),
        // An approved judgement had both.
        _ => Err(refused(&ApprovalRefusal::UnknownDraft)),
    }
}

/// The first step for a change draft, in this order: its body judged
/// again under its kind, its `scope.submitted` at its sequence, accepted
/// scope, an approval of its digest recorded after that sequence, and the
/// newest draft of the same kind and target, then the approval judgement
/// (D-02, D-03); then the change judged against that scope as it stands
/// now, refused as at submission with slot `digest` (D-12). A `phase-edit` that
/// gives no detail keeps the one in force, read here, since a transaction
/// has no payload read; one this project purged is never opened.
///
/// A `phase-edit` its phase's latest continuation names is a commitment:
/// a `draft-approved` refusal of it also names the planner order its
/// approval issued, when the `work_order` view holds it, and after the
/// approval judgement and before the scope judgement its selection is
/// rechecked by [`recheck`].
fn judge_change_draft<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    read: &Preview,
) -> Result<Judged, Value> {
    let (change, seq) = accept_change(read)?;
    let digest = change.digest();
    let recorded = submitted_at(store, project, digest, seq).map_err(busy)?;
    // Scope before the approvals: an approval landing after the approvals
    // read then lands after this read too, and its roadmap event moves a
    // document the second step confirms.
    let scope = read::scope(store, project).map_err(|error| scope_unread(&error))?;
    let approvals = approvals_of(store, project, digest).map_err(busy)?;
    let prior = approvals.iter().find(|approval| approval.seq > seq);
    let newest = match store
        .find(
            project,
            DRAFT_VIEW,
            &newest_query(change.kind().as_str(), change.target()),
        )
        .map_err(busy)?
        .items
        .into_iter()
        .next()
    {
        Some(document) => Some(accept_change(
            &preview_of(store, &document).map_err(preview_failed)?,
        )?),
        None => None,
    };
    let facts = ChangeApprovalFacts {
        target: Some(&change),
        recorded,
        approved: prior.map(|approval| PriorApproval {
            owner: &approval.owner,
            at: &approval.at,
        }),
        newest: newest.as_ref().map(|(newest, _)| newest),
    };
    let named = continuation_naming(store, project, &change)?;
    if let Err(refusal) = judge_change_approval(&facts) {
        let answer = change_refused(&refusal);
        return Err(match (&refusal, &named) {
            (ChangeApprovalRefusal::DraftApproved { .. }, Some(named))
                if order_held(store, project, &named.work_order)? =>
            {
                naming_order(answer, &named.work_order)
            }
            _ => answer,
        });
    }
    // An approved judgement had a newest draft.
    let (newest, newest_seq) =
        newest.ok_or_else(|| change_refused(&ChangeApprovalRefusal::UnknownDraft))?;
    // Readiness before the scope judgement, so a story in another phase is
    // `story-in-phase`, not `story-double-assigned` (D-10).
    let commitment = match named {
        Some(named) => Some(recheck(store, project, named)?),
        None => None,
    };
    let accepted = scope
        .accepted()
        .ok_or_else(|| scope_unread(&ReadError::Unreadable(String::new())))?;
    judge_scope_change(&change, &accepted, scope.started, &scope.progress())
        .map_err(|refusal| scope_refused(&refusal, "digest"))?;
    let detail = detail_in_force(store, project, &change)?;
    Ok(Judged {
        digest,
        seq,
        newest: (newest.digest(), newest_seq),
        approving: Approving::Change(Box::new(ChangeDraft {
            change,
            approved_before: approvals.iter().any(|approval| approval.seq < seq),
            scope,
            detail,
            commitment,
        })),
    })
}

/// The continuation naming `change` when it is a `phase-edit` and its
/// phase's latest continuation in `phase_plan` names its digest, or none.
fn continuation_naming<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    change: &ChangeSubmission,
) -> Result<Option<Named>, Value> {
    let ScopeChange::PhaseEdit { phase, .. } = &change.change else {
        return Ok(None);
    };
    let key = phase_plan_key(*phase);
    let document = store.get(project, PHASE_PLAN_VIEW, &key).map_err(busy)?;
    let latest = latest_continuation(document.as_ref().map(|document| &document.body))?;
    Ok(latest
        .filter(|latest| latest.edit == change.digest())
        .map(|latest| Named {
            phase: *phase,
            stories: latest.stories,
            work_order: latest.work_order,
            document: ObservedDocument {
                view: PHASE_PLAN_VIEW.into(),
                key,
                produced_seq: document.map(|document| document.produced_seq),
            },
        }))
}

/// Rechecks a commitment's selection over facts read now, through the
/// gathering `phase-plan` judges (D-01): a readiness refusal, then a pending
/// planner order of the phase, each at slot `digest`, so an approval beside
/// a pending order says why rather than failing `ledger-busy` at its second
/// step on every retry (D-11).
fn recheck<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    named: Named,
) -> Result<Commitment, Value> {
    const NOT_DONE: &str = "nothing was approved";
    let readiness = read_readiness(store, project, named.phase, &named.stories)?;
    judge_readiness(&readiness.readiness())
        .map_err(|refusal| readiness_refused(&refusal, "digest", NOT_DONE))?;
    if let Some(pending) = pending_planner(store, project, named.phase).map_err(busy)? {
        let refusal = CommitmentRefusal::PlannerOrderPending {
            work_order: pending,
        };
        return Err(readiness_refused(&refusal, "digest", NOT_DONE));
    }
    let mut documents = vec![named.document];
    documents.extend(readiness.documents().iter().cloned());
    Ok(Commitment {
        phase: named.phase,
        stories: named.stories,
        work_order: named.work_order,
        documents,
        absences: readiness.absences(),
    })
}

/// Whether the `work_order` view holds order `id`.
fn order_held<S: Views + ?Sized>(store: &S, project: &ProjectId, id: &str) -> Result<bool, Value> {
    crate::work_order::read::order(store, project, id)
        .map(|order| order.is_some())
        .map_err(busy)
}

/// A `draft-approved` refusal of a commitment, naming in its details and
/// its reason the planner order its approval issued, so a rerun still tells
/// the owner the order (flagged assumption 2).
fn naming_order(mut answer: Value, work_order: &str) -> Value {
    if let Some(reason) = answer["reason"].as_str() {
        answer["reason"] = json!(format!(
            "{reason}. That approval issued planner work order {work_order}, which a session's phase-plan with the same stories launches"
        ));
    }
    match answer["details"].as_object_mut() {
        Some(details) => {
            details.insert("work_order".into(), json!(work_order));
        }
        None => answer["details"] = json!({"work_order": work_order}),
    }
    answer
}

/// The detail in force of the phase a `phase-edit` giving no detail
/// changes, or none for any other change.
fn detail_in_force<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    change: &ChangeSubmission,
) -> Result<Option<CarriedDetail>, Value> {
    let ScopeChange::PhaseEdit { phase, fields } = &change.change else {
        return Ok(None);
    };
    if fields.detail.is_some() {
        return Ok(None);
    }
    // The document before the text, so a purge after this read moves it.
    let document = match read::detail_document(store, project, *phase) {
        Ok(document) => document,
        Err(ReadError::Store(error)) => return Err(busy(error)),
        Err(_) => None,
    };
    match (read::phase(store, project, *phase), document) {
        (Ok(held), Some((key, seq))) => match held.detail {
            Detail::Text(text) => Ok(Some(CarriedDetail { text, key, seq })),
            Detail::Purged(_) => Err(failed(
                TEXT_PURGED,
                format!(
                    "phase {phase}'s detail was purged from this project, so an edit cannot keep it and nothing was approved. The edit must give a new detail"
                ),
                "detail",
            )),
        },
        (Err(ReadError::Store(error)), _) => Err(busy(error)),
        _ => Err(failed(
            LEDGER_UNAVAILABLE,
            format!("phase {phase} is recorded, but its detail cannot be read"),
            PLACE_LEDGER,
        )),
    }
}

/// One recorded approval of a digest.
struct Approval {
    seq: u64,
    owner: String,
    at: String,
}

/// Every `scope.approved` on `project` naming `digest`, oldest first, read
/// from the events.
fn approvals_of<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    digest: Hash,
) -> Result<Vec<Approval>, StoreError> {
    let filter = HistoryFilter {
        types: vec![SCOPE_APPROVED.into()],
        git_commit: None,
    };
    let digest = json!(digest.to_hex());
    let text = |value: &Value, name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut approvals = Vec::new();
    let mut after = None;
    loop {
        let page = store.history(
            project,
            1..=u64::MAX,
            &filter,
            PageRequest {
                limit: EVENT_PAGE_BOUND,
                after,
            },
        )?;
        approvals.extend(
            page.items
                .iter()
                .filter(|event| {
                    event.stream == PROJECT_STREAM
                        && event.payload.get("submission") == Some(&digest)
                })
                .map(|event| Approval {
                    seq: event.seq,
                    owner: text(&event.payload, "owner"),
                    at: text(&event.payload, "at"),
                }),
        );
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(approvals),
        }
    }
}

/// A recorded change draft's body judged again under its kind. Judging
/// takes the members a caller sent and adds the kind itself, so the body
/// less its `kind` member is judged. One whose kind is not the draft's,
/// that no longer judges clean, or whose digest is not the draft's is a
/// ledger fault.
fn accept_change(read: &Preview) -> Result<(ChangeSubmission, u64), Value> {
    let kind = change_kind(&read.kind);
    let judged = match (kind, &read.submission) {
        (Some(kind), Value::Object(body)) if body.get("kind") == Some(&json!(read.kind)) => {
            let mut members = body.clone();
            members.remove("kind");
            judge_change(kind, &Value::Object(members)).ok()
        }
        _ => None,
    };
    match judged {
        Some(change) if change.digest() == read.digest => Ok((change, read.seq)),
        _ => Err(failed(
            LEDGER_UNAVAILABLE,
            "the draft is recorded, but its body no longer judges as the submission it was",
            PLACE_LEDGER,
        )),
    }
}

/// The `refused` answer for a change approval, slot `digest`.
fn change_refused(refusal: &ChangeApprovalRefusal) -> Value {
    let reason = match refusal {
        ChangeApprovalRefusal::UnknownDraft => {
            "no submission has this digest, so nothing was approved"
        }
        ChangeApprovalRefusal::DraftApproved { .. } => {
            "this draft was already approved after its latest submission, so it was not approved again. The details name that approval"
        }
        ChangeApprovalRefusal::StaleDraft { .. } => {
            "a newer draft of the same change was submitted after this one, so it was not approved. The details name the first part that differs"
        }
    };
    let answer = Refusal::new(refusal.code(), reason).slot("digest");
    match refusal.details() {
        Some(details) => answer.details(details),
        None => answer,
    }
    .value()
}

/// The second step: records the approval judged by the first step under a
/// copy of `command` whose actor is the owner, with `owner` as the owner's
/// name and the command's time as Baley's (D-18). The approval is the
/// owner's, given at the terminal by `baley scope approve`, which builds its
/// command as the owner's with no caller; the decision records under the
/// owner whatever actor the command carries. The caller, request id, digest
/// and policy version are the command's.
///
/// Before it appends anything the decision confirms what the first step
/// read: the draft at the sequence read and its `scope.submitted`, no
/// approval of it since, and the same newest draft of its change by digest
/// and sequence; for a change, also every scope document the first step
/// judged it against. When any of them moved, a submission or another
/// approval landed between the steps, and the decision returns
/// `StoreError::Stale` with nothing recorded. A change's events are taken
/// over the `roadmap` document read inside the transaction; numbering past
/// the largest is `StoreError::Unavailable` with nothing recorded (D-14). A
/// retry of a request the store holds is answered from its record before
/// any decision runs, so it needs no reads; a decision asked to run without
/// them records nothing.
///
/// A commitment (D-01) is recorded through [`record_commitment`], which
/// routes its planner order; given one here, the decision records nothing.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    judged: Option<&Judged>,
    owner: &str,
) -> Result<Recorded, StoreError> {
    record_with(store, command, judged, owner, None)
}

/// What a commitment's planner order is routed under at the second step.
#[derive(Debug, Clone, Copy)]
pub struct OrderRoute<'a> {
    /// The host's effective policy.
    pub policy: &'a EffectivePolicy,
    /// The host the order is composed for.
    pub host: Host,
    /// The host's accepted model names.
    pub names: &'a AcceptedNames,
}

/// The second step for a draft whose first step may have judged a
/// commitment (D-01): [`record`], and for a commitment its planner order
/// prepared first, outside the transaction, as the continuation's id for the
/// continuation's selection under `route`, then issued in the approval's
/// transaction after the stories. The decision declares no pending planner
/// order of the phase, and the active-index absence when the recheck found
/// no active phase, and observes the `phase_plan` document, the readiness
/// documents and the inputs', so an order issued, a phase made active or a
/// truth revised since the reads makes the approval `ledger-busy` with
/// nothing recorded. The receipt adds the order's receipt members. A fault
/// preparing the order is the answer, with nothing approved.
pub fn record_commitment<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    judged: &Judged,
    owner: &str,
    route: OrderRoute<'_>,
) -> Value {
    let prepared = match judged.commitment() {
        Some(commitment) => match order::prepare(
            store,
            command,
            commitment.phase,
            &commitment.stories,
            None,
            route.policy,
            route.host,
            route.names,
            &commitment.work_order,
        ) {
            Ok(prepared) => Some(prepared),
            Err(answer) => return answer,
        },
        None => None,
    };
    crate::scope::submit::answer(
        record_with(store, command, Some(judged), owner, prepared.as_ref()),
        store,
    )
}

/// [`record`] with a commitment's prepared planner order.
fn record_with<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    judged: Option<&Judged>,
    owner: &str,
    prepared: Option<&PreparedOrder>,
) -> Result<Recorded, StoreError> {
    let command = Command {
        actor: Actor::Owner,
        ..command.clone()
    };
    let at = command.recorded_at.clone();
    store.transact(&command, &mut |tx| {
        let Some(judged) = judged else {
            return Err(StoreError::Unavailable(
                "an approval reached its transaction without its first step's reads".into(),
            ));
        };
        let submission = match &judged.approving {
            Approving::Start(submission) => submission,
            Approving::Change(draft) => {
                return change_decision(tx, judged, draft, owner, &at, prepared);
            }
        };
        confirm(tx, judged)?;
        let (highest_phase, highest_story) = highest(tx.get(ROADMAP_VIEW, &roadmap_key())?)?;
        let mut details = Vec::with_capacity(submission.phases.len());
        for phase in &submission.phases {
            details.push(tx.put_payload(phase.detail.as_bytes(), RetentionClass::Record)?);
        }
        // Only a ledger already holding numbers near the largest gets here,
        // so it is a ledger fault, and nothing is recorded.
        let events = start_events(
            submission,
            highest_phase,
            highest_story,
            &details,
            owner,
            &at,
        )
        .map_err(|NumbersExhausted| {
            StoreError::Unavailable(
                "the roadmap has no phase numbers or story ids left for this approval".into(),
            )
        })?;
        // The receipt reads the assigned values off the records themselves.
        let assigned = |type_name: &str, field: &str| -> Vec<Value> {
            events
                .iter()
                .filter(|event| event.type_name == type_name)
                .map(|event| event.payload[field].clone())
                .collect()
        };
        let receipt = json!({
            "status": "ok",
            "kind": START_KIND,
            "digest": judged.digest.to_hex(),
            "owner": owner,
            "at": at,
            "phases": assigned(PHASE_DECLARED, "number"),
            "stories": assigned(STORY_DECLARED, "id"),
        });
        for event in events {
            tx.append(event)?;
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: receipt,
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })
}

/// Records an approved change inside the transaction, once what the first
/// step read is confirmed, and a commitment's planner order `prepared` after
/// its events. A commitment reaching here without its order records
/// nothing.
fn change_decision(
    tx: &mut dyn Transaction,
    judged: &Judged,
    draft: &ChangeDraft,
    owner: &str,
    at: &str,
    prepared: Option<&PreparedOrder>,
) -> Result<Decision, StoreError> {
    let change = &draft.change;
    let ordered = match (&draft.commitment, prepared) {
        (None, _) => None,
        (Some(commitment), Some(prepared)) if prepared.id() == commitment.work_order => {
            Some((commitment, prepared))
        }
        (Some(_), _) => {
            return Err(StoreError::Unavailable(
                "a commitment approval reached its transaction without its planner order".into(),
            ));
        }
    };
    confirm_draft(tx, judged)?;
    // A transaction cannot ask for an approval after a sequence, so when
    // one preceded the latest submission a second is left to the scope
    // documents. The first step read them before the approvals, so any
    // approval it missed landed after that read, and every change approval
    // appends an event the roadmap document projects, which moves it.
    let approved = approval_of(judged.digest);
    if !draft.approved_before && tx.event_exists(&approved)? {
        return Err(StoreError::Stale(StaleInput::Absence(Absence::Event(
            approved,
        ))));
    }
    confirm_newest(tx, judged, change.kind().as_str(), change.target())?;
    read::confirm(tx, &draft.scope)?;
    if let Some(carried) = &draft.detail {
        confirm_detail(tx, carried)?;
    }
    let text = match &change.change {
        ScopeChange::PhaseDeclare(declaration) => Some(declaration.detail.as_str()),
        // A carried detail is the detail in force, so it has that hash.
        ScopeChange::PhaseEdit { fields, .. } => fields
            .detail
            .as_deref()
            .or(draft.detail.as_ref().map(|carried| carried.text.as_str())),
        _ => None,
    };
    let detail = match text {
        Some(text) => Some(tx.put_payload(text.as_bytes(), RetentionClass::Record)?),
        None => None,
    };
    let roadmap = tx.get(ROADMAP_VIEW, &roadmap_key())?;
    let scope = AcceptedScope::read(roadmap.as_ref().map(|document| &document.body))
        .ok_or_else(|| StoreError::Unavailable("the roadmap document cannot be read".into()))?;
    let events =
        change_events(change, &scope, detail.as_ref(), owner, at).map_err(|NumbersExhausted| {
            StoreError::Unavailable(
                "the roadmap has no phase numbers or story ids left for this approval".into(),
            )
        })?;
    let named = |types: &[&str], field: &str| -> Vec<Value> {
        events
            .iter()
            .filter(|event| types.contains(&event.type_name.as_str()))
            .map(|event| event.payload[field].clone())
            .collect()
    };
    let mut receipt = json!({
        "status": "ok",
        "kind": change.kind().as_str(),
        "digest": judged.digest.to_hex(),
        "owner": owner,
        "at": at,
        "phases": named(&[PHASE_DECLARED, PHASE_WITHDRAWN], "number"),
        "stories": named(
            &[STORY_DECLARED, STORY_CORRECTED, STORY_REASSIGNED, STORY_DROPPED],
            "id",
        ),
    });
    for event in events {
        tx.append(event)?;
    }
    let mut observed = Observed::default();
    if let Some((commitment, prepared)) = ordered {
        let members = prepared.issue_within(tx, at)?;
        if let Some(receipt) = receipt.as_object_mut() {
            receipt.extend(members);
        }
        observed.absences = planner_absences(commitment.phase);
        observed
            .absences
            .extend(commitment.absences.iter().cloned());
        observed.documents = commitment.documents.clone();
        observed
            .documents
            .extend(prepared.documents().iter().cloned());
    }
    Ok(Decision {
        kind: OutcomeKind::Done,
        answer: receipt,
        sensitive: false,
        observed,
        git: None,
    })
}

/// The `phase_detail` document a carried detail was read under has not
/// moved and does not read purged. A purge changes only that document, so
/// without this a detail purged between the steps would be stored again.
fn confirm_detail(tx: &mut dyn Transaction, carried: &CarriedDetail) -> Result<(), StoreError> {
    let now = tx.get(PHASE_DETAIL_VIEW, &carried.key)?;
    let purged = now
        .as_ref()
        .is_some_and(|document| document.body.get("state").and_then(Value::as_str).is_some());
    let now = now.map(|document| document.produced_seq);
    if purged || now != Some(carried.seq) {
        return Err(StoreError::Stale(StaleInput::Document {
            view: PHASE_DETAIL_VIEW.into(),
            key: carried.key.clone(),
            seen: Some(carried.seq),
            now,
        }));
    }
    Ok(())
}

/// An approval of `digest` on the project stream.
fn approval_of(digest: Hash) -> EventMatch {
    EventMatch {
        type_name: SCOPE_APPROVED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::from([("submission".to_string(), json!(digest.to_hex()))]),
    }
}

/// Confirms inside the transaction what the first step read of a start. A
/// start approval is checked here rather than declared as an observed
/// absence, because this approval appends one itself.
fn confirm(tx: &mut dyn Transaction, judged: &Judged) -> Result<(), StoreError> {
    confirm_draft(tx, judged)?;
    if tx.event_exists(&start_approval())? {
        return Err(StoreError::Stale(StaleInput::Absence(Absence::Event(
            start_approval(),
        ))));
    }
    confirm_newest(tx, judged, START_KIND, START_TARGET)
}

/// The draft is at the sequence read and its `scope.submitted` is there.
fn confirm_draft(tx: &mut dyn Transaction, judged: &Judged) -> Result<(), StoreError> {
    let key = draft_key(judged.digest);
    let now = tx.get(DRAFT_VIEW, &key)?.as_ref().and_then(sequence);
    let submitted = EventMatch {
        type_name: SCOPE_SUBMITTED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::from([("digest".to_string(), json!(judged.digest.to_hex()))]),
    };
    if now != Some(judged.seq) || !tx.event_exists(&submitted)? {
        return Err(StoreError::Stale(StaleInput::Document {
            view: DRAFT_VIEW.into(),
            key,
            seen: Some(judged.seq),
            now,
        }));
    }
    Ok(())
}

/// The newest draft of the change filed under `kind` and `target` is the
/// one the first step read.
fn confirm_newest(
    tx: &mut dyn Transaction,
    judged: &Judged,
    kind: &str,
    target: &str,
) -> Result<(), StoreError> {
    // Compared by digest and sequence, never by time: two submissions can
    // share a time.
    let newest = tx
        .find(DRAFT_VIEW, &newest_query(kind, target))?
        .items
        .into_iter()
        .next();
    let now = newest
        .as_ref()
        .and_then(|document| Some((draft_digest(document)?, sequence(document)?)));
    if now != Some(judged.newest) {
        return Err(StoreError::Stale(StaleInput::Document {
            view: DRAFT_VIEW.into(),
            key: draft_key(judged.newest.0),
            seen: Some(judged.newest.1),
            now: now.map(|(_, seq)| seq),
        }));
    }
    Ok(())
}

/// The first page of one change's drafts, newest first: its kind and
/// target, so it reads only that change.
pub(crate) fn newest_query(kind: &str, target: &str) -> IndexQuery {
    IndexQuery {
        index: DRAFT_NEWEST_INDEX.into(),
        equals: vec![KeyValue::Text(kind.into()), KeyValue::Text(target.into())],
        page: PageRequest {
            limit: 1,
            after: None,
        },
    }
}

/// The latest submission's sequence a `draft` document holds.
fn sequence(document: &Document) -> Option<u64> {
    document.body.get("seq").and_then(Value::as_u64)
}

/// The digest a `draft` document holds.
fn draft_digest(document: &Document) -> Option<Hash> {
    document
        .body
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex)
}

/// The highest phase number and story id the roadmap holds, or none yet.
fn highest(roadmap: Option<Document>) -> Result<(u32, u32), StoreError> {
    let Some(roadmap) = roadmap else {
        return Ok((0, 0));
    };
    let whole = |name: &str| {
        roadmap
            .body
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    match (whole("highest_phase"), whole("highest_story")) {
        (Some(phase), Some(story)) => Ok((phase, story)),
        _ => Err(StoreError::Unavailable(
            "the roadmap document cannot be read".into(),
        )),
    }
}

/// Whether `project`'s `scope.submitted` at `seq` names `digest`.
pub(crate) fn submitted_at<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    digest: Hash,
    seq: u64,
) -> Result<bool, StoreError> {
    let filter = HistoryFilter {
        types: vec![SCOPE_SUBMITTED.into()],
        git_commit: None,
    };
    let page = PageRequest {
        limit: 1,
        after: None,
    };
    let digest = json!(digest.to_hex());
    Ok(store
        .history(project, seq..=seq, &filter, page)?
        .items
        .iter()
        .any(|event| event.payload.get("digest") == Some(&digest)))
}

/// The newest start draft's accepted submission and sequence.
fn newest_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<(Submission, u64), Value> {
    let read = preview_of(payloads, document).map_err(preview_failed)?;
    accept(&read)
}

/// A recorded draft's body judged again. One that no longer judges clean,
/// or whose digest is not the draft's, is a ledger fault.
fn accept(read: &Preview) -> Result<(Submission, u64), Value> {
    match judge_submission(&read.submission) {
        Ok(submission) if submission.digest() == read.digest => Ok((submission, read.seq)),
        _ => Err(failed(
            LEDGER_UNAVAILABLE,
            "the draft is recorded, but its body no longer judges as the submission it was",
            PLACE_LEDGER,
        )),
    }
}

fn preview_failed(error: PreviewError) -> Value {
    match error {
        PreviewError::Store(error) => busy(error),
        PreviewError::UnknownDraft | PreviewError::Unreadable(_) => failed(
            LEDGER_UNAVAILABLE,
            "the draft is recorded, but its document or body cannot be read",
            PLACE_LEDGER,
        ),
    }
}

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
}

/// The `refused` answer for a story's truth draft given to the scope
/// approval: `unknown-draft`, since no scope draft has the digest, its
/// reason naming the command that approves truths with this digest, so the
/// owner is never told the ledger is damaged.
fn truth_draft(digest: Hash) -> Value {
    Refusal::new(
        ApprovalRefusal::UnknownDraft.code(),
        format!(
            "this digest is a story's truth draft, not a scope draft, so nothing was approved. Approve it with baley story truths approve {}",
            digest.to_hex()
        ),
    )
    .slot("digest")
    .value()
}

/// The `refused` answer for an approval, slot `digest`.
fn refused(refusal: &ApprovalRefusal) -> Value {
    let answer = match refusal {
        ApprovalRefusal::UnknownDraft => Refusal::new(
            refusal.code(),
            "no submission has this digest, so nothing was approved",
        ),
        ApprovalRefusal::ProjectAlreadyStarted => Refusal::new(
            refusal.code(),
            "this project's first scope is already approved, so no other start can be approved. Accepted scope changes through the scope-change commands",
        ),
        ApprovalRefusal::StaleDraft { path } => Refusal::new(
            refusal.code(),
            "a newer draft was submitted after this one, so it was not approved. The details name the first part that differs",
        )
        .details(json!({"path": path})),
    };
    answer.slot("digest").value()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ops::RangeInclusive;

    use baley_core::scope::{
        BACKLOG_VIEW, PHASE_VIEW, ROADMAP_VIEW, backlog_key, phase_key, roadmap_key,
    };
    use baley_store::{
        Actor, Anchor, Caller, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim,
        DecideReconcile, DocKey, Event, Head, OutcomeKind, Page, PayloadBody, PayloadStatus,
        ReconcileAuthority, Refusal as StoreRefusal, VerifyReport,
    };
    use serde_json::json;

    use super::*;
    use crate::scope::fixtures::*;
    use crate::scope::submit::answer;

    const OWNER: &str = "Ada Owner";

    fn digest_of(recorded: &Recorded) -> Hash {
        Hash::from_hex(receipt(recorded)["digest"].as_str().unwrap()).unwrap()
    }

    fn approve(
        store: &baley_store_sqlite::SqliteStore,
        judged: Option<&Judged>,
        request_id: &str,
        digest: u8,
        at: &str,
    ) -> Result<Recorded, StoreError> {
        record(
            store,
            &command(APPROVE_COMMAND, request_id, digest, at),
            judged,
            OWNER,
        )
    }

    fn outcome(recorded: &Recorded) -> OutcomeKind {
        let (Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) = recorded;
        outcome.kind
    }

    fn assert_refused(refusal: &Value, code: &str) {
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
    }

    fn assert_busy(value: &Value) {
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], "ledger-busy", "{value}");
        assert_eq!(value["retryable"], true, "{value}");
    }

    #[test]
    fn an_approval_recorded_as_baley_or_numbering_phases_and_stories_wrongly_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let judged = judge(&store, &project(), digest).unwrap();
        let recorded = approve(&store, Some(&judged), R2, 2, T2).unwrap();
        assert!(matches!(recorded, Recorded::New { .. }), "{recorded:?}");
        assert_eq!(outcome(&recorded), OutcomeKind::Done);

        let events = history(&store);
        let approval: Vec<_> = events
            .iter()
            .filter(|event| event.request_id.0 == R2)
            .collect();
        let types: Vec<&str> = approval.iter().map(|e| e.type_name.as_str()).collect();
        assert_eq!(
            types,
            [
                "project.described",
                "story.declared",
                "story.declared",
                "story.declared",
                "phase.declared",
                "phase.declared",
                "phase.reordered",
                "scope.approved",
                "command.completed",
            ]
        );
        for event in &approval {
            assert_eq!(event.actor, Actor::Owner, "{}", event.type_name);
            assert_eq!(
                event.caller,
                Some(Caller::Server(caller())),
                "{}",
                event.type_name
            );
        }
        let stories: Vec<(Value, Value)> = approval[1..4]
            .iter()
            .map(|e| (e.payload["id"].clone(), e.payload["phase"].clone()))
            .collect();
        assert_eq!(
            stories,
            [
                (json!(1), json!(1)),
                (json!(2), json!(2)),
                (json!(3), Value::Null)
            ]
        );
        let phases: Vec<Value> = approval[4..6]
            .iter()
            .map(|e| {
                json!([
                    e.payload["number"],
                    e.payload["name"],
                    e.payload["depends_on"],
                    e.payload["stories"]
                ])
            })
            .collect();
        assert_eq!(
            phases,
            [
                json!([1, "Records", [], [1]]),
                json!([2, "Approval", [1], [2]])
            ]
        );
        assert_eq!(approval[6].payload["order"], json!([1, 2]));
        assert_eq!(
            approval[7].payload,
            json!({"submission": digest.to_hex(), "kind": "start", "owner": OWNER, "at": T2})
        );

        let roadmap = store
            .get(&project(), ROADMAP_VIEW, &roadmap_key())
            .unwrap()
            .unwrap()
            .body;
        assert_eq!(roadmap["order"], json!([1, 2]));
        let numbers: Vec<&Value> = roadmap["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| &p["number"])
            .collect();
        assert_eq!(numbers, [1, 2]);
        let ids: Vec<&Value> = roadmap["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["id"])
            .collect();
        assert_eq!(ids, [1, 2, 3]);
        for number in [1, 2] {
            let phase = store
                .get(&project(), PHASE_VIEW, &phase_key(number))
                .unwrap()
                .unwrap();
            assert_eq!(phase.body["status"], "declared", "{number}");
        }
        let backlog = store
            .get(&project(), BACKLOG_VIEW, &backlog_key())
            .unwrap()
            .unwrap()
            .body;
        let listed: Vec<Value> = backlog["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let stored = s.get("refinement").is_some();
                json!([s["id"], stored, s["set"], s["refined"], s["truths"]])
            })
            .collect();
        assert_eq!(
            listed,
            [
                json!([1, false, null, null, 0]),
                json!([2, false, null, null, 0]),
                json!([3, false, null, null, 0])
            ]
        );

        assert_eq!(
            receipt(&recorded),
            json!({"status": "ok", "kind": "start", "digest": digest.to_hex(),
                "owner": OWNER, "at": T2, "phases": [1, 2], "stories": [1, 2, 3]})
        );
    }

    #[test]
    fn a_stale_unknown_or_second_start_approval_recorded_or_misnamed_is_caught() {
        let (_dir, store) = store();
        let a = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let b = digest_of(&submit(&store, &s2(), R2, 2, T2).unwrap());

        let before = head(&store);
        let stale = judge(&store, &project(), a).unwrap_err();
        assert_refused(&stale, "stale-draft");
        assert_eq!(stale["details"], json!({"path": "roadmap[1].goal"}));
        assert_eq!(head(&store), before);

        let unknown = judge(&store, &project(), Hash([0; 32])).unwrap_err();
        assert_refused(&unknown, "unknown-draft");
        assert_eq!(head(&store), before);

        let judged = judge(&store, &project(), b).unwrap();
        let approved = approve(&store, Some(&judged), R3, 3, T3).unwrap();
        assert_eq!(outcome(&approved), OutcomeKind::Done);

        let before = head(&store);
        let started = judge(&store, &project(), a).unwrap_err();
        assert_refused(&started, "project-already-started");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn an_approval_after_a_b_a_refusing_a_or_accepting_b_is_caught() {
        let (_dir, store) = store();
        let a = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let b = digest_of(&submit(&store, &s2(), R2, 2, T2).unwrap());
        submit(&store, &s1(), R3, 1, T3).unwrap();

        let stale = judge(&store, &project(), b).unwrap_err();
        assert_refused(&stale, "stale-draft");
        assert_eq!(stale["details"], json!({"path": "roadmap[1].goal"}));

        let judged = judge(&store, &project(), a).unwrap();
        let approved = approve(&store, Some(&judged), R5, 5, T3).unwrap();
        assert_eq!(outcome(&approved), OutcomeKind::Done);
    }

    #[test]
    fn a_replayed_approval_recording_twice_or_rebuilding_its_receipt_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let judged = judge(&store, &project(), digest).unwrap();
        let first = approve(&store, Some(&judged), R2, 2, T2).unwrap();

        let again = approve(&store, None, R2, 2, T3).unwrap();
        assert!(matches!(again, Recorded::Replayed { .. }), "{again:?}");
        assert_eq!(receipt(&again), receipt(&first));
        assert_eq!(receipt(&again)["at"], T2);
        assert_eq!(of_type(&history(&store), "scope.approved").len(), 1);
    }

    #[test]
    fn an_approval_whose_newest_draft_moved_before_its_transaction_recorded_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let judged = judge(&store, &project(), digest).unwrap();
        submit(&store, &s2(), R2, 2, T1).unwrap();
        let after = head(&store);

        let result = approve(&store, Some(&judged), R3, 3, T1);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        assert_busy(&answer(result, &store));
    }

    #[test]
    fn an_approval_recorded_after_another_start_approved_since_its_first_step_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        let earlier = judge(&store, &project(), digest).unwrap();
        let judged = judge(&store, &project(), digest).unwrap();
        approve(&store, Some(&judged), R2, 2, T2).unwrap();
        let after = head(&store);

        let result = approve(&store, Some(&earlier), R3, 3, T3);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        assert_busy(&answer(result, &store));
        assert_eq!(of_type(&history(&store), "scope.approved").len(), 1);
    }

    /// The fixtures' store with S1 approved at T3.
    fn approved() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let (dir, store) = store();
        approve_draft(
            &store,
            digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap()),
        );
        (dir, store)
    }

    #[test]
    fn a_change_approval_stale_by_another_target_or_approved_twice_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let edit = |value: &Value, request_id: &str, at: &str| {
            digest_of(&submit_change(&store, ChangeKind::PhaseEdit, value, request_id, at).unwrap())
        };
        let a = edit(&e_a(), R4, T4);
        let b = edit(&e_b(), R6, T5);
        let a2 = edit(&e_a2(), R7, T6);

        let before = head(&store);
        let stale = judge(&store, &project(), a).unwrap_err();
        assert_refused(&stale, "stale-draft");
        assert_eq!(stale["details"], json!({"path": "goal"}));
        assert_eq!(head(&store), before);
        judge(&store, &project(), b).unwrap();

        append(&store, R8, T7, vec![scope_approved(b, "phase-edit", T7)]);
        let before = head(&store);
        let again = judge(&store, &project(), b).unwrap_err();
        assert_refused(&again, "draft-approved");
        assert_eq!(again["details"], json!({"owner": OWNER, "at": T7}));
        assert_eq!(head(&store), before);
        judge(&store, &project(), a2).unwrap();

        assert_eq!(edit(&e_b(), R9, T8), b);
        judge(&store, &project(), b).unwrap();
    }

    #[test]
    fn a_change_approved_against_scope_that_has_since_refused_it_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let story = json!({"sentence": "Owners read progress", "status": "active", "phase": 2});
        let recorded = submit_change(&store, ChangeKind::StoryDeclare, &story, R4, T4).unwrap();
        append(
            &store,
            R2,
            T5,
            vec![phase_withdrawn(2), story_dropped(2), phase_reordered(&[1])],
        );
        let before = head(&store);

        let refusal = judge(&store, &project(), digest_of(&recorded)).unwrap_err();
        assert_refused(&refusal, "no-such-phase");
        assert_eq!(refusal["details"], json!({"phase": 2}));
        assert_eq!(head(&store), before);
    }

    #[test]
    fn an_edit_keeping_a_detail_this_project_purged_approved_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let recorded = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R4, T4).unwrap();
        let detail =
            Hash::from_hex("0eae12e355314abb5595d44ce4f4014a294d8d4cf709555975ef5f41648d7339")
                .unwrap();
        purge_in(&store, &project(), detail, "pasted a secret");
        let after = head(&store);

        let answer = judge(&store, &project(), digest_of(&recorded)).unwrap_err();
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "text-purged", "{answer}");
        assert_eq!(answer["place"], "detail", "{answer}");
        assert_eq!(answer["retryable"], false, "{answer}");
        assert_eq!(head(&store), after);
    }

    #[test]
    fn an_edit_carrying_a_detail_purged_between_its_steps_recorded_is_caught() {
        use baley_core::scope::ChangeKind;

        // The other project holds the same detail, so the purge releases
        // only this project's references and the body stays stored.
        let (_dir, store) = two_projects();
        for held in [project(), project_b()] {
            let digest = digest_of(&submit_in(&store, &held, &s1(), R1, 1, T1).unwrap());
            approve_draft_in(&store, &held, digest);
        }
        let recorded = submit_change(&store, ChangeKind::PhaseEdit, &e_b(), R4, T4).unwrap();
        let judged = judge(&store, &project(), digest_of(&recorded)).unwrap();
        let detail =
            Hash::from_hex("0eae12e355314abb5595d44ce4f4014a294d8d4cf709555975ef5f41648d7339")
                .unwrap();
        purge_in(&store, &project(), detail, "pasted a secret");
        let after = head(&store);

        let result = approve(&store, Some(&judged), R6, 6, T5);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        assert_busy(&answer(result, &store));
        let phase = read::phase(&store, &project(), 2).unwrap();
        assert_eq!(phase.detail, Detail::Purged("pasted a secret".into()));
    }

    /// Records `value` as a change of `kind` under `submit_id` at
    /// `submitted`, then approves it through both steps under `approve_id`
    /// at `approved`.
    fn approve_change(
        store: &baley_store_sqlite::SqliteStore,
        kind: baley_core::scope::ChangeKind,
        value: &Value,
        (submit_id, submitted): (&str, &str),
        (approve_id, approved): (&str, &str),
    ) -> (Hash, Recorded) {
        let digest = digest_of(&submit_change(store, kind, value, submit_id, submitted).unwrap());
        let judged = judge(store, &project(), digest).unwrap();
        let recorded = approve(store, Some(&judged), approve_id, 0xa, approved).unwrap();
        (digest, recorded)
    }

    /// The events request `request_id` recorded.
    fn recorded_by(store: &baley_store_sqlite::SqliteStore, request_id: &str) -> Vec<Event> {
        history(store)
            .into_iter()
            .filter(|event| event.request_id.0 == request_id)
            .collect()
    }

    fn types(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .map(|event| event.type_name.as_str())
            .collect()
    }

    #[test]
    fn a_change_approval_recorded_as_baley_losing_the_old_declaration_or_reusing_a_number_is_caught()
     {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let db = json!({"payload": "0eae12e355314abb5595d44ce4f4014a294d8d4cf709555975ef5f41648d7339",
            "bytes": 18, "class": "record"});
        let dp = json!({"payload": "40cd320a31a7ab226dc42f8ca21d7c9159fcdbec87ff12f2b1a8fc362ac0642a",
            "bytes": 24, "class": "record"});
        let as_owner = |events: &[Event]| {
            for event in events {
                assert_eq!(event.actor, Actor::Owner, "{}", event.type_name);
                assert_eq!(
                    event.caller,
                    Some(Caller::Server(caller())),
                    "{}",
                    event.type_name
                );
            }
        };

        let (e_b, _) = approve_change(&store, ChangeKind::PhaseEdit, &e_b(), (R4, T4), (R6, T5));
        let events = recorded_by(&store, R6);
        assert_eq!(
            types(&events),
            ["phase.declared", "scope.approved", "command.completed"]
        );
        assert_eq!(
            events[0].payload,
            json!({"number": 2, "name": "Approval", "goal": "The owner approves every change",
                "detail": db, "depends_on": [1], "stories": [2], "submission": E_B_DIGEST})
        );
        assert_eq!(
            events[1].payload,
            json!({"submission": e_b.to_hex(), "kind": "phase-edit", "owner": OWNER, "at": T5})
        );
        as_owner(&events);
        let all = history(&store);
        let goals: Vec<&Value> = of_type(&all, "phase.declared")
            .into_iter()
            .filter(|event| event.payload["number"] == 2)
            .map(|event| &event.payload["goal"])
            .collect();
        assert_eq!(
            goals,
            ["The owner approves", "The owner approves every change"]
        );

        let (w, _) = approve_change(&store, ChangeKind::PhaseWithdraw, &w(), (R7, T6), (R8, T7));
        let events = recorded_by(&store, R8);
        assert_eq!(
            types(&events),
            [
                "phase.withdrawn",
                "story.reassigned",
                "phase.reordered",
                "scope.approved",
                "command.completed"
            ]
        );
        let w_hex = w.to_hex();
        assert_eq!(events[0].payload, json!({"number": 2, "submission": w_hex}));
        assert_eq!(
            events[1].payload,
            json!({"id": 2, "status": "active", "phase": 1, "submission": w_hex})
        );
        assert_eq!(
            events[2].payload,
            json!({"order": [1], "submission": w_hex})
        );
        assert_eq!(
            events[3].payload,
            json!({"submission": w_hex, "kind": "phase-withdraw", "owner": OWNER, "at": T7})
        );
        as_owner(&events);
        let roadmap = store
            .get(&project(), ROADMAP_VIEW, &roadmap_key())
            .unwrap()
            .unwrap()
            .body;
        assert_eq!(roadmap["order"], json!([1]));
        let held: Vec<Value> = roadmap["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phase| json!([phase["number"], phase["status"], phase["stories"]]))
            .collect();
        assert_eq!(
            held,
            [json!([1, "declared", [1, 2]]), json!([2, "withdrawn", []])]
        );
        let phase = store
            .get(&project(), PHASE_VIEW, &phase_key(2))
            .unwrap()
            .unwrap();
        assert_eq!(phase.body["status"], "withdrawn");

        let (p, recorded) =
            approve_change(&store, ChangeKind::PhaseDeclare, &p(), (R2, T8), (R3, T9));
        let events = recorded_by(&store, R3);
        assert_eq!(
            types(&events),
            [
                "phase.declared",
                "phase.reordered",
                "scope.approved",
                "command.completed"
            ]
        );
        let p_hex = p.to_hex();
        assert_eq!(
            events[0].payload,
            json!({"number": 3, "name": "Reports", "goal": "The owner reads progress",
                "detail": dp, "depends_on": [1], "stories": [], "submission": p_hex})
        );
        assert_eq!(
            events[1].payload,
            json!({"order": [1, 3], "submission": p_hex})
        );
        assert_eq!(
            events[2].payload,
            json!({"submission": p_hex, "kind": "phase-declare", "owner": OWNER, "at": T9})
        );
        as_owner(&events);
        assert_eq!(
            receipt(&recorded),
            json!({"status": "ok", "kind": "phase-declare", "digest": p_hex, "owner": OWNER,
                "at": T9, "phases": [3], "stories": []})
        );
    }

    #[test]
    fn a_change_approval_recorded_after_scope_moved_since_its_first_step_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let w = digest_of(&submit_change(&store, ChangeKind::PhaseWithdraw, &w(), R4, T4).unwrap());
        let judged = judge(&store, &project(), w).unwrap();
        append(&store, R6, T4, vec![story_reassigned(3, 1)]);
        let after = head(&store);

        let result = approve(&store, Some(&judged), R7, 7, T5);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        assert_busy(&answer(result, &store));
    }

    #[test]
    fn a_declaration_past_the_largest_number_recorded_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let detail = baley_store::PayloadRef {
            hash: Hash::from_hex(
                "0eae12e355314abb5595d44ce4f4014a294d8d4cf709555975ef5f41648d7339",
            )
            .unwrap(),
            bytes: 18,
            class: RetentionClass::Record,
        };
        append(
            &store,
            R6,
            T3,
            vec![
                phase_declared(u32::MAX, detail),
                phase_reordered(&[1, 2, u32::MAX]),
            ],
        );
        let p = digest_of(&submit_change(&store, ChangeKind::PhaseDeclare, &p(), R4, T4).unwrap());
        let judged = judge(&store, &project(), p).unwrap();
        let before = head(&store);

        let result = approve(&store, Some(&judged), R7, 7, T5);
        assert!(
            matches!(result, Err(StoreError::Unavailable(_))),
            "{result:?}"
        );
        assert_eq!(head(&store), before);
        let value = answer(result, &store);
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], "ledger-unavailable", "{value}");
        assert_eq!(value["retryable"], false, "{value}");
    }

    #[test]
    fn a_change_approved_twice_in_a_race_is_caught() {
        use baley_core::scope::ChangeKind;

        let (_dir, store) = approved();
        let w = digest_of(&submit_change(&store, ChangeKind::PhaseWithdraw, &w(), R4, T4).unwrap());
        let judged = judge(&store, &project(), w).unwrap();
        append(
            &store,
            R6,
            T5,
            vec![scope_approved(w, "phase-withdraw", T5)],
        );
        let after = head(&store);

        let result = approve(&store, Some(&judged), R7, 7, T6);
        assert!(matches!(result, Err(StoreError::Stale(_))), "{result:?}");
        assert_eq!(head(&store), after);
        let approvals = of_type(&history(&store), "scope.approved")
            .into_iter()
            .filter(|event| event.payload["submission"] == w.to_hex())
            .count();
        assert_eq!(approvals, 1);
        assert_busy(&answer(result, &store));
    }

    /// The real store, with `land` run once just after the `after`-th read
    /// of the approval history returns: the window in which another
    /// approval can commit between the first step's reads.
    struct ApprovesDuring<'a> {
        store: &'a baley_store_sqlite::SqliteStore,
        after: usize,
        reads: Cell<usize>,
        land: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl<'a> ApprovesDuring<'a> {
        fn new(
            store: &'a baley_store_sqlite::SqliteStore,
            after: usize,
            land: impl FnOnce() + 'a,
        ) -> Self {
            Self {
                store,
                after,
                reads: Cell::new(0),
                land: RefCell::new(Some(Box::new(land))),
            }
        }

        fn landed(&self) -> bool {
            self.land.borrow().is_none()
        }
    }

    impl Ledger for ApprovesDuring<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::transact(self.store, command, decide)
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut DecideClaim<'_>,
        ) -> Result<Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &ClaimId,
            owner: &ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &ClaimOwner,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &ClaimId,
            authority: ReconcileAuthority,
            decide: &mut DecideReconcile<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: RangeInclusive<u64>,
            filter: &HistoryFilter,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            let read = Ledger::history(self.store, project, range, filter, page);
            if filter.types == [SCOPE_APPROVED] {
                self.reads.set(self.reads.get() + 1);
                if self.reads.get() == self.after {
                    let land = self.land.borrow_mut().take();
                    if let Some(land) = land {
                        land();
                    }
                }
            }
            read
        }

        fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&Anchor>,
        ) -> Result<VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    impl Views for ApprovesDuring<'_> {
        fn get(
            &self,
            project: &ProjectId,
            view: &str,
            key: &DocKey,
        ) -> Result<Option<Document>, StoreError> {
            Views::get(self.store, project, view, key)
        }

        fn get_many(
            &self,
            project: &ProjectId,
            view: &str,
            keys: &[DocKey],
        ) -> Result<Vec<Option<Document>>, StoreError> {
            Views::get_many(self.store, project, view, keys)
        }

        fn find(
            &self,
            project: &ProjectId,
            view: &str,
            query: &IndexQuery,
        ) -> Result<Page<Document>, StoreError> {
            Views::find(self.store, project, view, query)
        }
    }

    impl Payloads for ApprovesDuring<'_> {
        fn open(&self, hash: &Hash) -> Result<PayloadBody<'_>, StoreError> {
            Payloads::open(self.store, hash)
        }

        fn status(&self, hash: &Hash) -> Result<PayloadStatus, StoreError> {
            Payloads::status(self.store, hash)
        }
    }

    #[test]
    fn a_resubmitted_change_approved_twice_by_an_approval_between_its_reads_is_caught() {
        use baley_core::scope::ChangeKind;

        // Each pass lands the other approval after one more of the first
        // step's approval-history reads, until a pass makes no such read.
        let mut after = 1;
        loop {
            let (_dir, store) = approved();
            let (declared, _) =
                approve_change(&store, ChangeKind::PhaseDeclare, &p(), (R2, T4), (R3, T5));
            let resubmitted =
                submit_change(&store, ChangeKind::PhaseDeclare, &p(), R4, T6).unwrap();
            assert_eq!(digest_of(&resubmitted), declared);
            let racing = ApprovesDuring::new(&store, after, || {
                let judged = judge(&store, &project(), declared).unwrap();
                approve(&store, Some(&judged), R6, 6, T7).unwrap();
            });

            let first = judge(&racing, &project(), declared);
            if !racing.landed() {
                break;
            }
            let reply = match first {
                Err(refusal) => refusal,
                Ok(judged) => answer(approve(&store, Some(&judged), R7, 7, T8), &store),
            };
            assert_ne!(reply["status"], "ok", "after read {after}: {reply}");
            let approvals = of_type(&history(&store), "scope.approved")
                .into_iter()
                .filter(|event| event.payload["submission"] == declared.to_hex())
                .count();
            assert_eq!(approvals, 2, "after read {after}");
            after += 1;
        }
        assert!(after > 1, "the first step read no approval history");
    }

    #[test]
    fn two_approvals_under_one_request_id_accepted_is_caught() {
        let (_dir, store) = store();
        submit(&store, &s1(), R1, 1, T1).unwrap();
        let b = digest_of(&submit(&store, &s2(), R2, 2, T2).unwrap());
        let judged = judge(&store, &project(), b).unwrap();
        approve(&store, Some(&judged), R3, 0xb, T3).unwrap();

        let reused = approve(&store, None, R3, 0xa, T3);
        assert!(
            matches!(
                reused,
                Err(StoreError::Refused(
                    StoreRefusal::RequestDigestMismatch { .. }
                ))
            ),
            "{reused:?}"
        );
        let value = answer(reused, &store);
        assert_eq!(value["status"], "refused", "{value}");
        assert_eq!(value["code"], "request-id-reuse", "{value}");
        assert_eq!(value["slot"], "request_id", "{value}");
    }

    #[test]
    fn a_truth_draft_given_to_scope_approve_answered_as_a_damaged_ledger_is_caught() {
        use crate::refine::questions::tests::D;
        use crate::refine::submit::tests::{drafting, t1, t2, truths_submitted};

        let (_dir, store, set) = drafting();
        let receipt = truths_submitted(&store, 1, &set, &json!([t1(), t2()]), D, R6, T4);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        let x = receipt["digest"].as_str().unwrap().to_owned();
        let before = head(&store);
        let refusal = judge(&store, &project(), Hash::from_hex(&x).unwrap()).unwrap_err();
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "unknown-draft", "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
        let reason = refusal["reason"].as_str().unwrap();
        assert!(
            reason.contains(&format!("baley story truths approve {x}")),
            "{reason}"
        );
        assert_eq!(head(&store), before);
    }

    /// The commitment tests: store C, the `[1, 4]` edit E1 and the `[4, 1]`
    /// edit E2 recorded with their continuations, approved under P0 and N0
    /// (design 0005 `phase plan`, design 0004 `phase edit`, D-01, D-09 to
    /// D-11).
    mod commitment {
        use baley_core::scope::ChangeKind;
        use baley_store_sqlite::SqliteStore;

        use super::*;
        use crate::planning::fixtures::{
            activate_phase_2, approve_commitment, edit, n0, p0, refined, reopen, store_c, versioned,
        };
        use crate::planning::order::tests::{O, O2};
        use crate::planning::phase_plan::tests::bound_stories;
        use crate::refine::order::tests::issued;
        use crate::refine::submit::tests::t1;

        /// Phase 1's stories as the roadmap holds them.
        pub(super) fn phase_1_stories(store: &SqliteStore) -> Vec<u32> {
            read::phase(store, &project(), 1).unwrap().stories
        }

        /// The first step of `digest`, which must refuse with `code` at slot
        /// `digest` and change neither phase 1's stories nor the orders.
        pub(super) fn refused_with(store: &SqliteStore, digest: Hash, code: &str) -> Value {
            let before = head(store);
            let refusal = judge(store, &project(), digest).expect_err("the approval is refused");
            assert_refused(&refusal, code);
            assert_eq!(head(store), before);
            assert_eq!(phase_1_stories(store), [1]);
            assert!(issued(store).is_empty());
            refusal
        }

        #[test]
        fn a_planner_launched_before_its_commitment_is_approved_or_a_refused_approval_recording_is_caught()
         {
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let e2 = edit(&store, &[4, 1], O2, R2, T5);
            refused_with(&store, e1, "stale-draft");

            let answer = approve_commitment(&store, e2, R3, T6);
            assert_eq!(answer["status"], "ok", "{answer}");
            assert_eq!(phase_1_stories(&store), [4, 1]);
            let events = history(&store);
            let named = of_type(&events, "plan.continuation")
                .into_iter()
                .find(|event| event.payload["edit"] == e2.to_hex())
                .expect("E2 has its continuation")
                .payload["work_order"]
                .clone();
            assert_eq!(named, O2);
            let orders = issued(&store);
            assert_eq!(orders.len(), 1);
            assert_eq!(orders[0].stream, "phase/1");
            assert_eq!(orders[0].payload["subject"], "phase/1");
            assert_eq!(orders[0].payload["id"], named);
            assert_eq!(bound_stories(&store, &orders[0]), [4, 1]);
            assert_eq!(answer["work_order"], named, "{answer}");

            // Story 4 moved to phase 2 by an approved reassignment.
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let reassign = submit_change(
                &store,
                ChangeKind::StoryReassign,
                &json!({"story": 4, "phase": 2}),
                R2,
                T5,
            )
            .unwrap();
            let judged = judge(&store, &project(), digest_of(&reassign)).unwrap();
            approve(&store, Some(&judged), R3, 6, T5).unwrap();
            let refusal = refused_with(&store, e1, "story-in-phase");
            assert_eq!(refusal["details"], json!({"story": 4, "phase": 2}));

            // Phase 2 made active.
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            activate_phase_2(&store, R2);
            let refusal = refused_with(&store, e1, "phase-active");
            assert_eq!(refusal["details"], json!({"phase": 2}));

            // Story 4's truths emptied.
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            append(
                &store,
                R2,
                T5,
                vec![refined(
                    4,
                    "4e4e4e4e-4e4e-4e4e-8e4e-4e4e4e4e4e4e",
                    Vec::new(),
                )],
            );
            let refusal = refused_with(&store, e1, "story-unrefined");
            assert_eq!(refusal["details"], json!({"story": 4}));
        }

        /// E1's second step as `Ada Owner` under P0 and N0, as
        /// `request_id` at `at`.
        fn second_step(store: &SqliteStore, judged: &Judged, request_id: &str, at: &str) -> Value {
            let (policy, names) = (p0(), n0());
            record_commitment(
                store,
                &command(APPROVE_COMMAND, request_id, 6, at),
                judged,
                OWNER,
                OrderRoute {
                    policy: &policy,
                    host: Host::ClaudeCode,
                    names: &names,
                },
            )
        }

        #[test]
        fn a_commitment_landing_beside_a_moved_truth_or_a_new_planner_order_is_caught() {
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let judged = judge(&store, &project(), e1).unwrap();
            append(
                &store,
                R2,
                T5,
                vec![refined(
                    4,
                    "4f4f4f4f-4f4f-4f4f-8f4f-4f4f4f4f4f4f",
                    vec![versioned(t1())],
                )],
            );
            let before = head(&store);
            assert_busy(&second_step(&store, &judged, R3, T6));
            assert_eq!(head(&store), before);

            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let judged = judge(&store, &project(), e1).unwrap();
            let (command_q, q) = crate::planning::order::tests::prepared(&store, &[1], O2, R2, T5);
            let issued_q = crate::planning::order::issue(&store, &command_q, &q);
            assert_eq!(issued_q["status"], "ok", "{issued_q}");
            let before = head(&store);
            assert_busy(&second_step(&store, &judged, R3, T6));
            assert_eq!(head(&store), before);
            assert_eq!(issued(&store).len(), 1);
        }

        #[test]
        fn a_commitment_approval_rerun_naming_no_order_or_recording_is_caught() {
            let (dir, store) = store_c();
            let e2 = edit(&store, &[4, 1], O, R1, T4);
            let approved = approve_commitment(&store, e2, R2, T5);
            assert_eq!(approved["status"], "ok", "{approved}");
            drop(store);
            let store = reopen(&dir);

            let before = head(&store);
            let refusal = judge(&store, &project(), e2).expect_err("E2 is approved");
            assert_refused(&refusal, "draft-approved");
            assert_eq!(refusal["details"]["work_order"], O, "{refusal}");
            assert!(refusal["reason"].as_str().unwrap().contains(O), "{refusal}");
            assert_eq!(head(&store), before);
            assert_eq!(issued(&store).len(), 1);
        }

        #[test]
        fn a_commitment_approved_beside_a_pending_planner_order_without_naming_it_is_caught() {
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let (command_q, q) = crate::planning::order::tests::prepared(&store, &[1], O2, R2, T5);
            let issued_q = crate::planning::order::issue(&store, &command_q, &q);
            assert_eq!(issued_q["status"], "ok", "{issued_q}");

            let before = head(&store);
            let refusal = judge(&store, &project(), e1).expect_err("Q is pending");
            assert_refused(&refusal, "planner-order-pending");
            assert_eq!(refusal["details"], json!({"work_order": O2}));
            assert_eq!(head(&store), before);
        }

        #[test]
        fn a_commitment_landing_beside_a_newly_active_phase_is_caught() {
            let (_dir, store) = store_c();
            let e1 = edit(&store, &[1, 4], O, R1, T4);
            let judged = judge(&store, &project(), e1).unwrap();
            activate_phase_2(&store, R2);
            let before = head(&store);
            assert_busy(&second_step(&store, &judged, R3, T6));
            assert_eq!(head(&store), before);
            assert_eq!(phase_1_stories(&store), [1]);
            assert!(issued(&store).is_empty());
        }

        #[test]
        fn a_plain_phase_edit_approval_issuing_an_order_is_caught() {
            let (_dir, store) = store_c();
            let fields = baley_core::scope::PhaseFields {
                goal: Some("The owner approves every change".into()),
                ..Default::default()
            };
            let digest = crate::scope::submit::submit_phase_edit(
                &store,
                &command(crate::scope::submit::SUBMIT_COMMAND, R1, 7, T4),
                2,
                &fields,
                None,
            )
            .unwrap();
            let judged = judge(&store, &project(), digest).unwrap();
            let answer = second_step(&store, &judged, R2, T5);
            assert_eq!(answer["status"], "ok", "{answer}");
            assert_eq!(
                read::phase(&store, &project(), 2).unwrap().goal,
                "The owner approves every change"
            );
            assert!(issued(&store).is_empty());
        }
    }
}
