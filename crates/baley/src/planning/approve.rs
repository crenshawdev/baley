//! The owner's approval of a plan draft by its digest (design 0005 section
//! 5 `plan approve`, PLN-R14, PLN-R15, PLN-R26, PLN-R27, D-02, D-06, D-07,
//! D-14, D-16), in two steps.
//!
//! The first step reads and judges outside any transaction, so a refusal
//! records nothing: the draft in D-16's order, then `phase-not-active` when
//! a different phase is active, then the plan judged again against the
//! phase as it stands now. Build 4 T14 adds `review-unsettled` after these
//! refusals, judged against the same digest. A replacement draft is judged
//! by the replacement judge again before its plan is. A first step whose
//! capacity or suite differs from the policy the approval is recorded under
//! is run again under that policy. The second step records one
//! `plan.approved` as the owner with the next plan number, and for a
//! replacement `plan.replaced` with it, confirming inside its transaction
//! that nothing the first step read has moved.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use baley_core::plan::{
    DraftRead, PHASE_PLAN_VIEW, PLAN_APPROVED, PLAN_APPROVED_VERSION, PLAN_KIND, PLAN_REPLACED,
    PLAN_REPLACED_VERSION, Plan, PlanApproval, PlanApprovalFacts, PlanApprovalRefusal, PlanDraft,
    RecordedPlan, Replaces, judge_plan, judge_plan_approval, judge_submit_phase, phase_plan_key,
    phase_stream, read_plan_draft, replaced_payload,
};
use baley_core::policy::{EffectivePolicy, Value as Setting};
use baley_core::questions::{QUESTIONS_VIEW, SetFacts, questions_key};
use baley_core::scope::{DRAFT_VIEW, PROJECT_STREAM, PriorApproval, SCOPE_SUBMITTED, draft_key};
use baley_store::{
    Absence, Actor, Command, Decision, Document, EVENT_PAGE_BOUND, EventMatch, Hash, HistoryFilter,
    Ledger, NewEvent, Observed, ObservedDocument, OutcomeKind, PageRequest, PayloadRef, Payloads,
    ProjectId, Recorded, StaleInput, StoreError, StreamName, Transaction, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::submit::{phase_not_active, plan_refused};
use crate::planning::{facts, replace};
use crate::questions::read::{SetRead, read as read_set};
use crate::scope::approve::newest_query;
use crate::scope::read::{Preview, PreviewError, preview, preview_of};
use crate::scope::submit::NOT_RECORDED_BUSY;

/// What a refused approval did not do.
const NOT_APPROVED: &str = "nothing was approved";

/// The setting the suite is read from.
const SUITE_SETTING: &str = "workflow.test_command";

/// What the first step read and judged, which the second step confirms and
/// records.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    /// The approved draft's digest.
    digest: Hash,
    /// The sequence of its latest submission.
    seq: u64,
    /// The newest plan draft of its phase, digest and sequence.
    newest: (Hash, u64),
    /// Whether a `plan.approved` naming the digest was recorded before its
    /// latest submission.
    approved_before: bool,
    /// The plan, judged again against the phase as it stands.
    plan: Plan,
    /// The approved plan the draft replaces, or none for a new plan.
    replaces: Option<Replaces>,
    /// The evidence items exactly as submitted.
    evidence_map: Vec<Value>,
    /// The id of the draft's question set.
    set: String,
    /// The capacity the plan was judged under, or none.
    capacity: Option<NonZeroU32>,
    /// The project's test command, or none (D-14).
    suite: Option<String>,
    /// The draft body's reference, as its `scope.submitted` holds it.
    body: PayloadRef,
    /// The `phase_plan` document, the draft's set's `questions` document,
    /// then the facts' documents, each at the sequence that produced it.
    documents: Vec<ObservedDocument>,
    /// What the recording relies on being absent.
    absences: Vec<Absence>,
}

/// One recorded approval of a digest.
struct Approval {
    seq: u64,
    owner: String,
    at: String,
}

/// The first step of approving the plan draft `digest` in `project` under
/// `policy`, the effective policy: reads the draft (none, or one that is
/// not kind `plan`, is `unknown-draft`; a body that does not read back as a
/// plan draft is a ledger fault, never approved), its `scope.submitted` at
/// its sequence with the body's reference, the newest plan draft of its
/// phase, the phase's `phase_plan` document with its current draft's set and
/// that set's facts (a different current draft there, submitted after the
/// newest read, standing as the newest, so the draft is `stale-draft`
/// rather than a ledger fault), and last every `plan.approved` naming the
/// digest, so one landing after that read moves the `phase_plan` document
/// the second step observes. It judges them in D-16's order, then refuses
/// `phase-not-active` naming another active phase (D-02), then gathers the
/// phase's facts now with the policy's capacity and judges the plan again
/// with `judge_plan` (D-16), every refusal at slot `digest`. The suite is
/// the policy's `workflow.test_command`, none when it is not set (D-14).
/// A draft whose body names the plan it replaces is judged, after
/// `phase-not-active`, by the replacement judge again over the phase now,
/// `no-such-plan` or `admitted-plan` at slot `replaces`, and its plan judged
/// with the replaced plan's tasks and items off the phase (D-04). Nothing is
/// recorded.
pub fn judge<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
    policy: &EffectivePolicy,
) -> Result<Judged, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let (target, seq) = match preview(store, project, digest) {
        Ok(read) if read.kind == PLAN_KIND => accept(&read)?,
        Ok(_) | Err(PreviewError::UnknownDraft) => {
            return Err(refused(&PlanApprovalRefusal::UnknownDraft));
        }
        Err(error) => return Err(preview_failed(error)),
    };
    let phase = target
        .plan
        .get("phase")
        .and_then(Value::as_u64)
        .and_then(|phase| u32::try_from(phase).ok())
        .ok_or_else(|| unreadable("the draft's phase cannot be read"))?;
    let body = submitted_body(store, project, digest, seq).map_err(busy)?;
    let newest = match store
        .find(
            project,
            DRAFT_VIEW,
            &newest_query(PLAN_KIND, &phase_stream(phase)),
        )
        .map_err(busy)?
        .items
        .into_iter()
        .next()
    {
        Some(document) => {
            let read = preview_of(store, &document).map_err(preview_failed)?;
            let (plan, newest_seq) = accept(&read)?;
            Some((read.digest, newest_seq, plan))
        }
        None => None,
    };
    let key = phase_plan_key(phase);
    let phase_plan = store.get(project, PHASE_PLAN_VIEW, &key).map_err(busy)?;
    let mut documents = vec![ObservedDocument {
        view: PHASE_PLAN_VIEW.into(),
        key,
        produced_seq: phase_plan.as_ref().map(|document| document.produced_seq),
    }];
    let current = current_draft(phase_plan.as_ref().map(|document| &document.body), digest)?;
    // The `phase_plan` document is read after the newest index, so a draft
    // it holds in this one's place was submitted since: judged the newest,
    // this draft is refused `stale-draft`, as a later run refuses it.
    let newest = match current {
        Current::Other(other)
            if newest
                .as_ref()
                .is_none_or(|(newest, _, _)| *newest != other) =>
        {
            let read = preview(store, project, other).map_err(preview_failed)?;
            let (plan, other_seq) = accept(&read)?;
            Some((read.digest, other_seq, plan))
        }
        _ => newest,
    };
    let set = match current {
        Current::This(set) => Some(set),
        Current::None | Current::Other(_) => None,
    };
    let facts = match &set {
        Some(set) => match read_set(store, project, set).map_err(busy)? {
            SetRead::Held(held) => {
                documents.push(ObservedDocument {
                    view: QUESTIONS_VIEW.into(),
                    key: questions_key(set),
                    produced_seq: Some(held.produced_seq),
                });
                held.facts
            }
            SetRead::NotHeld | SetRead::Unreadable(_) => {
                return Err(unreadable("the draft's question set cannot be read"));
            }
        },
        None => SetFacts {
            questions: Vec::new(),
            answers: Vec::new(),
            deferrals: Vec::new(),
            abandoned: None,
        },
    };
    // Read after the `phase_plan` document: an approval landing before this
    // read is seen here, and one landing after it moves that document.
    let approvals = approvals_of(store, project, phase, digest).map_err(busy)?;
    let prior = approvals.iter().find(|approval| approval.seq > seq);
    judge_plan_approval(&PlanApprovalFacts {
        target: Some(DraftRead {
            digest,
            plan: &target,
        }),
        recorded: body.is_some(),
        newest: newest.as_ref().map(|(digest, _, plan)| DraftRead {
            digest: *digest,
            plan,
        }),
        approved: prior.map(|approval| PriorApproval {
            owner: &approval.owner,
            at: &approval.at,
        }),
        set: &facts,
    })
    .map_err(|refusal| refused(&refusal))?;
    // An approved judgement had its event, a newest draft and, as the
    // phase's current draft, a set: a draft replaced since was refused
    // stale above, so a missing one here is the ledger's fault.
    let (Some(body), Some((newest_digest, newest_seq, _)), Some(set)) = (body, newest, set) else {
        return Err(unreadable(
            "the draft's record or its question set cannot be read",
        ));
    };

    let capacity = facts::capacity(policy);
    let read = facts::read(store, project, phase, capacity)?;
    judge_submit_phase(phase, &read.active)
        .map_err(|refusal| phase_not_active(&refusal, "digest", NOT_APPROVED))?;
    let facts = match target.replaces {
        None => read.facts.clone(),
        Some(replaces) => {
            replace::facts_without(store, project, phase, replaces, &read, NOT_APPROVED)?
        }
    };
    let plan = judge_plan(&target.plan, &facts)
        .map_err(|refusal| plan_refused(&refusal, "digest", NOT_APPROVED))?;
    let draft = match target.replaces {
        None => PlanDraft::new(&plan),
        Some(replaces) => PlanDraft::replacing(&plan, replaces),
    };
    if draft.digest() != digest {
        return Err(unreadable(
            "the draft's body no longer reads as the plan it was submitted as",
        ));
    }
    documents.extend_from_slice(read.documents());
    let evidence_map = target.plan["evidence_map"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(Judged {
        digest,
        seq,
        newest: (newest_digest, newest_seq),
        approved_before: approvals.iter().any(|approval| approval.seq < seq),
        plan,
        replaces: target.replaces,
        evidence_map,
        set,
        capacity,
        suite: suite(policy),
        body,
        documents,
        absences: read.absences().to_vec(),
    })
}

/// The first step's `judged` held to `policy`, the policy the approval is
/// recorded under: `judged` itself when `policy` sets the capacity and the
/// suite it was judged under, otherwise the first step run again under
/// `policy`. The terminal reads the settings once to judge and again at
/// the policy step, so a change between the two reads is judged under the
/// version the approval names, never recorded on the first read's
/// judgement. Nothing is recorded.
pub fn rejudged<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    judged: Judged,
    policy: &EffectivePolicy,
) -> Result<Judged, Value> {
    if judged.capacity == facts::capacity(policy) && judged.suite == suite(policy) {
        return Ok(judged);
    }
    judge(store, project, judged.digest, policy)
}

/// The second step: records the approval judged by the first step under a
/// copy of `command` whose actor is the owner, with `owner` as the owner's
/// name and the command's time as Baley's (D-06). Inside the transaction it
/// confirms the draft at the sequence read with its `scope.submitted`, the
/// newest plan draft of the phase unchanged by digest and sequence, and no
/// `plan.approved` of the digest since; then takes one more than the highest
/// number ever assigned in the phase from `phase_plan`, 1 when none, a
/// number past `u32::MAX` being a store fault with nothing recorded (D-07);
/// and appends `plan.approved` on `phase/<n>` with the phase, the number,
/// the digest, the stories, the size, the suite or null, the set, the
/// evidence map inline exactly as submitted and the body's reference
/// attached as it is held; for a replacement, then `plan.replaced` naming
/// the phase, the old number, the old digest and the new one, the owner and
/// the time, in the same transaction (D-04, D-07). The decision observes every document the first
/// step read and declares its absences, so anything moved since, a phase
/// made active among them, makes the call `ledger-busy` with nothing
/// recorded. A retry of a request the store holds is answered from its
/// record before any decision runs; a decision run without the first step's
/// reads records nothing.
///
/// The receipt: `status`, `kind`, `digest`, `phase`, `plan`, `stories`,
/// `size`, `suite` (null when none, with `notice` naming
/// `workflow.test_command` and why it matters), `set`, `owner`, `at` and,
/// for a replacement, `replaces` naming the plan replaced and its digest.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    judged: Option<&Judged>,
    owner: &str,
) -> Result<Recorded, StoreError> {
    let command = Command {
        actor: Actor::Owner,
        ..command.clone()
    };
    let at = command.recorded_at.clone();
    store.transact(&command, &mut |tx| {
        let Some(judged) = judged else {
            return Err(StoreError::Unavailable(
                "a plan approval reached its transaction without its first step's reads".into(),
            ));
        };
        let phase = judged.plan.phase();
        confirm_draft(tx, judged)?;
        // A transaction cannot ask for an approval after a sequence, so when
        // one preceded the latest submission a later one is left to the
        // `phase_plan` document, which every approval of the phase moves.
        let approved = approval_of(phase, judged.digest);
        if !judged.approved_before && tx.event_exists(&approved)? {
            return Err(StoreError::Stale(StaleInput::Absence(Absence::Event(
                approved,
            ))));
        }
        confirm_newest(tx, phase, judged)?;
        let number = next_number(tx, phase)?;
        let approval = PlanApproval {
            phase,
            plan: number,
            digest: judged.digest,
            stories: judged.plan.stories(),
            size: judged.plan.size(),
            suite: judged.suite.as_deref(),
            question_set: &judged.set,
            evidence_map: &judged.evidence_map,
            body: &judged.body,
            owner,
            at: &at,
        };
        tx.append(NewEvent {
            stream: StreamName(phase_stream(phase)),
            type_name: PLAN_APPROVED.into(),
            type_version: PLAN_APPROVED_VERSION,
            git: None,
            payload: approval.payload(),
            attachments: vec![judged.body.clone()],
        })?;
        // The new number is assigned first, so the replaced plan's number
        // stays counted in `highest` and is never assigned again (D-07).
        if let Some(replaces) = judged.replaces {
            tx.append(NewEvent {
                stream: StreamName(phase_stream(phase)),
                type_name: PLAN_REPLACED.into(),
                type_version: PLAN_REPLACED_VERSION,
                git: None,
                payload: replaced_payload(
                    phase,
                    replaces.plan,
                    replaces.digest,
                    judged.digest,
                    owner,
                    &at,
                ),
                attachments: Vec::new(),
            })?;
        }
        let mut receipt = json!({
            "status": "ok",
            "kind": PLAN_KIND,
            "digest": judged.digest.to_hex(),
            "phase": phase,
            "plan": number,
            "stories": judged.plan.stories(),
            "size": judged.plan.size(),
            "suite": judged.suite,
            "set": judged.set,
            "owner": owner,
            "at": at,
        });
        if let Some(replaces) = judged.replaces {
            receipt["replaces"] =
                json!({"plan": replaces.plan, "digest": replaces.digest.to_hex()});
        }
        if judged.suite.is_none() {
            receipt["notice"] = json!(format!(
                "{SUITE_SETTING} is not set, so the plan was approved with no suite and cannot be admitted to execution until it is set"
            ));
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: receipt,
            sensitive: false,
            observed: Observed {
                documents: judged.documents.clone(),
                absences: judged.absences.clone(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// A `plan.approved` on phase `phase`'s stream naming `digest`.
fn approval_of(phase: u32, digest: Hash) -> EventMatch {
    EventMatch {
        type_name: PLAN_APPROVED.into(),
        stream: Some(StreamName(phase_stream(phase))),
        fields: BTreeMap::from([("digest".to_string(), json!(digest.to_hex()))]),
    }
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

/// The newest plan draft of phase `phase` is the one the first step read,
/// by digest and sequence, never by time: two submissions can share a time.
fn confirm_newest(tx: &mut dyn Transaction, phase: u32, judged: &Judged) -> Result<(), StoreError> {
    let newest = tx
        .find(DRAFT_VIEW, &newest_query(PLAN_KIND, &phase_stream(phase)))?
        .items
        .into_iter()
        .next();
    let now = newest.as_ref().and_then(|document| {
        let digest = document
            .body
            .get("digest")
            .and_then(Value::as_str)
            .and_then(Hash::from_hex)?;
        Some((digest, sequence(document)?))
    });
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

/// One more than the highest plan number ever assigned in phase `phase`,
/// read from `phase_plan` inside `tx`, 1 when none was (D-07).
fn next_number(tx: &mut dyn Transaction, phase: u32) -> Result<u32, StoreError> {
    let highest = match tx.get(PHASE_PLAN_VIEW, &phase_plan_key(phase))? {
        None => 0,
        Some(document) => document
            .body
            .get("highest")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                StoreError::Unavailable(format!("phase {phase}'s plan numbers cannot be read"))
            })?,
    };
    highest
        .checked_add(1)
        .and_then(|next| u32::try_from(next).ok())
        .ok_or_else(|| StoreError::Unavailable(format!("phase {phase} has no plan number left")))
}

/// The latest submission's sequence a `draft` document holds.
fn sequence(document: &Document) -> Option<u64> {
    document.body.get("seq").and_then(Value::as_u64)
}

/// The suite `policy` sets, or none.
fn suite(policy: &EffectivePolicy) -> Option<String> {
    match policy
        .settings
        .get(SUITE_SETTING)
        .and_then(|effective| effective.value.as_ref())
    {
        Some(Setting::Command(command)) => Some(command.clone()),
        _ => None,
    }
}

/// A recorded plan draft read back, with its latest submission's sequence.
/// A body that does not read back as a plan draft is a ledger fault.
fn accept(read: &Preview) -> Result<(RecordedPlan, u64), Value> {
    read_plan_draft(&read.submission)
        .map(|plan| (plan, read.seq))
        .ok_or_else(|| unreadable("the draft's body no longer reads as a plan draft"))
}

/// The body reference of `project`'s `scope.submitted` at `seq` naming
/// `digest`, or none when no such event is there.
fn submitted_body<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    digest: Hash,
    seq: u64,
) -> Result<Option<PayloadRef>, StoreError> {
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
        .find(|event| event.payload.get("digest") == Some(&digest))
        .and_then(|event| PayloadRef::from_value(&event.payload["body"])))
}

/// The phase's current plan draft as its `phase_plan` document holds it,
/// beside the draft being approved.
enum Current {
    /// The phase holds no unapproved draft.
    None,
    /// The draft being approved is current, with its set's id.
    This(String),
    /// Another draft, by its digest, replaced it (D-03).
    Other(Hash),
}

/// The phase's current draft beside `digest`. A draft that cannot be read
/// is a ledger fault.
fn current_draft(phase_plan: Option<&Value>, digest: Hash) -> Result<Current, Value> {
    let draft = match phase_plan.and_then(|body| body.get("draft")) {
        None | Some(Value::Null) => return Ok(Current::None),
        Some(draft) => draft,
    };
    let held = draft
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex);
    match (held, draft.get("set").and_then(Value::as_str)) {
        (Some(held), Some(set)) if held == digest => Ok(Current::This(set.to_owned())),
        (Some(held), Some(_)) => Ok(Current::Other(held)),
        _ => Err(unreadable("the phase's current plan draft cannot be read")),
    }
}

/// Every `plan.approved` on phase `phase`'s stream naming `digest`, oldest
/// first, read from the events.
fn approvals_of<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    phase: u32,
    digest: Hash,
) -> Result<Vec<Approval>, StoreError> {
    let filter = HistoryFilter {
        types: vec![PLAN_APPROVED.into()],
        git_commit: None,
    };
    let stream = phase_stream(phase);
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
                    event.stream == stream && event.payload.get("digest") == Some(&digest)
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

fn preview_failed(error: PreviewError) -> Value {
    match error {
        PreviewError::Store(error) => store_failed(&error, NOT_RECORDED_BUSY),
        PreviewError::UnknownDraft | PreviewError::Unreadable(_) => {
            unreadable("the draft is recorded, but its document or body cannot be read")
        }
    }
}

fn unreadable(reason: &str) -> Value {
    failed(LEDGER_UNAVAILABLE, reason, PLACE_LEDGER)
}

/// The `refused` answer for a plan approval, slot `digest`, with what the
/// refusal names in its details.
fn refused(refusal: &PlanApprovalRefusal) -> Value {
    let reason = match refusal {
        PlanApprovalRefusal::UnknownDraft => {
            "no plan draft has this digest, so nothing was approved"
        }
        PlanApprovalRefusal::StaleDraft { .. } => {
            "a newer plan draft of the same phase was submitted after this one, so it was not approved. The details name the first part that differs"
        }
        PlanApprovalRefusal::DraftApproved { .. } => {
            "this draft was already approved after its latest submission, so it was not approved again. The details name that approval"
        }
        PlanApprovalRefusal::QuestionOpen { .. } => {
            "a question of the draft's set is neither answered nor deferred, so the plan was not approved. The details name each open question"
        }
        PlanApprovalRefusal::AnswerNotApplied { .. } => {
            "an answer on the draft's set rejected its recommended answer, so the plan does not yet apply it and was not approved. The details name each such question"
        }
    };
    let answer = Refusal::new(refusal.code(), reason).slot("digest");
    match refusal.details() {
        Some(details) => answer.details(details),
        None => answer,
    }
    .value()
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005
    //! sections 5 and 6, D-02, D-03, D-05, D-06, D-07, D-14, D-16 and the
    //! plan's data store C, P0, P3, PL1, PL2 and PL3, never from running this
    //! code.

    use std::cell::RefCell;
    use std::ops::RangeInclusive;
    use std::path::Path;

    use baley_core::policy::{Host, Schema, effective_policy};
    use baley_core::scope::{PROJECT_STREAM, SCOPE_SUBMITTED_VERSION, submitted_payload};
    use baley_store::{
        Anchor, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim, DecideReconcile,
        Decision, DocKey, Event, Head, IndexQuery, NewEvent, Observed, OutcomeKind, Page,
        PayloadBody, PayloadStatus, ReconcileAuthority, RetentionClass, StreamName, VerifyReport,
        canonical_json,
    };
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit::tests::{order_o, order_q, plan_submitted};
    use crate::scope::fixtures::*;

    /// P0 with a project file setting `planning.phase_capacity = 1`.
    fn capacity_one() -> EffectivePolicy {
        let file = crate::settings::file(
            Path::new("/r/baley.toml"),
            b"[planning]\nphase_capacity = 1\n".to_vec(),
        );
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            None,
            Some(&file),
        )
        .expect("the capacity is read")
    }

    /// The digest a submit receipt names.
    pub(crate) fn digest_of(receipt: &Value) -> Hash {
        assert_eq!(receipt["status"], "ok", "{receipt}");
        Hash::from_hex(receipt["digest"].as_str().unwrap()).unwrap()
    }

    /// The first step of `digest` under `policy`, which must refuse with
    /// `code` at slot `digest` and record nothing. Gives the refusal.
    fn refused_with(
        store: &SqliteStore,
        digest: Hash,
        policy: &EffectivePolicy,
        code: &str,
    ) -> Value {
        let before = head(store);
        let refusal =
            judge(store, &project(), digest, policy).expect_err("the approval is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
        assert_eq!(head(store), before);
        refusal
    }

    /// Records a draft of `kind` filed under `target` by hand, its body a
    /// `record` payload, under `request_id`. Gives its digest.
    fn drafted_by_hand(
        store: &SqliteStore,
        kind: &str,
        target: &str,
        digest: Hash,
        request_id: &str,
    ) -> Hash {
        let body = canonical_json(&json!({"kind": kind})).unwrap();
        store
            .transact(&command("test.draft", request_id, 9, T5), &mut |tx| {
                let body = tx.put_payload(&body, RetentionClass::Record)?;
                tx.append(NewEvent {
                    stream: StreamName(PROJECT_STREAM.into()),
                    type_name: SCOPE_SUBMITTED.into(),
                    type_version: SCOPE_SUBMITTED_VERSION,
                    git: None,
                    payload: submitted_payload(digest, kind, target, &body),
                    attachments: vec![body],
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .expect("the draft is recorded");
        digest
    }

    #[test]
    fn a_plan_approval_judged_against_the_phase_as_submitted_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        approve_by_hand(&store, R3, 1, 1, Hash([0x7d; 32]), &[1], 1, &[], Vec::new());
        let refusal = refused_with(&store, x1, &capacity_one(), "over-capacity");
        assert_eq!(refusal["details"], json!({"stories": [1], "size": 2}));
    }

    #[test]
    fn an_approval_recorded_on_settings_other_than_its_policy_steps_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        approve_by_hand(&store, R3, 1, 1, Hash([0x7d; 32]), &[1], 1, &[], Vec::new());
        // Judged under P0, with no capacity and no suite set; the policy
        // step then reads a capacity of 1, or a suite.
        let judged = judge(&store, &project(), x1, &p0()).expect("PL1 fits with no capacity");

        let before = head(&store);
        let refusal = rejudged(&store, &project(), judged.clone(), &capacity_one())
            .expect_err("PL1 takes phase 1 to 2 tasks, over a capacity of 1");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "over-capacity", "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
        assert_eq!(refusal["details"], json!({"stories": [1], "size": 2}));
        assert_eq!(head(&store), before);

        let suited = rejudged(&store, &project(), judged, &p3()).expect("PL1 fits under P3");
        assert_eq!(suited.suite.as_deref(), Some("cargo nextest run"));
    }

    #[test]
    fn a_truth_or_scope_draft_approved_as_a_plan_is_caught() {
        let (_dir, store) = store_c();
        let truths = drafted_by_hand(&store, "story-truths", "story/1", Hash([0x3a; 32]), R1);
        let phase_edit = edit(&store, &[1, 4], O, R2, T5);
        for digest in [truths, phase_edit] {
            refused_with(&store, digest, &p0(), "unknown-draft");
        }
    }

    #[test]
    fn a_replaced_plan_draft_approvable_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x2 = digest_of(&plan_submitted(&store, &pl2(), O, None, R2, T5));
        digest_of(&plan_submitted(&store, &pl3(), O, None, R3, T6));
        let refusal = refused_with(&store, x2, &p0(), "stale-draft");
        assert_eq!(refusal["details"], json!({"path": "goal"}));
    }

    #[test]
    fn a_draft_replaced_between_the_approvals_reads_answered_as_an_unreadable_ledger_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x2 = digest_of(&plan_submitted(&store, &pl2(), O, None, R2, T5));
        let reading = SubmitsDuring::new(&store, || {
            digest_of(&plan_submitted(&store, &pl3(), O, None, R3, T6));
        });
        let refusal =
            judge(&reading, &project(), x2, &p0()).expect_err("the replaced draft is refused");
        assert!(reading.landed(), "PL3 was submitted between the reads");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "stale-draft", "{refusal}");
        assert_eq!(refusal["slot"], "digest", "{refusal}");
        assert_eq!(refusal["details"], json!({"path": "goal"}));
    }

    /// The real store, with `land` run once just after the first read of
    /// the `draft` view's index returns: the window in which another plan
    /// draft of the phase can be submitted between the first step's reads.
    struct SubmitsDuring<'a> {
        store: &'a SqliteStore,
        land: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl<'a> SubmitsDuring<'a> {
        fn new(store: &'a SqliteStore, land: impl FnOnce() + 'a) -> Self {
            Self {
                store,
                land: RefCell::new(Some(Box::new(land))),
            }
        }

        fn landed(&self) -> bool {
            self.land.borrow().is_none()
        }
    }

    impl Ledger for SubmitsDuring<'_> {
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
            Ledger::history(self.store, project, range, filter, page)
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

    impl Views for SubmitsDuring<'_> {
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
            let read = Views::find(self.store, project, view, query);
            if view == DRAFT_VIEW {
                let land = self.land.borrow_mut().take();
                if let Some(land) = land {
                    land();
                }
            }
            read
        }
    }

    impl Payloads for SubmitsDuring<'_> {
        fn open(&self, hash: &Hash) -> Result<PayloadBody<'_>, StoreError> {
            Payloads::open(self.store, hash)
        }

        fn status(&self, hash: &Hash) -> Result<PayloadStatus, StoreError> {
            Payloads::status(self.store, hash)
        }
    }

    #[test]
    fn a_plan_approved_while_another_phase_is_active_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R3, T5));
        activate_phase_2(&store, R4);
        let refusal = refused_with(&store, x1, &p0(), "phase-not-active");
        assert_eq!(refusal["details"], json!({"phase": 2}));
    }

    /// P3: P0 with a project file setting
    /// `workflow.test_command = "cargo nextest run"`.
    fn p3() -> EffectivePolicy {
        let file = crate::settings::file(
            Path::new("/r/baley.toml"),
            b"[workflow]\ntest_command = \"cargo nextest run\"\n".to_vec(),
        );
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            None,
            Some(&file),
        )
        .expect("the test command is read")
    }

    /// The owner's command for an approval under `request_id` at `at`, as
    /// the terminal runs it with no session caller.
    fn owner_command(request_id: &str, at: &str) -> Command {
        let mut command = command("plan.approve", request_id, 10, at);
        command.caller = None;
        command
    }

    /// Records `judged` as `Ada Owner` under `request_id` at `at`. Gives the
    /// answer.
    fn recorded(store: &SqliteStore, judged: &Judged, request_id: &str, at: &str) -> Value {
        let command = owner_command(request_id, at);
        crate::scope::submit::answer(record(store, &command, Some(judged), "Ada Owner"), store)
    }

    /// Approves `digest` through both steps under `policy`. Gives the answer.
    pub(crate) fn approved(
        store: &SqliteStore,
        digest: Hash,
        policy: &EffectivePolicy,
        request_id: &str,
        at: &str,
    ) -> Value {
        match judge(store, &project(), digest, policy) {
            Ok(judged) => recorded(store, &judged, request_id, at),
            Err(refusal) => refusal,
        }
    }

    /// Every `plan.approved` in the store, oldest first.
    fn approvals(store: &SqliteStore) -> Vec<baley_store::Event> {
        history(store)
            .into_iter()
            .filter(|event| event.type_name == "plan.approved")
            .collect()
    }

    #[test]
    fn a_plan_number_reused_or_a_map_left_out_of_its_approval_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let submitted = plan_submitted(&store, &pl1(), O, None, R2, T5);
        let x1 = digest_of(&submitted);
        let first = approved(&store, x1, &p3(), R3, T6);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["plan"], 1);
        let x3 = digest_of(&plan_submitted(&store, &pl3(), O, None, R4, T7));
        let second = approved(&store, x3, &p3(), R5, T8);
        assert_eq!(second["status"], "ok", "{second}");

        let events = history(&store);
        let draft = events
            .iter()
            .find(|event| {
                event.type_name == "scope.submitted" && event.payload["digest"] == x1.to_hex()
            })
            .unwrap();
        let approvals = approvals(&store);
        assert_eq!(approvals.len(), 2);
        let event = &approvals[0];
        assert_eq!(event.stream, "phase/1");
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.payload["plan"], 1);
        assert_eq!(event.payload["digest"], x1.to_hex());
        assert_eq!(event.payload["evidence_map"], json!([e1(), e2()]));
        assert_eq!(event.payload["body"], draft.payload["body"]);
        assert_eq!(event.payload["question_set"], submitted["set"]);
        assert_eq!(event.payload["suite"], "cargo nextest run");
        assert_eq!(event.payload["owner"], "Ada Owner");
        assert_eq!(event.payload["stories"], json!([1]));
        assert_eq!(event.payload["size"], 1);
        assert_eq!(approvals[1].payload["plan"], 2);
        assert_eq!(approvals[1].payload["digest"], x3.to_hex());

        let phase_plan = store
            .get(&project(), PHASE_PLAN_VIEW, &phase_plan_key(1))
            .unwrap()
            .unwrap();
        assert_eq!(phase_plan.body["size"], 2);
        assert_eq!(phase_plan.body["highest"], 2);
    }

    #[test]
    fn a_missing_test_command_refusing_approval_or_inventing_a_suite_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        let answer = approved(&store, x1, &p0(), R3, T6);
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["suite"], Value::Null);
        assert!(
            answer["notice"]
                .as_str()
                .is_some_and(|notice| notice.contains("workflow.test_command")),
            "{answer}"
        );
        let approvals = approvals(&store);
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].payload["suite"], Value::Null);
    }

    #[test]
    fn an_approval_landing_twice_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        let one = judge(&store, &project(), x1, &p0()).expect("the first is judged");
        let two = judge(&store, &project(), x1, &p0()).expect("the second is judged");
        assert_eq!(recorded(&store, &one, R3, T6)["status"], "ok");
        let busy = recorded(&store, &two, R4, T7);
        assert_eq!(busy["status"], "failed", "{busy}");
        assert_eq!(busy["code"], "ledger-busy", "{busy}");
        assert_eq!(approvals(&store).len(), 1);
    }

    #[test]
    fn an_approval_landing_beside_a_newly_active_phase_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R3, T5));
        let judged = judge(&store, &project(), x1, &p0()).expect("PL1 is judged");
        activate_phase_2(&store, R4);
        let busy = recorded(&store, &judged, R5, T6);
        assert_eq!(busy["status"], "failed", "{busy}");
        assert_eq!(busy["code"], "ledger-busy", "{busy}");
        assert!(
            approvals(&store)
                .iter()
                .all(|event| event.payload["digest"] != x1.to_hex())
        );
    }

    #[test]
    fn a_plan_approved_on_an_unmade_or_rejected_decision_is_caught() {
        use crate::mcp::questions::tests::{REVISION, postgres, relayed};

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let submitted = plan_submitted(&store, &pl2(), O, None, R2, T5);
        let x2 = digest_of(&submitted);
        let s2 = submitted["set"].as_str().unwrap().to_owned();

        refused_with(&store, Hash([0x11; 32]), &p0(), "unknown-draft");
        let open = refused_with(&store, x2, &p0(), "question-open");
        assert_eq!(open["details"], json!({"questions": ["q1"]}));

        let closed = relayed(&store, &s2, &postgres(), R3, T6, REVISION);
        assert_eq!(closed["revision"]["work_order"], REVISION, "{closed}");
        let rejected = refused_with(&store, x2, &p0(), "answer-not-applied");
        assert_eq!(rejected["details"], json!({"questions": ["q1"]}));

        digest_of(&plan_submitted(&store, &pl1(), REVISION, None, R4, T7));
        refused_with(&store, x2, &p0(), "stale-draft");
    }
}
