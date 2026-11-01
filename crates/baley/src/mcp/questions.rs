//! `questions-answer` on `baley_apply`: a session relays the owner's answers
//! and deferrals for one round of a question set (design 0005 `questions
//! answer`, PLN-R24 to PLN-R26, HST-R9).
//!
//! The arguments, the request id and the call's grammar are judged before
//! anything is prepared. The write is then prepared with the answer check as
//! its own check, so every refusal comes before checkout admission and the
//! policy step and records nothing, and the call is recorded through the
//! same library function `baley questions answer` calls. The command is
//! Baley's with the session as its caller: over MCP Baley cannot tell
//! whether the owner gave the answer, so the record does not claim it, and
//! the owner's name travels in each event's payload. A call that closes a
//! story's set also issues the analyzer's drafting order with the answers
//! (D-08), and one that closes a plan draft's set with an answer rejecting a
//! recommendation the planner's revision order (D-15), which `baley
//! questions answer` never does (ADR 0044). A plan draft's set closed on
//! its recommendations answers `ready` on either surface.

use std::cell::RefCell;
use std::collections::BTreeSet;

use baley_core::policy::{AcceptedNames, EffectivePolicy, Host};
use baley_core::questions::{Call, SetFacts, judge_call};
use baley_core::work_order::WORK_ORDER_VIEW;
use baley_store::{
    Command, CommandKind, Ledger, ObservedDocument, Payloads, ProjectId, RequestId, Views,
    request_digest,
};
use baley_store_sqlite::SqliteStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::store_failed;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::WriteRequest;
use crate::mcp::work_order::{judge_request_id, prepared_write, shape_fault};
use crate::planning::inputs::Revision;
use crate::planning::order::{pending_planner_order, prepare_revision};
use crate::providers::identity::{self, OWNER_IDENTITY_MISSING};
use crate::questions::answer::{self, ANSWER_COMMAND, Drafting};
use crate::questions::read::PlanSubject;
use crate::refine::order::{pending_analyzer_order, prepare_drafting};
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::route::host_names;

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `questions-answer` takes beside its `operation`. Unknown
/// fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerShape {
    /// A fresh UUID for this call in lowercase hyphenated form. A retry of
    /// the same call sends the same id.
    pub request_id: String,
    /// The question set's id, as its round read or an earlier receipt gave
    /// it.
    pub set: String,
    /// One item for each question the owner settles in this round:
    /// `{"question":"<id>","act":"take"}` takes the recommended answer,
    /// `{"question":"<id>","act":"answer","answer":"<the owner's words>"}`
    /// records the owner's own answer, and
    /// `{"question":"<id>","act":"defer","reason":"<the owner's reason>"}`
    /// leaves the question undecided. Each question at most once, every
    /// text at most 4096 bytes of UTF-8.
    // Typed as a list of objects in the schema, but taken as a value so the
    // call's own grammar judges it and a fault is `typed-content`.
    #[schemars(with = "Vec<Map<String, Value>>")]
    pub items: Value,
}

/// The tagged form `questions-answer`'s arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum AnswerCall {
    #[serde(rename = "questions-answer")]
    Answer(AnswerShape),
}

/// An answer call whose arguments passed every check made before
/// preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedAnswer {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The set answered.
    pub set: String,
    /// The call, its grammar judged.
    pub call: Call,
}

/// Judges `questions-answer`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the call's grammar. A
/// fault is the `refused` answer, which records nothing. The request digest
/// covers the command kind, the set and the items and never the policy
/// version, so a retry after a policy edit replays.
pub fn judge_arguments(arguments: &Value) -> Result<JudgedAnswer, Value> {
    let shape = match serde_json::from_value::<AnswerCall>(arguments.clone()) {
        Ok(AnswerCall::Answer(shape)) => shape,
        Err(error) => return Err(shape_fault(&error)),
    };
    judge_request_id(&shape.request_id, "call")?;
    let call = judge_call(&shape.items).map_err(answer::call_fault)?;
    let digest =
        request_digest(&json!({"command": ANSWER_COMMAND, "set": shape.set, "items": shape.items}))
            .expect("judged items are canonical");
    Ok(JudgedAnswer {
        request: WriteRequest {
            kind: CommandKind(ANSWER_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        set: shape.set,
        call,
    })
}

/// What a relayed answer call's check read: the answer check's read of the
/// set and the live order of its subject: for a story's set, the story's
/// pending analyzer order; for a plan draft's, the phase's pending planner
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relayed {
    /// The answer check's read.
    pub checked: answer::Checked,
    /// The `work_order` document of that order pending at the check, at the
    /// sequence that produced it, or none.
    pub pending: Option<ObservedDocument>,
}

/// The order a relayed call's close issues, with the facts it binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Closing {
    /// The analyzer's drafting order of this story (D-08).
    Drafting(u32, SetFacts),
    /// The planner's revision order of this plan draft (D-15).
    Revision(PlanSubject, SetFacts),
}

impl Relayed {
    /// The order the call, recorded by `owner` at `at`, issues with its set's
    /// facts after the call, when no order of the set's subject was pending
    /// at the check: the drafting order when it closes a story's set (D-08),
    /// the planner's revision order when it closes a plan draft's set with an
    /// answer rejecting a recommendation (D-15, PLN-R27). None otherwise: a
    /// pending order keeps its subject's one live order (D-09, D-11), which
    /// the session's next `story-refine` or `phase-plan` follows once it is
    /// settled, and a plan draft's set closed on recommendations is `ready`.
    pub(crate) fn closing(&self, owner: &str, at: &str) -> Option<Closing> {
        if self.pending.is_some() {
            return None;
        }
        self.due(owner, at)
    }

    /// The order the call would issue were nothing pending, by the rules
    /// [`Self::closing`] gives.
    fn due(&self, owner: &str, at: &str) -> Option<Closing> {
        let facts = self.checked.closed_facts(owner, at)?;
        match (self.checked.story, self.checked.plan) {
            (Some(story), _) => Some(Closing::Drafting(story, facts)),
            (None, Some(plan)) if facts.answers.iter().any(|answer| !answer.accepted) => {
                Some(Closing::Revision(plan, facts))
            }
            _ => None,
        }
    }

    /// The pending order's document when the call, recorded by `owner` at
    /// `at`, would issue an order beside it, so that order alone keeps the
    /// call from issuing one. None otherwise.
    fn held_by(&self, owner: &str, at: &str) -> Option<&ObservedDocument> {
        self.due(owner, at)?;
        self.pending.as_ref()
    }
}

/// A relayed answer call's check, run before admission and recording
/// nothing: the answer check, then the pending order of the set's subject
/// with the document it was read from: a story's analyzer order, or a plan
/// draft's phase's planner order. A refusal or a read fault is the answer to
/// give.
pub(crate) fn relay_check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    set: &str,
    call: &Call,
) -> Result<Relayed, Value> {
    let checked = answer::check(store, project, set, call)?;
    let pending = match (checked.story, checked.plan) {
        (Some(story), _) => pending_analyzer_order(store, project, story),
        (None, Some(plan)) => pending_planner_order(store, project, plan.phase),
        (None, None) => Ok(None),
    }
    .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
    .map(|document| ObservedDocument {
        view: WORK_ORDER_VIEW.into(),
        key: document.key,
        produced_seq: Some(document.produced_seq),
    });
    Ok(Relayed { checked, pending })
}

/// Records a relayed call `relayed` under `command` as `owner` once the write
/// is prepared. When the call closes a story's set with no analyzer order of
/// the story pending, the drafting order is prepared as `order_id` under
/// `policy`, the write's effective policy, for `host` with the catalog
/// `names`, its `questions` input holding the set's answers and deferrals
/// with this call's own, and is issued with the answers. When it closes a
/// plan draft's set with an answer rejecting a recommendation and no planner
/// order of the phase pending, the planner's revision order is prepared the
/// same way, attempt 1 of a new chain, binding the planning inputs for the
/// phase's stories, the draft and the set's facts with this call's own, and
/// is issued with the answers. When a pending order held the new one back,
/// the recording observes that order's document, so an order settled since
/// the check makes the call `ledger-busy` and the session's retry checks
/// again. The answer is the receipt, or why nothing was recorded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn relay_record<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    owner: &str,
    relayed: &Relayed,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    order_id: &str,
) -> Value {
    let at = &command.recorded_at;
    let (mut drafting, mut revision) = (None, None);
    match relayed.closing(owner, at) {
        Some(Closing::Drafting(story, facts)) => {
            match prepare_drafting(store, command, story, &facts, policy, host, names, order_id) {
                Ok(prepared) => drafting = Some(prepared),
                Err(answer) => return answer,
            }
        }
        Some(Closing::Revision(plan, facts)) => {
            let draft = Revision {
                draft: plan.draft,
                set: &relayed.checked.set,
            };
            match prepare_revision(
                store, command, plan.phase, draft, &facts, policy, host, names, order_id,
            ) {
                Ok(prepared) => revision = Some(prepared),
                Err(answer) => return answer,
            }
        }
        None => {}
    }
    let order = match (&drafting, &revision, relayed.held_by(owner, at)) {
        (Some(order), _, _) => Drafting::Issue(order),
        (None, Some(order), _) => Drafting::Revise(order),
        (None, None, Some(pending)) => Drafting::HeldBy(pending),
        (None, None, None) => Drafting::None,
    };
    answer::answer(
        answer::record(store, command, owner, Some(&relayed.checked), order),
        store,
    )
}

/// Serves one `questions-answer` call for the session's `preparation`. The
/// arguments are judged first; then the write is prepared under the
/// session's caller with the relayed check as its own check; then the owner
/// is read from the write's policy, falling back to git's `user.name`, and
/// the call is recorded over the set the check read, with the drafting order
/// when it closes a story's set (D-08) and the revision order when it closes
/// a plan draft's set on a rejected recommendation (D-15). A replay goes to
/// the recording with no read, and the store answers it from its record.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    // Kept for the recording to observe. A replay skips the check.
    let kept = RefCell::new(None);
    let check = |store: &SqliteStore, project: &ProjectId| {
        *kept.borrow_mut() = Some(relay_check(store, project, &judged.set, &judged.call)?);
        Ok(())
    };
    let (store, command, stepped) =
        match prepared_write(preparation, &preparation.caller, &judged.request, &check) {
            Ok(prepared) => prepared,
            Err(answer) => return answer,
        };
    let kept = kept.borrow();
    // A replay ran no steps and is answered from its record, so it needs no
    // owner.
    let (Some(stepped), Some(relayed)) = (stepped, kept.as_ref()) else {
        return answer::answer(
            answer::record(store, &command, "", None, Drafting::None),
            store,
        );
    };
    let owner = match identity::owner(
        identity::owner_name(&stepped.policy),
        &mut crate::process::System,
    ) {
        Ok(owner) => owner,
        Err(reason) => return Refusal::new(OWNER_IDENTITY_MISSING, reason).value(),
    };
    // The catalog is read only for an order to issue, and unused otherwise.
    let names = if relayed.closing(&owner, &command.recorded_at).is_some() {
        match host_names(store, preparation.host) {
            Ok(names) => names,
            Err(error) => return store_failed(&error, NOT_RECORDED_BUSY),
        }
    } else {
        AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        }
    };
    relay_record(
        store,
        &command,
        &owner,
        relayed,
        &stepped.policy,
        preparation.host,
        &names,
        &uuid::Uuid::new_v4().to_string(),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    //! Expected answers are written out from design 0005 `questions answer`
    //! and design 0012's write rules, never from running this code. Stores
    //! are real SQLite stores in fresh temporary directories, with times
    //! supplied, built from the plan's data store C, P0, N0 and PL2.

    use baley_core::questions::judge_call;
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::{n0, p0, pl2, store_c};
    use crate::planning::order::tests::{O, fail_by_hand, prepared};
    use crate::planning::submit::tests::{as_drafted, order_o, plan_submitted};
    use crate::refine::order::tests::{input_body, input_names, issued};
    use crate::scope::fixtures::{R1, R2, R3, R4, R5, T5, T6, T7, command, head, project};

    const SET: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const OTHER: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn arguments(set: &str, items: Value) -> Value {
        json!({"operation": "questions-answer", "request_id": REQUEST, "set": set,
            "items": items})
    }

    #[test]
    fn a_questions_answer_shape_or_request_id_fault_accepted_is_caught() {
        let take = json!([{"question": "q1", "act": "take"}]);
        let mut extra = arguments(SET, take.clone());
        extra["instruction"] = json!("bal-rounds");
        let mut upper = arguments(SET, take);
        upper["request_id"] = json!(REQUEST.to_uppercase());
        for (call, code, slot) in [
            (extra, "invalid-arguments", "arguments"),
            (upper, "invalid-arguments", "request_id"),
            (arguments(SET, json!("q1")), "typed-content", "items"),
        ] {
            let refusal = judge_arguments(&call).expect_err("refused");
            assert_eq!(refusal["status"], "refused", "{call}");
            assert_eq!(refusal["code"], code, "{call}");
            assert_eq!(refusal["slot"], slot, "{call}");
        }
    }

    fn digest(set: &str, items: Value) -> baley_store::Hash {
        judge_arguments(&arguments(set, items))
            .unwrap()
            .request
            .digest
    }

    #[test]
    fn a_relayed_answer_digest_covering_the_policy_or_missing_the_items_is_caught() {
        let take = |question: &str| json!([{"question": question, "act": "take"}]);
        let first = digest(SET, take("q1"));
        assert_eq!(digest(SET, take("q1")), first);
        assert_ne!(digest(SET, take("q2")), first);
        assert_ne!(digest(OTHER, take("q1")), first);
        let own = |answer: &str| json!([{"question": "q1", "act": "answer", "answer": answer}]);
        assert_ne!(digest(SET, own("SQLite")), digest(SET, own("Postgres")));
        let defer = |reason: &str| json!([{"question": "q2", "act": "defer", "reason": reason}]);
        assert_ne!(digest(SET, defer("a")), digest(SET, defer("b")));
    }

    /// R, the revision order's id.
    pub(crate) const REVISION: &str = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
    /// Q, a planning order of phase 1 issued by hand.
    const Q: &str = "8d9e0f1a-2b3c-4d4e-9f5a-6b7c8d9e0f1a";

    /// `q1` answered `Postgres`, not its recommendation.
    pub(crate) fn postgres() -> Value {
        json!([{"question": "q1", "act": "answer", "answer": "Postgres"}])
    }

    /// Relays `items` on `set` under `request_id` at `at` through the relay
    /// check and record under P0 and N0 for Claude Code as `Ada Owner`, as
    /// `questions-answer` runs them below the session, any order issued as
    /// `order_id`. Gives the answer.
    pub(crate) fn relayed(
        store: &SqliteStore,
        set: &str,
        items: &Value,
        request_id: &str,
        at: &str,
        order_id: &str,
    ) -> Value {
        let call = judge_call(items).expect("the call's grammar holds");
        let relayed = match relay_check(store, &project(), set, &call) {
            Ok(relayed) => relayed,
            Err(refusal) => return refusal,
        };
        let command = command(ANSWER_COMMAND, request_id, 9, at);
        relay_record(
            store,
            &command,
            "Ada Owner",
            &relayed,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            order_id,
        )
    }

    /// Store C with O, PL2 submitted under O by R2 at T5. Gives the store
    /// and S2.
    fn pl2_drafted() -> (tempfile::TempDir, SqliteStore, String) {
        let (dir, store) = store_c();
        order_o(&store, R1);
        let receipt = plan_submitted(&store, &pl2(), O, None, R2, T5);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        let s2 = receipt["set"].as_str().unwrap().to_owned();
        (dir, store, s2)
    }

    /// The ids of every order the store issued, oldest first.
    fn issued_ids(store: &SqliteStore) -> Vec<String> {
        issued(store)
            .iter()
            .map(|event| event.payload["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_plan_set_closed_on_a_rejected_recommendation_without_its_revision_order_is_caught() {
        let (_dir, store, s2) = pl2_drafted();
        let receipt = relayed(&store, &s2, &postgres(), R3, T6, REVISION);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["now"]["state"], "closed", "{receipt}");
        assert_eq!(receipt["revision"]["work_order"], REVISION, "{receipt}");
        assert!(receipt.get("ready").is_none(), "{receipt}");

        let order = crate::work_order::read::order(&store, &project(), REVISION)
            .unwrap()
            .expect("R is held");
        assert_eq!(order["state"], "pending", "{order}");
        let events = issued(&store);
        let event = events
            .iter()
            .find(|event| event.payload["id"] == REVISION)
            .expect("R is issued");
        assert_eq!(event.request_id.0, R3);
        let payload = &event.payload;
        assert_eq!(payload["role"], "planner");
        assert_eq!(payload["subject"], "phase/1");
        assert_eq!(payload["attempt"], 1);
        assert_eq!(payload["request"], REVISION);
        assert_eq!(payload["parent"], Value::Null);
        let identities: Vec<&str> = payload["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|evidence| evidence["identity"].as_str().unwrap())
            .collect();
        assert_eq!(identities, ["bal-phase-planner"]);
        assert_eq!(
            input_names(event),
            [
                "phase",
                "stories",
                "description",
                "approved_items",
                "draft",
                "questions"
            ]
        );
        assert_eq!(input_body(&store, event, "draft"), as_drafted(pl2()));
        let questions = input_body(&store, event, "questions");
        assert_eq!(questions["set"], s2.as_str());
        assert_eq!(
            questions["answers"],
            json!([{"question": "q1", "answer": "Postgres", "accepted": false, "round": 1,
                "owner": "Ada Owner", "at": T6}])
        );
        let stories = input_body(&store, event, "stories");
        let ids: Vec<&Value> = stories
            .as_array()
            .unwrap()
            .iter()
            .map(|story| &story["id"])
            .collect();
        assert_eq!(ids, [&json!(1)]);
    }

    #[test]
    fn a_plan_set_closing_without_ready_or_a_revision_beside_a_pending_planner_order_is_caught() {
        let (_dir, store, s2) = pl2_drafted();
        let take = json!([{"question": "q1", "act": "take"}]);
        let receipt = relayed(&store, &s2, &take, R3, T6, REVISION);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["now"]["state"], "closed", "{receipt}");
        assert_eq!(receipt["ready"], true, "{receipt}");
        assert!(receipt.get("revision").is_none(), "{receipt}");
        assert_eq!(issued_ids(&store), [O]);

        // Q pending at the check holds the revision back.
        let (_dir, store, s2) = pl2_drafted();
        let (command_q, q) = prepared(&store, &[1], Q, R3, T6);
        assert_eq!(
            crate::planning::order::issue(&store, &command_q, &q)["status"],
            "ok"
        );
        let receipt = relayed(&store, &s2, &postgres(), R4, T7, REVISION);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["now"]["state"], "closed", "{receipt}");
        assert!(receipt.get("revision").is_none(), "{receipt}");
        assert!(receipt.get("ready").is_none(), "{receipt}");
        assert_eq!(issued_ids(&store), [O, Q]);

        // Q failed between the check and the record: the set does not close
        // with no revision.
        let (_dir, store, s2) = pl2_drafted();
        let (command_q, q) = prepared(&store, &[1], Q, R3, T6);
        assert_eq!(
            crate::planning::order::issue(&store, &command_q, &q)["status"],
            "ok"
        );
        let call = judge_call(&postgres()).unwrap();
        let checked = relay_check(&store, &project(), &s2, &call).expect("checked");
        fail_by_hand(&store, Q, R4);
        let before = head(&store);
        let busy = relay_record(
            &store,
            &command(ANSWER_COMMAND, R5, 9, T7),
            "Ada Owner",
            &checked,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            REVISION,
        );
        assert_eq!(busy["status"], "failed", "{busy}");
        assert_eq!(busy["code"], "ledger-busy", "{busy}");
        assert_eq!(head(&store), before);
    }
}
