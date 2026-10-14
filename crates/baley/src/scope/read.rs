//! Scope read back from its views (design 0004 sections 5 and 6). A draft
//! is read by its digest, as the latest submission of that digest recorded
//! it, so the owner sees exactly what an approval would record; phase
//! numbers and story ids are assigned at approval, so a preview has none.
//! Accepted scope is read from the `roadmap`, `phase` and `backlog` views,
//! which only an approval changes. `document` and the owner's `baley scope`,
//! `phase`, `backlog` and `story` commands read through here. Nothing is
//! appended.

use std::io::Read;

use baley_core::scope::{
    BACKLOG_VIEW, DRAFT_VIEW, PHASE_VIEW, ROADMAP_VIEW, backlog_key, draft_key, phase_key,
    roadmap_key,
};
use baley_store::{Document, Hash, PayloadBody, Payloads, ProjectId, StoreError, Views};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The code for a story id the accepted roadmap does not hold (PRJ-R18).
pub const NO_SUCH_STORY: &str = "no-such-story";

/// A draft as its latest submission recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The change kind, such as `start`.
    pub kind: String,
    /// The submission's digest.
    pub digest: Hash,
    /// The sequence of the latest submission of this digest.
    pub seq: u64,
    /// When the latest submission of this digest was recorded.
    pub recorded_at: String,
    /// The submission as recorded.
    pub submission: Value,
}

/// Why a preview could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// No submission has this digest, answered `unknown-draft`.
    UnknownDraft,
    /// The draft is recorded, but its document or body cannot be read: a
    /// body no longer held, or one that is not JSON. Callers answer it
    /// `failed` `ledger-unavailable`, never as a missing draft.
    Unreadable(String),
    /// The store's own error.
    Store(StoreError),
}

/// Reads the draft with `digest` in `project`: its `draft` document, then
/// its body by hash, parsed as JSON. Nothing is appended.
pub fn preview<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Preview, PreviewError> {
    match store.get(project, DRAFT_VIEW, &draft_key(digest)) {
        Ok(Some(document)) => preview_of(store, &document),
        Ok(None) => Err(PreviewError::UnknownDraft),
        Err(error) => Err(PreviewError::Store(error)),
    }
}

/// The preview a `draft` document stands for, its body opened through
/// `payloads`.
pub(crate) fn preview_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<Preview, PreviewError> {
    let unreadable = || PreviewError::Unreadable("the draft's view document cannot be read".into());
    let body = &document.body;
    let text = |name: &str| body.get(name).and_then(Value::as_str);
    let kind = text("kind").ok_or_else(unreadable)?;
    let digest = text("digest")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    let seq = body
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(unreadable)?;
    let recorded_at = text("recorded_at").ok_or_else(unreadable)?;
    let hash = text("hash")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    let mut reader = match payloads.open(&hash).map_err(PreviewError::Store)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => {
            return Err(PreviewError::Unreadable(
                "the draft is recorded, but its body is no longer held".into(),
            ));
        }
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| PreviewError::Store(StoreError::Unavailable(error.to_string())))?;
    let submission = serde_json::from_slice(&bytes).map_err(|_| {
        PreviewError::Unreadable("the draft is recorded, but its body is not JSON".into())
    })?;
    Ok(Preview {
        kind: kind.into(),
        digest,
        seq,
        recorded_at: recorded_at.into(),
        submission,
    })
}

/// The accepted project description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    /// What the project is.
    pub what: String,
    /// The one thing it must get right.
    pub core_value: String,
    /// Its constraints, possibly none.
    pub constraints: Vec<String>,
}

/// One accepted phase as the roadmap lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapPhase {
    /// Its number, assigned at approval and never reused.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its status, such as `declared`.
    pub status: String,
    /// The numbers of the phases it depends on.
    pub depends_on: Vec<u32>,
    /// The ids of its stories.
    pub stories: Vec<u32>,
}

/// One accepted story as the roadmap lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    /// Its id, assigned at approval.
    pub id: u32,
    /// The story itself.
    pub sentence: String,
    /// Its status, such as `active` or `deferred`.
    pub status: String,
    /// The number of the phase it is committed to, or none in the backlog.
    pub phase: Option<u32>,
    /// How many truths it carries.
    pub truths: u64,
}

/// The accepted roadmap: the description, the phases in roadmap order and
/// the stories in priority order. A project with no approved scope has no
/// description, phase or story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Roadmap {
    /// The project description, or none before the first approval.
    pub description: Option<Description>,
    /// The phases in roadmap order.
    pub phases: Vec<RoadmapPhase>,
    /// The stories in priority order.
    pub stories: Vec<Story>,
}

/// One accepted phase with its detail text, read from its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// Its number.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its status, such as `declared`.
    pub status: String,
    /// Its detail text, as approved.
    pub detail: String,
    /// The numbers of the phases it depends on.
    pub depends_on: Vec<u32>,
    /// The ids of its stories.
    pub stories: Vec<u32>,
}

/// One story as the backlog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogStory {
    /// Its id.
    pub id: u32,
    /// The story itself.
    pub sentence: String,
    /// Its status.
    pub status: String,
    /// The number of the phase it is committed to, or none.
    pub phase: Option<u32>,
    /// Its refinement state, such as `unrefined`.
    pub refinement: String,
    /// How many truths it carries.
    pub truths: u64,
    /// The number of tasks its approved plans need, or none before planning.
    pub size: Option<u64>,
}

/// Why accepted scope could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// No accepted phase has this number, answered `no-such-phase`.
    NoSuchPhase,
    /// No accepted story has this id, answered `no-such-story`.
    NoSuchStory,
    /// The phase is recorded, but its detail body is no longer held.
    /// Callers answer it `failed` `ledger-unavailable`, never as a phase
    /// with empty or missing detail.
    BodyGone,
    /// A view document or body cannot be read as what it should hold.
    Unreadable(String),
    /// The store's own error.
    Store(StoreError),
}

/// The `roadmap` document as the projector writes it; the members a read
/// does not need are left out.
#[derive(Deserialize)]
struct RoadmapDocument {
    description: Option<Description>,
    phases: Vec<RoadmapPhase>,
    order: Vec<u32>,
    stories: Vec<Story>,
}

/// A `phase` document: its detail is the hex hash of the body.
#[derive(Deserialize)]
struct PhaseDocument {
    number: u32,
    name: String,
    goal: String,
    status: String,
    detail: String,
    depends_on: Vec<u32>,
    stories: Vec<u32>,
}

/// The `backlog` document.
#[derive(Deserialize)]
struct BacklogDocument {
    stories: Vec<BacklogStory>,
}

/// Reads `project`'s accepted roadmap from its `roadmap` document, the
/// phases in the document's `order`. No document is a project with no
/// approved scope.
pub fn roadmap<S: Views + ?Sized>(store: &S, project: &ProjectId) -> Result<Roadmap, ReadError> {
    let Some(document) = read::<RoadmapDocument, _>(store, project, ROADMAP_VIEW, &roadmap_key())?
    else {
        return Ok(Roadmap {
            description: None,
            phases: Vec::new(),
            stories: Vec::new(),
        });
    };
    // Every phase is in the order exactly once; anything else is a broken
    // document, never a roadmap with a phase dropped.
    if document.order.len() != document.phases.len() {
        return Err(unreadable(ROADMAP_VIEW));
    }
    let mut phases = Vec::with_capacity(document.order.len());
    for number in &document.order {
        let phase = document
            .phases
            .iter()
            .find(|phase| phase.number == *number)
            .ok_or_else(|| unreadable(ROADMAP_VIEW))?;
        if phases.contains(phase) {
            return Err(unreadable(ROADMAP_VIEW));
        }
        phases.push(phase.clone());
    }
    Ok(Roadmap {
        description: document.description,
        phases,
        stories: document.stories,
    })
}

/// Reads accepted phase `number` from the `phase` view, with its detail
/// text from its body.
pub fn phase<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    number: u32,
) -> Result<Phase, ReadError> {
    let document = read::<PhaseDocument, _>(store, project, PHASE_VIEW, &phase_key(number))?
        .ok_or(ReadError::NoSuchPhase)?;
    let hash = Hash::from_hex(&document.detail).ok_or_else(|| unreadable(PHASE_VIEW))?;
    let mut reader = match store.open(&hash).map_err(ReadError::Store)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => return Err(ReadError::BodyGone),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| ReadError::Store(StoreError::Unavailable(error.to_string())))?;
    // The body was stored from the submission's UTF-8 text.
    let detail = String::from_utf8(bytes).map_err(|_| {
        ReadError::Unreadable("the phase is recorded, but its detail body is not text".into())
    })?;
    Ok(Phase {
        number: document.number,
        name: document.name,
        goal: document.goal,
        status: document.status,
        detail,
        depends_on: document.depends_on,
        stories: document.stories,
    })
}

/// Reads `project`'s backlog from its `backlog` document, in priority order.
/// No document is an empty backlog.
pub fn backlog<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
) -> Result<Vec<BacklogStory>, ReadError> {
    Ok(
        read::<BacklogDocument, _>(store, project, BACKLOG_VIEW, &backlog_key())?
            .map(|document| document.stories)
            .unwrap_or_default(),
    )
}

/// Reads accepted story `id` from the roadmap.
pub fn story<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    id: u32,
) -> Result<Story, ReadError> {
    roadmap(store, project)?
        .stories
        .into_iter()
        .find(|story| story.id == id)
        .ok_or(ReadError::NoSuchStory)
}

/// One view document read as `T`, or none when the view holds no such key.
fn read<T: for<'de> Deserialize<'de>, S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: &baley_store::DocKey,
) -> Result<Option<T>, ReadError> {
    match store.get(project, view, key).map_err(ReadError::Store)? {
        Some(document) => serde_json::from_value(document.body)
            .map(Some)
            .map_err(|_| unreadable(view)),
        None => Ok(None),
    }
}

fn unreadable(view: &str) -> ReadError {
    ReadError::Unreadable(format!("the {view} view document cannot be read"))
}

#[cfg(test)]
mod tests {
    use baley_core::scope::{
        BACKLOG_VIEW, PHASE_STATUS_INDEX, PHASE_VIEW, ROADMAP_VIEW, backlog_key, roadmap_key,
    };
    use baley_store::{IndexQuery, KeyValue, PageRequest};

    use super::*;
    use crate::scope::fixtures::*;

    fn digest_of(recorded: &baley_store::Recorded) -> Hash {
        Hash::from_hex(receipt(recorded)["digest"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_draft_lost_on_reopening_or_projected_as_accepted_scope_is_caught() {
        let (dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());

        assert!(
            store
                .get(&project(), ROADMAP_VIEW, &roadmap_key())
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get(&project(), BACKLOG_VIEW, &backlog_key())
                .unwrap()
                .is_none()
        );
        let declared = IndexQuery {
            index: PHASE_STATUS_INDEX.into(),
            equals: vec![KeyValue::Text("declared".into())],
            page: PageRequest {
                limit: 10,
                after: None,
            },
        };
        assert!(
            store
                .find(&project(), PHASE_VIEW, &declared)
                .unwrap()
                .items
                .is_empty()
        );

        drop(store);
        let reopened = crate::ledger::open::store(
            &dir.path().join("home"),
            T2,
            crate::ledger::open::options(),
        )
        .unwrap();
        let read = preview(&reopened, &project(), digest).unwrap();
        assert_eq!(read.kind, "start");
        assert_eq!(read.digest, digest);
        assert_eq!(read.recorded_at, T1);
        assert_eq!(read.submission, s1());
    }

    #[test]
    fn an_unknown_digest_read_as_an_empty_draft_is_caught() {
        let (_dir, store) = store();
        assert_eq!(
            preview(&store, &project(), Hash([0; 32])),
            Err(PreviewError::UnknownDraft)
        );
    }

    #[test]
    fn a_resubmitted_draft_read_with_its_first_time_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &m(), R1, 1, T1).unwrap());
        submit(&store, &m(), R2, 1, T2).unwrap();
        assert_eq!(preview(&store, &project(), digest).unwrap().recorded_at, T2);
    }

    /// A store holding S1, submitted at T1 and approved at T3.
    fn approved() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let (dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        approve_draft(&store, digest);
        (dir, store)
    }

    fn text(value: &str) -> String {
        value.into()
    }

    /// Approved S1's stories as the plan's Notes give them.
    fn s1_stories() -> Vec<Story> {
        let story = |id, sentence: &str, status: &str, phase| Story {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            truths: 0,
        };
        vec![
            story(1, "Every step is recorded", "active", Some(1)),
            story(2, "The owner approves scope", "active", Some(2)),
            story(3, "Teammates see the backlog", "deferred", None),
        ]
    }

    /// Appends a `phase.reordered` giving `order` under a test command, as
    /// a later reorder would leave it.
    fn reorder(store: &baley_store_sqlite::SqliteStore, order: &[u32]) {
        use baley_core::scope::{PHASE_REORDERED, ROADMAP_STREAM, phase_reordered_payload};
        use baley_store::{Decision, NewEvent, Observed, OutcomeKind, StreamName};

        let command = baley_store::Command {
            project: project(),
            kind: baley_store::CommandKind("test.reorder".into()),
            request_id: baley_store::RequestId("4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a".into()),
            digest: Hash([0xdd; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T3.into(),
            actor: baley_store::Actor::Owner,
            caller: None,
        };
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(ROADMAP_STREAM.into()),
                    type_name: PHASE_REORDERED.into(),
                    type_version: 1,
                    git: None,
                    payload: phase_reordered_payload(order, Hash([0x5c; 32])),
                    attachments: Vec::new(),
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: serde_json::json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }

    #[test]
    fn a_roadmap_read_out_of_order_or_missing_a_story_is_caught() {
        let (_dir, store) = approved();
        // The view keeps phases by number, so only an order other than
        // 1, 2 tells the recorded order from the stored one.
        reorder(&store, &[2, 1]);
        let read = roadmap(&store, &project()).unwrap();
        assert_eq!(
            read.description,
            Some(Description {
                what: text("A ledger for agent work"),
                core_value: text("Nothing moves on an unproven claim"),
                constraints: vec![text("One binary")],
            })
        );
        assert_eq!(
            read.phases,
            [
                RoadmapPhase {
                    number: 2,
                    name: text("Approval"),
                    goal: text("The owner approves"),
                    status: text("declared"),
                    depends_on: vec![1],
                    stories: vec![2],
                },
                RoadmapPhase {
                    number: 1,
                    name: text("Records"),
                    goal: text("Steps are recorded"),
                    status: text("declared"),
                    depends_on: vec![],
                    stories: vec![1],
                },
            ]
        );
        assert_eq!(read.stories, s1_stories());
    }

    #[test]
    fn a_phase_read_without_its_detail_text_or_an_unknown_phase_read_as_empty_is_caught() {
        let (_dir, store) = approved();
        assert_eq!(
            phase(&store, &project(), 2),
            Ok(Phase {
                number: 2,
                name: text("Approval"),
                goal: text("The owner approves"),
                status: text("declared"),
                detail: text("Approval by digest"),
                depends_on: vec![1],
                stories: vec![2],
            })
        );
        assert_eq!(phase(&store, &project(), 3), Err(ReadError::NoSuchPhase));

        let held = store
            .get(&project(), PHASE_VIEW, &phase_key(2))
            .unwrap()
            .unwrap();
        let hash = Hash::from_hex(held.body["detail"].as_str().unwrap()).unwrap();
        let purge = baley_store::Command {
            project: project(),
            kind: baley_store::CommandKind("payload.purge".into()),
            request_id: baley_store::RequestId("5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b".into()),
            digest: Hash([9; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T3.into(),
            actor: baley_store::Actor::Owner,
            caller: None,
        };
        baley_store::Admin::purge(&store, &purge, &[hash], "pasted a secret").unwrap();
        assert_eq!(phase(&store, &project(), 2), Err(ReadError::BodyGone));
    }

    #[test]
    fn a_story_read_from_a_draft_or_an_unknown_story_read_as_empty_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());
        assert_eq!(story(&store, &project(), 1), Err(ReadError::NoSuchStory));

        approve_draft(&store, digest);
        assert_eq!(
            story(&store, &project(), 3),
            Ok(Story {
                id: 3,
                sentence: text("Teammates see the backlog"),
                status: text("deferred"),
                phase: None,
                truths: 0,
            })
        );
        assert_eq!(story(&store, &project(), 4), Err(ReadError::NoSuchStory));
    }

    #[test]
    fn a_project_with_no_approved_scope_refused_instead_of_read_as_empty_is_caught() {
        let (_dir, store) = store();
        assert_eq!(
            roadmap(&store, &project()),
            Ok(Roadmap {
                description: None,
                phases: vec![],
                stories: vec![],
            })
        );
        assert_eq!(backlog(&store, &project()), Ok(vec![]));
    }

    #[test]
    fn a_backlog_read_out_of_priority_order_or_without_its_phases_is_caught() {
        let (_dir, store) = approved();
        let listed = |id, sentence: &str, status: &str, phase| BacklogStory {
            id,
            sentence: text(sentence),
            status: text(status),
            phase,
            refinement: text("unrefined"),
            truths: 0,
            size: None,
        };
        assert_eq!(
            backlog(&store, &project()),
            Ok(vec![
                listed(1, "Every step is recorded", "active", Some(1)),
                listed(2, "The owner approves scope", "active", Some(2)),
                listed(3, "Teammates see the backlog", "deferred", None),
            ])
        );
    }
}
