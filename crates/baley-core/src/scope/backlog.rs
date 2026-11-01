//! The `backlog` view: one document per project listing its stories in
//! priority order (design 0005's view row).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::{
    PHASE_DECLARED, STORY_CORRECTED, STORY_DECLARED, STORY_DROPPED, STORY_REASSIGNED,
    STORY_REPRIORITIZED,
};
use super::roadmap::reprioritized;
use crate::plan::{PLAN_APPROVED, PLAN_REPLACED};
use crate::questions::QUESTIONS_OPENED;
use crate::truth::STORY_REFINED;

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
        version: 4,
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

/// Keeps the `backlog` view current from the story events,
/// `phase.declared`, a story's `questions.opened`, `story.refined`,
/// `plan.approved` and `plan.replaced`.
///
/// The stories are kept in declaration order, which is priority order
/// until stories are reprioritized. Each records its latest question set
/// (`set`), the set its latest `story.refined` names (`refined`) and that
/// refinement's truth count, and holds none, none and 0 when declared. Each
/// also keeps the plans in force that serve it (`plans`), each by phase,
/// number and size, and its `size` is their tasks summed, or null when none
/// serves it (PLN-R9). An approval adds the plan to every story it serves,
/// and a replacement takes the old plan off every story that holds it, so
/// exactly what that plan added comes off. A plan naming a story not held
/// is refused. Its refinement state is never stored:
/// the reader works it out from the latest set's round, so nothing stored
/// can drift from it. A story's set makes that set its latest and a plan
/// draft's set changes nothing; a refinement replaces the marker and the
/// count, and a later set leaves both, since approved truths stay until new
/// ones are approved. A set or a refinement of a story not held is refused,
/// as a correction of one is. A later declaration of an
/// id already held replaces that entry in place, so it keeps its priority,
/// and keeps its plans and size, as every other story event does.
/// A correction, a move and a drop change a story in place; a dropped
/// story stays with status `dropped` and no phase. A reprioritization puts
/// the stories it lists first and every other story after them in its
/// current relative order. A phase declaration commits each story it lists
/// to that phase and returns to the backlog each story it no longer lists.
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
        &[
            STORY_DECLARED,
            PHASE_DECLARED,
            STORY_CORRECTED,
            STORY_REASSIGNED,
            STORY_DROPPED,
            STORY_REPRIORITIZED,
            QUESTIONS_OPENED,
            STORY_REFINED,
            PLAN_APPROVED,
            PLAN_REPLACED,
        ]
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
        let no_story = || refuse("the story is not declared");
        let whole = |name: &str| payload.get(name).and_then(Value::as_u64).ok_or_else(fault);
        let text = |name: &str| payload.get(name).and_then(Value::as_str).ok_or_else(fault);
        let phase = || match payload.get("phase") {
            Some(Value::Null) => Ok(Value::Null),
            Some(phase) => Ok(json!(phase.as_u64().ok_or_else(fault)?)),
            None => Err(fault()),
        };
        // A plan draft's set is not the backlog's.
        if event.type_name == QUESTIONS_OPENED && text("subject")? != "story" {
            return Ok(Vec::new());
        }
        let mut stories = match documents.first() {
            Some((_, document)) => document
                .get("stories")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| refuse("the backlog document holds no story list"))?,
            None => Vec::new(),
        };
        match event.type_name.as_str() {
            STORY_DECLARED => {
                let id = whole("id")?;
                let mut entry = json!({
                    "id": id,
                    "sentence": text("sentence")?,
                    "status": text("status")?,
                    "phase": phase()?,
                    "set": null,
                    "refined": null,
                    "truths": 0,
                    "plans": [],
                    "size": null,
                });
                match stories.iter().position(|story| story["id"] == id) {
                    Some(at) => {
                        entry["plans"] = stories[at]["plans"].take();
                        entry["size"] = stories[at]["size"].take();
                        stories[at] = entry;
                    }
                    None => stories.push(entry),
                }
            }
            PHASE_DECLARED => {
                let number = whole("number")?;
                let listed: Vec<u64> = payload
                    .get("stories")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_u64).collect())
                    .ok_or_else(fault)?;
                for story in &mut stories {
                    let held = story["id"].as_u64().is_some_and(|id| listed.contains(&id));
                    if held {
                        story["phase"] = json!(number);
                    } else if story["phase"] == number {
                        story["phase"] = Value::Null;
                    }
                }
            }
            STORY_CORRECTED => {
                let (sentence, status) = (text("sentence")?, text("status")?);
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["sentence"] = json!(sentence);
                story["status"] = json!(status);
            }
            STORY_REASSIGNED => {
                let (status, phase) = (text("status")?, phase()?);
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = phase;
            }
            STORY_DROPPED => {
                let status = text("status")?;
                let story = story_mut(&mut stories, whole("id")?).ok_or_else(no_story)?;
                story["status"] = json!(status);
                story["phase"] = Value::Null;
            }
            STORY_REPRIORITIZED => {
                let order: Vec<u64> = payload
                    .get("order")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_u64).collect())
                    .ok_or_else(fault)?;
                stories = reprioritized(stories, &order).ok_or_else(no_story)?;
            }
            QUESTIONS_OPENED => {
                let set = text("set")?;
                let story = story_mut(&mut stories, whole("story")?).ok_or_else(no_story)?;
                story["set"] = json!(set);
            }
            STORY_REFINED => {
                let set = text("set")?;
                let count = payload
                    .get("truths")
                    .and_then(Value::as_array)
                    .ok_or_else(fault)?
                    .len();
                let story = story_mut(&mut stories, whole("story")?).ok_or_else(no_story)?;
                story["refined"] = json!(set);
                story["truths"] = json!(count);
            }
            PLAN_APPROVED => {
                let (number, plan, size) = (whole("phase")?, whole("plan")?, whole("size")?);
                let listed: Vec<u64> = payload
                    .get("stories")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.iter().map(Value::as_u64).collect())
                    .ok_or_else(fault)?;
                if !listed
                    .iter()
                    .all(|id| stories.iter().any(|s| s["id"] == *id))
                {
                    return Err(no_story());
                }
                let served = json!({"phase": number, "plan": plan, "size": size});
                for story in &mut stories {
                    if story["id"].as_u64().is_some_and(|id| listed.contains(&id)) {
                        plans_mut(story).ok_or_else(fault)?.push(served.clone());
                        settle(story)
                            .ok_or_else(|| refuse("the story's size would exceed a u64"))?;
                    }
                }
            }
            PLAN_REPLACED => {
                let (number, plan) = (whole("phase")?, whole("plan")?);
                for story in &mut stories {
                    let plans = plans_mut(story).ok_or_else(fault)?;
                    let before = plans.len();
                    plans.retain(|held| held["phase"] != number || held["plan"] != plan);
                    if plans.len() != before {
                        settle(story).ok_or_else(fault)?;
                    }
                }
            }
            _ => return Err(refuse("not a backlog event")),
        }
        Ok(vec![Change::Put {
            key: backlog_key(),
            body: json!({KEY_FIELD: PROJECT_KEY, "stories": stories}),
        }])
    }
}

/// The entry of story `id`.
fn story_mut(stories: &mut [Value], id: u64) -> Option<&mut Value> {
    stories.iter_mut().find(|story| story["id"] == id)
}

/// A story's plans in force, `None` when the entry holds no list.
fn plans_mut(story: &mut Value) -> Option<&mut Vec<Value>> {
    story.get_mut("plans").and_then(Value::as_array_mut)
}

/// Sets a story's `size` to its plans' sizes summed, null when it has none;
/// `None` when a size is unreadable or the sum passes `u64`.
fn settle(story: &mut Value) -> Option<()> {
    let plans = story.get("plans")?.as_array()?;
    let mut total: Option<u64> = None;
    for plan in plans {
        total = Some(
            total
                .unwrap_or(0)
                .checked_add(plan.get("size")?.as_u64()?)?,
        );
    }
    story["size"] = json!(total);
    Some(())
}

#[cfg(test)]
mod tests {
    //! Expected documents are written out from design 0005's backlog shape
    //! and the plan's approved S1, never from running this code.

    use std::collections::BTreeMap;

    use baley_store::{Actor, Hash, PayloadRef, ProjectId, RequestId, RetentionClass};
    use serde_json::json;

    use super::super::event::{
        phase_declared_payload, story_corrected_payload, story_declared_payload,
        story_dropped_payload, story_reassigned_payload, story_reprioritized_payload,
    };
    use super::super::submission::StoryStatus;
    use super::*;

    fn story(seq: u64, id: u32, sentence: &str, status: StoryStatus, phase: Option<u32>) -> Event {
        event(
            seq,
            "story.declared",
            story_declared_payload(id, sentence, status, phase, Hash([0x5c; 32])),
        )
    }

    fn event(seq: u64, type_name: &str, payload: Value) -> Event {
        Event {
            project_id: ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into()),
            seq,
            stream: "roadmap".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: "2026-10-13T09:00:02Z".into(),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            git: None,
            policy_version: 0,
            payload,
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
        // Ids declared out of ascending order, so a sort by id cannot pass
        // for declaration order.
        let events = [
            story(
                11,
                3,
                "Teammates see the backlog",
                StoryStatus::Deferred,
                None,
            ),
            story(
                12,
                1,
                "Every step is recorded",
                StoryStatus::Active,
                Some(1),
            ),
            story(
                13,
                2,
                "The owner approves scope",
                StoryStatus::Active,
                Some(2),
            ),
        ];
        assert_eq!(
            fold(&events),
            json!([
                {"id": 3, "sentence": "Teammates see the backlog", "status": "deferred",
                    "phase": null, "set": null, "refined": null, "truths": 0, "plans": [],
                    "size": null},
                {"id": 1, "sentence": "Every step is recorded", "status": "active", "phase": 1,
                    "set": null, "refined": null, "truths": 0, "plans": [], "size": null},
                {"id": 2, "sentence": "The owner approves scope", "status": "active",
                    "phase": 2, "set": null, "refined": null, "truths": 0, "plans": [],
                    "size": null}
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

    /// Phase `number` declared with `stories`.
    fn phase(seq: u64, number: u32, stories: &[u32]) -> Event {
        let detail = PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 24,
            class: RetentionClass::Record,
        };
        event(
            seq,
            "phase.declared",
            phase_declared_payload(
                number,
                "Phase",
                "A goal",
                &detail,
                &[],
                stories,
                Hash([0x5c; 32]),
            ),
        )
    }

    /// The stories after the approved S1, then `events`.
    fn after_s1(events: Vec<Event>) -> Value {
        let mut all = approved_s1();
        all.extend(events);
        fold(&all)
    }

    fn ids(stories: &Value) -> Vec<u64> {
        stories
            .as_array()
            .unwrap()
            .iter()
            .map(|story| story["id"].as_u64().unwrap())
            .collect()
    }

    fn entry(stories: &Value, id: u64) -> Value {
        stories
            .as_array()
            .unwrap()
            .iter()
            .find(|story| story["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("story {id} in {stories}"))
    }

    #[test]
    fn a_backlog_changed_by_an_event_it_does_not_handle_is_caught() {
        assert_eq!(
            BacklogProjector::new().handles(),
            [
                "story.declared",
                "phase.declared",
                "story.corrected",
                "story.reassigned",
                "story.dropped",
                "story.reprioritized",
                "questions.opened",
                "story.refined",
                "plan.approved",
                "plan.replaced"
            ]
        );
        assert_eq!(backlog_spec().version, 4);
    }

    #[test]
    fn a_backlog_story_change_lost_or_moving_its_place_is_caught() {
        // The correction and story 2's move target stories that are not
        // last, so a handler that removes its story and appends it again
        // shows in the order. Story 3 moves first so that nothing after
        // story 2's move puts the order back. Story 2 stays active, so
        // story 3 leaving the backlog shows a reassignment that ignores its
        // status.
        let digest = Hash([0x5c; 32]);
        let stories = after_s1(vec![
            event(
                14,
                "story.corrected",
                story_corrected_payload(
                    1,
                    "Every step is recorded in order",
                    StoryStatus::Active,
                    digest,
                ),
            ),
            event(
                15,
                "story.reassigned",
                story_reassigned_payload(3, StoryStatus::Active, Some(1), digest),
            ),
            event(
                16,
                "story.reassigned",
                story_reassigned_payload(2, StoryStatus::Active, Some(1), digest),
            ),
            event(17, "story.dropped", story_dropped_payload(1, digest)),
        ]);
        assert_eq!(ids(&stories), [1, 2, 3]);
        let first = entry(&stories, 1);
        assert_eq!(first["sentence"], "Every step is recorded in order");
        assert_eq!(first["status"], "dropped");
        assert_eq!(first["phase"], Value::Null);
        let second = entry(&stories, 2);
        assert_eq!(second["status"], "active");
        assert_eq!(second["phase"], 1);
        let third = entry(&stories, 3);
        assert_eq!(third["status"], "active");
        assert_eq!(third["phase"], 1);
        for story in stories.as_array().unwrap() {
            assert_eq!(story["set"], Value::Null, "{story}");
            assert_eq!(story["refined"], Value::Null, "{story}");
            assert_eq!(story["truths"], 0, "{story}");
            assert_eq!(story["size"], Value::Null, "{story}");
        }
    }

    #[test]
    fn a_reprioritized_backlog_out_of_order_or_losing_a_dropped_story_is_caught() {
        let digest = Hash([0x5c; 32]);
        let stories = after_s1(vec![
            event(14, "story.dropped", story_dropped_payload(3, digest)),
            event(
                15,
                "story.reprioritized",
                story_reprioritized_payload(&[2, 1], digest),
            ),
        ]);
        assert_eq!(ids(&stories), [2, 1, 3]);
        assert_eq!(entry(&stories, 3)["status"], "dropped");
    }

    #[test]
    fn a_story_a_declaration_commits_left_in_the_backlog_is_caught() {
        let stories = after_s1(vec![
            story(14, 4, "Reports show progress", StoryStatus::Active, None),
            phase(15, 3, &[4]),
            phase(16, 2, &[]),
        ]);
        assert_eq!(entry(&stories, 4)["phase"], 3);
        assert_eq!(entry(&stories, 2)["phase"], Value::Null);
    }
    const S: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const S2: &str = "7f3e4d5c-6071-4c8d-8e9f-1a2b3c4d5e6f";
    const O1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";

    /// Story `id`'s set `set`, opened with one question.
    fn story_set(seq: u64, id: u32, set: &str) -> Event {
        event(
            seq,
            "questions.opened",
            json!({"set": set, "subject": "story", "story": id, "work_order": O1,
                "questions": [{"id": "q1", "question": "Which store backs the ledger?",
                    "recommended": "SQLite", "depends_on": []}],
                "at": "2026-10-22T09:00:00Z"}),
        )
    }

    /// A `story.refined` for story `id` naming set `set`, with `count`
    /// truths.
    fn refined(seq: u64, id: u32, set: &str, count: usize) -> Event {
        let truths: Vec<Value> = (1..=count)
            .map(|n| {
                json!({"id": format!("t{n}"), "form": "when",
                    "trigger": "the owner approves a draft by a stale digest",
                    "observer": "the owner", "verb": "is refused",
                    "outcome": "stale-draft naming the first differing part", "kind": "literal",
                    "observable": true, "fixed_oracle": true, "version": 1})
            })
            .collect();
        event(
            seq,
            "story.refined",
            json!({"story": id, "set": set, "submission": "5c".repeat(32),
                "owner": "Ada Owner", "at": "2026-10-22T09:00:00Z", "truths": truths}),
        )
    }

    /// Story `id`'s latest set, refined marker and truth count.
    fn refinement(stories: &Value, id: u64) -> (Value, Value, Value) {
        let story = entry(stories, id);
        assert!(story.get("refinement").is_none(), "{story}");
        (
            story["set"].clone(),
            story["refined"].clone(),
            story["truths"].clone(),
        )
    }

    #[test]
    fn a_backlog_storing_a_state_or_missing_the_latest_set_is_caught() {
        assert_eq!(backlog_spec().version, 4);
        let plan_set = event(
            17,
            "questions.opened",
            json!({"set": S2, "subject": "plan", "phase": 1, "plan_digest": "5c".repeat(32),
                "questions": [], "at": "2026-10-22T09:00:00Z"}),
        );
        let mut events = vec![story_set(14, 1, S), refined(15, 1, S, 2)];
        let before_plan = after_s1(events.clone());
        events.push(plan_set);
        let stories = after_s1(events.clone());
        assert_eq!(stories, before_plan, "a plan set changes nothing");
        assert_eq!(refinement(&stories, 1), (json!(S), json!(S), json!(2)));
        for id in [2, 3] {
            assert_eq!(
                refinement(&stories, id),
                (Value::Null, Value::Null, json!(0))
            );
        }

        events.push(story_set(18, 1, S2));
        let asked_again = after_s1(events);
        assert_eq!(refinement(&asked_again, 1), (json!(S2), json!(S), json!(2)));
    }

    #[test]
    fn a_set_or_refinement_for_an_undeclared_story_accepted_is_caught() {
        let held = after_s1(Vec::new());
        let document = json!({KEY_FIELD: PROJECT_KEY, "stories": held});
        for undeclared in [story_set(14, 9, S), refined(15, 9, S, 2)] {
            let answer =
                BacklogProjector::new().apply(&undeclared, &[(backlog_key(), document.clone())]);
            assert!(answer.is_err(), "{}: {answer:?}", undeclared.type_name);
        }
    }

    /// Plan `plan` of phase 1 approved, serving `stories` with `size` tasks.
    fn approved(seq: u64, plan: u32, stories: &[u32], size: usize) -> Event {
        let body = PayloadRef {
            hash: Hash([0xab; 32]),
            bytes: 24,
            class: RetentionClass::Record,
        };
        let approval = crate::plan::PlanApproval {
            phase: 1,
            plan,
            digest: Hash([0x5c; 32]),
            stories,
            size,
            suite: None,
            question_set: S,
            evidence_map: &[],
            body: &body,
            owner: "Ada Owner",
            at: "2026-10-31T09:00:00Z",
        };
        event(seq, "plan.approved", approval.payload())
    }

    /// The sizes of stories 1, 2 and 3.
    fn sizes(stories: &Value) -> [Value; 3] {
        [1, 2, 3].map(|id| entry(stories, id)["size"].clone())
    }

    #[test]
    fn a_story_size_counted_from_estimates_or_left_after_replacement_is_caught() {
        let mut events = vec![approved(14, 1, &[1, 2], 2)];
        assert_eq!(
            sizes(&after_s1(events.clone())),
            [json!(2), json!(2), Value::Null]
        );
        events.push(approved(15, 2, &[1], 3));
        assert_eq!(
            sizes(&after_s1(events.clone())),
            [json!(5), json!(2), Value::Null]
        );
        events.push(approved(16, 3, &[2], 1));
        let replaced = crate::plan::replaced_payload(
            1,
            1,
            Hash([0x5c; 32]),
            Hash([0x7e; 32]),
            "Ada Owner",
            "2026-10-31T09:00:00Z",
        );
        events.push(event(17, "plan.replaced", replaced));
        let stories = after_s1(events);
        assert_eq!(sizes(&stories), [json!(3), json!(1), Value::Null]);

        let document = json!({KEY_FIELD: PROJECT_KEY, "stories": stories});
        let unheld = approved(18, 4, &[9], 1);
        let answer = BacklogProjector::new().apply(&unheld, &[(backlog_key(), document)]);
        assert!(answer.is_err(), "{answer:?}");
    }
}
