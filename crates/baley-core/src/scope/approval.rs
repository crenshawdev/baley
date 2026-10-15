//! The judgement of an approval by digest, and the records an approved
//! start becomes (design 0004 sections 5 and 6).
//!
//! An approval is judged on facts the binary read: whether the target draft
//! is recorded, whether a start is already approved, and which draft is the
//! newest. The order is fixed, so one approval always gets one answer.

use baley_store::{NewEvent, PayloadRef, StreamName};
use serde_json::Value;

use super::event::{
    PHASE_DECLARED, PHASE_DECLARED_VERSION, PHASE_REORDERED, PHASE_REORDERED_VERSION,
    PROJECT_DESCRIBED, PROJECT_DESCRIBED_VERSION, PROJECT_STREAM, ROADMAP_STREAM, SCOPE_APPROVED,
    SCOPE_APPROVED_VERSION, START_KIND, STORY_DECLARED, STORY_DECLARED_VERSION, approved_payload,
    described_payload, phase_declared_payload, phase_reordered_payload, story_declared_payload,
};
use super::submission::{PROJECT_ALREADY_STARTED, STALE_DRAFT, Submission, UNKNOWN_DRAFT};

/// Why an approval is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalRefusal {
    /// No recorded submission has the digest.
    UnknownDraft,
    /// A start is already approved, so no other start draft can be.
    ProjectAlreadyStarted,
    /// A newer draft was submitted. `path` names the first part of the
    /// approved content that differs from the newest, such as
    /// `roadmap[1].goal`.
    StaleDraft { path: String },
}

impl ApprovalRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownDraft => UNKNOWN_DRAFT,
            Self::ProjectAlreadyStarted => PROJECT_ALREADY_STARTED,
            Self::StaleDraft { .. } => STALE_DRAFT,
        }
    }
}

/// What an approval of one start draft is judged on.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalFacts<'a> {
    /// The target draft's accepted submission, or `None` when no `draft`
    /// document has its digest.
    pub target: Option<&'a Submission>,
    /// Whether the target's `scope.submitted` was found among the events.
    pub recorded: bool,
    /// Whether a `scope.approved` of kind `start` exists.
    pub started: bool,
    /// The newest start draft's accepted submission, or `None` when there
    /// is no start draft.
    pub newest: Option<&'a Submission>,
}

/// Judges an approval: `unknown-draft` when the target's document or its
/// event is missing, since the event confirms what the view says (EVD-R27);
/// then `project-already-started` when a start is approved, ahead of
/// staleness (D-05); then `stale-draft` when the newest start draft is not
/// the target, compared by digest, with the first differing path (D-03).
/// With no start draft listed at all, the target is not a start draft, so
/// it is unknown.
pub fn judge_approval(facts: &ApprovalFacts) -> Result<(), ApprovalRefusal> {
    let Some(target) = facts.target.filter(|_| facts.recorded) else {
        return Err(ApprovalRefusal::UnknownDraft);
    };
    if facts.started {
        return Err(ApprovalRefusal::ProjectAlreadyStarted);
    }
    let Some(newest) = facts.newest else {
        return Err(ApprovalRefusal::UnknownDraft);
    };
    if newest.digest() == target.digest() {
        return Ok(());
    }
    // A digest is taken over every member the walk compares, so two
    // digests differ only when some member does.
    let path = first_difference(target, newest).expect("differing digests differ in content");
    Err(ApprovalRefusal::StaleDraft { path })
}

/// The first part of two accepted submissions that differs, as a path in
/// the shape's member order: the description, then the stories entry by
/// entry, then the roadmap entry by entry. List entries are counted from 0,
/// and where one list is longer the path names the first entry only one
/// holds. `None` when they are equal.
fn first_difference(a: &Submission, b: &Submission) -> Option<String> {
    let (a, b) = (a.value(), b.value());
    members(
        "description",
        &a["description"],
        &b["description"],
        &DESCRIPTION,
    )
    .or_else(|| entries("stories", &a["stories"], &b["stories"], &STORY))
    .or_else(|| entries("roadmap", &a["roadmap"], &b["roadmap"], &PHASE))
}

// The start shape's member order, which is the order a path is looked for in.
const DESCRIPTION: [&str; 3] = ["what", "core_value", "constraints"];
const STORY: [&str; 3] = ["label", "sentence", "status"];
const PHASE: [&str; 5] = ["name", "goal", "detail", "depends_on", "stories"];

/// The first of `names`, in that order, whose values differ under `path`.
fn members(path: &str, a: &Value, b: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| leaf(format!("{path}.{name}"), &a[*name], &b[*name]))
}

/// The first differing entry of two lists of objects with `names`.
fn entries(path: &str, a: &Value, b: &Value, names: &[&str]) -> Option<String> {
    list(path, a, b, |at, x, y| members(&at, x, y, names))
}

/// `path` when two values differ, looking inside lists entry by entry.
fn leaf(path: String, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Array(_), Value::Array(_)) => list(&path, a, b, leaf),
        _ => (a != b).then_some(path),
    }
}

/// A list's entries, or none when the value is not a list.
fn items(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// The first entry `each` finds differing, or the first index only one
/// list holds.
fn list(
    path: &str,
    a: &Value,
    b: &Value,
    each: impl Fn(String, &Value, &Value) -> Option<String>,
) -> Option<String> {
    let (a, b) = (items(a), items(b));
    (0..a.len().max(b.len())).find_map(|index| {
        let at = format!("{path}[{index}]");
        match (a.get(index), b.get(index)) {
            (Some(x), Some(y)) => each(at, x, y),
            _ => Some(at),
        }
    })
}

/// The events an approved start appends, each on its stream, in order:
/// `project.described`; one `story.declared` per story in submission order;
/// one `phase.declared` per phase in roadmap order; one `phase.reordered`
/// with every new number in roadmap order; and `scope.approved` last, after
/// what it approves.
///
/// Story ids continue from `highest_story` and phase numbers from
/// `highest_phase`, the highest ever declared, so neither is ever reused
/// (D-09, D-10). Each story's phase is the number of the phase that lists it
/// by label, or none. `details` holds one stored detail reference per phase
/// in roadmap order. `owner` and `at` are the approval's owner and time.
///
/// When a number or id would pass `u32::MAX` it is [`NumbersExhausted`]
/// and no events are given, since wrapping would reuse a number.
pub fn start_events(
    submission: &Submission,
    highest_phase: u32,
    highest_story: u32,
    details: &[PayloadRef],
    owner: &str,
    at: &str,
) -> Result<Vec<NewEvent>, NumbersExhausted> {
    assert_eq!(
        details.len(),
        submission.phases.len(),
        "one detail reference per phase"
    );
    let digest = submission.digest();
    let ids = next_numbers(highest_story, submission.stories.len())?;
    let numbers = next_numbers(highest_phase, submission.phases.len())?;
    let story_id = |index: usize| ids[index];
    let phase_number = |index: usize| numbers[index];

    let mut events = vec![event(
        PROJECT_STREAM,
        PROJECT_DESCRIBED,
        PROJECT_DESCRIBED_VERSION,
        described_payload(&submission.description, digest),
        Vec::new(),
    )];
    for (index, story) in submission.stories.iter().enumerate() {
        // Phases list stories by label, already resolved to story indexes.
        let phase = submission
            .phases
            .iter()
            .position(|phase| phase.stories.contains(&index))
            .map(phase_number);
        events.push(event(
            ROADMAP_STREAM,
            STORY_DECLARED,
            STORY_DECLARED_VERSION,
            story_declared_payload(
                story_id(index),
                &story.sentence,
                story.status,
                phase,
                digest,
            ),
            Vec::new(),
        ));
    }
    for ((index, phase), detail) in submission.phases.iter().enumerate().zip(details) {
        // Dependencies are 1-based roadmap positions.
        let depends_on: Vec<u32> = phase
            .depends_on
            .iter()
            .map(|position| phase_number(position - 1))
            .collect();
        let stories: Vec<u32> = phase.stories.iter().map(|&story| story_id(story)).collect();
        events.push(event(
            ROADMAP_STREAM,
            PHASE_DECLARED,
            PHASE_DECLARED_VERSION,
            phase_declared_payload(
                numbers[index],
                &phase.name,
                &phase.goal,
                detail,
                &depends_on,
                &stories,
                digest,
            ),
            vec![detail.clone()],
        ));
    }
    // A start declares its whole roadmap at once, so one order states it.
    events.push(event(
        ROADMAP_STREAM,
        PHASE_REORDERED,
        PHASE_REORDERED_VERSION,
        phase_reordered_payload(&numbers, digest),
        Vec::new(),
    ));
    events.push(event(
        PROJECT_STREAM,
        SCOPE_APPROVED,
        SCOPE_APPROVED_VERSION,
        approved_payload(digest, START_KIND, owner, at),
        Vec::new(),
    ));
    Ok(events)
}

/// Numbering an approved start would pass `u32::MAX`, so a phase number or
/// story id would wrap onto one already used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumbersExhausted;

/// The `count` whole numbers after `highest`, or [`NumbersExhausted`] when the
/// last would pass `u32::MAX`.
fn next_numbers(highest: u32, count: usize) -> Result<Vec<u32>, NumbersExhausted> {
    (1..=count)
        .map(|step| {
            u32::try_from(step)
                .ok()
                .and_then(|step| highest.checked_add(step))
                .ok_or(NumbersExhausted)
        })
        .collect()
}

fn event(
    stream: &str,
    type_name: &str,
    type_version: u32,
    payload: Value,
    attachments: Vec<PayloadRef>,
) -> NewEvent {
    NewEvent {
        stream: StreamName(stream.into()),
        type_name: type_name.into(),
        type_version,
        git: None,
        payload,
        attachments,
    }
}

#[cfg(test)]
mod tests {
    //! Submissions are written out as design 0004's start shape, and every
    //! expected path is read off the two values by hand.

    use serde_json::json;

    use super::*;
    use crate::scope::judge_submission;

    fn s1() -> Value {
        json!({"description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]},
            "stories": [
                {"label": "record", "sentence": "Every step is recorded", "status": "active"},
                {"label": "approve", "sentence": "The owner approves scope", "status": "active"},
                {"label": "later", "sentence": "Teammates see the backlog", "status": "deferred"}],
            "roadmap": [
                {"name": "Records", "goal": "Steps are recorded",
                    "detail": "The ledger and its views", "depends_on": [],
                    "stories": ["record"]},
                {"name": "Approval", "goal": "The owner approves",
                    "detail": "Approval by digest", "depends_on": [1],
                    "stories": ["approve"]}]})
    }

    fn s1_with(changes: &[(&str, Value)]) -> Value {
        let mut value = s1();
        for (pointer, to) in changes {
            *value.pointer_mut(pointer).unwrap() = to.clone();
        }
        value
    }

    fn s2() -> Value {
        s1_with(&[("/roadmap/1/goal", json!("The owner approves by digest"))])
    }

    fn accepted(value: &Value) -> Submission {
        judge_submission(value).unwrap()
    }

    #[test]
    fn the_first_difference_named_by_another_path_is_caught() {
        let mut three_phases = s1();
        three_phases["roadmap"].as_array_mut().unwrap().push(
            json!({"name": "Reads", "goal": "g", "detail": "d", "depends_on": [], "stories": []}),
        );
        for (other, path) in [
            (s2(), "roadmap[1].goal"),
            (
                s1_with(&[("/stories/2/sentence", json!("Teammates see it"))]),
                "stories[2].sentence",
            ),
            (three_phases, "roadmap[2]"),
            (
                s1_with(&[("/description/constraints", json!(["One binary", "Linux"]))]),
                "description.constraints[1]",
            ),
            (
                s1_with(&[
                    ("/description/what", json!("Another ledger")),
                    ("/description/constraints", json!(["Two binaries"])),
                ]),
                "description.what",
            ),
            (
                s1_with(&[
                    ("/stories/0/sentence", json!("Each step is recorded")),
                    ("/roadmap/0/name", json!("Ledger")),
                ]),
                "stories[0].sentence",
            ),
        ] {
            assert_eq!(
                first_difference(&accepted(&s1()), &accepted(&other)).as_deref(),
                Some(path),
                "{other}"
            );
        }
    }

    #[test]
    fn an_approval_judged_in_another_order_is_caught() {
        let a = accepted(&s1());
        let b = accepted(&s2());
        // S1 judged again, as the newest draft after A, B, A reads it.
        let a_again = accepted(&s1());
        let facts = |target, recorded, started, newest| ApprovalFacts {
            target,
            recorded,
            started,
            newest,
        };
        for (case, expected) in [
            (
                facts(None, false, true, Some(&b)),
                Err(ApprovalRefusal::UnknownDraft),
            ),
            (
                facts(Some(&a), true, true, Some(&b)),
                Err(ApprovalRefusal::ProjectAlreadyStarted),
            ),
            (
                facts(Some(&a), true, false, Some(&b)),
                Err(ApprovalRefusal::StaleDraft {
                    path: "roadmap[1].goal".into(),
                }),
            ),
            (facts(Some(&a), true, false, Some(&a)), Ok(())),
            (
                facts(Some(&a), false, false, Some(&a)),
                Err(ApprovalRefusal::UnknownDraft),
            ),
            (facts(Some(&a), true, false, Some(&a_again)), Ok(())),
        ] {
            assert_eq!(judge_approval(&case), expected, "{case:?}");
        }
    }

    const T2: &str = "2026-10-13T09:00:02Z";

    fn detail(byte: u8) -> PayloadRef {
        PayloadRef {
            hash: baley_store::Hash([byte; 32]),
            bytes: 24,
            class: baley_store::RetentionClass::Record,
        }
    }

    fn events_for(value: &Value, highest_phase: u32, highest_story: u32) -> Vec<NewEvent> {
        start_events(
            &accepted(value),
            highest_phase,
            highest_story,
            &[detail(0xa1), detail(0xa2)],
            "Ada Owner",
            T2,
        )
        .expect("the numbers fit")
    }

    fn of<'a>(events: &'a [NewEvent], type_name: &str) -> Vec<&'a Value> {
        events
            .iter()
            .filter(|event| event.type_name == type_name)
            .map(|event| &event.payload)
            .collect()
    }

    /// Each story's id and phase, and each phase's number, dependencies,
    /// stories and detail hash.
    fn numbering(events: &[NewEvent]) -> (Vec<(Value, Value)>, Vec<Value>) {
        let stories = of(events, "story.declared")
            .into_iter()
            .map(|story| (story["id"].clone(), story["phase"].clone()))
            .collect();
        let phases = of(events, "phase.declared")
            .into_iter()
            .map(|phase| {
                json!([
                    phase["number"],
                    phase["depends_on"],
                    phase["stories"],
                    phase["detail"]["payload"]
                ])
            })
            .collect();
        (stories, phases)
    }

    #[test]
    fn a_phase_number_or_story_id_reused_after_earlier_declarations_is_caught() {
        let events = events_for(&s1(), 3, 5);
        let (stories, phases) = numbering(&events);
        assert_eq!(
            stories,
            [
                (json!(6), json!(4)),
                (json!(7), json!(5)),
                (json!(8), Value::Null)
            ]
        );
        assert_eq!(
            phases,
            [
                json!([4, [], [6], "a1".repeat(32)]),
                json!([5, [4], [7], "a2".repeat(32)])
            ]
        );
        let reordered = of(&events, "phase.reordered");
        assert_eq!(reordered.len(), 1);
        assert_eq!(reordered[0]["order"], json!([4, 5]));
    }

    #[test]
    fn a_phase_number_or_story_id_wrapped_past_the_largest_is_caught() {
        // S1 has three stories and two phases, so these leave exactly room.
        let max = u64::from(u32::MAX);
        let events = events_for(&s1(), u32::MAX - 2, u32::MAX - 3);
        let (stories, phases) = numbering(&events);
        assert_eq!(
            stories,
            [
                (json!(max - 2), json!(max - 1)),
                (json!(max - 1), json!(max)),
                (json!(max), Value::Null)
            ]
        );
        assert_eq!(phases[0][0], json!(max - 1));
        assert_eq!(phases[1][0], json!(max));
        assert_eq!(phases[1][1], json!([max - 1]));

        // One fewer free number for either, and nothing is given.
        let start = |highest_phase, highest_story| {
            start_events(
                &accepted(&s1()),
                highest_phase,
                highest_story,
                &[detail(0xa1), detail(0xa2)],
                "Ada Owner",
                T2,
            )
        };
        assert_eq!(start(u32::MAX - 1, 0), Err(NumbersExhausted));
        assert_eq!(start(0, u32::MAX - 2), Err(NumbersExhausted));
    }

    #[test]
    fn a_story_given_to_a_phase_by_position_instead_of_its_label_is_caught() {
        let s3 = s1_with(&[
            ("/roadmap/0/stories", json!(["approve"])),
            ("/roadmap/1/stories", json!(["record"])),
        ]);
        let events = events_for(&s3, 3, 5);
        let sentences: Vec<&Value> = of(&events, "story.declared")
            .into_iter()
            .map(|story| &story["sentence"])
            .collect();
        assert_eq!(
            sentences,
            [
                "Every step is recorded",
                "The owner approves scope",
                "Teammates see the backlog"
            ]
        );
        let (stories, phases) = numbering(&events);
        assert_eq!(
            stories,
            [
                (json!(6), json!(5)),
                (json!(7), json!(4)),
                (json!(8), Value::Null)
            ]
        );
        assert_eq!(phases[0][2], json!([7]));
        assert_eq!(phases[1][2], json!([6]));
    }

    #[test]
    fn an_approval_appending_its_events_in_another_order_or_stream_is_caught() {
        let submission = accepted(&s1());
        let events = start_events(
            &submission,
            0,
            0,
            &[detail(0xa1), detail(0xa2)],
            "Ada Owner",
            T2,
        )
        .expect("the numbers fit");
        let placed: Vec<(&str, &str)> = events
            .iter()
            .map(|event| (event.type_name.as_str(), event.stream.0.as_str()))
            .collect();
        assert_eq!(
            placed,
            [
                ("project.described", "project"),
                ("story.declared", "roadmap"),
                ("story.declared", "roadmap"),
                ("story.declared", "roadmap"),
                ("phase.declared", "roadmap"),
                ("phase.declared", "roadmap"),
                ("phase.reordered", "roadmap"),
                ("scope.approved", "project"),
            ]
        );
        assert_eq!(
            events[7].payload,
            json!({"submission": submission.digest().to_hex(), "kind": "start",
                "owner": "Ada Owner", "at": T2})
        );
    }
}
