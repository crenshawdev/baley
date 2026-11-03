//! `phase plan` (design 0005 section 5, PLN-R7, D-01, D-02, D-08 to D-11,
//! D-15): a session asks for one phase's planner with an ordered selection of
//! stories. The check runs before admission and records nothing: it gathers
//! the facts the commitment judge decides on, with the documents they came
//! from, and judges them; for a changed selection it also judges the `phase
//! edit` of the stories and checks it against accepted scope, so the edit's
//! own refusals come before admission too. The run then takes the judged
//! path: a changed selection is recorded as that edit with its planning
//! continuation and answered with the draft and the commands that preview
//! and approve it, issuing no order; an unchanged one issues a planning
//! order, or the pending revision when an answer rejected a recommendation;
//! and the order a commitment's approval issued is handed over.
//!
//! Every recording observes the documents the check read, so anything moved
//! since makes the call `ledger-busy` with nothing recorded. The commitment's
//! approval rechecks a continuation's selection through the same gathering
//! ([`read_readiness`]).

use baley_core::plan::{
    ACTIVE, CommitmentFacts, CommitmentRefusal, Continuation, PHASE_PLAN_STATUS_INDEX,
    PHASE_PLAN_VIEW, PhasePlanPath, Readiness, SelectedStory, judge_commitment, phase_plan_key,
};
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host};
use baley_core::questions::{QUESTIONS_VIEW, SetState, questions_key};
use baley_core::scope::{ChangeKind, PhaseFields, ROADMAP_VIEW, judge_change, roadmap_key};
use baley_core::truth::{TRUTHS_VIEW, truths_key};
use baley_core::work_order::PENDING;
use baley_store::{
    Absence, Command, Decision, DocKey, Hash, IndexQuery, KeyValue, Ledger, Observed,
    ObservedDocument, OutcomeKind, PageRequest, Payloads, ProjectId, StoreError, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::checker::{checker_absences, checker_order_pending, pending_checker};
use crate::planning::inputs::Revision;
use crate::planning::order::{
    self, HandOver, pending_planner, planner_absences, planner_order_pending, read_hand_over,
};
use crate::questions::read::{SetRead, read as read_set};
use crate::scope::read;
use crate::scope::submit::{
    NOT_RECORDED_BUSY, PlanningContinuation, check_change, scope_unread, submission_refused,
    submit_planning_edit,
};
use crate::work_order::issue;

/// The command kind a `phase-plan` call is recorded under.
pub const PHASE_PLAN_COMMAND: &str = "phase.plan";

/// The status a withdrawn phase holds in the roadmap.
const WITHDRAWN: &str = "withdrawn";
/// The status a dropped story holds in the roadmap.
const DROPPED: &str = "dropped";
/// The `phase_plan` view's page bound, so the active phases are read in as
/// few pages as it allows.
const ACTIVE_PAGE: u32 = 100;

/// A selection's readiness facts with the documents they came from, read
/// outside any transaction. `phase-plan`'s check and the commitment's
/// approval both gather them here, so the approval rechecks the same facts
/// the call was judged on (D-01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessRead {
    /// The phase being planned.
    phase: u32,
    /// Whether the roadmap holds the phase and it is not withdrawn.
    phase_held: bool,
    /// The phase's committed stories, in order, none when it is not held.
    committed: Vec<u32>,
    /// The active phases from `phase_plan`'s status index.
    active: Vec<u32>,
    /// The selection, in order, as the roadmap and `truths` hold it.
    stories: Vec<SelectedStory>,
    /// The `roadmap` document, then each selected story's `truths`
    /// document, each at the sequence that produced it.
    documents: Vec<ObservedDocument>,
}

impl ReadinessRead {
    /// The facts `judge_readiness` takes.
    pub fn readiness(&self) -> Readiness<'_> {
        Readiness {
            phase: self.phase,
            phase_held: self.phase_held,
            active: &self.active,
            stories: &self.stories,
        }
    }

    /// The phase's committed stories, in order.
    pub fn committed(&self) -> &[u32] {
        &self.committed
    }

    /// The documents the facts were read from.
    pub fn documents(&self) -> &[ObservedDocument] {
        &self.documents
    }

    /// What a recording relies on being absent. An empty read of the active
    /// phases leaves no document to observe, so when none was active a
    /// recording declares that `phase_plan`'s status index still holds no
    /// active phase, and a phase made active since makes it `ledger-busy`
    /// (D-02). With one active, the readiness judgement names it, so
    /// nothing is declared.
    pub fn absences(&self) -> Vec<Absence> {
        if !self.active.is_empty() {
            return Vec::new();
        }
        vec![Absence::Documents {
            view: PHASE_PLAN_VIEW.into(),
            index: PHASE_PLAN_STATUS_INDEX.into(),
            equals: active_prefix(),
        }]
    }
}

/// Gathers the readiness of `stories`, in that order, for phase `phase` of
/// `project`: the phase and its stories and each selected story's status and
/// phase from the `roadmap` view, whether each holds an approved truth from
/// the `truths` view, and the active phases from `phase_plan`'s status
/// index. Nothing is judged or recorded. A fault is the `failed` answer.
pub fn read_readiness<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    stories: &[u32],
) -> Result<ReadinessRead, Value> {
    // The sequence before the content, so a roadmap that moves after its
    // read shows as moved when the recording confirms it.
    let roadmap_seq = produced(store, project, ROADMAP_VIEW, roadmap_key())?;
    let roadmap = read::roadmap(store, project).map_err(|error| scope_unread(&error))?;
    let mut documents = vec![observed(ROADMAP_VIEW, roadmap_key(), roadmap_seq)];
    let held = roadmap
        .phases
        .iter()
        .find(|held| held.number == phase && held.status != WITHDRAWN);
    let committed = held.map(|held| held.stories.clone()).unwrap_or_default();
    let mut selected = Vec::with_capacity(stories.len());
    for id in stories {
        let truths = store
            .get(project, TRUTHS_VIEW, &truths_key(*id))
            .map_err(busy)?;
        documents.push(observed(
            TRUTHS_VIEW,
            truths_key(*id),
            truths.as_ref().map(|document| document.produced_seq),
        ));
        // An approved truth is one the `truths` document lists (D-10).
        let refined = truths
            .as_ref()
            .and_then(|document| document.body.get("truths"))
            .and_then(Value::as_array)
            .is_some_and(|listed| !listed.is_empty());
        let entry = roadmap
            .stories
            .iter()
            .find(|story| story.id == *id && story.status != DROPPED);
        selected.push(SelectedStory {
            id: *id,
            held: entry.is_some(),
            phase: entry.and_then(|story| story.phase),
            refined,
        });
    }
    Ok(ReadinessRead {
        phase,
        phase_held: held.is_some(),
        committed,
        active: active_phases(store, project)?,
        stories: selected,
        documents,
    })
}

/// The refusal for `refusal`, a readiness judgement's, at `slot`, its reason
/// ending `not_done` and its details naming the story, phase or order.
pub fn readiness_refused(refusal: &CommitmentRefusal, slot: &str, not_done: &str) -> Value {
    let reason = match refusal {
        CommitmentRefusal::PhaseActive { phase } => format!(
            "phase {phase} is active, and one phase is planned at a time, so {not_done}. A phase stays active until it is completed or withdrawn"
        ),
        CommitmentRefusal::NoSuchPhase { phase } => {
            format!("this project has no phase {phase} that can be planned, so {not_done}")
        }
        CommitmentRefusal::NoSuchStory { story } => {
            format!("this project has no story {story} that can be committed, so {not_done}")
        }
        CommitmentRefusal::StoryUnrefined { story } => format!(
            "story {story} holds no approved truth, so it cannot be committed and {not_done}. It is refined with story-refine first"
        ),
        CommitmentRefusal::StoryInPhase { story, phase } => format!(
            "story {story} is committed to phase {phase}, and a story belongs to one phase at most, so {not_done}"
        ),
        CommitmentRefusal::PlannerOrderPending { work_order } => format!(
            "the phase's planner work order {work_order} is still pending, and a phase has one live planner order, so {not_done}"
        ),
    };
    Refusal::new(refusal.code(), reason)
        .slot(slot)
        .details(refusal.details())
        .value()
}

/// What `phase-plan`'s check found, which the run records and observes.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    /// The phase being planned.
    phase: u32,
    /// The selection, in order.
    stories: Vec<u32>,
    /// The path the commitment judge chose.
    path: Planned,
    /// The documents the check read, at the sequence that produced each.
    documents: Vec<ObservedDocument>,
    /// What a recording relies on being absent beside its own.
    absences: Vec<Absence>,
}

/// A judged path, with what its run needs.
#[derive(Debug, Clone, PartialEq)]
enum Planned {
    /// Hand over the order the commitment's approval issued.
    HandOver(Box<HandOver>),
    /// Record the `phase edit` of the stories with its continuation.
    Draft(PhaseFields),
    /// Issue the revision of the phase's current draft and its set.
    Revision { draft: Hash, set: String },
    /// Issue a planning order.
    Planning,
}

/// The latest continuation as the `phase_plan` document holds it.
pub(crate) struct Latest {
    /// The selection it binds, in order.
    pub stories: Vec<u32>,
    /// The `phase edit` draft it was recorded with.
    pub edit: Hash,
    /// The planner order its approval issues.
    pub work_order: String,
    /// Whether that order was handed to a session.
    pub followed: bool,
}

/// `phase-plan`'s check for phase `phase` of `project` with the selection
/// `stories`, run before admission and recording nothing. It gathers the
/// readiness facts, the phase's `phase_plan` document with its latest
/// continuation and current draft, the phase's pending planner order and
/// the continuation's order's state from `work_order`, and whether the
/// current draft's set closed with an answer that rejected a recommendation,
/// then judges them (D-10, D-11, D-15). A refusal names its code at slot
/// `phase`, `stories` or `operation`. Once the commitment judge passes, a
/// pending checker order of the phase refuses every path
/// `checker-order-pending` at slot `operation`, naming it (D-03). A changed
/// selection's `phase edit` is judged and checked against accepted scope
/// here, its refusals at slot `stories`; a hand-over is read here, so an
/// order no longer pending is refused here. A read fault is the `failed` answer.
pub fn check<S: Ledger + Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    stories: &[u32],
) -> Result<Checked, Value> {
    let readiness = read_readiness(store, project, phase, stories)?;
    let mut documents = readiness.documents().to_vec();
    let key = phase_plan_key(phase);
    let phase_plan = store.get(project, PHASE_PLAN_VIEW, &key).map_err(busy)?;
    documents.push(observed(
        PHASE_PLAN_VIEW,
        key,
        phase_plan.as_ref().map(|document| document.produced_seq),
    ));
    let body = phase_plan.map(|document| document.body);
    let pending = pending_planner(store, project, phase).map_err(busy)?;
    let latest = latest_continuation(body.as_ref())?;
    let latest_pending = match &latest {
        Some(latest) => crate::work_order::read::order(store, project, &latest.work_order)
            .map_err(busy)?
            .is_some_and(|order| order.get("state").and_then(Value::as_str) == Some(PENDING)),
        None => false,
    };
    let (revision, revision_due) = match current_draft(body.as_ref())? {
        Some((draft, set)) => match read_set(store, project, &set).map_err(busy)? {
            SetRead::Held(held) => {
                documents.push(observed(
                    QUESTIONS_VIEW,
                    questions_key(&set),
                    Some(held.produced_seq),
                ));
                let due = held.round.state == SetState::Closed
                    && held.facts.answers.iter().any(|answer| !answer.accepted);
                (Some((draft, set)), due)
            }
            SetRead::NotHeld | SetRead::Unreadable(_) => return Err(unreadable(QUESTIONS_VIEW)),
        },
        None => (None, false),
    };
    let facts = CommitmentFacts {
        readiness: readiness.readiness(),
        committed: readiness.committed(),
        pending_order: pending.as_deref(),
        continuation: latest.as_ref().map(|latest| Continuation {
            stories: &latest.stories,
            work_order: &latest.work_order,
            pending: latest_pending,
            followed: latest.followed,
        }),
        revision_due,
    };
    let judged = judge_commitment(&facts).map_err(|refusal| refused(phase, &refusal))?;
    // A blocker's revision is issued only once the phase's checker returns,
    // so no path runs beside a pending checker order (D-03, D-07).
    if let Some(pending) = pending_checker(store, project, phase).map_err(busy)? {
        return Err(checker_order_pending(
            phase,
            &pending,
            "no planner order was issued and nothing was recorded",
        ));
    }
    let mut absences = readiness.absences();
    absences.extend(checker_absences(phase));
    let path = match judged {
        PhasePlanPath::HandOver { work_order } => {
            // The judge hands over only the latest continuation's order.
            let edit = latest
                .as_ref()
                .map(|latest| latest.edit)
                .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
            // The hand-over observes what this check read, so a phase made
            // active or a story unrefined since it is not handed over.
            let read = read_hand_over(store, project, phase, edit, &work_order)?
                .observing(&documents, absences.clone());
            Planned::HandOver(Box::new(read))
        }
        PhasePlanPath::Draft => {
            let members = json!({"phase": phase, "stories": stories});
            let change = judge_change(ChangeKind::PhaseEdit, &members)
                .map_err(|refusal| at_stories(submission_refused(&refusal)))?;
            check_change(store, project, &change).map_err(at_stories)?;
            Planned::Draft(PhaseFields {
                stories: Some(stories.to_vec()),
                ..PhaseFields::default()
            })
        }
        PhasePlanPath::Revision => match revision {
            Some((draft, set)) => Planned::Revision { draft, set },
            // A revision is due only over a current draft.
            None => return Err(unreadable(PHASE_PLAN_VIEW)),
        },
        PhasePlanPath::Planning => Planned::Planning,
    };
    Ok(Checked {
        phase,
        stories: stories.to_vec(),
        path,
        absences,
        documents,
    })
}

/// Runs the path `checked` judged under `command`, the command its caller
/// prepared. A changed selection records the `phase edit` of the stories
/// with its continuation naming `id` as the order its approval issues, and
/// answers with the recorded receipt: the draft's digest, its kind, the time
/// it was recorded, the phase, the selection and the commands `baley scope
/// preview <digest>` and `baley scope approve <digest>`, issuing no order
/// (D-09), so a replay answered from the record gets the same answer. A
/// planning order, or the due revision, is issued as `id` for the selection
/// under `policy`, the host's effective policy, for `host` with the catalog
/// `names` (D-15); its decision declares no pending planner order of the
/// phase and the check's absences, among them no pending checker order of
/// the phase, and observes the check's documents and the inputs'. An
/// approved continuation's order is handed over (D-01), observing the
/// check's documents and declaring its absences beside the hand-over's
/// own, as a changed selection's edit declares them.
#[allow(clippy::too_many_arguments)]
pub fn run<S: Ledger + Views + Payloads>(
    store: &S,
    command: &Command,
    checked: &Checked,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Value {
    let phase = checked.phase;
    let revision = match &checked.path {
        Planned::HandOver(read) => return order::hand_over(store, command, read),
        Planned::Draft(fields) => {
            let continuation = PlanningContinuation {
                stories: &checked.stories,
                work_order: id,
                documents: &checked.documents,
                absences: &checked.absences,
            };
            return submit_planning_edit(store, command, phase, fields, continuation)
                .unwrap_or_else(|answer| answer);
        }
        Planned::Revision { draft, set } => Some(Revision {
            draft: *draft,
            set: set.as_str(),
        }),
        Planned::Planning => None,
    };
    let prepared = match order::prepare(
        store,
        command,
        phase,
        &checked.stories,
        revision,
        policy,
        host,
        names,
        id,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let mut absences = planner_absences(phase);
    absences.extend(checked.absences.iter().cloned());
    let documents: Vec<ObservedDocument> = prepared
        .documents()
        .iter()
        .chain(&checked.documents)
        .cloned()
        .collect();
    let result = store.transact(command, &mut |tx| {
        let mut receipt = Map::new();
        receipt.insert("status".into(), json!("ok"));
        receipt.extend(prepared.issue_within(tx, &command.recorded_at)?);
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: Value::Object(receipt),
            sensitive: false,
            observed: Observed {
                absences: absences.clone(),
                documents: documents.clone(),
                ..Observed::default()
            },
            git: None,
        })
    });
    issue::answer(result, store)
}

/// The refusal a `phase-plan` call gets for `refusal`: a pending planner
/// order at slot `operation` as a retry beside one gets it, an active phase
/// there too, the phase at `phase` and a story at `stories`.
fn refused(phase: u32, refusal: &CommitmentRefusal) -> Value {
    let slot = match refusal {
        CommitmentRefusal::PlannerOrderPending { work_order } => {
            return planner_order_pending(
                phase,
                work_order,
                "so no other planner order was issued",
            );
        }
        CommitmentRefusal::PhaseActive { .. } => "operation",
        CommitmentRefusal::NoSuchPhase { .. } => "phase",
        _ => "stories",
    };
    readiness_refused(refusal, slot, "nothing was recorded")
}

/// A refusal of the drafted edit, put at slot `stories`, the only member a
/// session sent. A `failed` answer is returned as it is.
fn at_stories(mut answer: Value) -> Value {
    if answer["status"] == "refused" {
        answer["slot"] = json!("stories");
    }
    answer
}

/// The latest continuation a `phase_plan` document holds, or none. One
/// that cannot be read is a ledger fault.
pub(crate) fn latest_continuation(body: Option<&Value>) -> Result<Option<Latest>, Value> {
    let latest = match body.and_then(|body| body.get("continuation")) {
        None | Some(Value::Null) => return Ok(None),
        Some(latest) => latest,
    };
    let read = || -> Option<Latest> {
        let stories = latest
            .get("stories")?
            .as_array()?
            .iter()
            .map(|story| story.as_u64().and_then(|story| u32::try_from(story).ok()))
            .collect::<Option<Vec<u32>>>()?;
        Some(Latest {
            stories,
            edit: Hash::from_hex(latest.get("edit")?.as_str()?)?,
            work_order: latest.get("work_order")?.as_str()?.to_owned(),
            followed: latest.get("followed")?.as_bool()?,
        })
    };
    read().map(Some).ok_or_else(|| unreadable(PHASE_PLAN_VIEW))
}

/// The phase's current unapproved draft and its set, or none.
fn current_draft(body: Option<&Value>) -> Result<Option<(Hash, String)>, Value> {
    let draft = match body.and_then(|body| body.get("draft")) {
        None | Some(Value::Null) => return Ok(None),
        Some(draft) => draft,
    };
    let digest = draft
        .get("digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex);
    let set = draft.get("set").and_then(Value::as_str);
    match (digest, set) {
        (Some(digest), Some(set)) => Ok(Some((digest, set.to_owned()))),
        _ => Err(unreadable(PHASE_PLAN_VIEW)),
    }
}

/// Every active phase in number order, read from `phase_plan`'s status
/// index page by page.
fn active_phases<S: Views + ?Sized>(store: &S, project: &ProjectId) -> Result<Vec<u32>, Value> {
    let mut phases = Vec::new();
    let mut after = None;
    loop {
        let query = IndexQuery {
            index: PHASE_PLAN_STATUS_INDEX.into(),
            equals: active_prefix(),
            page: PageRequest {
                limit: ACTIVE_PAGE,
                after,
            },
        };
        let page = store.find(project, PHASE_PLAN_VIEW, &query).map_err(busy)?;
        for document in &page.items {
            let phase = document
                .body
                .get("phase")
                .and_then(Value::as_u64)
                .and_then(|phase| u32::try_from(phase).ok())
                .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
            phases.push(phase);
        }
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(phases),
        }
    }
}

/// The status index prefix of the active phases.
fn active_prefix() -> Vec<KeyValue> {
    vec![KeyValue::Text(ACTIVE.into())]
}

/// The sequence that produced the document `key` of `view`, or none.
fn produced<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: DocKey,
) -> Result<Option<u64>, Value> {
    Ok(store
        .get(project, view, &key)
        .map_err(busy)?
        .map(|document| document.produced_seq))
}

fn observed(view: &str, key: DocKey, produced_seq: Option<u64>) -> ObservedDocument {
    ObservedDocument {
        view: view.into(),
        key,
        produced_seq,
    }
}

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
}

fn unreadable(view: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("the {view} view document cannot be read, so nothing was recorded"),
        PLACE_LEDGER,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005
    //! section 5, design 0004's `phase edit`, D-02, D-09 to D-11, D-15 and
    //! the plan's data store C, P0, N0 and PL2, never from running this code.
    //! The digests are `sha256sum` over each edit's canonical body.

    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::refine::order::tests::{input_body, input_names, issued};
    use crate::scope::fixtures::*;

    /// The `[1, 4]` edit of phase 1, `{"kind":"phase-edit","phase":1,"stories":[1,4]}`.
    pub(crate) const E1_DIGEST: &str =
        "8eabf037f141c7d617bbb783e762cfb4622014a0927fcad74d0b2feef8dc0de1";

    /// Phase 1's `phase-plan` with `stories`, checked and run under P0 and
    /// N0 as `request_id` at `at`, naming `id` as its order.
    pub(crate) fn planned(
        store: &SqliteStore,
        stories: &[u32],
        request_id: &str,
        at: &str,
        id: &str,
    ) -> Value {
        let checked = check(store, &project(), 1, stories).expect("the call passes its check");
        run(
            store,
            &command(PHASE_PLAN_COMMAND, request_id, 7, at),
            &checked,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    /// The story ids an order's `stories` input binds, in order.
    pub(crate) fn bound_stories(store: &SqliteStore, event: &baley_store::Event) -> Vec<u64> {
        input_body(store, event, "stories")
            .as_array()
            .expect("the stories input is a list")
            .iter()
            .map(|story| story["id"].as_u64().unwrap())
            .collect()
    }

    /// The check of phase `phase` with `stories`, which must refuse with
    /// `code` at `slot` and record nothing.
    fn refused_with(
        store: &SqliteStore,
        phase: u32,
        stories: &[u32],
        code: &str,
        slot: &str,
    ) -> Value {
        let before = head(store);
        let refusal = check(store, &project(), phase, stories).expect_err("the call is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], slot, "{refusal}");
        assert_eq!(head(store), before);
        refusal
    }

    #[test]
    fn a_phase_plan_refusal_recording_anything_is_caught() {
        let (_dir, store) = store_c();
        let refusal = refused_with(&store, 9, &[1], "no-such-phase", "phase");
        assert_eq!(refusal["details"], json!({"phase": 9}));
        let refusal = refused_with(&store, 1, &[1, 9], "no-such-story", "stories");
        assert_eq!(refusal["details"], json!({"story": 9}));
        let refusal = refused_with(&store, 1, &[1, 3], "story-unrefined", "stories");
        assert_eq!(refusal["details"], json!({"story": 3}));
        let refusal = refused_with(&store, 1, &[1, 2], "story-in-phase", "stories");
        assert_eq!(refusal["details"], json!({"story": 2, "phase": 2}));
        refused_with(&store, 1, &[1, 4, 4], "malformed-submission", "stories");

        activate_phase_2(&store, R1);
        let refusal = refused_with(&store, 1, &[1], "phase-active", "operation");
        assert_eq!(refusal["details"], json!({"phase": 2}));
    }

    #[test]
    fn an_unchanged_selection_ordered_twice_or_a_changed_one_ordered_at_all_is_caught() {
        let (_dir, store) = store_c();
        let answer = planned(&store, &[1], R1, T4, O);
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["work_order"], O, "{answer}");
        let orders = issued(&store);
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].payload["subject"], "phase/1");
        assert_eq!(bound_stories(&store, &orders[0]), [1]);

        let refusal = refused_with(&store, 1, &[1], "planner-order-pending", "operation");
        assert_eq!(refusal["details"], json!({"work_order": O}));

        let (_dir, store) = store_c();
        let answer = planned(&store, &[1, 4], R1, T4, O);
        assert_eq!(
            answer,
            json!({"status": "ok", "digest": E1_DIGEST, "kind": "phase-edit",
                "recorded_at": T4, "phase": 1, "stories": [1, 4],
                "preview": format!("baley scope preview {E1_DIGEST}"),
                "approve": format!("baley scope approve {E1_DIGEST}")})
        );
        let events = history(&store);
        let edits: Vec<_> = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["kind"] == "phase-edit")
            .collect();
        assert_eq!(edits.len(), 1);
        assert_eq!(of_type(&events, "plan.continuation").len(), 1);
        assert!(issued(&store).is_empty());
    }

    #[test]
    fn a_phase_plan_landing_beside_a_newly_active_phase_is_caught() {
        let (_dir, store) = store_c();
        let checked = check(&store, &project(), 1, &[1]).expect("nothing is active yet");
        activate_phase_2(&store, R1);
        let before = head(&store);
        let answer = run(
            &store,
            &command(PHASE_PLAN_COMMAND, R2, 7, T6),
            &checked,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            O,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
        assert!(issued(&store).is_empty());
    }

    #[test]
    fn a_replayed_draft_answered_without_its_phase_plan_members_is_caught() {
        let (_dir, store) = store_c();
        let first = planned(&store, &[1, 4], R1, T4, O);
        assert_eq!(first["status"], "ok", "{first}");
        // A retry of the same request is answered from its record, as
        // `mcp::planning::serve` answers a replay, so it must be the first
        // answer whole.
        let replayed = issue::answer(
            issue::record(&store, &command(PHASE_PLAN_COMMAND, R1, 7, T5), None),
            &store,
        );
        assert_eq!(replayed, first);
        assert_eq!(
            replayed,
            json!({"status": "ok", "digest": E1_DIGEST, "kind": "phase-edit",
                "recorded_at": T4, "phase": 1, "stories": [1, 4],
                "preview": format!("baley scope preview {E1_DIGEST}"),
                "approve": format!("baley scope approve {E1_DIGEST}")})
        );
    }

    #[test]
    fn a_draft_landing_beside_a_newly_active_phase_is_caught() {
        let (_dir, store) = store_c();
        let checked = check(&store, &project(), 1, &[1, 4]).expect("nothing is active yet");
        activate_phase_2(&store, R1);
        let before = head(&store);
        let answer = run(
            &store,
            &command(PHASE_PLAN_COMMAND, R2, 7, T6),
            &checked,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            O,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(&store), before);
        assert!(of_type(&history(&store), "plan.continuation").is_empty());
    }

    #[test]
    fn a_due_revision_issued_as_a_planning_order_is_caught() {
        let (_dir, store) = store_c();
        let (draft, _set) = pl2_rejected(&store, R1, R2);
        let answer = planned(&store, &[1], R3, T6, O);
        assert_eq!(answer["status"], "ok", "{answer}");
        let orders = issued(&store);
        assert_eq!(orders.len(), 1);
        assert_eq!(
            input_names(&orders[0]),
            [
                "phase",
                "stories",
                "description",
                "approved_items",
                "draft",
                "questions"
            ]
        );
        assert_eq!(input_body(&store, &orders[0], "draft"), draft);
        let questions = input_body(&store, &orders[0], "questions");
        assert_eq!(questions["answers"][0]["question"], "q1");
        assert_eq!(questions["answers"][0]["answer"], "Postgres");
        assert_eq!(questions["answers"][0]["accepted"], false);
    }

    #[test]
    fn a_planner_order_issued_beside_a_pending_checker_order_is_caught() {
        use crate::planning::checker::tests::{C1, drafted_with_check};
        use crate::planning::order::tests::fail_by_hand;
        use crate::planning::submit::tests::order_o;

        let (_dir, store) = store_c();
        order_o(&store, R1);
        drafted_with_check(&store, &pl2(), C1, R2, T5);
        let refusal = refused_with(&store, 1, &[1], "checker-order-pending", "operation");
        assert_eq!(refusal["details"], json!({"work_order": C1}));

        fail_by_hand(&store, C1, R3);
        let checked = check(&store, &project(), 1, &[1]);
        assert!(checked.is_ok(), "{checked:?}");
    }

    #[test]
    fn a_commitment_approval_rerun_issuing_a_second_order_or_losing_its_order_is_caught() {
        use crate::planning::order::tests::O2;

        let (dir, store) = store_c();
        let e2 = edit(&store, &[4, 1], O, R1, T4);
        let approved = approve_commitment(&store, e2, R2, T5);
        assert_eq!(approved["status"], "ok", "{approved}");
        drop(store);
        let store = reopen(&dir);

        let answer = planned(&store, &[4, 1], R3, T6, O2);
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["work_order"], O, "{answer}");
        assert_eq!(
            answer["prompt"],
            format!("Read Baley work order {O} and follow it.")
        );
        assert_eq!(answer["launch"], approved["launch"], "{answer}");
        let events = history(&store);
        let followed = of_type(&events, "plan.continuation_followed");
        assert_eq!(followed.len(), 1);
        assert_eq!(
            followed[0].payload,
            json!({"phase": 1, "edit": e2.to_hex(), "work_order": O})
        );
        let orders = issued(&store);
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].payload["subject"], "phase/1");

        let refusal = refused_with(&store, 1, &[4, 1], "planner-order-pending", "operation");
        assert_eq!(refusal["details"], json!({"work_order": O}));
    }

    /// Store C with the `[4, 1]` edit E2 approved through the commitment
    /// approval's two steps, issuing O, and phase 1's `phase-plan` with
    /// `[4, 1]` checked, so its run hands O over.
    fn hand_over_checked() -> (tempfile::TempDir, SqliteStore, Checked) {
        let (dir, store) = store_c();
        let e2 = edit(&store, &[4, 1], O, R1, T4);
        let approved = approve_commitment(&store, e2, R2, T5);
        assert_eq!(approved["status"], "ok", "{approved}");
        let checked = check(&store, &project(), 1, &[4, 1]).expect("O is handed over");
        (dir, store, checked)
    }

    /// The run of `checked`, which must fail `ledger-busy` with nothing
    /// recorded and O not handed over.
    fn handed_over_busy(store: &SqliteStore, checked: &Checked) {
        use crate::planning::order::tests::O2;

        let before = head(store);
        let answer = run(
            store,
            &command(PHASE_PLAN_COMMAND, R4, 7, T7),
            checked,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            O2,
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(head(store), before);
        assert!(of_type(&history(store), "plan.continuation_followed").is_empty());
    }

    #[test]
    fn a_hand_over_landing_beside_a_newly_active_phase_or_an_unrefined_story_is_caught() {
        // Phase 2 made active after the check, which a fresh check refuses
        // `phase-active`.
        let (_dir, store, checked) = hand_over_checked();
        activate_phase_2(&store, R3);
        handed_over_busy(&store, &checked);

        // Story 4's approved truths emptied after the check, which a fresh
        // check refuses `story-unrefined`.
        let (_dir, store, checked) = hand_over_checked();
        append(
            &store,
            R3,
            T6,
            vec![refined(
                4,
                "4f4f4f4f-4f4f-4f4f-8f4f-4f4f4f4f4f4f",
                Vec::new(),
            )],
        );
        handed_over_busy(&store, &checked);
    }
}
