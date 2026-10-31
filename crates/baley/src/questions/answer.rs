//! The recording of one answer call (design 0005 `questions answer`,
//! PLN-R24 to PLN-R26, HST-R9): a check that judges the call against its
//! set as read, recording nothing, and a recording that writes every
//! answer and deferral of the call in one domain transaction under the
//! command its caller built. `questions-answer` over MCP and
//! `baley questions answer` at the terminal both call these two functions
//! (ADR 0044), so the two surfaces judge and record a call the same way and
//! differ only in the command's actor and caller.
//!
//! The recording observes the set's document at the check's read, so a set
//! that moved since makes the store refuse the command as stale with
//! nothing recorded.

use baley_core::questions::{
    AnswerRefusal, Call, CallFault, QUESTIONS_VIEW, Question, Round, SetFacts, SetState, Settled,
    TYPED_CONTENT, judge_answers, questions_key,
};
use baley_store::{
    Command, Decision, Ledger, NewEvent, Observed, ObservedDocument, OutcomeKind, Payloads,
    ProjectId, Recorded, StoreError, StreamName, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::parts::PART_BOUND;
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::order::{self as planner, planner_absences};
use crate::questions::read::{
    PlanSubject, RELAY, SetRead, no_such_question_set, read as read_set, round_value,
};
use crate::refine::order::{PreparedOrder, analyzer_absences};

/// The command kind an answer call is recorded under, on either surface.
pub const ANSWER_COMMAND: &str = "questions.answer";

/// A busy ledger's reason when an answer call was not checked or recorded.
const NOT_RECORDED_BUSY: &str =
    "the ledger is busy, so no answer or deferral was recorded. Send the same call again";

/// What an answer call's check read of its set and judged of the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The set's id.
    pub set: String,
    /// The stream the set's records go on.
    pub stream: String,
    /// The sequence that last produced the set's document.
    pub produced_seq: u64,
    /// The call judged against the set.
    pub settled: Settled,
    /// The story whose set this is, or none for a plan draft's set.
    pub story: Option<u32>,
    /// The plan draft whose set this is, or none for a story's set.
    pub plan: Option<PlanSubject>,
    /// The set's questions as submitted.
    pub questions: Vec<Question>,
}

impl Checked {
    /// The set's facts once this call is recorded by `owner` at `at`, when
    /// the call closes the set: its questions with every answer and
    /// deferral, the call's own at its round. None while a question stays
    /// open.
    pub fn closed_facts(&self, owner: &str, at: &str) -> Option<SetFacts> {
        let after = self.settled.round_after(owner, at);
        (after.state == SetState::Closed).then(|| SetFacts {
            questions: self.questions.clone(),
            answers: after.answered,
            deferrals: after.deferred,
            abandoned: None,
        })
    }
}

/// The `typed-content` refusal for a call whose grammar is refused, naming
/// the faulty item's position when there is one.
pub fn call_fault(fault: CallFault) -> Value {
    let reason = "items must be a non-empty list of objects, each naming its question once with `act` `take`, `answer` (with `answer`) or `defer` (with `reason`), every text at most 4096 bytes of UTF-8, so nothing was recorded";
    let refusal = Refusal::new(TYPED_CONTENT, reason).slot("items");
    match fault.position {
        Some(position) => refusal.entry(position).value(),
        None => refusal.value(),
    }
}

/// The refusal for a call judged against its set, naming the question.
fn refused(refusal: &AnswerRefusal) -> Value {
    let Some(id) = refusal.question() else {
        return Refusal::new(
            refusal.code(),
            "the question set was abandoned, so it takes no answer and nothing was recorded",
        )
        .slot("set")
        .value();
    };
    let why = match refusal {
        AnswerRefusal::NoSuchQuestion(_) => "is not a question of this set",
        AnswerRefusal::QuestionClosed(_) => {
            "is already answered or deferred, and a recorded decision never changes"
        }
        AnswerRefusal::NotInRound(_) => {
            "is not in the current round, so it can be deferred but not yet answered"
        }
        AnswerRefusal::AnswerBlank(_) => "has an answer that is empty or only whitespace",
        AnswerRefusal::DeferralReason(_) => "has a deferral with no reason",
        AnswerRefusal::SetAbandoned => unreachable!("an abandoned set names no question"),
    };
    Refusal::new(
        refusal.code(),
        format!("question `{id}` {why}, so nothing was recorded"),
    )
    .slot("items")
    .id(id)
    .value()
}

/// An answer call's check, run before admission and recording nothing: the
/// set by id, none `no-such-question-set`, then the call judged against the
/// set's facts as read. A refusal is the `refused` answer naming the
/// question; a pass gives back what the recording needs.
pub fn check<V: Views + ?Sized>(
    store: &V,
    project: &ProjectId,
    set: &str,
    call: &Call,
) -> Result<Checked, Value> {
    let held = match read_set(store, project, set) {
        Ok(SetRead::Held(held)) => held,
        Ok(SetRead::NotHeld) => return Err(no_such_question_set("set")),
        Ok(SetRead::Unreadable(unreadable)) => {
            return Err(failed(LEDGER_UNAVAILABLE, unreadable.0, PLACE_LEDGER));
        }
        Err(error) => return Err(store_failed(&error, NOT_RECORDED_BUSY)),
    };
    let settled = judge_answers(&held.facts, call).map_err(|refusal| refused(&refusal))?;
    Ok(Checked {
        set: set.into(),
        stream: held.stream,
        produced_seq: held.produced_seq,
        settled,
        story: held.story,
        plan: held.plan,
        questions: held.facts.questions,
    })
}

/// What an answer call's recording does about the order its set's close
/// issues.
#[derive(Debug, Clone, Copy)]
pub enum Drafting<'a> {
    /// Nothing: the call leaves a question open, closes a plan draft's set
    /// with no answer rejecting a recommendation, or is given at the
    /// terminal, where an owner command issues no work order (ADR 0044).
    None,
    /// The call closes a story's set over MCP, so this prepared drafting
    /// order is issued with the answers (D-08).
    Issue(&'a PreparedOrder),
    /// The call closes a plan draft's set over MCP with an answer rejecting
    /// a recommendation, so this prepared planner revision order is issued
    /// with the answers (D-15, PLN-R27).
    Revise(&'a planner::PreparedOrder),
    /// The call closes a story's set over MCP beside the story's pending
    /// analyzer order, or a plan draft's set that a revision would follow
    /// beside the phase's pending planner order, read from this `work_order`
    /// document, so no order is issued (D-09, D-11).
    HeldBy(&'a ObservedDocument),
}

/// Records the call `checked` holds under `command`, every answer and
/// deferral in one transaction on the set's stream, each carrying the
/// call's round, `owner` and the command's time. The command's actor and
/// caller are recorded as the caller built them. A drafting order to issue
/// is issued in the same transaction (D-08), its decision declaring that no
/// analyzer order of the story is pending (D-09) and observing the documents
/// its inputs were read from beside the set's; a planner revision order to
/// issue is issued the same way, declaring that no planner order of the
/// phase is pending (D-11, D-15). An order held back by a pending analyzer
/// or planner order observes that order's document instead, so an order
/// settled since makes the call `ledger-busy` with nothing recorded rather
/// than closing the set with no order. A request the store
/// already holds is replayed from its record before any decision runs; a
/// decision asked to run without the check's read records nothing.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    owner: &str,
    checked: Option<&Checked>,
    drafting: Drafting<'_>,
) -> Result<Recorded, StoreError> {
    store.transact(command, &mut |tx| {
        let Some(checked) = checked else {
            return Err(StoreError::Unavailable(
                "an answer call reached its transaction without its check's read".into(),
            ));
        };
        let round = checked.settled.round();
        for settlement in checked.settled.settlements() {
            tx.append(NewEvent {
                stream: StreamName(checked.stream.clone()),
                type_name: settlement.type_name().into(),
                type_version: settlement.type_version(),
                git: None,
                payload: settlement.payload(&checked.set, round, owner, &command.recorded_at),
                attachments: Vec::new(),
            })?;
        }
        let mut observed = Observed {
            documents: vec![ObservedDocument {
                view: QUESTIONS_VIEW.into(),
                key: questions_key(&checked.set),
                produced_seq: Some(checked.produced_seq),
            }],
            ..Observed::default()
        };
        let issued = match drafting {
            Drafting::Issue(order) => {
                let members = order.issue_within(tx, &checked.set, &command.recorded_at)?;
                observed.documents.extend(order.documents().iter().cloned());
                observed.absences = analyzer_absences(order.story());
                Some(("drafting", members))
            }
            Drafting::Revise(order) => {
                let members = order.issue_within(tx, &command.recorded_at)?;
                observed.documents.extend(order.documents().iter().cloned());
                observed.absences = planner_absences(order.phase());
                Some(("revision", members))
            }
            Drafting::HeldBy(pending) => {
                observed.documents.push(pending.clone());
                None
            }
            Drafting::None => None,
        };
        let after = checked.settled.round_after(owner, &command.recorded_at);
        let ready = checked.plan.is_some() && after.answered.iter().all(|answer| answer.accepted);
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: receipt_value(
                &checked.set,
                round,
                &command.recorded_at,
                &after,
                ready,
                issued,
            ),
            sensitive: false,
            observed,
            git: None,
        })
    })
}

/// The receipt of a call recorded in `round` at `at`: the set, the round
/// recorded, the time, and in `now` the set's round as it stands after the
/// call. A call that closed a story's set over MCP adds `drafting`, the
/// analyzer's drafting order it issued (D-08). A call that closed a plan
/// draft's set with no answer rejecting a recommendation adds `ready` true
/// on either surface, since the draft may then be approved; one that closed
/// it over MCP with such an answer adds `revision`, the planner's revision
/// order it issued (D-15, PLN-R27). A receipt cannot be cut into parts, so
/// when that round would pass the part bound `now` names the `document` read
/// instead.
fn receipt_value(
    set: &str,
    round: u32,
    at: &str,
    after: &Round,
    ready: bool,
    issued: Option<(&str, Map<String, Value>)>,
) -> Value {
    let mut receipt = Map::new();
    receipt.insert("status".into(), json!("ok"));
    receipt.insert("set".into(), json!(set));
    receipt.insert("round".into(), json!(round));
    receipt.insert("recorded_at".into(), json!(at));
    if after.state == SetState::Closed {
        // Every question is answered or deferred, so the close's outputs
        // are given: a story's drafting order, or a plan draft's `ready` or
        // revision order. A close at the terminal issues no order.
        if ready {
            receipt.insert("ready".into(), json!(true));
        }
        if let Some((name, order)) = issued {
            receipt.insert(name.into(), Value::Object(order));
        }
    }
    with_now(receipt, set, after)
}

/// `receipt` with `now`, set `set`'s round `after` in [`round_value`]'s
/// shape. A receipt cannot be cut into parts, so when the whole receipt would
/// pass the part bound `now` gives the round's state and number and names
/// the `document` read instead.
pub(crate) fn with_now(mut receipt: Map<String, Value>, set: &str, after: &Round) -> Value {
    receipt.insert("now".into(), round_value(after));
    let text = serde_json::to_string(&receipt).expect("a receipt serializes");
    if text.len() > PART_BOUND {
        receipt.insert(
            "now".into(),
            json!({"state": after.state.as_str(), "round": after.number,
                "read": {"kind": "questions", "set": set}, "relay": RELAY}),
        );
    }
    Value::Object(receipt)
}

/// The answer for an answer call's transaction result: the recorded or
/// replayed receipt, or why nothing was recorded. A set that moved since
/// the check is `ledger-busy`, since the round it was judged against is
/// gone.
pub fn answer(result: Result<Recorded, StoreError>, payloads: &dyn Payloads) -> Value {
    match result {
        Err(StoreError::Stale(_)) => failed(
            LEDGER_BUSY,
            "the question set moved while this call was prepared, so nothing was recorded. Read the round again before answering",
            PLACE_LEDGER,
        ),
        result => crate::scope::submit::answer(result, payloads),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Sets are opened with `questions::record` over the real store, and
    //! calls run through the grammar, the check and the recording as the
    //! two surfaces run them. Expected records are written out from design
    //! 0005 section 6, the phase's decisions and the plan's set R2.

    use baley_core::questions::{QuestionSet, judge_call, judge_set};
    use baley_store::{Event, PageRequest, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::questions::record::tests::{declare, open_set, story};
    use crate::scope::fixtures::*;

    pub(crate) const OWNER: &str = "Ada Lovelace";
    pub(crate) const AT1: &str = "2026-10-22T09:00:01Z";

    /// Set R2 as the plan writes it out, accepted.
    pub(crate) fn r2() -> QuestionSet {
        judge_set(&json!([
            {"id": "q1", "question": "Which store backs the ledger?", "recommended": "SQLite",
                "depends_on": []},
            {"id": "q2", "question": "Do sessions share one ledger?",
                "recommended": "Yes, one per user", "depends_on": []},
            {"id": "q3", "question": "Where does the ledger live?",
                "recommended": "Outside the checkout", "depends_on": ["q1"]}
        ]))
        .unwrap()
    }

    pub(crate) fn take(question: &str) -> Value {
        json!({"question": question, "act": "take"})
    }

    pub(crate) fn own(question: &str, answer: &str) -> Value {
        json!({"question": question, "act": "answer", "answer": answer})
    }

    pub(crate) fn defer(question: &str, reason: &str) -> Value {
        json!({"question": question, "act": "defer", "reason": reason})
    }

    /// Runs one call as a surface runs it: the grammar, the check, then
    /// the recording under `command` as `owner`.
    pub(crate) fn run(
        store: &SqliteStore,
        set: &str,
        items: &Value,
        command: &Command,
        owner: &str,
    ) -> Value {
        let call = match judge_call(items) {
            Ok(call) => call,
            Err(fault) => return call_fault(fault),
        };
        match check(store, &project(), set, &call) {
            Ok(checked) => answer(
                record(store, command, owner, Some(&checked), Drafting::None),
                store,
            ),
            Err(refusal) => refusal,
        }
    }

    pub(crate) fn on_stream(store: &SqliteStore, stream: &str) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(&project(), &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
            .into_iter()
            .filter(|event| {
                event.type_name == "question.answered" || event.type_name == "question.deferred"
            })
            .collect()
    }

    #[test]
    fn a_partly_recorded_or_edited_answer_call_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r2(), &story(1));
        let refused = |items: Value, code: &str, question: Option<&str>| {
            let before = head(&store);
            let command = command(ANSWER_COMMAND, R2, 2, AT1);
            let answer = run(&store, &set, &items, &command, OWNER);
            assert_eq!(answer["status"], "refused", "{items}: {answer}");
            assert_eq!(answer["code"], code, "{items}: {answer}");
            if let Some(question) = question {
                assert_eq!(answer["id"], question, "{items}: {answer}");
            }
            assert_eq!(head(&store), before, "{items}");
        };
        refused(
            json!([take("q1"), own("q3", "In the home folder")]),
            "question-not-in-round",
            Some("q3"),
        );
        refused(
            json!([take("q1"), own("q2", "   ")]),
            "answer-blank",
            Some("q2"),
        );
        refused(
            json!([take("q1"), defer("q2", "")]),
            "deferral-reason",
            Some("q2"),
        );
        refused(
            json!([take("q1"), take("q9")]),
            "no-such-question",
            Some("q9"),
        );
        refused(json!([take("q1"), take("q1")]), "typed-content", None);
        refused(json!([]), "typed-content", None);
        assert!(on_stream(&store, "roadmap").is_empty());

        let command = command(ANSWER_COMMAND, R3, 3, AT1);
        let answer = run(
            &store,
            &set,
            &json!([take("q1"), defer("q2", "after the spike")]),
            &command,
            OWNER,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let recorded = on_stream(&store, "roadmap");
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].type_name, "question.answered");
        assert_eq!(
            recorded[0].payload,
            json!({"set": set, "question": "q1", "round": 1, "answer": "SQLite",
                "accepted": true, "owner": OWNER, "at": AT1})
        );
        assert_eq!(recorded[1].type_name, "question.deferred");
        assert_eq!(
            recorded[1].payload,
            json!({"set": set, "question": "q2", "round": 1, "reason": "after the spike",
                "owner": OWNER, "at": AT1})
        );
        for event in &recorded {
            assert_eq!(event.request_id.0, R3);
        }

        refused(
            json!([own("q1", "Postgres")]),
            "question-closed",
            Some("q1"),
        );
        refused(json!([take("q2")]), "question-closed", Some("q2"));
        let again = on_stream(&store, "roadmap");
        assert_eq!(again.len(), 2);
        assert_eq!(again[0].payload, recorded[0].payload);
        assert_eq!(again[1].payload, recorded[1].payload);
    }

    fn call(items: &Value) -> Call {
        judge_call(items).unwrap()
    }

    fn checked(store: &SqliteStore, set: &str, items: &Value) -> Checked {
        check(store, &project(), set, &call(items)).unwrap()
    }

    #[test]
    fn an_answer_recorded_over_a_set_that_moved_since_its_check_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r2(), &story(1));
        let first = checked(&store, &set, &json!([take("q1")]));
        let second = run(
            &store,
            &set,
            &json!([defer("q2", "later")]),
            &command(ANSWER_COMMAND, R2, 2, T2),
            OWNER,
        );
        assert_eq!(second["status"], "ok", "{second}");
        let before = head(&store);
        let late = answer(
            record(
                &store,
                &command(ANSWER_COMMAND, R3, 3, T3),
                OWNER,
                Some(&first),
                Drafting::None,
            ),
            &store,
        );
        assert_eq!(late["status"], "failed", "{late}");
        assert_eq!(late["code"], "ledger-busy", "{late}");
        assert_eq!(head(&store), before);
        let recorded = on_stream(&store, "roadmap");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].type_name, "question.deferred");
    }

    #[test]
    fn an_answer_recorded_off_its_sets_stream_is_caught() {
        let (_dir, store) = store();
        let plan = baley_core::questions::Subject::Plan {
            phase: 2,
            plan_digest: baley_store::Hash([0x5c; 32]),
        };
        let set = open_set(&store, R1, &r2(), &plan);
        let answer = run(
            &store,
            &set,
            &json!([take("q1")]),
            &command(ANSWER_COMMAND, R2, 2, T2),
            OWNER,
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let recorded = on_stream(&store, "phase/2");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].type_name, "question.answered");
        assert!(on_stream(&store, "roadmap").is_empty());
    }

    fn ids(list: &Value) -> Vec<&str> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_receipt_without_the_next_round_or_the_closed_set_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r2(), &story(1));
        let first = run(
            &store,
            &set,
            &json!([take("q1")]),
            &command(ANSWER_COMMAND, R2, 2, T2),
            OWNER,
        );
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["set"], set.as_str());
        assert_eq!(first["round"], 1);
        assert_eq!(first["recorded_at"], T2);
        assert_eq!(first["now"]["state"], "open");
        assert_eq!(first["now"]["round"], 2);
        assert_eq!(ids(&first["now"]["current"]), ["q2", "q3"]);
        assert!(
            first["now"]["relay"]
                .as_str()
                .unwrap()
                .contains("`bal-rounds`")
        );

        let second = run(
            &store,
            &set,
            &json!([take("q2"), take("q3")]),
            &command(ANSWER_COMMAND, R3, 3, T3),
            OWNER,
        );
        assert_eq!(second["round"], 2);
        assert_eq!(second["now"]["state"], "closed", "{second}");

        let other = open_set(&store, R4, &r2(), &story(1));
        run(
            &store,
            &other,
            &json!([defer("q1", "later")]),
            &command(ANSWER_COMMAND, R5, 5, T5),
            OWNER,
        );
        let last = run(
            &store,
            &other,
            &json!([take("q2")]),
            &command(ANSWER_COMMAND, R6, 6, T6),
            OWNER,
        );
        assert_eq!(last["round"], 2);
        assert_eq!(last["now"]["state"], "open", "{last}");
        assert_eq!(last["now"]["current"], json!([]));
        assert_eq!(
            last["now"]["waiting"],
            json!([{"id": "q3", "question": "Where does the ledger live?",
                "waits_on": [{"id": "q1", "state": "deferred"}]}])
        );
    }

    #[test]
    fn a_receipt_carrying_a_round_past_the_part_bound_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let text = "x".repeat(4096);
        let questions: Vec<Value> = (1..=10)
            .map(|n| {
                json!({"id": format!("q{n}"), "question": text, "recommended": "Yes",
                    "depends_on": []})
            })
            .collect();
        let long = open_set(
            &store,
            R1,
            &judge_set(&Value::Array(questions)).unwrap(),
            &story(1),
        );
        let receipt = run(
            &store,
            &long,
            &json!([take("q1")]),
            &command(ANSWER_COMMAND, R2, 2, T2),
            OWNER,
        );
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert!(serde_json::to_string(&receipt).unwrap().len() <= 24_576);
        assert_eq!(receipt["now"]["state"], "open");
        assert_eq!(receipt["now"]["round"], 2);
        assert_eq!(
            receipt["now"]["read"],
            json!({"kind": "questions", "set": long})
        );
        assert!(receipt["now"].get("current").is_none(), "{receipt}");

        let short = open_set(&store, R3, &r2(), &story(1));
        let receipt = run(
            &store,
            &short,
            &json!([take("q1")]),
            &command(ANSWER_COMMAND, R4, 4, T4),
            OWNER,
        );
        assert_eq!(ids(&receipt["now"]["current"]), ["q2", "q3"]);
        assert!(receipt["now"].get("read").is_none(), "{receipt}");
    }

    #[test]
    fn a_replayed_answer_call_recorded_twice_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let set = open_set(&store, R1, &r2(), &story(1));
        let kept = checked(&store, &set, &json!([take("q1")]));
        let command = command(ANSWER_COMMAND, R2, 2, T2);
        let first = answer(
            record(&store, &command, OWNER, Some(&kept), Drafting::None),
            &store,
        );
        let again = answer(
            record(&store, &command, OWNER, Some(&kept), Drafting::None),
            &store,
        );
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(again, first);
        assert_eq!(on_stream(&store, "roadmap").len(), 1);
    }

    #[test]
    fn an_answer_recorded_without_its_checks_read_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        open_set(&store, R1, &r2(), &story(1));
        let before = head(&store);
        let unchecked = record(
            &store,
            &command(ANSWER_COMMAND, R2, 2, T2),
            OWNER,
            None,
            Drafting::None,
        );
        assert!(unchecked.is_err(), "{unchecked:?}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn an_unknown_set_checked_as_anything_but_no_such_question_set_is_caught() {
        let (_dir, store) = store();
        declare(&store, &project(), 1);
        open_set(&store, R1, &r2(), &story(1));
        let before = head(&store);
        let unknown = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
        let refusal = check(&store, &project(), unknown, &call(&json!([take("q1")])))
            .expect_err("no such set");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "no-such-question-set");
        assert!(!refusal.to_string().contains(unknown), "{refusal}");
        assert_eq!(head(&store), before);
    }

    /// The drafting order a closing call issues in these tests.
    pub(crate) const D1: &str = "4f5a6b7c-8d9e-4f0a-9b1c-3d4e5f6a7b8c";
    /// The owner the plan's data names.
    pub(crate) const ADA: &str = "Ada Owner";

    /// Runs one call as `questions-answer` runs it below the session: the
    /// grammar, the relayed check and the relayed recording under `request`
    /// at `at` as [`ADA`], with the default policy, which sets nothing for
    /// the analyzer, and an empty catalog; a drafting order is issued as
    /// [`D1`].
    pub(crate) fn relay(
        store: &SqliteStore,
        set: &str,
        items: &Value,
        request: &str,
        at: &str,
    ) -> Value {
        use crate::mcp::questions::relay_check;

        let call = match judge_call(items) {
            Ok(call) => call,
            Err(fault) => return call_fault(fault),
        };
        match relay_check(store, &project(), set, &call) {
            Ok(relayed) => relay_recorded(store, &relayed, request, at),
            Err(refusal) => refusal,
        }
    }

    /// The relayed recording [`relay`] runs after its check.
    fn relay_recorded(
        store: &SqliteStore,
        relayed: &crate::mcp::questions::Relayed,
        request: &str,
        at: &str,
    ) -> Value {
        use std::collections::BTreeSet;

        use baley_core::policy::{AcceptedNames, Host, Schema, effective_policy};

        use crate::mcp::questions::relay_record;

        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        relay_record(
            store,
            &command(ANSWER_COMMAND, request, 2, at),
            ADA,
            relayed,
            &policy,
            Host::ClaudeCode,
            &names,
            D1,
        )
    }

    /// The `work_order.issued` events naming `subject` whose instructions
    /// include `identity`.
    fn issued_for(store: &SqliteStore, subject: &str, identity: &str) -> Vec<Event> {
        history(store)
            .into_iter()
            .filter(|event| {
                event.type_name == "work_order.issued"
                    && event.payload["subject"] == subject
                    && event.payload["instructions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|entry| entry["identity"] == identity)
            })
            .collect()
    }

    fn order(store: &SqliteStore, id: &str) -> Value {
        crate::work_order::read::order(store, &project(), id)
            .unwrap()
            .unwrap_or_else(|| panic!("order {id} is held"))
    }

    #[test]
    fn a_drafting_order_issued_with_a_question_open_or_a_second_analyzer_order_recorded_is_caught()
    {
        use crate::questions::record::tests::q1;
        use crate::refine::order::check as refine_check;
        use crate::refine::order::tests::{approved, input_body, input_names, issue_checked};
        use crate::refine::questions::tests::{D, Q, Q2, submitted};

        let (_dir, store) = approved();
        let checked = refine_check(&store, &project(), 1).unwrap();
        let questions_order = issue_checked(&store, &checked, Q, R1, T1);
        assert_eq!(
            questions_order["analyzer"], "questions",
            "{questions_order}"
        );
        assert_eq!(
            order(&store, Q)["instructions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["identity"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["bal-analyzer-questions"]
        );
        let before = head(&store);
        let pending = refine_check(&store, &project(), 1).expect_err("one live order");
        assert_eq!(pending["status"], "refused", "{pending}");
        assert_eq!(pending["code"], "analyzer-order-pending");
        assert_eq!(pending["details"], json!({"work_order": Q}));
        assert_eq!(head(&store), before);

        let opened = submitted(&store, 1, Q, &q1(), R2, T2);
        assert_eq!(opened["status"], "ok", "{opened}");
        assert_eq!(opened["now"]["round"], 1);
        assert_eq!(ids(&opened["now"]["current"]), ["q1"]);
        assert!(opened.get("drafting").is_none(), "{opened}");
        assert_eq!(order(&store, Q)["state"], "answered");
        let set = opened["set"].as_str().unwrap().to_owned();

        let first = relay(&store, &set, &json!([take("q1")]), R3, T3);
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["now"]["state"], "open");
        assert_eq!(first["now"]["round"], 2);
        assert!(first.get("drafting").is_none(), "{first}");
        assert!(issued_for(&store, "story/1", "bal-analyzer-drafting").is_empty());
        let before = head(&store);
        let open = refine_check(&store, &project(), 1).expect_err("a question is open");
        assert_eq!(open["code"], "question-open", "{open}");
        assert_eq!(open["details"], json!({"set": set, "question": "q2"}));
        assert_eq!(head(&store), before);

        let close = relay(
            &store,
            &set,
            &json!([own("q2", "In the owner's data folder")]),
            R4,
            T4,
        );
        assert_eq!(close["status"], "ok", "{close}");
        assert_eq!(close["now"]["state"], "closed", "{close}");
        assert_eq!(close["drafting"]["work_order"], D1, "{close}");
        let drafting = order(&store, D1);
        assert_eq!(drafting["state"], "pending", "{drafting}");
        assert_eq!(drafting["role"], "analyzer");
        assert_eq!(drafting["subject"], "story/1");
        assert_eq!(
            drafting["instructions"][0]["identity"],
            "bal-analyzer-drafting"
        );
        assert_eq!(drafting["instructions"].as_array().unwrap().len(), 1);
        let issued = issued_for(&store, "story/1", "bal-analyzer-drafting");
        assert_eq!(issued.len(), 1);
        assert_eq!(
            input_names(&issued[0]),
            ["story", "description", "roadmap", "questions"]
        );
        assert_eq!(
            input_body(&store, &issued[0], "questions")["answers"],
            json!([
                {"question": "q1", "answer": "SQLite", "accepted": true, "round": 1,
                    "owner": ADA, "at": T3},
                {"question": "q2", "answer": "In the owner's data folder", "accepted": false,
                    "round": 2, "owner": ADA, "at": T4}])
        );

        let checked = refine_check(&store, &project(), 2).unwrap();
        let story_two = issue_checked(&store, &checked, Q2, R5, T5);
        assert_eq!(story_two["analyzer"], "questions", "{story_two}");
        let empty = submitted(&store, 2, Q2, &judge_set(&json!([])).unwrap(), R6, T6);
        assert_eq!(empty["status"], "ok", "{empty}");
        assert_eq!(empty["now"]["state"], "closed", "{empty}");
        assert_eq!(empty["drafting"]["work_order"], D, "{empty}");
        let drafting = order(&store, D);
        assert_eq!(drafting["state"], "pending", "{drafting}");
        assert_eq!(drafting["subject"], "story/2");
        assert_eq!(
            drafting["instructions"][0]["identity"],
            "bal-analyzer-drafting"
        );
        assert_eq!(order(&store, Q2)["state"], "answered");
    }

    #[test]
    fn a_close_at_the_terminal_issuing_an_order_is_caught() {
        use crate::questions::record::tests::q1;
        use crate::refine::order::tests::approved;
        use crate::refine::order::{OrderKind, check as refine_check};

        let (_dir, store) = approved();
        let set = open_set(&store, R1, &q1(), &story(1));
        let first = run(
            &store,
            &set,
            &json!([take("q1")]),
            &command(ANSWER_COMMAND, R2, 2, T2),
            ADA,
        );
        assert_eq!(first["status"], "ok", "{first}");
        let close = run(
            &store,
            &set,
            &json!([take("q2")]),
            &command(ANSWER_COMMAND, R3, 3, T3),
            ADA,
        );
        assert_eq!(close["status"], "ok", "{close}");
        assert_eq!(close["now"]["state"], "closed", "{close}");
        assert!(close.get("drafting").is_none(), "{close}");
        assert!(of_type(&history(&store), "work_order.issued").is_empty());
        let next = refine_check(&store, &project(), 1).expect("the story can be refined");
        assert_eq!(next.kind, OrderKind::Drafting);
    }

    #[test]
    fn a_close_beside_a_pending_analyzer_order_issuing_a_second_is_caught() {
        use crate::questions::record::tests::q1;
        use crate::refine::order::pending_analyzer;
        use crate::refine::order::tests::{approved, issue_by_hand};
        use crate::refine::questions::tests::Q;

        let (_dir, store) = approved();
        let set = open_set(&store, R1, &q1(), &story(1));
        issue_by_hand(&store, Q, 1, R2);
        let first = relay(&store, &set, &json!([take("q1")]), R3, T3);
        assert_eq!(first["status"], "ok", "{first}");
        let close = relay(&store, &set, &json!([take("q2")]), R4, T4);
        assert_eq!(close["status"], "ok", "{close}");
        assert_eq!(close["now"]["state"], "closed", "{close}");
        assert!(close.get("drafting").is_none(), "{close}");
        assert!(issued_for(&store, "story/1", "bal-analyzer-drafting").is_empty());
        assert_eq!(
            pending_analyzer(&store, &project(), 1).unwrap().as_deref(),
            Some(Q)
        );
    }

    #[test]
    fn a_close_held_back_by_an_analyzer_order_settled_since_its_check_recorded_is_caught() {
        use crate::mcp::questions::relay_check;
        use crate::mcp::work_order::tests::fail_order;
        use crate::questions::record::tests::q1;
        use crate::refine::order::tests::{approved, issue_by_hand};
        use crate::refine::questions::tests::Q;

        let (_dir, store) = approved();
        let set = open_set(&store, R1, &q1(), &story(1));
        issue_by_hand(&store, Q, 1, R2);
        let first = relay(&store, &set, &json!([take("q1")]), R3, T3);
        assert_eq!(first["status"], "ok", "{first}");

        // The closing call's check sees Q pending, then Q is failed before
        // the call is recorded.
        let call = judge_call(&json!([take("q2")])).unwrap();
        let relayed = relay_check(&store, &project(), &set, &call).unwrap();
        fail_order(&store, &project(), Q, R4, T4);
        let before = head(&store);
        let late = relay_recorded(&store, &relayed, R5, T5);
        assert_eq!(late["status"], "failed", "{late}");
        assert_eq!(late["code"], "ledger-busy", "{late}");
        assert_eq!(head(&store), before);

        // The session's retry of the same call checks again and drafts.
        let retry = relay(&store, &set, &json!([take("q2")]), R5, T5);
        assert_eq!(retry["status"], "ok", "{retry}");
        assert_eq!(retry["now"]["state"], "closed", "{retry}");
        assert_eq!(retry["drafting"]["work_order"], D1, "{retry}");
        assert_eq!(order(&store, D1)["state"], "pending");
    }

    #[test]
    fn a_plan_set_close_issuing_a_drafting_order_is_caught() {
        use crate::refine::order::tests::approved;
        use crate::refine::questions::tests::q3;

        let (_dir, store) = approved();
        let plan = baley_core::questions::Subject::Plan {
            phase: 1,
            plan_digest: baley_store::Hash([0x5c; 32]),
        };
        let set = open_set(&store, R1, &q3(), &plan);
        let close = relay(&store, &set, &json!([take("q3")]), R2, T2);
        assert_eq!(close["status"], "ok", "{close}");
        assert_eq!(close["now"]["state"], "closed", "{close}");
        assert!(close.get("drafting").is_none(), "{close}");
        assert!(of_type(&history(&store), "work_order.issued").is_empty());
    }

    #[test]
    fn a_terminal_close_issuing_a_revision_or_a_story_set_saying_ready_is_caught() {
        use crate::planning::fixtures::{pl2, store_c};
        use crate::planning::order::tests::O;
        use crate::planning::submit::tests::{order_o, plan_submitted};

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let submitted = plan_submitted(&store, &pl2(), O, None, R2, T5);
        assert_eq!(submitted["status"], "ok", "{submitted}");
        let s2 = submitted["set"].as_str().unwrap().to_owned();
        let orders = |store: &SqliteStore| {
            of_type(&history(store), "work_order.issued")
                .iter()
                .map(|event| event.payload["id"].clone())
                .collect::<Vec<_>>()
        };
        let before = orders(&store);
        assert_eq!(before, [json!(O)]);

        // As `baley questions answer` runs it: the owner's command, no
        // session caller.
        let terminal = |request_id: &str| {
            let mut command = command(ANSWER_COMMAND, request_id, 4, T6);
            command.actor = baley_store::Actor::Owner;
            command.caller = None;
            command
        };
        let plan = run(
            &store,
            &s2,
            &json!([own("q1", "Postgres")]),
            &terminal(R3),
            OWNER,
        );
        assert_eq!(plan["status"], "ok", "{plan}");
        assert_eq!(plan["now"]["state"], "closed", "{plan}");
        assert!(plan.get("ready").is_none(), "{plan}");
        assert!(plan.get("revision").is_none(), "{plan}");

        let one = judge_set(
            &json!([{"id": "q1", "question": "Which store backs the ledger?",
            "recommended": "SQLite", "depends_on": []}]),
        )
        .unwrap();
        let set = open_set(&store, R4, &one, &story(1));
        let story_set = run(&store, &set, &json!([take("q1")]), &terminal(R5), OWNER);
        assert_eq!(story_set["status"], "ok", "{story_set}");
        assert_eq!(story_set["now"]["state"], "closed", "{story_set}");
        assert!(story_set.get("ready").is_none(), "{story_set}");
        assert!(story_set.get("drafting").is_none(), "{story_set}");
        assert_eq!(orders(&store), before);
    }
}
