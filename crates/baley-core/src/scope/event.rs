//! The scope events: their types, their registration, their streams and
//! their payloads (design 0004 section 6, design 0001 Streams and
//! Payloads).
//!
//! No payload holds a caller or a time of its own, except an approval's
//! owner and time: the envelope carries both, hashed with the event.

use baley_store::{Hash, PayloadRef};
use serde_json::{Value, json};

use super::submission::{Description, StoryStatus};
use crate::registry::{Registry, RegistryError};

/// The stream a project's submissions, description and approvals go on.
pub const PROJECT_STREAM: &str = "project";
/// The stream a project's stories and phases go on.
pub const ROADMAP_STREAM: &str = "roadmap";

/// A scope submission held as a draft: `{digest, kind, body}`, the body a
/// reference to a `record` payload holding the submission's canonical JSON.
/// Accepted scope never changes on it.
pub const SCOPE_SUBMITTED: &str = "scope.submitted";
/// The current `scope.submitted` payload version.
pub const SCOPE_SUBMITTED_VERSION: u32 = 1;
/// The project description an approval accepted:
/// `{what, core_value, constraints, submission}`.
pub const PROJECT_DESCRIBED: &str = "project.described";
/// The current `project.described` payload version.
pub const PROJECT_DESCRIBED_VERSION: u32 = 1;
/// The owner's approval of a submission by digest:
/// `{submission, kind, owner, at}`.
pub const SCOPE_APPROVED: &str = "scope.approved";
/// The current `scope.approved` payload version.
pub const SCOPE_APPROVED_VERSION: u32 = 1;
/// One story with its Baley-assigned id:
/// `{id, sentence, status, phase, submission}`.
pub const STORY_DECLARED: &str = "story.declared";
/// The current `story.declared` payload version.
pub const STORY_DECLARED_VERSION: u32 = 1;
/// One phase with its Baley-assigned number:
/// `{number, name, goal, detail, depends_on, stories, submission}`.
pub const PHASE_DECLARED: &str = "phase.declared";
/// The current `phase.declared` payload version.
pub const PHASE_DECLARED_VERSION: u32 = 1;
/// The full order of phase numbers after a change: `{order, submission}`.
pub const PHASE_REORDERED: &str = "phase.reordered";
/// The current `phase.reordered` payload version.
pub const PHASE_REORDERED_VERSION: u32 = 1;

/// The change kind of a submission that starts a project.
pub const START_KIND: &str = "start";

/// Registers the six scope types at version 1, with no upcasters.
pub fn register_scope_events(registry: &mut Registry) -> Result<(), RegistryError> {
    for (name, version) in [
        (SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION),
        (PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION),
        (SCOPE_APPROVED, SCOPE_APPROVED_VERSION),
        (STORY_DECLARED, STORY_DECLARED_VERSION),
        (PHASE_DECLARED, PHASE_DECLARED_VERSION),
        (PHASE_REORDERED, PHASE_REORDERED_VERSION),
    ] {
        registry.register(name, version, [])?;
    }
    Ok(())
}

/// The payload of `scope.submitted`: the submission's digest, its change
/// kind and the reference to its stored body, never the body's text.
pub fn submitted_payload(digest: Hash, kind: &str, body: &PayloadRef) -> Value {
    json!({
        "digest": digest.to_hex(),
        "kind": kind,
        "body": body.to_value(),
    })
}

/// The payload of `project.described` for `description`, accepted from the
/// submission with digest `submission`.
pub fn described_payload(description: &Description, submission: Hash) -> Value {
    json!({
        "what": description.what,
        "core_value": description.core_value,
        "constraints": description.constraints,
        "submission": submission.to_hex(),
    })
}

/// The payload of `scope.approved`: the approved digest, its change kind,
/// the owner's name and Baley's time of the approval.
pub fn approved_payload(submission: Hash, kind: &str, owner: &str, at: &str) -> Value {
    json!({
        "submission": submission.to_hex(),
        "kind": kind,
        "owner": owner,
        "at": at,
    })
}

/// The payload of `story.declared`: story `id` in `phase`, or in the
/// backlog when `phase` is `None`.
pub fn story_declared_payload(
    id: u32,
    sentence: &str,
    status: StoryStatus,
    phase: Option<u32>,
    submission: Hash,
) -> Value {
    json!({
        "id": id,
        "sentence": sentence,
        "status": status.as_str(),
        "phase": phase,
        "submission": submission.to_hex(),
    })
}

/// The payload of `phase.declared`: `detail` is the reference to a `record`
/// payload holding the detail text, and dependencies and stories are phase
/// numbers and story ids, never labels.
pub fn phase_declared_payload(
    number: u32,
    name: &str,
    goal: &str,
    detail: &PayloadRef,
    depends_on: &[u32],
    stories: &[u32],
    submission: Hash,
) -> Value {
    json!({
        "number": number,
        "name": name,
        "goal": goal,
        "detail": detail.to_value(),
        "depends_on": depends_on,
        "stories": stories,
        "submission": submission.to_hex(),
    })
}

/// The payload of `phase.reordered`: the full order of phase numbers.
pub fn phase_reordered_payload(order: &[u32], submission: Hash) -> Value {
    json!({
        "order": order,
        "submission": submission.to_hex(),
    })
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from design 0004 section 6's field
    //! names, never from running this code.

    use baley_store::{EventSchema, RetentionClass};

    use super::*;

    const NAMES: [&str; 6] = [
        "scope.submitted",
        "project.described",
        "scope.approved",
        "story.declared",
        "phase.declared",
        "phase.reordered",
    ];

    fn digest() -> Hash {
        Hash([0x5c; 32])
    }

    fn reference(bytes: u64) -> PayloadRef {
        PayloadRef {
            hash: Hash([0xab; 32]),
            bytes,
            class: RetentionClass::Record,
        }
    }

    #[test]
    fn a_scope_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_scope_events(&mut registry).unwrap();
        for name in NAMES {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert!(register_scope_events(&mut registry).is_err());
    }

    #[test]
    fn a_submission_event_carrying_its_body_or_missing_its_digest_is_caught() {
        assert_eq!(
            submitted_payload(digest(), START_KIND, &reference(300)),
            json!({"digest": "5c".repeat(32), "kind": "start",
                "body": {"payload": "ab".repeat(32), "bytes": 300, "class": "record"}})
        );
    }

    #[test]
    fn an_approval_payload_missing_its_owner_time_or_kind_is_caught() {
        assert_eq!(
            approved_payload(digest(), START_KIND, "Ada Owner", "2026-10-13T09:00:02Z"),
            json!({"submission": "5c".repeat(32), "kind": "start", "owner": "Ada Owner",
                "at": "2026-10-13T09:00:02Z"})
        );
    }

    #[test]
    fn a_declaration_payload_with_inline_detail_or_story_labels_is_caught() {
        let detail = reference(18);
        assert_eq!(
            phase_declared_payload(
                2,
                "Approval",
                "The owner approves",
                &detail,
                &[1],
                &[2],
                digest()
            ),
            json!({"number": 2, "name": "Approval", "goal": "The owner approves",
                "detail": {"payload": "ab".repeat(32), "bytes": 18, "class": "record"},
                "depends_on": [1], "stories": [2], "submission": "5c".repeat(32)})
        );
        assert_eq!(
            story_declared_payload(
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
                digest()
            ),
            json!({"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                "phase": null, "submission": "5c".repeat(32)})
        );
        let description = Description {
            what: "A ledger for agent work".into(),
            core_value: "Nothing moves on an unproven claim".into(),
            constraints: vec!["One binary".into()],
        };
        assert_eq!(
            described_payload(&description, digest()),
            json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"], "submission": "5c".repeat(32)})
        );
        assert_eq!(
            phase_reordered_payload(&[1, 2], digest()),
            json!({"order": [1, 2], "submission": "5c".repeat(32)})
        );
    }
}
