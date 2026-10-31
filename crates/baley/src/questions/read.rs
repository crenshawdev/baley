//! The read of one set's round from the ledger (design 0005 `questions
//! round`, PLN-R24).
//!
//! One `get` of the set's `questions` view document, handed to the core's
//! pure reading and round: nothing is appended and no clock is read. The
//! `document` read, `baley questions round` and the answer check share it,
//! and the `document` read and the answer receipt share the round's shape.

use baley_core::questions::{
    Pending, QUESTIONS_VIEW, Round, SetFacts, UnreadableSet, questions_key, round_of, set_facts,
};
use baley_store::{Hash, ProjectId, StoreError, Views};
use serde_json::{Value, json};

use crate::envelope::Refusal;

/// The code for a set id this project's `questions` view does not hold.
pub const NO_SUCH_QUESTION_SET: &str = "no-such-question-set";

/// The refusal for a set this project does not hold, in `slot`. It echoes
/// none of the id, so a long or odd id cannot grow the answer.
pub fn no_such_question_set(slot: &str) -> Value {
    Refusal::new(
        NO_SUCH_QUESTION_SET,
        "this project holds no question set with that id",
    )
    .slot(slot)
    .value()
}

/// The sentence a round's answer carries, sending the session to the rounds
/// relay instruction by its identity (design 0005 section 10).
pub const RELAY: &str = "Put this round to the owner as the instruction `bal-rounds` says: read it by calling `baley_query` with `{\"operation\":\"instruction\",\"identity\":\"bal-rounds\"}`, and follow it.";

/// A set's round as the `document` read answers it and an answer's receipt
/// carries it: the derived state, the round number, the current round's
/// questions with their recommended answers, each waiting question with its
/// unanswered direct dependencies marked `open` or `deferred`, the answers
/// and deferrals recorded, an abandonment, and [`RELAY`].
pub fn round_value(round: &Round) -> Value {
    let current: Vec<Value> = round
        .current
        .iter()
        .map(|asked| {
            json!({"id": asked.id, "question": asked.question,
                "recommended": asked.recommended})
        })
        .collect();
    let waiting: Vec<Value> = round
        .waiting
        .iter()
        .map(|waiting| {
            let on: Vec<Value> = waiting
                .on
                .iter()
                .map(|(id, pending)| {
                    let state = match pending {
                        Pending::Open => "open",
                        Pending::Deferred => "deferred",
                    };
                    json!({"id": id, "state": state})
                })
                .collect();
            json!({"id": waiting.id, "question": waiting.question, "waits_on": on})
        })
        .collect();
    let answered: Vec<Value> = round
        .answered
        .iter()
        .map(|answer| {
            json!({"id": answer.question, "answer": answer.answer, "accepted": answer.accepted,
                "round": answer.round, "owner": answer.owner, "at": answer.at})
        })
        .collect();
    let deferred: Vec<Value> = round
        .deferred
        .iter()
        .map(|deferral| {
            json!({"id": deferral.question, "reason": deferral.reason, "round": deferral.round,
                "owner": deferral.owner, "at": deferral.at})
        })
        .collect();
    let abandoned = round
        .abandoned
        .as_ref()
        .map(|abandoned| json!({"cause": abandoned.cause.as_str(), "at": abandoned.at}));
    json!({"state": round.state.as_str(), "round": round.number, "current": current,
        "waiting": waiting, "answered": answered, "deferred": deferred,
        "abandoned": abandoned, "relay": RELAY})
}

/// A set the project's view holds, with its round worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSet {
    /// The round, with the set's derived state, which also says whether the
    /// set is open.
    pub round: Round,
    /// The sequence that last produced the set's document, which an answer's
    /// transaction confirms unchanged.
    pub produced_seq: u64,
    /// The facts the round was worked out from, which an answer call is
    /// judged against.
    pub facts: SetFacts,
    /// The stream the set and every record on it go on.
    pub stream: String,
    /// The story whose set this is, read from the set's subject, or none for
    /// a plan draft's set.
    pub story: Option<u32>,
    /// The plan draft whose set this is, read from the set's subject, or none
    /// for a story's set.
    pub plan: Option<PlanSubject>,
}

/// The plan draft a set belongs to: its phase and the draft's digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSubject {
    /// The phase the draft plans.
    pub phase: u32,
    /// The draft's digest.
    pub draft: Hash,
}

/// What a read of one set found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetRead {
    /// The set and its round.
    Held(Box<HeldSet>),
    /// This project's view holds no set with that id, another project's set
    /// included. Callers answer it `no-such-question-set`.
    NotHeld,
    /// The set's document could not be read into its facts.
    Unreadable(UnreadableSet),
}

/// Reads the set `set` of `project` and works out its round. A store error
/// comes back unchanged.
pub fn read<V: Views + ?Sized>(
    store: &V,
    project: &ProjectId,
    set: &str,
) -> Result<SetRead, StoreError> {
    let Some(document) = store.get(project, QUESTIONS_VIEW, &questions_key(set))? else {
        return Ok(SetRead::NotHeld);
    };
    let facts = match set_facts(&document.body) {
        Ok(facts) => facts,
        Err(unreadable) => return Ok(SetRead::Unreadable(unreadable)),
    };
    let Some(stream) = document.body.get("stream").and_then(Value::as_str) else {
        return Ok(SetRead::Unreadable(UnreadableSet(
            "the set's stream cannot be read".into(),
        )));
    };
    let Some((story, plan)) = subject_of(&document.body) else {
        return Ok(SetRead::Unreadable(UnreadableSet(
            "the set's subject cannot be read".into(),
        )));
    };
    Ok(SetRead::Held(Box::new(HeldSet {
        round: round_of(&facts),
        produced_seq: document.produced_seq,
        facts,
        stream: stream.into(),
        story,
        plan,
    })))
}

/// The subject a set's document names: the story for a story's set, the
/// phase and draft digest for a plan draft's, and `None` for a subject that
/// cannot be read.
fn subject_of(document: &Value) -> Option<(Option<u32>, Option<PlanSubject>)> {
    let subject = document.get("subject")?;
    let whole = |name: &str| {
        subject
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    match subject.get("kind").and_then(Value::as_str)? {
        "story" => Some((Some(whole("story")?), None)),
        "plan" => {
            let draft = subject
                .get("plan_digest")
                .and_then(Value::as_str)
                .and_then(Hash::from_hex)?;
            Some((
                None,
                Some(PlanSubject {
                    phase: whole("phase")?,
                    draft,
                }),
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Sets are opened with `questions::record` and their records appended
    //! in test transactions over the real store; expected rounds are written
    //! out from PLN-R24 to PLN-R26 and the plan's set R.

    use baley_core::questions::{
        Answer, Asked, Deferral, Pending, QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION,
        QUESTION_DEFERRED, QUESTION_DEFERRED_VERSION, QuestionSet, SetState, Waiting,
        answered_payload, deferred_payload, judge_set,
    };
    use baley_store::{Decision, NewEvent, Observed, OutcomeKind, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::questions::record::tests::{declare, open_set, story, transact};
    use crate::scope::fixtures::*;

    const OWNER: &str = "Ada Lovelace";

    /// Set R as the plan writes it out, accepted.
    fn r() -> QuestionSet {
        judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Do sessions share one ledger?",
                "recommended": "Yes, one per user", "depends_on": []},
            {"id": "q3", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1", "q2"]},
            {"id": "q4", "question": "Is the ledger encrypted at rest?", "recommended": "No",
                "depends_on": []},
            {"id": "q5", "question": "Who may read the ledger?",
                "recommended": "Only the owner's account", "depends_on": ["q4"]}
        ]))
        .unwrap()
    }

    fn answered(set: &str, question: &str, round: u32, answer: &str, accepted: bool) -> NewEvent {
        NewEvent {
            stream: StreamName("roadmap".into()),
            type_name: QUESTION_ANSWERED.into(),
            type_version: QUESTION_ANSWERED_VERSION,
            git: None,
            payload: answered_payload(set, question, round, answer, accepted, OWNER, T2),
            attachments: Vec::new(),
        }
    }

    fn deferred(set: &str, question: &str, round: u32, reason: &str) -> NewEvent {
        NewEvent {
            stream: StreamName("roadmap".into()),
            type_name: QUESTION_DEFERRED.into(),
            type_version: QUESTION_DEFERRED_VERSION,
            git: None,
            payload: deferred_payload(set, question, round, reason, OWNER, T2),
            attachments: Vec::new(),
        }
    }

    fn held(store: &SqliteStore, set: &str) -> HeldSet {
        match read(store, &project(), set).unwrap() {
            SetRead::Held(held) => *held,
            other => panic!("{set}: {other:?}"),
        }
    }

    /// The round read, with the ledger head checked unmoved by it.
    fn round(store: &SqliteStore, set: &str) -> Round {
        let before = head(store);
        let round = held(store, set).round;
        assert_eq!(head(store), before, "a read appended");
        round
    }

    fn asked(id: &str, question: &str, recommended: &str) -> Asked {
        Asked {
            id: id.into(),
            question: question.into(),
            recommended: recommended.into(),
        }
    }

    fn waiting(id: &str, question: &str, on: &[(&str, Pending)]) -> Waiting {
        Waiting {
            id: id.into(),
            question: question.into(),
            on: on
                .iter()
                .map(|(id, pending)| ((*id).to_owned(), *pending))
                .collect(),
        }
    }

    fn answer(question: &str, text: &str, accepted: bool, round: u32) -> Answer {
        Answer {
            question: question.into(),
            answer: text.into(),
            accepted,
            round,
            owner: OWNER.into(),
            at: T2.into(),
        }
    }

    /// `round` with every record's round set to 0, so two sets settled in
    /// different orders compare on everything else.
    fn without_record_rounds(mut round: Round) -> Round {
        for answer in &mut round.answered {
            answer.round = 0;
        }
        for deferral in &mut round.deferred {
            deferral.round = 0;
        }
        round
    }

    #[test]
    fn a_round_read_from_arrival_order_or_appending_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let a = open_set(&store, R1, &r(), &story(1));
        let b = open_set(&store, R2, &r(), &story(1));

        let first = Round {
            number: 1,
            state: SetState::Open,
            current: vec![
                asked("q1", "Which store backs the ledger?", "SQLite"),
                asked("q2", "Do sessions share one ledger?", "Yes, one per user"),
                asked("q4", "Is the ledger encrypted at rest?", "No"),
            ],
            waiting: vec![
                waiting(
                    "q3",
                    "Where does the ledger live?",
                    &[("q1", Pending::Open), ("q2", Pending::Open)],
                ),
                waiting("q5", "Who may read the ledger?", &[("q4", Pending::Open)]),
            ],
            answered: Vec::new(),
            deferred: Vec::new(),
            abandoned: None,
        };
        assert_eq!(round(&store, &a), first);
        assert_eq!(round(&store, &b), first);

        append(&store, R3, T2, vec![answered(&a, "q1", 1, "SQLite", true)]);
        append(
            &store,
            R4,
            T2,
            vec![answered(&a, "q2", 2, "No, one per project", false)],
        );
        append(
            &store,
            R5,
            T2,
            vec![deferred(&a, "q4", 3, "after the security review")],
        );
        append(
            &store,
            R6,
            T2,
            vec![deferred(&b, "q4", 1, "after the security review")],
        );
        append(
            &store,
            R7,
            T2,
            vec![answered(&b, "q2", 2, "No, one per project", false)],
        );
        append(&store, R8, T2, vec![answered(&b, "q1", 3, "SQLite", true)]);

        let (in_a, in_b) = (round(&store, &a), round(&store, &b));
        let expected = Round {
            number: 4,
            state: SetState::Open,
            current: vec![asked(
                "q3",
                "Where does the ledger live?",
                "Outside the checkout",
            )],
            waiting: vec![waiting(
                "q5",
                "Who may read the ledger?",
                &[("q4", Pending::Deferred)],
            )],
            answered: vec![
                answer("q1", "SQLite", true, 1),
                answer("q2", "No, one per project", false, 2),
            ],
            deferred: vec![Deferral {
                question: "q4".into(),
                reason: "after the security review".into(),
                round: 3,
                owner: OWNER.into(),
                at: T2.into(),
            }],
            abandoned: None,
        };
        assert_eq!(in_a, expected);
        assert_eq!(without_record_rounds(in_a), without_record_rounds(in_b));
    }

    #[test]
    fn an_unknown_or_another_projects_set_read_as_a_set_is_caught() {
        let (_dir, store) = two_projects();
        declare(&store, &project_b(), 1);
        assert_eq!(
            read(&store, &project(), "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f"),
            Ok(SetRead::NotHeld)
        );
        let mut other = None;
        store
            .transact(
                &command_in(&project_b(), "test.questions", R1, 3, T1),
                &mut |tx| {
                    other = Some(crate::questions::record::open(tx, &r(), &story(1), T1)?);
                    Ok(Decision {
                        kind: OutcomeKind::Done,
                        answer: json!({"status": "ok"}),
                        sensitive: false,
                        observed: Observed::default(),
                        git: None,
                    })
                },
            )
            .unwrap();
        let other = other.unwrap();
        assert!(matches!(
            read(&store, &project_b(), &other),
            Ok(SetRead::Held(_))
        ));
        assert_eq!(read(&store, &project(), &other), Ok(SetRead::NotHeld));
    }

    #[test]
    fn a_set_read_after_reopening_lost_or_changed_is_caught() {
        let (dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r(), &story(1));
        append(
            &store,
            R2,
            T2,
            vec![answered(&set, "q1", 1, "SQLite", true)],
        );
        let before = held(&store, &set);
        drop(store);

        let reopened = crate::ledger::open::store(
            &dir.path().join("home"),
            T3,
            crate::ledger::open::options(),
        )
        .unwrap();
        assert_eq!(held(&reopened, &set), before);
        assert_eq!(before.round.number, 2);
    }

    #[test]
    fn an_abandoned_set_read_as_open_or_without_its_answers_is_caught() {
        use baley_core::questions::{Abandonment, Cause};

        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r(), &story(1));
        append(
            &store,
            R2,
            T2,
            vec![answered(&set, "q1", 1, "SQLite", true)],
        );
        transact(&store, R3, &mut |tx| {
            crate::questions::record::abandon(tx, &set, Cause::Superseded, T3)
                .map_err(|error| StoreError::Unavailable(format!("{error:?}")))
        })
        .unwrap();
        let round = round(&store, &set);
        assert_eq!(round.state, SetState::Abandoned);
        assert_eq!(
            round.abandoned,
            Some(Abandonment {
                cause: Cause::Superseded,
                at: T3.into(),
            })
        );
        assert_eq!(round.answered, [answer("q1", "SQLite", true, 1)]);
    }

    #[test]
    fn a_read_giving_another_documents_sequence_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r(), &story(1));
        let events = history(&store);
        let opened = of_type(&events, "questions.opened")[0].seq;
        assert_eq!(held(&store, &set).produced_seq, opened);

        append(
            &store,
            R2,
            T2,
            vec![answered(&set, "q1", 1, "SQLite", true)],
        );
        let events = history(&store);
        let answered = of_type(&events, "question.answered")[0].seq;
        assert!(answered > opened);
        assert_eq!(held(&store, &set).produced_seq, answered);
    }
}
