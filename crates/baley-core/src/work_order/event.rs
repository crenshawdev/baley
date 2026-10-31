//! The work-order events: their types, their registration, their streams
//! and their payloads (design 0001 Streams and Payloads, design 0012).
//!
//! The start's and a story's orders go on `roadmap`, and a phase planner's
//! on its phase's stream, `phase/<n>`. An order's failure goes on the
//! stream its issue went on. No payload holds a caller or a time: the
//! envelope carries both, hashed with the event.

use std::borrow::Cow;

use baley_store::{InstructionEvidence, PayloadRef};
use serde_json::{Value, json};

use crate::plan::phase_stream;
use crate::policy::{Role, Rung};
use crate::registry::{Registry, RegistryError};
use crate::scope::ROADMAP_STREAM;

/// One work order issued, on its subject's stream (`roadmap` for the start
/// and a story's analyzer orders, `phase/<n>` for a phase's planner orders):
/// `{id, request, parent, role, subject, attempt, route, agent,
/// instructions, inputs}`. `request` is the id of its chain's first order,
/// `parent` the failed order it retries or null, `instructions` a list of
/// `{identity, version, hash}` and `inputs` a list of `{name, body}`, each
/// body a reference to a `record` payload.
pub const WORK_ORDER_ISSUED: &str = "work_order.issued";
/// The current `work_order.issued` payload version.
pub const WORK_ORDER_ISSUED_VERSION: u32 = 1;
/// One work order recorded as failed, on the stream its issue went on:
/// `{id, reason}`.
pub const WORK_ORDER_FAILED: &str = "work_order.failed";
/// The current `work_order.failed` payload version.
pub const WORK_ORDER_FAILED_VERSION: u32 = 1;

/// The subject of the start planner's order.
pub const START_SUBJECT: &str = "start";

/// Registers the two work-order types at version 1 with no upcasters.
pub fn register_work_order_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION, [])?;
    registry.register(WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION, [])
}

/// What an analyzer order's subject starts with, before its story's id.
const STORY_SUBJECT_PREFIX: &str = "story/";

/// The subject of story `story`'s analyzer orders, `story/<id>`. Both the
/// questions order and the drafting order carry it, so one look at the
/// `live` index finds either one pending (D-09).
pub fn story_subject(story: u32) -> String {
    format!("{STORY_SUBJECT_PREFIX}{story}")
}

/// The story an analyzer order's subject names, or none for any other
/// text. Only the spelling [`story_subject`] writes is read, decimal with
/// no sign or leading zero, so one story never has two subjects.
pub fn subject_story(subject: &str) -> Option<u32> {
    canonical_number(subject.strip_prefix(STORY_SUBJECT_PREFIX)?)
}

/// What a planner order's subject starts with, before its phase's number.
const PHASE_SUBJECT_PREFIX: &str = "phase/";

/// The subject of phase `phase`'s planner orders, `phase/<n>`. The planning
/// and revision orders both carry it, so one look at the `live` index finds
/// either one pending (D-11).
pub fn phase_subject(phase: u32) -> String {
    format!("{PHASE_SUBJECT_PREFIX}{phase}")
}

/// The phase a planner order's subject names, or none for any other text.
/// Only the spelling [`phase_subject`] writes is read, decimal with no sign
/// or leading zero, so one phase never has two subjects.
pub fn subject_phase(subject: &str) -> Option<u32> {
    canonical_number(subject.strip_prefix(PHASE_SUBJECT_PREFIX)?)
}

/// `digits` as a number when it is spelled decimal with no sign or leading
/// zero, or none.
fn canonical_number(digits: &str) -> Option<u32> {
    let canonical = !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    canonical.then(|| digits.parse().ok()).flatten()
}

/// The stream an order of `subject` is recorded on: `roadmap` for the
/// start and for a story's analyzer orders, and the phase's own stream,
/// `phase/<n>`, for a phase's planner orders (D-11). Other subjects get
/// theirs with the builds that issue them.
pub fn order_stream(subject: &str) -> Option<Cow<'static, str>> {
    if subject == START_SUBJECT || subject_story(subject).is_some() {
        return Some(Cow::Borrowed(ROADMAP_STREAM));
    }
    subject_phase(subject).map(|phase| Cow::Owned(phase_stream(phase)))
}

/// The agent stub a role runs at a rung, `bal-<role>-<rung>`. Install places
/// a stub under every such name, so an order always names one that exists.
pub fn agent_name(role: Role, rung: Rung) -> String {
    format!("bal-{}-{}", role.name(), rung.name())
}

/// One input an order names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderInput {
    /// The input's name, such as `brief`.
    pub name: String,
    /// The reference to the `record` payload holding its body.
    pub body: PayloadRef,
}

/// Everything `work_order.issued` records for one order.
#[derive(Debug, Clone, Copy)]
pub struct IssuedOrder<'a> {
    /// The order's id, a lowercase UUID.
    pub id: &'a str,
    /// The id of its chain's first order, which is its own id at attempt 1.
    pub request: &'a str,
    /// The failed order it retries, if any.
    pub parent: Option<&'a str>,
    /// The role it is for.
    pub role: Role,
    /// What it is about, such as `start`.
    pub subject: &'a str,
    /// The attempt, from 1.
    pub attempt: u32,
    /// The route record's JSON.
    pub route: &'a Value,
    /// The agent stub's name.
    pub agent: &'a str,
    /// The instructions it carries, in order.
    pub instructions: &'a [InstructionEvidence],
    /// The inputs it carries, in order.
    pub inputs: &'a [OrderInput],
}

/// The payload of `work_order.issued` for `order`.
pub fn issued_payload(order: &IssuedOrder<'_>) -> Value {
    let instructions: Vec<Value> = order
        .instructions
        .iter()
        .map(|entry| {
            json!({
                "identity": entry.identity(),
                "version": entry.version(),
                "hash": entry.hash(),
            })
        })
        .collect();
    let inputs: Vec<Value> = order
        .inputs
        .iter()
        .map(|input| json!({"name": input.name, "body": input.body.to_value()}))
        .collect();
    json!({
        "id": order.id,
        "request": order.request,
        "parent": order.parent,
        "role": order.role.name(),
        "subject": order.subject,
        "attempt": order.attempt,
        "route": order.route,
        "agent": order.agent,
        "instructions": instructions,
        "inputs": inputs,
    })
}

/// The payload of `work_order.failed`: the order and why it failed.
pub fn failed_payload(id: &str, reason: &str) -> Value {
    json!({"id": id, "reason": reason})
}

#[cfg(test)]
mod tests {
    //! Expected payloads are written out from the phase's decisions and the
    //! plan's test data, never from running this code.

    use std::collections::BTreeSet;

    use baley_store::{EventSchema, Hash, RetentionClass};

    use super::*;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";

    fn brief() -> Vec<OrderInput> {
        vec![OrderInput {
            name: "brief".into(),
            body: PayloadRef {
                hash: Hash([0xab; 32]),
                bytes: 300,
                class: RetentionClass::Record,
            },
        }]
    }

    fn evidence() -> Vec<InstructionEvidence> {
        vec![InstructionEvidence::new("bal-start-planner", "1", &"c".repeat(64)).unwrap()]
    }

    #[test]
    fn a_work_order_event_left_unregistered_or_at_another_version_is_caught() {
        let mut registry = Registry::new();
        register_work_order_events(&mut registry).unwrap();
        for name in ["work_order.issued", "work_order.failed"] {
            assert_eq!(registry.current_version(name), Some(1), "{name}");
            assert!(!registry.reads(name, 2), "{name}");
        }
        assert!(register_work_order_events(&mut registry).is_err());
    }

    #[test]
    fn an_issued_payload_missing_a_member_or_carrying_another_is_caught() {
        let route = json!({"rung": "high"});
        let (instructions, inputs) = (evidence(), brief());
        let o1 = IssuedOrder {
            id: I0,
            request: I0,
            parent: None,
            role: Role::Planner,
            subject: "start",
            attempt: 1,
            route: &route,
            agent: "bal-planner-high",
            instructions: &instructions,
            inputs: &inputs,
        };
        let expected = json!({"id": I0, "request": I0, "parent": null, "role": "planner",
            "subject": "start", "attempt": 1, "route": {"rung": "high"},
            "agent": "bal-planner-high",
            "instructions": [{"identity": "bal-start-planner", "version": "1",
                "hash": "c".repeat(64)}],
            "inputs": [{"name": "brief",
                "body": {"payload": "ab".repeat(32), "bytes": 300, "class": "record"}}]});
        assert_eq!(issued_payload(&o1), expected);

        let o2 = IssuedOrder {
            id: I1,
            parent: Some(I0),
            attempt: 2,
            agent: "bal-planner-xhigh",
            ..o1
        };
        let mut retry = expected;
        retry["id"] = json!(I1);
        retry["parent"] = json!(I0);
        retry["attempt"] = json!(2);
        retry["agent"] = json!("bal-planner-xhigh");
        assert_eq!(issued_payload(&o2), retry);
        assert_eq!(issued_payload(&o2)["request"], I0);
    }

    #[test]
    fn a_failed_payload_without_its_order_or_reason_is_caught() {
        assert_eq!(
            failed_payload(I0, "the worker could not launch"),
            json!({"id": I0, "reason": "the worker could not launch"})
        );
    }

    #[test]
    fn a_start_order_off_the_roadmap_stream_or_an_agent_off_the_name_rule_is_caught() {
        assert_eq!(order_stream("start").as_deref(), Some("roadmap"));
        let names: BTreeSet<String> = Role::ALL
            .into_iter()
            .flat_map(|role| Rung::ALL.map(|rung| agent_name(role, rung)))
            .collect();
        assert_eq!(names.len(), 30);
        for name in [
            "bal-planner-high",
            "bal-analyzer-medium",
            "bal-checker-low",
            "bal-reviewer-max",
        ] {
            assert!(names.contains(name), "{name}");
        }
    }

    #[test]
    fn an_analyzer_order_off_the_roadmap_stream_or_misread_subject_is_caught() {
        assert_eq!(story_subject(12), "story/12");
        assert_eq!(order_stream("story/12").as_deref(), Some("roadmap"));
        assert_eq!(subject_story("story/12"), Some(12));
        for other in ["story/012", "story/0", "story/", "story/x"] {
            assert_eq!(subject_story(other), None, "{other}");
            assert_eq!(order_stream(other), None, "{other}");
        }
        assert_eq!(subject_story("phase/12"), None);
    }

    #[test]
    fn a_phase_subject_spelled_two_ways_or_recorded_off_its_stream_is_caught() {
        assert_eq!(phase_subject(7), "phase/7");
        assert_eq!(subject_phase("phase/7"), Some(7));
        for other in ["phase/07", "phase/", "phase/x", "phase/+7"] {
            assert_eq!(subject_phase(other), None, "{other}");
            assert_eq!(order_stream(other), None, "{other}");
        }
        assert_eq!(order_stream("phase/7").as_deref(), Some("phase/7"));
        assert_eq!(order_stream("story/1").as_deref(), Some("roadmap"));
        assert_eq!(order_stream("start").as_deref(), Some("roadmap"));
    }
}
