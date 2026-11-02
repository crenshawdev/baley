//! The six plan events: their types, their registration, their stream and
//! their payloads (design 0005 section 6, design 0001 Streams).
//!
//! Every one of the six goes on its phase's stream, `phase/<n>`, the stream
//! a plan draft's question set takes. Owner and time are payload members, as
//! `scope.approved` and `story.refined` carry them; the two check events
//! record neither, which their envelope carries.

use baley_store::{Hash, PayloadRef};
use serde_json::{Value, json};

use crate::policy::RecheckScope;
use crate::registry::{Registry, RegistryError};
use crate::work_order::OrderInput;

/// A plan approved, on its phase's stream: `{phase, plan, digest, stories,
/// size, suite, question_set, evidence_map, body, owner, at}`.
///
/// The evidence map is inline so its view rebuilds from events alone and
/// survives a purge of the draft's body; the plan's text is only the
/// draft's `record` reference, since plan text is an attachment (D-06).
pub const PLAN_APPROVED: &str = "plan.approved";
/// The current `plan.approved` payload version.
pub const PLAN_APPROVED_VERSION: u32 = 1;
/// A plan replaced, on its phase's stream: `{phase, plan, old_digest,
/// new_digest, owner, at}`, `plan` the old number (D-07).
pub const PLAN_REPLACED: &str = "plan.replaced";
/// The current `plan.replaced` payload version.
pub const PLAN_REPLACED_VERSION: u32 = 1;
/// A planning continuation, on its phase's stream: `{phase, stories, edit,
/// work_order}`, recorded with the `phase edit` draft it waits on (D-09).
pub const PLAN_CONTINUATION: &str = "plan.continuation";
/// The current `plan.continuation` payload version.
pub const PLAN_CONTINUATION_VERSION: u32 = 1;
/// A continuation's planner order handed to the session, on its phase's
/// stream: `{phase, edit, work_order}` (D-09).
pub const PLAN_CONTINUATION_FOLLOWED: &str = "plan.continuation_followed";
/// The current `plan.continuation_followed` payload version.
pub const PLAN_CONTINUATION_FOLLOWED_VERSION: u32 = 1;
/// The checker's round 2 issued, on its phase's stream `phase/<n>`:
/// `{phase, work_order, round, previous_check, original_plan_digest,
/// plan_digest, truth_versions, scope, scope_policy_version, material}`,
/// `round` always 2 (design 0005 section 6, D-09).
pub const PLAN_CHECK_ISSUED: &str = "plan.check_issued";
/// The current `plan.check_issued` payload version.
pub const PLAN_CHECK_ISSUED_VERSION: u32 = 1;
/// A checker round's result, on its phase's stream `phase/<n>`: `{phase,
/// plan_digest, findings, round, work_order, scope, previous_check,
/// truth_versions, material, scope_policy_version, revision}`,
/// `previous_check` only in round 2 and `revision` the planner revision
/// order a round-1 blocker issued, or null (design 0005 section 6, D-02).
pub const PLAN_CHECKED: &str = "plan.checked";
/// The current `plan.checked` payload version.
pub const PLAN_CHECKED_VERSION: u32 = 1;

/// Registers the six plan types at version 1 with no upcasters.
pub fn register_plan_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(PLAN_APPROVED, PLAN_APPROVED_VERSION, [])?;
    registry.register(PLAN_REPLACED, PLAN_REPLACED_VERSION, [])?;
    registry.register(PLAN_CONTINUATION, PLAN_CONTINUATION_VERSION, [])?;
    registry.register(
        PLAN_CONTINUATION_FOLLOWED,
        PLAN_CONTINUATION_FOLLOWED_VERSION,
        [],
    )?;
    registry.register(PLAN_CHECK_ISSUED, PLAN_CHECK_ISSUED_VERSION, [])?;
    registry.register(PLAN_CHECKED, PLAN_CHECKED_VERSION, [])
}

/// The stream of phase `phase`: `phase/<n>`, spelled as a plan draft's
/// question set stream is.
pub fn phase_stream(phase: u32) -> String {
    format!("phase/{phase}")
}

/// What one plan approval records.
#[derive(Debug, Clone, Copy)]
pub struct PlanApproval<'a> {
    /// The phase the plan is for.
    pub phase: u32,
    /// The number the plan is given.
    pub plan: u32,
    /// The approved draft's digest.
    pub digest: Hash,
    /// The stories the plan serves, in its order.
    pub stories: &'a [u32],
    /// The plan's size in tasks.
    pub size: usize,
    /// The project's test command, or `None` when none is set (D-14).
    pub suite: Option<&'a str>,
    /// The id of the draft's question set.
    pub question_set: &'a str,
    /// The evidence items exactly as submitted.
    pub evidence_map: &'a [Value],
    /// The draft's `record` payload reference, never its text.
    pub body: &'a PayloadRef,
    /// The owner's name.
    pub owner: &'a str,
    /// The time of the approval.
    pub at: &'a str,
}

impl PlanApproval<'_> {
    /// The payload of `plan.approved`. An absent suite is written as null.
    pub fn payload(&self) -> Value {
        json!({
            "phase": self.phase,
            "plan": self.plan,
            "digest": self.digest.to_hex(),
            "stories": self.stories,
            "size": self.size,
            "suite": self.suite,
            "question_set": self.question_set,
            "evidence_map": self.evidence_map,
            "body": self.body.to_value(),
            "owner": self.owner,
            "at": self.at,
        })
    }
}

/// The payload of `plan.replaced`: plan `plan` of phase `phase`, approved
/// as `old_digest`, replaced by the draft `new_digest` by `owner` at `at`.
pub fn replaced_payload(
    phase: u32,
    plan: u32,
    old_digest: Hash,
    new_digest: Hash,
    owner: &str,
    at: &str,
) -> Value {
    json!({
        "phase": phase,
        "plan": plan,
        "old_digest": old_digest.to_hex(),
        "new_digest": new_digest.to_hex(),
        "owner": owner,
        "at": at,
    })
}

/// The payload of `plan.continuation`: phase `phase` waits on the `phase
/// edit` draft `edit` to commit `stories` in order, and that edit's
/// approval issues the planner order `work_order`, fixed when the edit is
/// drafted so a rerun finds the same order.
pub fn continuation_payload(phase: u32, stories: &[u32], edit: Hash, work_order: &str) -> Value {
    json!({
        "phase": phase,
        "stories": stories,
        "edit": edit.to_hex(),
        "work_order": work_order,
    })
}

/// The payload of `plan.continuation_followed`: the planner order
/// `work_order` the approved edit `edit` issued was handed to the session.
pub fn continuation_followed_payload(phase: u32, edit: Hash, work_order: &str) -> Value {
    json!({
        "phase": phase,
        "edit": edit.to_hex(),
        "work_order": work_order,
    })
}

/// The round-1 result a re-check answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviousCheck {
    /// The sequence of the round-1 `plan.checked`.
    pub seq: u64,
    /// The digest of the draft round 1 checked.
    pub plan_digest: Hash,
}

impl PreviousCheck {
    fn to_value(self) -> Value {
        json!({"seq": self.seq, "plan_digest": self.plan_digest.to_hex()})
    }
}

/// One truth a check is bound to, at the version it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruthVersion {
    /// The story holding the truth.
    pub story: u32,
    /// The truth's id within its story.
    pub truth: String,
    /// The truth's version.
    pub version: u32,
}

fn truth_versions(truths: &[TruthVersion]) -> Vec<Value> {
    truths
        .iter()
        .map(|t| json!({"story": t.story, "truth": t.truth, "version": t.version}))
        .collect()
}

// Written as an order's inputs are, so a check's material reads the same.
fn material(inputs: &[OrderInput]) -> Vec<Value> {
    inputs
        .iter()
        .map(|input| json!({"name": input.name, "body": input.body.to_value()}))
        .collect()
}

/// What the issue of the checker's round 2 records: the binding a retry and
/// the result copy (D-09).
#[derive(Debug, Clone, Copy)]
pub struct CheckIssue<'a> {
    /// The phase the plan is for.
    pub phase: u32,
    /// The round-2 checker order.
    pub work_order: &'a str,
    /// The round-1 result this round re-checks.
    pub previous_check: PreviousCheck,
    /// The digest of the draft round 1 checked.
    pub original_plan_digest: Hash,
    /// The digest of the submitted revision.
    pub plan_digest: Hash,
    /// The truths bound at round 1, unchanged.
    pub truth_versions: &'a [TruthVersion],
    /// The resolved reading scope.
    pub scope: RecheckScope,
    /// The policy version that selected the scope.
    pub scope_policy_version: u64,
    /// The retained material, in order.
    pub material: &'a [OrderInput],
}

impl CheckIssue<'_> {
    /// The payload of `plan.check_issued`, `round` always 2.
    pub fn payload(&self) -> Value {
        json!({
            "phase": self.phase,
            "work_order": self.work_order,
            "round": 2,
            "previous_check": self.previous_check.to_value(),
            "original_plan_digest": self.original_plan_digest.to_hex(),
            "plan_digest": self.plan_digest.to_hex(),
            "truth_versions": truth_versions(self.truth_versions),
            "scope": self.scope.name(),
            "scope_policy_version": self.scope_policy_version,
            "material": material(self.material),
        })
    }
}

/// What one checker round's result records. The round is 2 exactly when a
/// previous check is named, so a round-1 result can never name one.
#[derive(Debug, Clone, Copy)]
pub struct CheckResult<'a> {
    /// The phase the plan is for.
    pub phase: u32,
    /// The digest of the draft checked.
    pub plan_digest: Hash,
    /// The findings exactly as the checker returned them.
    pub findings: &'a [Value],
    /// The checker order this result answers.
    pub work_order: &'a str,
    /// The scope the order was issued with.
    pub scope: RecheckScope,
    /// The round-1 result, for round 2 only.
    pub previous_check: Option<PreviousCheck>,
    /// The truths bound to the order.
    pub truth_versions: &'a [TruthVersion],
    /// The material bound to the order, in order.
    pub material: &'a [OrderInput],
    /// The policy version that selected the scope.
    pub scope_policy_version: u64,
    /// The planner revision order a round-1 blocker issued, or `None`.
    pub revision: Option<&'a str>,
}

impl CheckResult<'_> {
    /// The payload of `plan.checked`: `previous_check` present for round 2
    /// and absent for round 1, `revision` written as null when none.
    pub fn payload(&self) -> Value {
        let mut payload = json!({
            "phase": self.phase,
            "plan_digest": self.plan_digest.to_hex(),
            "findings": self.findings,
            "round": if self.previous_check.is_some() { 2 } else { 1 },
            "work_order": self.work_order,
            "scope": self.scope.name(),
            "truth_versions": truth_versions(self.truth_versions),
            "material": material(self.material),
            "scope_policy_version": self.scope_policy_version,
            "revision": self.revision,
        });
        if let Some(previous) = self.previous_check {
            payload["previous_check"] = previous.to_value();
        }
        payload
    }
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0005 section 6, D-06,
    //! D-07, D-14 and the plan's test data, never from running this code.

    use baley_store::RetentionClass;
    use serde_json::json;

    use super::*;
    use crate::plan::fixtures::c1;
    use crate::questions::{Subject, register_question_events, set_stream};
    use crate::scope::register_scope_events;
    use crate::truth::register_truth_events;

    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const T: &str = "2026-10-31T09:00:00Z";

    /// R: a `record` reference with hash Y and 1,024 bytes.
    fn r() -> PayloadRef {
        PayloadRef {
            hash: Hash([0x6d; 32]),
            bytes: 1_024,
            class: RetentionClass::Record,
        }
    }

    #[test]
    fn a_plan_approval_payload_without_its_map_or_with_its_body_inlined_is_caught() {
        let map = [c1()];
        let body = r();
        let mut approval = PlanApproval {
            phase: 1,
            plan: 2,
            digest: Hash([0x5c; 32]),
            stories: &[1],
            size: 1,
            suite: Some("cargo test"),
            question_set: S,
            evidence_map: &map,
            body: &body,
            owner: "Ada Owner",
            at: T,
        };
        let payload = approval.payload();
        let mut members: Vec<&str> = payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        assert_eq!(
            members,
            [
                "at",
                "body",
                "digest",
                "evidence_map",
                "owner",
                "phase",
                "plan",
                "question_set",
                "size",
                "stories",
                "suite"
            ]
        );
        assert_eq!(
            payload,
            json!({"phase": 1, "plan": 2, "digest": "5c".repeat(32), "stories": [1],
                "size": 1, "suite": "cargo test", "question_set": S,
                "evidence_map": [c1()],
                "body": {"payload": "6d".repeat(32), "bytes": 1_024, "class": "record"},
                "owner": "Ada Owner", "at": T})
        );
        assert!(payload.get("tasks").is_none());
        approval.suite = None;
        assert_eq!(approval.payload()["suite"], Value::Null);
    }

    #[test]
    fn a_replacement_payload_missing_a_digest_or_the_old_number_is_caught() {
        assert_eq!(
            replaced_payload(1, 1, Hash([0x5c; 32]), Hash([0x6d; 32]), "Ada Owner", T),
            json!({"phase": 1, "plan": 1, "old_digest": "5c".repeat(32),
                "new_digest": "6d".repeat(32), "owner": "Ada Owner", "at": T})
        );
    }

    #[test]
    fn a_phase_stream_spelled_apart_from_its_question_sets_or_an_event_left_unregistered_is_caught()
    {
        let set = Subject::Plan {
            phase: 7,
            plan_digest: Hash([0x5c; 32]),
        };
        assert_eq!(phase_stream(7), "phase/7");
        assert_eq!(phase_stream(7), set_stream(&set));

        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        register_question_events(&mut registry).unwrap();
        register_truth_events(&mut registry).unwrap();
        register_plan_events(&mut registry).unwrap();
        for name in [
            "plan.approved",
            "plan.replaced",
            "plan.continuation",
            "plan.continuation_followed",
        ] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
        }
        assert!(register_plan_events(&mut registry).is_err());
    }

    const W1: &str = "0b1c2d3e-4f50-4a61-8b72-9c83d4e5f601";
    const W2: &str = "1c2d3e4f-5061-4b72-8c83-9d94e5f60712";
    const V: &str = "2d3e4f50-6172-4c83-9d94-aea5f6071823";

    /// TV: story 1's `t1` and `t2`, each at version 1.
    fn tv() -> Vec<TruthVersion> {
        ["t1", "t2"]
            .into_iter()
            .map(|truth| TruthVersion {
                story: 1,
                truth: truth.into(),
                version: 1,
            })
            .collect()
    }

    fn tv_json() -> Value {
        json!([{"story": 1, "truth": "t1", "version": 1},
            {"story": 1, "truth": "t2", "version": 1}])
    }

    fn record(byte: u8, bytes: u64) -> PayloadRef {
        PayloadRef {
            hash: Hash([byte; 32]),
            bytes,
            class: RetentionClass::Record,
        }
    }

    /// M: the plan at X1 with 512 bytes, the truths at `7e` with 256.
    fn m() -> Vec<OrderInput> {
        vec![
            OrderInput {
                name: "plan".into(),
                body: record(0x5c, 512),
            },
            OrderInput {
                name: "truths".into(),
                body: record(0x7e, 256),
            },
        ]
    }

    fn m_json() -> Value {
        json!([{"name": "plan", "body": {"payload": "5c".repeat(32), "bytes": 512,
                "class": "record"}},
            {"name": "truths", "body": {"payload": "7e".repeat(32), "bytes": 256,
                "class": "record"}}])
    }

    fn fb() -> Value {
        json!({"severity": "blocker", "location": "tasks[0]",
            "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    fn fw() -> Value {
        json!({"severity": "warning", "location": "tasks[0].verify[0]",
            "claim": "The verify runs every ledger test", "fix": "Name the round test"})
    }

    #[test]
    fn a_check_issued_payload_missing_its_binding_or_carrying_a_round_but_two_is_caught() {
        let (truths, material) = (tv(), m());
        let issue = CheckIssue {
            phase: 1,
            work_order: W2,
            previous_check: PreviousCheck {
                seq: 41,
                plan_digest: Hash([0x5c; 32]),
            },
            original_plan_digest: Hash([0x5c; 32]),
            plan_digest: Hash([0x6d; 32]),
            truth_versions: &truths,
            scope: RecheckScope::Diff,
            scope_policy_version: 5,
            material: &material,
        };
        assert_eq!(
            issue.payload(),
            json!({"phase": 1, "work_order": W2, "round": 2,
                "previous_check": {"seq": 41, "plan_digest": "5c".repeat(32)},
                "original_plan_digest": "5c".repeat(32), "plan_digest": "6d".repeat(32),
                "truth_versions": tv_json(), "scope": "diff", "scope_policy_version": 5,
                "material": m_json()})
        );
    }

    #[test]
    fn a_check_result_carrying_a_previous_check_in_round_one_or_dropping_its_revision_is_caught() {
        let (truths, material) = (tv(), m());
        let (first_findings, second_findings) = ([fb()], [fw()]);
        let first = CheckResult {
            phase: 1,
            plan_digest: Hash([0x5c; 32]),
            findings: &first_findings,
            work_order: W1,
            scope: RecheckScope::Full,
            previous_check: None,
            truth_versions: &truths,
            material: &material,
            scope_policy_version: 5,
            revision: Some(V),
        }
        .payload();
        let mut members: Vec<&str> = first
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        assert_eq!(
            members,
            [
                "findings",
                "material",
                "phase",
                "plan_digest",
                "revision",
                "round",
                "scope",
                "scope_policy_version",
                "truth_versions",
                "work_order"
            ]
        );
        assert_eq!(first["revision"], json!(V));
        assert_eq!(first["round"], json!(1));
        assert!(first.get("previous_check").is_none());

        let second = CheckResult {
            phase: 1,
            plan_digest: Hash([0x6d; 32]),
            findings: &second_findings,
            work_order: W2,
            scope: RecheckScope::Diff,
            previous_check: Some(PreviousCheck {
                seq: 41,
                plan_digest: Hash([0x5c; 32]),
            }),
            truth_versions: &truths,
            material: &material,
            scope_policy_version: 5,
            revision: None,
        }
        .payload();
        assert_eq!(
            second["previous_check"],
            json!({"seq": 41, "plan_digest": "5c".repeat(32)})
        );
        assert_eq!(second["revision"], Value::Null);
        assert_eq!(second["round"], json!(2));
    }

    #[test]
    fn a_check_event_left_unregistered_or_registered_twice_is_caught() {
        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        register_question_events(&mut registry).unwrap();
        register_truth_events(&mut registry).unwrap();
        register_plan_events(&mut registry).unwrap();
        for name in ["plan.check_issued", "plan.checked"] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
        }
        assert!(register_plan_events(&mut registry).is_err());
    }
}
