//! The `roadmap` view: one document per project holding its accepted
//! scope (design 0004 section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::{PHASE_DECLARED, PHASE_REORDERED, PROJECT_DESCRIBED, STORY_DECLARED};

/// The view's name.
pub const ROADMAP_VIEW: &str = "roadmap";

/// The key field the one document holds.
const KEY_FIELD: &str = "roadmap";
/// The one document's key text.
const PROJECT_KEY: &str = "project";

/// The view's declaration: one text key, no indexes, and page bound 1,
/// since every read is a `get` of the one document.
pub fn roadmap_spec() -> ViewSpec {
    ViewSpec {
        name: ROADMAP_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: KEY_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of the project's roadmap document.
pub fn roadmap_key() -> DocKey {
    DocKey(vec![KeyValue::Text(PROJECT_KEY.into())])
}

/// Keeps the `roadmap` view current from the events an approval records.
///
/// The document holds the latest description, the declared phases by
/// number, the latest phase order, the stories in priority order, and the
/// highest phase number and story id ever declared. Those two never go
/// down, so an approval numbers past every declaration, withdrawn phases
/// included. A later declaration of a number or id already held replaces
/// that entry in place. Submissions and approvals themselves are not
/// handled: accepted scope changes only on the events an approval records.
pub struct RoadmapProjector {
    spec: ViewSpec,
}

impl RoadmapProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: roadmap_spec(),
        }
    }
}

impl Default for RoadmapProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for RoadmapProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[
            PROJECT_DESCRIBED,
            STORY_DECLARED,
            PHASE_DECLARED,
            PHASE_REORDERED,
        ]
    }

    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        vec![roadmap_key()]
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let mut document = match documents.first() {
            Some((_, Value::Object(document))) => document.clone(),
            Some(_) => return Err(refuse("the roadmap document is not an object")),
            None => empty(),
        };
        let fault = || refuse("a member is missing or of the wrong type");
        match event.type_name.as_str() {
            PROJECT_DESCRIBED => {
                let description = json!({
                    "what": text(payload, "what").ok_or_else(fault)?,
                    "core_value": text(payload, "core_value").ok_or_else(fault)?,
                    "constraints": texts(payload, "constraints").ok_or_else(fault)?,
                    "submission": text(payload, "submission").ok_or_else(fault)?,
                });
                document.insert("description".into(), description);
            }
            STORY_DECLARED => {
                let id = whole(payload, "id").ok_or_else(fault)?;
                let phase = match payload.get("phase") {
                    Some(Value::Null) => Value::Null,
                    _ => json!(whole(payload, "phase").ok_or_else(fault)?),
                };
                let entry = json!({
                    "id": id,
                    "sentence": text(payload, "sentence").ok_or_else(fault)?,
                    "status": text(payload, "status").ok_or_else(fault)?,
                    "phase": phase,
                    "truths": 0,
                });
                let stories = entries(&mut document, "stories").ok_or_else(fault)?;
                match stories.iter().position(|story| story["id"] == id) {
                    Some(at) => stories[at] = entry,
                    None => stories.push(entry),
                }
                raise(&mut document, "highest_story", id);
            }
            PHASE_DECLARED => {
                let number = whole(payload, "number").ok_or_else(fault)?;
                let entry = json!({
                    "number": number,
                    "name": text(payload, "name").ok_or_else(fault)?,
                    "goal": text(payload, "goal").ok_or_else(fault)?,
                    "depends_on": wholes(payload, "depends_on").ok_or_else(fault)?,
                    "stories": wholes(payload, "stories").ok_or_else(fault)?,
                    "status": "declared",
                });
                let phases = entries(&mut document, "phases").ok_or_else(fault)?;
                let at = phases.partition_point(|phase| {
                    phase["number"].as_u64().is_some_and(|held| held < number)
                });
                if phases
                    .get(at)
                    .is_some_and(|phase| phase["number"] == number)
                {
                    phases[at] = entry;
                } else {
                    phases.insert(at, entry);
                }
                raise(&mut document, "highest_phase", number);
            }
            PHASE_REORDERED => {
                let order = wholes(payload, "order").ok_or_else(fault)?;
                document.insert("order".into(), json!(order));
            }
            _ => return Err(refuse("not a roadmap event")),
        }
        Ok(vec![Change::Put {
            key: roadmap_key(),
            body: Value::Object(document),
        }])
    }
}

/// The document of a project with no accepted scope yet.
fn empty() -> Map<String, Value> {
    let Value::Object(document) = json!({
        KEY_FIELD: PROJECT_KEY,
        "description": null,
        "phases": [],
        "order": [],
        "stories": [],
        "highest_phase": 0,
        "highest_story": 0,
    }) else {
        unreachable!("an object literal")
    };
    document
}

fn text<'a>(payload: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    payload.get(name).and_then(Value::as_str)
}

fn texts<'a>(payload: &'a Map<String, Value>, name: &str) -> Option<Vec<&'a str>> {
    payload
        .get(name)?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect()
}

fn whole(payload: &Map<String, Value>, name: &str) -> Option<u64> {
    payload.get(name).and_then(Value::as_u64)
}

fn wholes(payload: &Map<String, Value>, name: &str) -> Option<Vec<u64>> {
    payload
        .get(name)?
        .as_array()?
        .iter()
        .map(Value::as_u64)
        .collect()
}

/// The document's list `name`, which the empty document always has.
fn entries<'a>(document: &'a mut Map<String, Value>, name: &str) -> Option<&'a mut Vec<Value>> {
    document.get_mut(name)?.as_array_mut()
}

/// Raises `name` to `value`, never lowering it.
fn raise(document: &mut Map<String, Value>, name: &str, value: u64) {
    let held = document.get(name).and_then(Value::as_u64).unwrap_or(0);
    document.insert(name.into(), json!(held.max(value)));
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0004's roadmap shape
    //! and the approved S1 of the plan, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};
    use serde_json::json;

    use super::super::event::{
        described_payload, phase_declared_payload, phase_reordered_payload, story_declared_payload,
    };
    use super::super::submission::{Description, StoryStatus};
    use super::*;

    const AT: &str = "2026-10-13T09:00:02Z";

    fn digest() -> Hash {
        Hash([0x5c; 32])
    }

    fn event(type_name: &str, seq: u64, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: AT.into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    fn detail() -> PayloadRef {
        PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 24,
            class: RetentionClass::Record,
        }
    }

    fn story(seq: u64, id: u32, sentence: &str, status: StoryStatus, phase: Option<u32>) -> Event {
        event(
            "story.declared",
            seq,
            story_declared_payload(id, sentence, status, phase, digest()),
        )
    }

    fn phase(
        seq: u64,
        number: u32,
        name: &str,
        goal: &str,
        depends_on: &[u32],
        stories: &[u32],
    ) -> Event {
        event(
            "phase.declared",
            seq,
            phase_declared_payload(number, name, goal, &detail(), depends_on, stories, digest()),
        )
    }

    /// The document after `events`, each applied to what the one before it
    /// left, as the store hands documents over.
    fn fold(events: &[Event]) -> Value {
        let projector = RoadmapProjector::new();
        let mut view = BTreeMap::new();
        for event in events {
            let read: Vec<(DocKey, Value)> = projector
                .keys(event)
                .into_iter()
                .filter_map(|key| view.get(&key).cloned().map(|body| (key, body)))
                .collect();
            for change in projector.apply(event, &read).unwrap() {
                match change {
                    Change::Put { key, body } => {
                        view.insert(key, body);
                    }
                    Change::Delete { key } => {
                        view.remove(&key);
                    }
                }
            }
        }
        assert_eq!(view.len(), 1, "one document per project");
        view.remove(&roadmap_key()).expect("the project's document")
    }

    #[test]
    fn an_approved_start_projected_out_of_order_or_without_its_description_is_caught() {
        let description = Description {
            what: "A ledger for agent work".into(),
            core_value: "Nothing moves on an unproven claim".into(),
            constraints: vec!["One binary".into()],
        };
        let document = fold(&[
            event(
                "project.described",
                10,
                described_payload(&description, digest()),
            ),
            story(
                11,
                1,
                "Every step is recorded",
                StoryStatus::Active,
                Some(1),
            ),
            story(
                12,
                2,
                "The owner approves scope",
                StoryStatus::Active,
                Some(2),
            ),
            story(
                13,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
            phase(14, 1, "Records", "Steps are recorded", &[], &[1]),
            phase(15, 2, "Approval", "The owner approves", &[1], &[2]),
            event(
                "phase.reordered",
                16,
                phase_reordered_payload(&[1, 2], digest()),
            ),
        ]);
        let submission = "5c".repeat(32);
        assert_eq!(
            document,
            json!({
                "roadmap": "project",
                "description": {"what": "A ledger for agent work",
                    "core_value": "Nothing moves on an unproven claim",
                    "constraints": ["One binary"], "submission": submission},
                "phases": [
                    {"number": 1, "name": "Records", "goal": "Steps are recorded",
                        "depends_on": [], "stories": [1], "status": "declared"},
                    {"number": 2, "name": "Approval", "goal": "The owner approves",
                        "depends_on": [1], "stories": [2], "status": "declared"}
                ],
                "order": [1, 2],
                "stories": [
                    {"id": 1, "sentence": "Every step is recorded", "status": "active",
                        "phase": 1, "truths": 0},
                    {"id": 2, "sentence": "The owner approves scope", "status": "active",
                        "phase": 2, "truths": 0},
                    {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                        "phase": null, "truths": 0}
                ],
                "highest_phase": 2,
                "highest_story": 3
            })
        );
    }

    #[test]
    fn a_submission_projected_as_accepted_scope_is_caught() {
        assert_eq!(
            RoadmapProjector::new().handles(),
            [
                "project.described",
                "story.declared",
                "phase.declared",
                "phase.reordered"
            ]
        );
    }

    #[test]
    fn a_redeclared_phase_or_story_listed_twice_or_lowering_the_highest_value_is_caught() {
        let document = fold(&[
            phase(1, 2, "Approval", "The owner approves", &[1], &[2]),
            phase(2, 1, "Records", "Steps are recorded", &[], &[1]),
            phase(3, 1, "Records", "New goal", &[], &[1]),
            story(
                4,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
            story(5, 1, "Every step is recorded", StoryStatus::Active, Some(1)),
            story(6, 1, "New sentence", StoryStatus::Active, Some(1)),
        ]);
        let phases: Vec<(u64, &str)> = document["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phase| {
                (
                    phase["number"].as_u64().unwrap(),
                    phase["goal"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(phases, [(1, "New goal"), (2, "The owner approves")]);
        assert_eq!(document["highest_phase"], 2);
        let stories: Vec<(u64, &str)> = document["stories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|story| {
                (
                    story["id"].as_u64().unwrap(),
                    story["sentence"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            stories,
            [(3, "Teammates see the backlog"), (1, "New sentence")]
        );
        assert_eq!(document["highest_story"], 3);
    }
}
