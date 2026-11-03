//! What a planner order binds at issue (design 0005 section 4 Planner row,
//! D-12, D-15): the phase's number, goal and detail; the stories it commits,
//! in the order given, each with its current truths; the approved
//! description; the capacity and the phase's size when a capacity is set;
//! and the evidence items of the phase's plans in force. A revision order
//! also binds the draft it revises and its question set with every answer
//! and deferral. Each input is canonical JSON a `record` payload holds.
//!
//! Nothing here records anything. The selection is the caller's, never read
//! from the phase, so an approval binds the stories it is committing. A
//! retry carries its parent's inputs as they were.

use std::num::NonZeroU32;

use baley_core::plan::{
    EVIDENCE_MAP_VIEW, IN_FORCE, PHASE_PLAN_VIEW, PLAN_KIND, approved_items, evidence_map_key,
    phase_plan_key,
};
use baley_core::policy::{EffectivePolicy, Value as Setting};
use baley_core::questions::{QUESTIONS_VIEW, SetFacts, questions_key};
use baley_core::scope::{
    DRAFT_VIEW, PHASE_DETAIL_VIEW, PHASE_VIEW, ROADMAP_VIEW, draft_key, phase_key, roadmap_key,
};
use baley_core::truth::{TRUTHS_VIEW, truths_key};
use baley_store::{Hash, ObservedDocument, Payloads, ProjectId, StoreError, Views};
use serde_json::{Map, Value, json};

use crate::envelope::LEDGER_BUSY;
use crate::mcp::capture::{PLACE_LEDGER, TEXT_PURGED, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::questions::read::{SetRead, read as read_set};
use crate::refine::inputs::{bytes, questions_body};
use crate::scope::read::{self, Detail, PreviewError, Roadmap};
use crate::scope::submit::{NOT_RECORDED_BUSY, scope_unread};

/// The input holding the phase's number, goal and detail.
pub const PHASE_INPUT: &str = "phase";
/// The input holding the committed stories in order, each with its current
/// truths.
pub const STORIES_INPUT: &str = "stories";
/// The input holding the approved description.
pub const DESCRIPTION_INPUT: &str = "description";
/// The input holding the capacity and the phase's size, present only when a
/// capacity is set.
pub const CAPACITY_INPUT: &str = "capacity";
/// The input holding the evidence items of the phase's plans in force.
pub const APPROVED_ITEMS_INPUT: &str = "approved_items";
/// The input holding a revision order's draft plan.
pub const DRAFT_INPUT: &str = "draft";
/// The input holding a revision order's question set with every answer and
/// deferral.
pub const QUESTIONS_INPUT: &str = "questions";

/// The setting the capacity is read from.
const CAPACITY_SETTING: &str = "planning.phase_capacity";

/// The members of a truth a planner order carries, besides `human_verify`,
/// which it carries only when set.
const TRUTH_MEMBERS: [&str; 8] = [
    "id", "version", "form", "trigger", "observer", "verb", "outcome", "kind",
];

/// The draft a revision order revises and its question set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revision<'a> {
    /// The draft's digest.
    pub draft: Hash,
    /// The id of the draft's question set.
    pub set: &'a str,
}

/// What a planner order binds, read outside any transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerInputs {
    /// Each input's name and canonical JSON bytes, in order: `phase`,
    /// `stories`, `description`, `capacity` when one is set,
    /// `approved_items`, then for a revision `draft` and `questions`.
    pub inputs: Vec<(&'static str, Vec<u8>)>,
    /// The documents the inputs were read from, each at the sequence that
    /// produced it and read before its content, so the issue can confirm
    /// none moved: `roadmap`, each story's `truths`, `phase`, the phase's
    /// `phase_detail`, `phase_plan`, and for a revision the draft's `draft`
    /// and the set's `questions`.
    pub documents: Vec<ObservedDocument>,
}

/// Reads what a planner order of phase `phase` in `project` binds for the
/// selection `stories`, in that order, under `policy`, and for a revision
/// the draft and set `revision` names. A phase detail this project purged is
/// `text-purged`, as the approval of an edit keeping it answers. A fault is
/// the `failed` answer to give.
pub fn read<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    stories: &[u32],
    policy: &EffectivePolicy,
    revision: Option<Revision<'_>>,
) -> Result<PlannerInputs, Value> {
    read_with(store, project, phase, stories, policy, revision, None)
}

/// Reads what the revision order a set's close issues binds, as [`read`]
/// reads a revision, but with its `questions` input built from `closed`,
/// the set's facts with the closing call's own answers and deferrals, which
/// are not yet recorded (D-15). The closing call's recording observes the
/// set's document, so it is not among the documents read here.
pub fn read_closing<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    stories: &[u32],
    policy: &EffectivePolicy,
    revision: Revision<'_>,
    closed: &SetFacts,
) -> Result<PlannerInputs, Value> {
    read_with(
        store,
        project,
        phase,
        stories,
        policy,
        Some(revision),
        Some(closed),
    )
}

/// [`read`], with a revision's set facts taken from `closed` when given.
fn read_with<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    stories: &[u32],
    policy: &EffectivePolicy,
    revision: Option<Revision<'_>>,
    closed: Option<&SetFacts>,
) -> Result<PlannerInputs, Value> {
    // Every sequence is read before the content it covers, so a document
    // that moves before its content is read shows as moved at the issue.
    let StoriesRead {
        entries: committed,
        roadmap: accepted,
        mut documents,
    } = read_stories(store, project, stories)?;
    let phase_seq = produced(store, project, PHASE_VIEW, phase_key(phase))?;
    let detail =
        read::detail_document(store, project, phase).map_err(|error| scope_unread(&error))?;
    documents.push(observed(PHASE_VIEW, phase_key(phase), phase_seq));
    if let Some((key, seq)) = detail {
        documents.push(observed(PHASE_DETAIL_VIEW, key, Some(seq)));
    }
    let phase_plan = store
        .get(project, PHASE_PLAN_VIEW, &phase_plan_key(phase))
        .map_err(busy)?;
    documents.push(observed(
        PHASE_PLAN_VIEW,
        phase_plan_key(phase),
        phase_plan.as_ref().map(|document| document.produced_seq),
    ));
    let phase_plan = phase_plan.map(|document| document.body);

    let held = read::phase(store, project, phase).map_err(|error| scope_unread(&error))?;
    let detail = match held.detail {
        Detail::Text(text) => text,
        Detail::Purged(_) => return Err(detail_purged(phase)),
    };
    let description = accepted
        .description
        .as_ref()
        .ok_or_else(|| unreadable(ROADMAP_VIEW))?;

    let mut inputs = vec![
        (
            PHASE_INPUT,
            encoded(&json!({"number": phase, "goal": held.goal, "detail": detail}))?,
        ),
        (STORIES_INPUT, encoded(&committed)?),
        (DESCRIPTION_INPUT, encoded(description)?),
    ];
    if let Some(capacity) = capacity(policy) {
        let size = match &phase_plan {
            Some(document) => document
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?,
            None => 0,
        };
        inputs.push((
            CAPACITY_INPUT,
            encoded(&json!({"capacity": capacity.get(), "size": size}))?,
        ));
    }
    let items = in_force_items(store, project, phase, phase_plan.as_ref())?;
    inputs.push((APPROVED_ITEMS_INPUT, encoded(&items)?));

    if let Some(revision) = revision {
        let (draft, draft_seq, set, set_seq) = revised(store, project, revision, closed)?;
        inputs.push((DRAFT_INPUT, encoded(&draft)?));
        inputs.push((QUESTIONS_INPUT, encoded(&set)?));
        documents.push(observed(
            DRAFT_VIEW,
            draft_key(revision.draft),
            Some(draft_seq),
        ));
        if let Some(set_seq) = set_seq {
            documents.push(observed(
                QUESTIONS_VIEW,
                questions_key(revision.set),
                Some(set_seq),
            ));
        }
    }
    Ok(PlannerInputs { inputs, documents })
}

/// The `stories` input's entries for a selection, read with the roadmap
/// and the documents they came from.
pub(crate) struct StoriesRead {
    /// Each story in the selection's order, `{id, sentence, truths}`, its
    /// truths its current ones with the members a planner reads.
    pub entries: Vec<Value>,
    /// The roadmap the sentences came from.
    pub roadmap: Roadmap,
    /// The `roadmap` document, then each story's `truths` document, each at
    /// the sequence that produced it.
    pub documents: Vec<ObservedDocument>,
}

/// Reads the `stories` input's entries for `stories` of `project`, in that
/// order: each story's sentence from the roadmap and its current truths.
/// The planner's `stories` input and the checker's `truths` input both take
/// their entries from here, so a checker reads a story as the planner did.
/// A story the roadmap no longer holds means it moved since the caller
/// judged the selection. A fault is the `failed` answer to give.
pub(crate) fn read_stories<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    stories: &[u32],
) -> Result<StoriesRead, Value> {
    let roadmap_seq = produced(store, project, ROADMAP_VIEW, roadmap_key())?;
    let mut documents = vec![observed(ROADMAP_VIEW, roadmap_key(), roadmap_seq)];
    let mut truths = Vec::with_capacity(stories.len());
    for story in stories {
        let document = store
            .get(project, TRUTHS_VIEW, &truths_key(*story))
            .map_err(busy)?;
        documents.push(observed(
            TRUTHS_VIEW,
            truths_key(*story),
            document.as_ref().map(|document| document.produced_seq),
        ));
        truths.push(document.map(|document| document.body));
    }
    let roadmap = read::roadmap(store, project).map_err(|error| scope_unread(&error))?;
    let mut entries = Vec::with_capacity(stories.len());
    for (story, truths) in stories.iter().zip(&truths) {
        let entry = roadmap
            .stories
            .iter()
            .find(|entry| entry.id == *story)
            .ok_or_else(moved)?;
        entries.push(json!({
            "id": entry.id,
            "sentence": entry.sentence,
            "truths": current_truths(truths.as_ref())?,
        }));
    }
    Ok(StoriesRead {
        entries,
        roadmap,
        documents,
    })
}

/// The capacity `policy` sets, or none.
fn capacity(policy: &EffectivePolicy) -> Option<NonZeroU32> {
    match policy
        .settings
        .get(CAPACITY_SETTING)
        .and_then(|effective| effective.value.as_ref())
    {
        Some(Setting::Capacity(capacity)) => Some(*capacity),
        _ => None,
    }
}

/// A story's current truths from its `truths` document, each with the
/// members a planner reads, or none before its first refinement.
fn current_truths(document: Option<&Value>) -> Result<Vec<Value>, Value> {
    let Some(document) = document else {
        return Ok(Vec::new());
    };
    let listed = document
        .get("truths")
        .and_then(Value::as_array)
        .ok_or_else(|| unreadable(TRUTHS_VIEW))?;
    listed
        .iter()
        .map(|truth| {
            let mut carried = Map::new();
            for name in TRUTH_MEMBERS {
                let value = truth.get(name).ok_or_else(|| unreadable(TRUTHS_VIEW))?;
                carried.insert(name.into(), value.clone());
            }
            match truth.get("human_verify") {
                None | Some(Value::Null) => {}
                Some(value) => {
                    carried.insert("human_verify".into(), value.clone());
                }
            }
            Ok(Value::Object(carried))
        })
        .collect()
}

/// The evidence items of the plans in force that `phase_plan`, phase
/// `phase`'s document, lists, in plan-number order, each `{plan, item}`,
/// read through the judge's reading of each plan's `evidence_map` document.
fn in_force_items<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    phase_plan: Option<&Value>,
) -> Result<Vec<Value>, Value> {
    let Some(document) = phase_plan else {
        return Ok(Vec::new());
    };
    let plans = document
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
    let mut items = Vec::new();
    // The view keeps a phase's plans in number order.
    for held in plans {
        if held.get("state").and_then(Value::as_str) != Some(IN_FORCE) {
            continue;
        }
        let plan = held
            .get("plan")
            .and_then(Value::as_u64)
            .and_then(|plan| u32::try_from(plan).ok())
            .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
        let map = store
            .get(project, EVIDENCE_MAP_VIEW, &evidence_map_key(phase, plan))
            .map_err(busy)?
            .ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?;
        let approved = approved_items(&map.body).ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?;
        items.extend(
            approved
                .into_iter()
                .map(|item| json!({"plan": item.plan, "item": item.definition})),
        );
    }
    Ok(items)
}

/// A revision's draft plan and its set's body, with the sequences that
/// produced the `draft` and `questions` documents, each read before its
/// content. With `closed` given, the set's body is built from it and the
/// set is not read, so no `questions` sequence comes back.
fn revised<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    revision: Revision<'_>,
    closed: Option<&SetFacts>,
) -> Result<(Value, u64, Value, Option<u64>), Value> {
    let draft_seq = produced(store, project, DRAFT_VIEW, draft_key(revision.draft))?
        .ok_or_else(|| unreadable(DRAFT_VIEW))?;
    let draft = match read::preview(store, project, revision.draft) {
        Ok(preview) if preview.kind == PLAN_KIND => preview.submission,
        Ok(_) | Err(PreviewError::UnknownDraft | PreviewError::Unreadable(_)) => {
            return Err(unreadable(DRAFT_VIEW));
        }
        Err(PreviewError::Store(error)) => return Err(busy(error)),
    };
    if let Some(closed) = closed {
        return Ok((draft, draft_seq, questions_body(revision.set, closed), None));
    }
    let held = match read_set(store, project, revision.set).map_err(busy)? {
        SetRead::Held(held) => held,
        SetRead::NotHeld | SetRead::Unreadable(_) => return Err(unreadable(QUESTIONS_VIEW)),
    };
    let set = questions_body(revision.set, &held.facts);
    Ok((draft, draft_seq, set, Some(held.produced_seq)))
}

/// The sequence that produced the document `key` of `view`, or none.
fn produced<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: baley_store::DocKey,
) -> Result<Option<u64>, Value> {
    Ok(store
        .get(project, view, &key)
        .map_err(busy)?
        .map(|document| document.produced_seq))
}

fn observed(view: &str, key: baley_store::DocKey, produced_seq: Option<u64>) -> ObservedDocument {
    ObservedDocument {
        view: view.into(),
        key,
        produced_seq,
    }
}

fn encoded<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Value> {
    bytes(value).map_err(|error| scope_unread(&error))
}

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
}

fn moved() -> Value {
    failed(
        LEDGER_BUSY,
        "the project's scope moved while this call was prepared, so nothing was recorded. The same call can be sent again",
        PLACE_LEDGER,
    )
}

fn detail_purged(phase: u32) -> Value {
    failed(
        TEXT_PURGED,
        format!(
            "phase {phase}'s detail was purged from this project, so no planner order can carry it and nothing was recorded. An approved phase edit can give it a new detail"
        ),
        "detail",
    )
}

fn unreadable(view: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("the {view} view document cannot be read, so nothing was recorded"),
        PLACE_LEDGER,
    )
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected inputs are written out from design 0005,
    //! D-12, D-15 and the plan's data store C, P0, P2, E1, E2 and PL2, never
    //! from running this code.

    use baley_core::plan::PlanDraft;
    use baley_core::plan::{CommittedStory, CurrentTruth, PLAN_REPLACED, PLAN_REPLACED_VERSION};
    use baley_core::plan::{PlanFacts, judge_plan, phase_stream, replaced_payload};
    use baley_core::questions::answered_payload;
    use baley_core::questions::{QUESTION_ANSWERED, QUESTION_ANSWERED_VERSION, Subject};
    use baley_core::scope::submitted_payload;
    use baley_core::scope::{PROJECT_STREAM, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION};
    use baley_store::{Decision, NewEvent, Observed, OutcomeKind, RetentionClass, StreamName};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::scope::fixtures::*;

    fn parsed(read: &PlannerInputs, name: &str) -> Option<Value> {
        read.inputs
            .iter()
            .find(|(input, _)| *input == name)
            .map(|(_, bytes)| serde_json::from_slice(bytes).unwrap())
    }

    fn names(read: &PlannerInputs) -> Vec<&str> {
        read.inputs.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn a_planner_order_bound_to_the_phase_as_stored_instead_of_the_selection_is_caught() {
        let (_dir, store) = store_c();
        let read = read(&store, &project(), 1, &[1, 4], &p0(), None).unwrap();

        assert_eq!(
            names(&read),
            ["phase", "stories", "description", "approved_items"]
        );
        assert_eq!(
            parsed(&read, "phase"),
            Some(json!({"number": 1, "goal": "Steps are recorded",
                "detail": "The ledger and its views"}))
        );
        let t1 = json!({"id": "t1", "version": 1, "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal"});
        let t2 = json!({"id": "t2", "version": 1, "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property"});
        assert_eq!(
            parsed(&read, "stories"),
            Some(json!([
                {"id": 1, "sentence": "Every step is recorded", "truths": [t1, t2.clone()]},
                {"id": 4, "sentence": "Plans are approved by digest", "truths": [t2]}]))
        );
        assert_eq!(
            parsed(&read, "description"),
            Some(json!({"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]}))
        );
        assert_eq!(parsed(&read, "capacity"), None);

        let read = super::read(&store, &project(), 1, &[1, 4], &p2(), None).unwrap();
        assert_eq!(
            parsed(&read, "capacity"),
            Some(json!({"capacity": 2, "size": 0}))
        );
    }

    #[test]
    fn an_approved_plans_items_left_out_of_a_planner_order_or_a_replaced_ones_kept_is_caught() {
        let (_dir, store) = store_c();
        let x = Hash([0x5c; 32]);
        let y = Hash([0x6d; 32]);
        approve_by_hand(&store, R1, 1, 1, x, &[1], 1, &[e1()], Vec::new());
        let items = |store: &SqliteStore| {
            let read = read(store, &project(), 1, &[1], &p0(), None).unwrap();
            parsed(&read, "approved_items").expect("the items are bound")
        };
        assert_eq!(items(&store), json!([{"plan": 1, "item": e1()}]));

        let replaced = NewEvent {
            stream: StreamName(phase_stream(1)),
            type_name: PLAN_REPLACED.into(),
            type_version: PLAN_REPLACED_VERSION,
            git: None,
            payload: replaced_payload(1, 1, x, y, "Ada Owner", T5),
            attachments: Vec::new(),
        };
        approve_by_hand(&store, R2, 1, 2, y, &[1], 1, &[e2()], vec![replaced]);
        assert_eq!(items(&store), json!([{"plan": 2, "item": e2()}]));
    }

    /// The facts PL2 is judged under: story 1 with T1 and T2 at version 1.
    fn pl2_facts() -> PlanFacts {
        let truth = |id: &str, trigger: &str, observer: &str, outcome: &str| CurrentTruth {
            id: id.into(),
            version: 1,
            trigger: trigger.into(),
            observer: observer.into(),
            outcome: outcome.into(),
            human_verify: None,
        };
        PlanFacts {
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
        }
    }

    #[test]
    fn a_revision_order_bound_without_its_draft_or_its_answers_is_caught() {
        let (_dir, store) = store_c();
        let plan = judge_plan(&pl2(), &pl2_facts()).expect("PL2 is accepted");
        let draft = PlanDraft::new(&plan);
        let mut set = String::new();
        store
            .transact(&command("test.plan", R1, 8, T4), &mut |tx| {
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
            .unwrap();
        append(
            &store,
            R2,
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

        let revision = Revision {
            draft: draft.digest(),
            set: &set,
        };
        let read = read(&store, &project(), 1, &[1], &p0(), Some(revision)).unwrap();
        assert_eq!(
            names(&read),
            [
                "phase",
                "stories",
                "description",
                "approved_items",
                "draft",
                "questions"
            ]
        );
        let recorded: Value = serde_json::from_slice(draft.body()).unwrap();
        assert_eq!(parsed(&read, "draft"), Some(recorded));
        let questions = parsed(&read, "questions").expect("the set is bound");
        assert_eq!(questions["set"], set.as_str());
        assert_eq!(
            questions["answers"],
            json!([{"question": "q1", "answer": "Postgres", "accepted": false, "round": 1,
                "owner": "Ada Owner", "at": T5}])
        );
    }
}
