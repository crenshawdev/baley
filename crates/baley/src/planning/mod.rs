//! A phase's planning over the storage port (design 0005 sections 4 to 6,
//! PLN-R16, D-01, D-08 to D-12, D-15): what a planner order binds at issue,
//! read from the views; the planner order itself, issued on its phase's
//! stream with one live per phase and handed to a session once an approval
//! issued it; the plan checker's order, one live per phase too;
//! and `phase plan`, which chooses between drafting a changed commitment,
//! issuing an order and handing one over; then `plan submit`, which checks a
//! planner's plan against its order and the phase, over facts read from the
//! views, and records it as the phase's one draft, with its question set or,
//! when the plan gate is not `off`, with the plan checker's round-1 order;
//! the checker's return, whose round-1 result buys a blocker one planner
//! revision and otherwise opens the draft's set and previews it; the
//! re-check at submit, which finds a revision's first check and binds its
//! round 2;
//! `plan replace`, recorded as a submit is and approved into a new number;
//! and the owner's approval of that draft, in two steps, at the terminal
//! through `baley plan approve`; and a phase's approved plans read back with
//! their evidence maps and readiness, for `plan-read` and `baley plan show`.

/// The owner's approval of a plan draft: the first step and the recording.
pub mod approve;

/// The plan checker's return: the check of a checker order's findings and
/// the recording of its round-1 result.
pub mod check_return;

/// The plan checker's work order: its round-1 preparation and issue on
/// `phase/<n>`, and the pending lookup and refusal.
pub mod checker;

/// `baley plan approve`: the owner's approval of a plan draft at the
/// terminal.
pub mod command;

/// The facts a plan is judged on, read from the views.
pub mod facts;

/// What a planner order binds at issue, read outside any transaction.
pub mod inputs;

/// The planner's work order: its preparation, its issue on `phase/<n>`, the
/// pending lookup and refusal, and the hand-over of an approved order.
pub mod order;

/// `phase plan`: the check run before admission and the run after it.
pub mod phase_plan;

/// A phase's approved plans with their evidence maps, read back.
pub mod read;

/// The re-check at submit: a submit's place in its check lineage and the
/// round-2 binding it carries.
pub mod recheck;

/// `plan replace`: the check of a replacement, recorded as a submit is.
pub mod replace;

/// `plan submit`: the check run before admission and the recording after
/// it.
pub mod submit;

#[cfg(test)]
pub(crate) mod fixtures {
    //! The store, policies, catalog and plans the planning tests share,
    //! written out from design 0005, the phase's decisions and phase 8's
    //! check items, never produced by the code under test.

    use std::collections::BTreeSet;
    use std::path::Path;

    use baley_core::plan::{
        CommittedStory, CurrentTruth, PLAN_APPROVED, PLAN_APPROVED_VERSION, PLAN_KIND,
        PlanApproval, PlanDraft, PlanFacts, judge_plan, phase_stream,
    };
    use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, Schema, effective_policy};
    use baley_core::questions::answered_payload;
    use baley_core::questions::{QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, Subject};
    use baley_core::scope::{
        ChangeKind, PROJECT_STREAM, PhaseFields, ROADMAP_STREAM, SCOPE_SUBMITTED,
        SCOPE_SUBMITTED_VERSION, submitted_payload,
    };
    use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};
    use baley_store::{
        Decision, Hash, NewEvent, Observed, OutcomeKind, RetentionClass, StreamName, canonical_json,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};

    use crate::planning::phase_plan::PHASE_PLAN_COMMAND;
    use crate::refine::submit::tests::{t1, t2};
    use crate::scope::approve::{self, APPROVE_COMMAND, OrderRoute};
    use crate::scope::fixtures::*;
    use crate::scope::submit::{PlanningContinuation, submit_phase_edit};

    /// The request ids store C is built under, apart from every test's own.
    const DECLARE: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819203";
    const DECLARE_APPROVE: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819204";
    const REFINED: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819205";

    /// The question set every hand-appended approval names.
    pub(crate) const APPROVAL_SET: &str = "3c4d5e6f-7081-4a2b-8c3d-4e5f60718293";

    /// `truth` at version 1, as a refinement carries it.
    pub(crate) fn versioned(truth: Value) -> Value {
        let mut truth = truth;
        truth["version"] = json!(1);
        truth
    }

    /// A `story.refined` of `story` carrying `truths`, naming a made-up set.
    pub(crate) fn refined(story: u32, set: &str, truths: Vec<Value>) -> NewEvent {
        NewEvent {
            stream: StreamName(ROADMAP_STREAM.into()),
            type_name: STORY_REFINED.into(),
            type_version: STORY_REFINED_VERSION,
            git: None,
            payload: json!({"story": story, "set": set, "submission": "5c".repeat(32),
                "owner": "Ada Owner", "at": T2, "truths": truths}),
            attachments: Vec::new(),
        }
    }

    /// Store C: S1 approved; story 4, active in the backlog, declared by an
    /// approved `story-declare`; story 1 refined with T1 and T2, story 2
    /// with T1 and story 4 with T2, each at version 1. Story 3 holds no
    /// truth. Phase 1 commits `[1]`, phase 2 `[2]` and depends on 1.
    pub(crate) fn store_c() -> (tempfile::TempDir, SqliteStore) {
        let (dir, store) = crate::refine::order::tests::approved();
        let story = json!({"sentence": "Plans are approved by digest", "status": "active",
            "phase": null});
        let declared = submit_change(&store, ChangeKind::StoryDeclare, &story, DECLARE, T2)
            .expect("story 4 is submitted");
        let digest = receipt(&declared)["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .expect("the receipt names its digest");
        let judged = approve::judge(&store, &project(), digest).expect("story 4 is approvable");
        approve::record(
            &store,
            &command(APPROVE_COMMAND, DECLARE_APPROVE, 6, T2),
            Some(&judged),
            "Ada Owner",
        )
        .expect("story 4 is approved");
        append(
            &store,
            REFINED,
            T2,
            vec![
                refined(
                    1,
                    "1a1a1a1a-1a1a-4a1a-8a1a-1a1a1a1a1a1a",
                    vec![versioned(t1()), versioned(t2())],
                ),
                refined(
                    2,
                    "2b2b2b2b-2b2b-4b2b-8b2b-2b2b2b2b2b2b",
                    vec![versioned(t1())],
                ),
                refined(
                    4,
                    "4d4d4d4d-4d4d-4d4d-8d4d-4d4d4d4d4d4d",
                    vec![versioned(t2())],
                ),
            ],
        );
        (dir, store)
    }

    /// P0: the standard schema for Claude Code with no settings file.
    pub(crate) fn p0() -> EffectivePolicy {
        effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None)
            .expect("the defaults merge")
    }

    /// P2: P0 with a project file setting `planning.phase_capacity = 2`.
    pub(crate) fn p2() -> EffectivePolicy {
        let file = crate::settings::file(
            Path::new("/r/baley.toml"),
            b"[planning]\nphase_capacity = 2\n".to_vec(),
        );
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            None,
            Some(&file),
        )
        .expect("the capacity is read")
    }

    /// POFF: P0 with a project file setting the plan gate `off`.
    pub(crate) fn poff() -> EffectivePolicy {
        gated("off")
    }

    /// PB: P0 with a project file setting the plan gate `blocking`.
    pub(crate) fn pb() -> EffectivePolicy {
        gated("blocking")
    }

    /// PDF: P0 with a project file setting the plan gate `deferred`.
    pub(crate) fn pdf() -> EffectivePolicy {
        gated("deferred")
    }

    /// PAJ: P0 with a project file setting the plan gate `adjudicated`.
    pub(crate) fn paj() -> EffectivePolicy {
        gated("adjudicated")
    }

    /// PD: P0 with a project file setting the re-check scope `diff`.
    pub(crate) fn pd() -> EffectivePolicy {
        plan_trigger("recheck", "diff")
    }

    /// P0 with a project file `/r/baley.toml` setting the plan gate to
    /// `gate`.
    fn gated(gate: &str) -> EffectivePolicy {
        plan_trigger("gate", gate)
    }

    /// P0 with a project file `/r/baley.toml` setting the plan trigger's
    /// `name` to `value`.
    fn plan_trigger(name: &str, value: &str) -> EffectivePolicy {
        let file = crate::settings::file(
            Path::new("/r/baley.toml"),
            format!("[review.triggers.plan]\n{name} = \"{value}\"\n").into_bytes(),
        );
        effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            None,
            Some(&file),
        )
        .expect("the plan gate is read")
    }

    /// N0: no accepted name, at version 0.
    pub(crate) fn n0() -> AcceptedNames {
        AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        }
    }

    /// K, PL1's one task.
    pub(crate) fn k() -> Value {
        json!({"id": "k1", "title": "Record", "files": ["src/ledger.rs"],
            "action": "Record each step.", "verify": ["cargo test ledger"]})
    }

    /// E1, a unit check of story 1's `t1` at version 1.
    pub(crate) fn e1() -> Value {
        json!({"id": "e1", "story": 1, "truth": "t1", "truth_version": 1, "kind": "check",
            "spec": {"command": "cargo test stale",
                "expected": {"kind": "literal", "value": "stale-draft"},
                "test": {"file": "src/ledger.rs", "function": "stale_digest"},
                "setup": "a draft approved by a stale digest", "call": "approve the draft",
                "level": "unit", "boundary": "the approval judge", "fakes": []},
            "reason": "a stale digest approved"})
    }

    /// E2, an integration check of story 1's `t2` at version 1.
    pub(crate) fn e2() -> Value {
        json!({"id": "e2", "story": 1, "truth": "t2", "truth_version": 1, "kind": "check",
            "spec": {"command": "cargo test round",
                "expected": {"kind": "property", "value": "the same round"},
                "test": {"file": "src/ledger.rs", "function": "same_round"},
                "setup": "a set answered in two orders", "call": "read the round",
                "level": "integration", "boundary": "the answer recording and the round read",
                "fakes": ["the clock"]},
            "reason": "a round that depends on the answers' order"})
    }

    /// PL1: one plan of phase 1 serving story 1 with task K and checks E1
    /// and E2.
    pub(crate) fn pl1() -> Value {
        json!({"phase": 1, "stories": [1], "files": ["src/ledger.rs"], "directories": [],
            "goal": "Record every step", "context": "", "notes": "", "tasks": [k()],
            "evidence_map": [e1(), e2()], "questions": []})
    }

    /// PL2: PL1 asking `q1`.
    pub(crate) fn pl2() -> Value {
        let mut plan = pl1();
        plan["questions"] = json!([{"id": "q1", "question": "Which store holds the ledger?",
            "recommended": "SQLite", "depends_on": []}]);
        plan
    }

    /// PL3: a second plan of phase 1, PL1 with another goal and task id and
    /// PL1's items unchanged.
    pub(crate) fn pl3() -> Value {
        let mut plan = pl1();
        plan["goal"] = json!("Record again");
        plan["tasks"][0]["id"] = json!("k2");
        plan
    }

    /// PL2r: PL2 with goal `Record every step, revised`.
    pub(crate) fn pl2r() -> Value {
        let mut plan = pl2();
        plan["goal"] = json!("Record every step, revised");
        plan
    }

    /// PL2v: PL2 with E2 naming its truth at version 2.
    pub(crate) fn pl2v() -> Value {
        let mut plan = pl2();
        plan["evidence_map"][1]["truth_version"] = json!(2);
        plan
    }

    /// PL1r: PL1 with goal `Record every step, revised`.
    pub(crate) fn pl1r() -> Value {
        let mut plan = pl1();
        plan["goal"] = json!("Record every step, revised");
        plan
    }

    /// Submits `plan` answering order `work_order` as the replacement of plan
    /// `number` at `digest`, with no capacity, through the replacement's
    /// check and the submit's recording, as `plan-replace` runs them below
    /// the session, under `request_id` at `at`. Gives the answer.
    pub(crate) fn plan_replaced(
        store: &SqliteStore,
        plan: &Value,
        work_order: &str,
        number: u32,
        digest: Hash,
        request_id: &str,
        at: &str,
    ) -> Value {
        let replaces = baley_core::plan::Replaces {
            plan: number,
            digest,
        };
        let checked = match crate::planning::replace::check(
            store,
            &project(),
            plan,
            work_order,
            replaces,
            None,
        ) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.replace", request_id, 8, at);
        let on = caller()
            .with_work_order(work_order)
            .expect("an order id")
            .with_instructions(checked.evidence.clone())
            .expect("the plan's instructions");
        command.caller = Some(baley_store::Caller::Server(on));
        crate::scope::submit::answer(
            crate::planning::submit::record(store, &command, Some(&checked)),
            store,
        )
    }

    /// Records the `phase edit` of phase 1's stories to `stories` with its
    /// continuation naming order `work_order`, through `submit_phase_edit`
    /// under `request_id` at `at`, as `phase-plan` records a changed
    /// selection. Gives the draft's digest.
    pub(crate) fn edit(
        store: &SqliteStore,
        stories: &[u32],
        work_order: &str,
        request_id: &str,
        at: &str,
    ) -> Hash {
        let fields = PhaseFields {
            stories: Some(stories.to_vec()),
            ..PhaseFields::default()
        };
        let continuation = PlanningContinuation {
            stories,
            work_order,
            documents: &[],
            absences: &[],
        };
        submit_phase_edit(
            store,
            &command(PHASE_PLAN_COMMAND, request_id, 7, at),
            1,
            &fields,
            Some(continuation),
        )
        .expect("the edit is recorded")
    }

    /// The commitment `digest` approved through the approval's two steps as
    /// `Ada Owner` under `request_id` at `at`, its order routed under P0
    /// for Claude Code with N0. Gives the second step's answer.
    pub(crate) fn approve_commitment(
        store: &SqliteStore,
        digest: Hash,
        request_id: &str,
        at: &str,
    ) -> Value {
        let judged = approve::judge(store, &project(), digest).expect("the commitment is judged");
        let (policy, names) = (p0(), n0());
        approve::record_commitment(
            store,
            &command(APPROVE_COMMAND, request_id, 6, at),
            &judged,
            "Ada Owner",
            OrderRoute {
                policy: &policy,
                host: Host::ClaudeCode,
                names: &names,
            },
        )
    }

    /// The store in `dir` opened again, as a restart opens it.
    pub(crate) fn reopen(dir: &tempfile::TempDir) -> SqliteStore {
        crate::ledger::open::store(&dir.path().join("home"), T9, crate::ledger::open::options())
            .expect("the store opens again")
    }

    /// Makes phase 2 active: its plan 1 approved by hand under `request_id`.
    pub(crate) fn activate_phase_2(store: &SqliteStore, request_id: &str) {
        approve_by_hand(
            store,
            request_id,
            2,
            1,
            Hash([0x7e; 32]),
            &[2],
            1,
            &[],
            Vec::new(),
        );
    }

    /// Records PL2 as phase 1's draft with its set opened, under `request_id`
    /// at T4, then `q1` answered `Postgres`, not the recommendation, under
    /// `answer_request_id` at T5, as phase 8's set close leaves a rejected
    /// recommendation. Gives the draft's body as recorded and the set's id.
    pub(crate) fn pl2_rejected(
        store: &SqliteStore,
        request_id: &str,
        answer_request_id: &str,
    ) -> (Value, String) {
        let truth = |id: &str, trigger: &str, observer: &str, outcome: &str| CurrentTruth {
            id: id.into(),
            version: 1,
            trigger: trigger.into(),
            observer: observer.into(),
            outcome: outcome.into(),
            human_verify: None,
        };
        let facts = PlanFacts {
            stories: vec![CommittedStory {
                id: 1,
                truths: vec![
                    truth(
                        "t1",
                        "the owner approves a draft by a stale digest",
                        "the owner",
                        "stale-draft naming the first differing part",
                    ),
                    truth(
                        "t2",
                        "order the answers of a set are recorded in",
                        "a ledger reader",
                        "the same round",
                    ),
                ],
            }],
            approved: Vec::new(),
            size: 0,
            capacity: None,
        };
        let plan = judge_plan(&pl2(), &facts).expect("PL2 is accepted");
        let draft = PlanDraft::new(&plan);
        let mut set = String::new();
        store
            .transact(&command("test.plan", request_id, 8, T4), &mut |tx| {
                let body = tx.put_payload(draft.body(), RetentionClass::Record)?;
                tx.append(NewEvent {
                    stream: StreamName(PROJECT_STREAM.into()),
                    type_name: SCOPE_SUBMITTED.into(),
                    type_version: SCOPE_SUBMITTED_VERSION,
                    git: None,
                    payload: submitted_payload(draft.digest(), PLAN_KIND, draft.target(), &body),
                    attachments: vec![body],
                })?;
                let subject = Subject::Plan {
                    phase: 1,
                    plan_digest: draft.digest(),
                };
                set = crate::questions::record::open(tx, plan.questions(), &subject, T4)?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .expect("PL2 is drafted");
        append(
            store,
            answer_request_id,
            T5,
            vec![NewEvent {
                stream: StreamName(phase_stream(1)),
                type_name: QUESTION_ANSWERED.into(),
                type_version: QUESTION_ANSWERED_VERSION,
                git: None,
                payload: answered_payload(&set, "q1", 1, "Postgres", false, "Ada Owner", T5),
                attachments: Vec::new(),
            }],
        );
        let body = serde_json::from_slice(draft.body()).expect("a draft body is JSON");
        (body, set)
    }

    /// Appends, under `request_id` in its own transaction, a `plan.approved`
    /// of plan `plan` of phase `phase` at `digest`, serving `stories` with
    /// `size` tasks and evidence map `map`, as an approval would leave it,
    /// with a body stored as a `record` payload in the same transaction so
    /// its reference is held. Every `events` entry is appended after it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn approve_by_hand(
        store: &SqliteStore,
        request_id: &str,
        phase: u32,
        plan: u32,
        digest: Hash,
        stories: &[u32],
        size: usize,
        map: &[Value],
        events: Vec<NewEvent>,
    ) {
        let body = canonical_json(&json!({"phase": phase, "plan": plan, "evidence_map": map}))
            .expect("a plan body is canonical");
        let command = command("test.approve", request_id, 9, T5);
        store
            .transact(&command, &mut |tx| {
                let body = tx.put_payload(&body, RetentionClass::Record)?;
                let approval = PlanApproval {
                    phase,
                    plan,
                    digest,
                    stories,
                    size,
                    suite: None,
                    question_set: APPROVAL_SET,
                    evidence_map: map,
                    body: &body,
                    owner: "Ada Owner",
                    at: T5,
                };
                tx.append(NewEvent {
                    stream: StreamName(phase_stream(phase)),
                    type_name: PLAN_APPROVED.into(),
                    type_version: PLAN_APPROVED_VERSION,
                    git: None,
                    payload: approval.payload(),
                    attachments: vec![body],
                })?;
                for event in events.clone() {
                    tx.append(event)?;
                }
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .expect("the approval is appended");
    }
}
