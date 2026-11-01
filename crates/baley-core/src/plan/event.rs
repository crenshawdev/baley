//! The four plan events: their types, their registration, their stream and
//! their payloads (design 0005 section 6, design 0001 Streams).
//!
//! Every plan event goes on its phase's stream, `phase/<n>`, the stream a
//! plan draft's question set takes. Owner and time are payload members, as
//! `scope.approved` and `story.refined` carry them.

use baley_store::{Hash, PayloadRef};
use serde_json::{Value, json};

use crate::registry::{Registry, RegistryError};

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

/// Registers the four plan types at version 1 with no upcasters.
pub fn register_plan_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(PLAN_APPROVED, PLAN_APPROVED_VERSION, [])?;
    registry.register(PLAN_REPLACED, PLAN_REPLACED_VERSION, [])?;
    registry.register(PLAN_CONTINUATION, PLAN_CONTINUATION_VERSION, [])?;
    registry.register(
        PLAN_CONTINUATION_FOLLOWED,
        PLAN_CONTINUATION_FOLLOWED_VERSION,
        [],
    )
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
}
