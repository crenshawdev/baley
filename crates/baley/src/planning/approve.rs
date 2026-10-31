//! The owner's approval of a plan draft by its digest (design 0005 section
//! 5 `plan approve`, PLN-R14, PLN-R15, PLN-R26, PLN-R27, D-02, D-06, D-07,
//! D-14, D-16), in two steps.
//!
//! The first step reads and judges outside any transaction, so a refusal
//! records nothing: the draft in D-16's order, then `phase-not-active` when
//! a different phase is active, then the plan judged again against the
//! phase as it stands now. Build 4 T14 adds `review-unsettled` after these
//! refusals, judged against the same digest.

use baley_core::plan::{
    DraftRead, PHASE_PLAN_VIEW, PLAN_APPROVED, PLAN_KIND, Plan, PlanApprovalFacts,
    PlanApprovalRefusal, PlanDraft, RecordedPlan, judge_plan, judge_plan_approval,
    judge_submit_phase, phase_plan_key, phase_stream, read_plan_draft,
};
use baley_core::policy::{EffectivePolicy, Value as Setting};
use baley_core::questions::{QUESTIONS_VIEW, SetFacts, questions_key};
use baley_core::scope::{DRAFT_VIEW, PriorApproval, SCOPE_SUBMITTED};
use baley_store::{
    Absence, EVENT_PAGE_BOUND, Hash, HistoryFilter, Ledger, ObservedDocument, PageRequest,
    PayloadRef, Payloads, ProjectId, StoreError, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::facts;
use crate::planning::submit::{phase_not_active, plan_refused};
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
// The recording reads these fields.
#[allow(dead_code)]
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
    /// The evidence items exactly as submitted.
    evidence_map: Vec<Value>,
    /// The id of the draft's question set.
    set: String,
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
/// that set's facts, and last every `plan.approved` naming the digest, so
/// one landing after that read moves the `phase_plan` document the second
/// step observes. It judges them in D-16's order, then refuses
/// `phase-not-active` naming another active phase (D-02), then gathers the
/// phase's facts now with the policy's capacity and judges the plan again
/// with `judge_plan` (D-16), every refusal at slot `digest`. The suite is
/// the policy's `workflow.test_command`, none when it is not set (D-14).
/// Nothing is recorded. A replacement draft's judgement is Plan 7's, in
/// this same step.
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
    let set = current_set(phase_plan.as_ref().map(|document| &document.body), digest)?;
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
    // phase's current draft, a set.
    let (Some(body), Some((newest_digest, newest_seq, _)), Some(set)) = (body, newest, set) else {
        return Err(unreadable(
            "the draft's record or its question set cannot be read",
        ));
    };

    let read = facts::read(store, project, phase, facts::capacity(policy))?;
    judge_submit_phase(phase, &read.active)
        .map_err(|refusal| phase_not_active(&refusal, "digest", NOT_APPROVED))?;
    let plan = judge_plan(&target.plan, &read.facts)
        .map_err(|refusal| plan_refused(&refusal, "digest", NOT_APPROVED))?;
    if PlanDraft::new(&plan).digest() != digest {
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
        evidence_map,
        set,
        suite: suite(policy),
        body,
        documents,
        absences: read.absences().to_vec(),
    })
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

/// The set of the phase's current draft when that draft is `digest`, or
/// none. A draft that cannot be read is a ledger fault.
fn current_set(phase_plan: Option<&Value>, digest: Hash) -> Result<Option<String>, Value> {
    let draft = match phase_plan.and_then(|body| body.get("draft")) {
        None | Some(Value::Null) => return Ok(None),
        Some(draft) => draft,
    };
    let held = draft
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex);
    match (held, draft.get("set").and_then(Value::as_str)) {
        (Some(held), Some(set)) => Ok((held == digest).then(|| set.to_owned())),
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

    use std::path::Path;

    use baley_core::policy::{Host, Schema, effective_policy};
    use baley_core::scope::{PROJECT_STREAM, SCOPE_SUBMITTED_VERSION, submitted_payload};
    use baley_store::{
        Decision, NewEvent, Observed, OutcomeKind, RetentionClass, StreamName, canonical_json,
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
    fn a_plan_approved_while_another_phase_is_active_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        order_q(&store, R2);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R3, T5));
        activate_phase_2(&store, R4);
        let refusal = refused_with(&store, x1, &p0(), "phase-not-active");
        assert_eq!(refusal["details"], json!({"phase": 2}));
    }
}
