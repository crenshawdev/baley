//! The owner's approval of a draft by its digest (design 0004 section 5),
//! in two steps that `scope-approve` and `baley scope approve` both call
//! (D-20).
//!
//! The first step reads and judges outside any transaction, before its
//! caller prepares the write, so a refusal records nothing at all: no
//! checkout admission, no policy step and no `command.completed`. The second
//! records the approval as the owner, confirming inside its transaction that
//! nothing the first step read has moved.

use std::collections::BTreeMap;

use baley_core::scope::{
    ApprovalFacts, ApprovalRefusal, DRAFT_NEWEST_INDEX, DRAFT_VIEW, PHASE_DECLARED, PROJECT_STREAM,
    ROADMAP_VIEW, SCOPE_SUBMITTED, START_KIND, STORY_DECLARED, Submission, draft_key,
    judge_approval, judge_submission, roadmap_key, start_events,
};
use baley_store::{
    Absence, Actor, Command, Decision, Document, EventMatch, Hash, HistoryFilter, IndexQuery,
    KeyValue, Ledger, Observed, OutcomeKind, PageRequest, Payloads, ProjectId, Recorded,
    RetentionClass, StaleInput, StoreError, StreamName, Transaction, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::scope::read::{Preview, PreviewError, preview, preview_of};
use crate::scope::submit::{NOT_RECORDED_BUSY, start_approval, start_approved};

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
    /// The newest start draft's digest and sequence when it was read.
    newest: (Hash, u64),
    /// The approved draft's accepted submission.
    submission: Submission,
}

/// The first step: reads the draft with `digest`, its `scope.submitted`,
/// whether a start is approved and the newest start draft, then judges the
/// approval. It runs outside any transaction, since a transaction has no
/// payload read (D-02). A refusal is the `refused` answer, slot `digest`; a
/// body that is gone or no longer judges clean is a ledger fault, never
/// approved. Nothing is recorded.
pub fn judge<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Judged, Value> {
    let target = match preview(store, project, digest) {
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
        .find(project, DRAFT_VIEW, &newest_query())
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
            submission,
        }),
        // An approved judgement had both.
        _ => Err(refused(&ApprovalRefusal::UnknownDraft)),
    }
}

/// The second step: records the approval judged by the first step under a
/// copy of `command` whose actor is the owner, so the approval is the
/// owner's whichever surface relayed it, with `owner` as the owner's name
/// and the command's time as Baley's (D-18). The caller, request id, digest
/// and policy version are the command's.
///
/// Before it appends anything the decision confirms what the first step
/// read: the draft at the sequence read, its `scope.submitted`, still no
/// start approved, and the same newest start draft by digest and sequence.
/// When any of them moved, a submission or another approval landed between
/// the steps, and the decision returns `StoreError::Stale` with nothing
/// recorded. A retry of a request the store holds is answered from its
/// record before any decision runs, so it needs no reads; a decision asked
/// to run without them records nothing.
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
                "an approval reached its transaction without its first step's reads".into(),
            ));
        };
        confirm(tx, judged)?;
        let (highest_phase, highest_story) = highest(tx.get(ROADMAP_VIEW, &roadmap_key())?)?;
        let mut details = Vec::with_capacity(judged.submission.phases.len());
        for phase in &judged.submission.phases {
            details.push(tx.put_payload(phase.detail.as_bytes(), RetentionClass::Record)?);
        }
        let events = start_events(
            &judged.submission,
            highest_phase,
            highest_story,
            &details,
            owner,
            &at,
        );
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

/// Confirms inside the transaction what the first step read. A start
/// approval is checked here rather than declared as an observed absence,
/// because this approval appends one itself.
fn confirm(tx: &mut dyn Transaction, judged: &Judged) -> Result<(), StoreError> {
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
    if tx.event_exists(&start_approval())? {
        return Err(StoreError::Stale(StaleInput::Absence(Absence::Event(
            start_approval(),
        ))));
    }
    // Compared by digest and sequence, never by time: two submissions can
    // share a time.
    let newest = tx
        .find(DRAFT_VIEW, &newest_query())?
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

/// The first page of one start draft, newest first.
fn newest_query() -> IndexQuery {
    IndexQuery {
        index: DRAFT_NEWEST_INDEX.into(),
        equals: vec![KeyValue::Text(START_KIND.into())],
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
fn submitted_at<L: Ledger + ?Sized>(
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
    use baley_core::scope::{
        BACKLOG_VIEW, PHASE_VIEW, ROADMAP_VIEW, backlog_key, phase_key, roadmap_key,
    };
    use baley_store::{Actor, Caller, OutcomeKind, Refusal as StoreRefusal};
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
            .map(|s| json!([s["id"], s["refinement"], s["truths"]]))
            .collect();
        assert_eq!(
            listed,
            [
                json!([1, "unrefined", 0]),
                json!([2, "unrefined", 0]),
                json!([3, "unrefined", 0])
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
}
