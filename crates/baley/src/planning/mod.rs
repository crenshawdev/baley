//! A phase's planning over the storage port (design 0005 sections 4 and 5,
//! D-11, D-12, D-15): what a planner order binds at issue, read from the
//! views, and the planner order itself, issued on its phase's stream with
//! one live per phase.

/// What a planner order binds at issue, read outside any transaction.
pub mod inputs;

/// The planner's work order: its preparation, its issue on `phase/<n>`, and
/// the pending lookup and refusal.
pub mod order;

#[cfg(test)]
pub(crate) mod fixtures {
    //! The store, policies, catalog and plans the planning tests share,
    //! written out from design 0005, the phase's decisions and phase 8's
    //! check items, never produced by the code under test.

    use std::collections::BTreeSet;
    use std::path::Path;

    use baley_core::plan::{PLAN_APPROVED, PLAN_APPROVED_VERSION, PlanApproval, phase_stream};
    use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, Schema, effective_policy};
    use baley_core::scope::{ChangeKind, ROADMAP_STREAM};
    use baley_core::truth::{STORY_REFINED, STORY_REFINED_VERSION};
    use baley_store::{
        Decision, Hash, NewEvent, Observed, OutcomeKind, RetentionClass, StreamName, canonical_json,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};

    use crate::refine::submit::tests::{t1, t2};
    use crate::scope::approve::{self, APPROVE_COMMAND};
    use crate::scope::fixtures::*;

    /// The request ids store C is built under, apart from every test's own.
    const DECLARE: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819203";
    const DECLARE_APPROVE: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819204";
    const REFINED: &str = "4d5e6f70-8192-4a3b-9c4d-5e6f70819205";

    /// The question set every hand-appended approval names.
    pub(crate) const APPROVAL_SET: &str = "3c4d5e6f-7081-4a2b-8c3d-4e5f60718293";

    /// `truth` at version 1, as a refinement carries it.
    fn versioned(truth: Value) -> Value {
        let mut truth = truth;
        truth["version"] = json!(1);
        truth
    }

    /// A `story.refined` of `story` carrying `truths`, naming a made-up set.
    fn refined(story: u32, set: &str, truths: Vec<Value>) -> NewEvent {
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
    #[allow(dead_code)]
    pub(crate) fn pl3() -> Value {
        let mut plan = pl1();
        plan["goal"] = json!("Record again");
        plan["tasks"][0]["id"] = json!("k2");
        plan
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
