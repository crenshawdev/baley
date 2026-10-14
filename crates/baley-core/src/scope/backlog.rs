//! The `backlog` view: one document per project listing its stories in
//! priority order (design 0005's view row).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::STORY_DECLARED;

/// The view's name.
pub const BACKLOG_VIEW: &str = "backlog";

/// The key field the one document holds.
const KEY_FIELD: &str = "backlog";
/// The one document's key text.
const PROJECT_KEY: &str = "project";

/// The view's declaration: one text key, no indexes, and page bound 1,
/// since every read is a `get` of the one document.
pub fn backlog_spec() -> ViewSpec {
    ViewSpec {
        name: BACKLOG_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: KEY_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of the project's backlog document.
pub fn backlog_key() -> DocKey {
    DocKey(vec![KeyValue::Text(PROJECT_KEY.into())])
}

/// Keeps the `backlog` view current from `story.declared`.
///
/// The stories are kept in declaration order, which is priority order
/// until stories are reprioritized. Each is unrefined with no truths and no
/// size until refinement and sizing record them. A later declaration of an
/// id already held replaces that entry in place, so it keeps its priority.
pub struct BacklogProjector {
    spec: ViewSpec,
}

impl BacklogProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: backlog_spec(),
        }
    }
}

impl Default for BacklogProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for BacklogProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[STORY_DECLARED]
    }

    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        vec![backlog_key()]
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
        let payload = &event.payload;
        let fault = || refuse("a member is missing or of the wrong type");
        let id = payload
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(fault)?;
        let phase = match payload.get("phase") {
            Some(Value::Null) => Value::Null,
            Some(phase) => json!(phase.as_u64().ok_or_else(fault)?),
            None => return Err(fault()),
        };
        let text = |name: &str| payload.get(name).and_then(Value::as_str).ok_or_else(fault);
        let entry = json!({
            "id": id,
            "sentence": text("sentence")?,
            "status": text("status")?,
            "phase": phase,
            "refinement": "unrefined",
            "truths": 0,
            "size": null,
        });
        let mut stories = match documents.first() {
            Some((_, document)) => document
                .get("stories")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| refuse("the backlog document holds no story list"))?,
            None => Vec::new(),
        };
        match stories.iter().position(|story| story["id"] == id) {
            Some(at) => stories[at] = entry,
            None => stories.push(entry),
        }
        Ok(vec![Change::Put {
            key: backlog_key(),
            body: json!({KEY_FIELD: PROJECT_KEY, "stories": stories}),
        }])
    }
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0005's backlog shape
    //! and the plan's approved S1, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, ProjectId, RequestId};
    use serde_json::json;

    use super::super::event::story_declared_payload;
    use super::super::submission::StoryStatus;
    use super::*;

    fn story(seq: u64, id: u32, sentence: &str, status: StoryStatus, phase: Option<u32>) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: "story.declared".into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: "2026-10-13T09:00:02Z".into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload: story_declared_payload(id, sentence, status, phase, Hash([0x5c; 32])),
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    /// The approved S1's three story declarations.
    fn approved_s1() -> Vec<Event> {
        vec![
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
        ]
    }

    /// The stories after `events`, each applied to what the one before it
    /// left, as the store hands documents over.
    fn fold(events: &[Event]) -> Value {
        let projector = BacklogProjector::new();
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
        view.remove(&backlog_key()).expect("the project's document")["stories"].clone()
    }

    #[test]
    fn a_backlog_out_of_priority_order_or_missing_refinement_and_truths_is_caught() {
        assert_eq!(
            fold(&approved_s1()),
            json!([
                {"id": 1, "sentence": "Every step is recorded", "status": "active", "phase": 1,
                    "refinement": "unrefined", "truths": 0, "size": null},
                {"id": 2, "sentence": "The owner approves scope", "status": "active",
                    "phase": 2, "refinement": "unrefined", "truths": 0, "size": null},
                {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                    "phase": null, "refinement": "unrefined", "truths": 0, "size": null}
            ])
        );
    }

    #[test]
    fn a_redeclared_story_appended_to_the_backlog_a_second_time_is_caught() {
        let mut events = approved_s1();
        events.push(story(
            14,
            2,
            "The owner approves scope",
            StoryStatus::Deferred,
            None,
        ));
        let stories = fold(&events);
        let listed: Vec<(u64, &str, &Value)> = stories
            .as_array()
            .unwrap()
            .iter()
            .map(|story| {
                (
                    story["id"].as_u64().unwrap(),
                    story["status"].as_str().unwrap(),
                    &story["phase"],
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (1, "active", &json!(1)),
                (2, "deferred", &Value::Null),
                (3, "deferred", &Value::Null)
            ]
        );
    }

    #[test]
    fn a_backlog_changed_by_anything_but_a_story_declaration_is_caught() {
        assert_eq!(BacklogProjector::new().handles(), ["story.declared"]);
    }
}
