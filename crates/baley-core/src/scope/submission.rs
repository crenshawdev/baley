//! The judgement of a start submission and its digest (design 0004
//! sections 5 and 6).
//!
//! A submission arrives as plain JSON and is judged whole: its shape first,
//! then story assignment, then dependency cycles, so one submission always
//! gets one answer. Nothing here records anything.

use std::collections::{HashMap, HashSet};

use baley_store::{Hash, canonical_json, request_digest};
use serde_json::{Map, Value};

/// The refusal code for a submission with a missing, blank, wrong-type or
/// unknown field.
pub const MALFORMED_SUBMISSION: &str = "malformed-submission";
/// The refusal code for one story listed by two phases.
pub const STORY_DOUBLE_ASSIGNED: &str = "story-double-assigned";
/// The refusal code for phases that depend on each other in a cycle.
pub const DEPENDENCY_CYCLE: &str = "dependency-cycle";
/// The refusal code for a start submitted or approved after a start was
/// already approved.
pub const PROJECT_ALREADY_STARTED: &str = "project-already-started";
/// The refusal code for an approval naming a digest no submission has.
pub const UNKNOWN_DRAFT: &str = "unknown-draft";
/// The refusal code for an approval naming a draft that is not the newest.
pub const STALE_DRAFT: &str = "stale-draft";

const ROOT_MEMBERS: &[&str] = &["description", "stories", "roadmap"];
const DESCRIPTION_MEMBERS: &[&str] = &["what", "core_value", "constraints"];
const STORY_MEMBERS: &[&str] = &["label", "sentence", "status"];
const PHASE_MEMBERS: &[&str] = &["name", "goal", "detail", "depends_on", "stories"];

/// Why a start submission is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionRefusal {
    /// A field is missing, null, blank, of another type, unknown or out of
    /// range. `section` is the first member of `field`, or `submission` when
    /// the submission is not an object. `field` is a path such as
    /// `roadmap[1].goal`, with list entries counted from 0.
    Malformed { section: String, field: String },
    /// `story` is listed by the phases at these 1-based positions, the first
    /// two that list it.
    StoryDoubleAssigned { story: String, phases: [usize; 2] },
    /// Every phase lying on a dependency cycle, by 1-based position,
    /// ascending. A phase depending on itself is on one.
    DependencyCycle { phases: Vec<usize> },
}

impl SubmissionRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Malformed { .. } => MALFORMED_SUBMISSION,
            Self::StoryDoubleAssigned { .. } => STORY_DOUBLE_ASSIGNED,
            Self::DependencyCycle { .. } => DEPENDENCY_CYCLE,
        }
    }
}

/// A story's status as a submission states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryStatus {
    /// Promised, and the only status a phase may list.
    Active,
    /// Wanted later.
    Deferred,
    /// Ruled out.
    Excluded,
}

impl StoryStatus {
    /// The status as submissions and events spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deferred => "deferred",
            Self::Excluded => "excluded",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "deferred" => Some(Self::Deferred),
            "excluded" => Some(Self::Excluded),
            _ => None,
        }
    }
}

/// The project description of an accepted submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    /// What the project is.
    pub what: String,
    /// The one thing it must get right.
    pub core_value: String,
    /// Its constraints, possibly none.
    pub constraints: Vec<String>,
}

/// One story of an accepted submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    /// The label phases name it by inside the submission. Baley assigns the
    /// story's id at approval.
    pub label: String,
    /// The story itself.
    pub sentence: String,
    /// Its status.
    pub status: StoryStatus,
}

/// One phase of an accepted submission, in roadmap order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// The phase's name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// Its detail text, stored as a payload when recorded.
    pub detail: String,
    /// The 1-based roadmap positions of the phases it depends on.
    pub depends_on: Vec<usize>,
    /// Its stories, as indexes into [`Submission::stories`].
    pub stories: Vec<usize>,
}

/// A start submission judged whole and accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    /// The project description.
    pub description: Description,
    /// The stories in submission order, which is their priority order.
    pub stories: Vec<Story>,
    /// The phases in roadmap order.
    pub phases: Vec<Phase>,
    value: Value,
    digest: Hash,
    body: Vec<u8>,
}

impl Submission {
    /// The submission as it was given.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The SHA-256 of the submission's canonical JSON and nothing else, so
    /// identical content has one digest from any session.
    pub fn digest(&self) -> Hash {
        self.digest
    }

    /// The canonical JSON bytes the digest is taken over, stored as the
    /// submission's body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Judges a start submission given as plain JSON: its shape, then story
/// assignment, then dependency cycles. Text is taken as written, never
/// trimmed.
pub fn judge_submission(value: &Value) -> Result<Submission, SubmissionRefusal> {
    let Value::Object(root) = value else {
        return Err(malformed("submission", "submission"));
    };
    if let Some(unknown) = unknown_member(root, ROOT_MEMBERS) {
        return Err(malformed(unknown, unknown));
    }
    let description = judge_description(root.get("description"))?;
    let stories = judge_stories(root.get("stories"))?;
    let phases = judge_roadmap(root.get("roadmap"), &stories)?;
    double_assigned(&stories, &phases)?;
    let on_cycle = phases_on_cycles(&phases);
    if !on_cycle.is_empty() {
        return Err(SubmissionRefusal::DependencyCycle { phases: on_cycle });
    }
    // Every leaf of an accepted submission is text or a position within the
    // roadmap, so it always has a canonical form.
    let digest = request_digest(value).expect("an accepted submission is canonical");
    let body = canonical_json(value).expect("an accepted submission is canonical");
    Ok(Submission {
        description,
        stories,
        phases,
        value: value.clone(),
        digest,
        body,
    })
}

type Judged<T> = Result<T, SubmissionRefusal>;

fn malformed(section: &str, field: &str) -> SubmissionRefusal {
    SubmissionRefusal::Malformed {
        section: section.into(),
        field: field.into(),
    }
}

/// The first member, by name, that `allowed` lacks. By name rather than by
/// position, so member order never changes the answer.
fn unknown_member<'a>(object: &'a Map<String, Value>, allowed: &[&str]) -> Option<&'a str> {
    object
        .keys()
        .map(String::as_str)
        .filter(|name| !allowed.contains(name))
        .min()
}

/// The object at `field`, refusing an unknown member before any other fault.
fn object<'a>(
    value: Option<&'a Value>,
    section: &str,
    field: &str,
    allowed: &[&str],
) -> Judged<&'a Map<String, Value>> {
    let Some(Value::Object(object)) = value else {
        return Err(malformed(section, field));
    };
    if let Some(unknown) = unknown_member(object, allowed) {
        return Err(malformed(section, &format!("{field}.{unknown}")));
    }
    Ok(object)
}

/// Text that is not empty or only whitespace, kept as written.
fn text(value: Option<&Value>, section: &str, field: &str) -> Judged<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        _ => Err(malformed(section, field)),
    }
}

fn list<'a>(value: Option<&'a Value>, section: &str, field: &str) -> Judged<&'a [Value]> {
    match value {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(malformed(section, field)),
    }
}

fn judge_description(value: Option<&Value>) -> Judged<Description> {
    const SECTION: &str = "description";
    let object = object(value, SECTION, SECTION, DESCRIPTION_MEMBERS)?;
    let what = text(object.get("what"), SECTION, "description.what")?;
    let core_value = text(object.get("core_value"), SECTION, "description.core_value")?;
    let entries = list(
        object.get("constraints"),
        SECTION,
        "description.constraints",
    )?;
    let constraints = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            text(
                Some(entry),
                SECTION,
                &format!("description.constraints[{index}]"),
            )
        })
        .collect::<Judged<_>>()?;
    Ok(Description {
        what,
        core_value,
        constraints,
    })
}

fn judge_stories(value: Option<&Value>) -> Judged<Vec<Story>> {
    const SECTION: &str = "stories";
    let entries = list(value, SECTION, SECTION)?;
    if entries.is_empty() {
        return Err(malformed(SECTION, SECTION));
    }
    let mut labels = HashSet::new();
    let mut stories = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("stories[{index}]");
        let object = object(Some(entry), SECTION, &path, STORY_MEMBERS)?;
        let label_path = format!("{path}.label");
        let label = text(object.get("label"), SECTION, &label_path)?;
        // A repeated label would let a phase's entry mean either story.
        if !labels.insert(label.clone()) {
            return Err(malformed(SECTION, &label_path));
        }
        let sentence = text(object.get("sentence"), SECTION, &format!("{path}.sentence"))?;
        let status_path = format!("{path}.status");
        let status = match object.get("status") {
            Some(Value::String(status)) => StoryStatus::parse(status),
            _ => None,
        }
        .ok_or_else(|| malformed(SECTION, &status_path))?;
        stories.push(Story {
            label,
            sentence,
            status,
        });
    }
    Ok(stories)
}

fn judge_roadmap(value: Option<&Value>, stories: &[Story]) -> Judged<Vec<Phase>> {
    const SECTION: &str = "roadmap";
    let entries = list(value, SECTION, SECTION)?;
    if entries.is_empty() {
        return Err(malformed(SECTION, SECTION));
    }
    let by_label: HashMap<&str, usize> = stories
        .iter()
        .enumerate()
        .map(|(index, story)| (story.label.as_str(), index))
        .collect();
    let count = entries.len();
    let mut phases = Vec::with_capacity(count);
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("roadmap[{index}]");
        let object = object(Some(entry), SECTION, &path, PHASE_MEMBERS)?;
        let name = text(object.get("name"), SECTION, &format!("{path}.name"))?;
        let goal = text(object.get("goal"), SECTION, &format!("{path}.goal"))?;
        let detail = text(object.get("detail"), SECTION, &format!("{path}.detail"))?;
        let depends_path = format!("{path}.depends_on");
        let mut depends_on = Vec::new();
        let mut seen = HashSet::new();
        for (at, entry) in list(object.get("depends_on"), SECTION, &depends_path)?
            .iter()
            .enumerate()
        {
            let position = entry
                .as_u64()
                .and_then(|position| usize::try_from(position).ok())
                .filter(|position| (1..=count).contains(position) && seen.insert(*position))
                .ok_or_else(|| malformed(SECTION, &format!("{depends_path}[{at}]")))?;
            depends_on.push(position);
        }
        let stories_path = format!("{path}.stories");
        let mut served = Vec::new();
        let mut serving = HashSet::new();
        for (at, entry) in list(object.get("stories"), SECTION, &stories_path)?
            .iter()
            .enumerate()
        {
            // Only an active story is promised, so only one may be served.
            let story = entry
                .as_str()
                .and_then(|label| by_label.get(label).copied())
                .filter(|story| {
                    stories[*story].status == StoryStatus::Active && serving.insert(*story)
                })
                .ok_or_else(|| malformed(SECTION, &format!("{stories_path}[{at}]")))?;
            served.push(story);
        }
        phases.push(Phase {
            name,
            goal,
            detail,
            depends_on,
            stories: served,
        });
    }
    Ok(phases)
}

/// Refuses the first story, in story order, that two phases list.
fn double_assigned(stories: &[Story], phases: &[Phase]) -> Judged<()> {
    let mut listed_by: Vec<Vec<usize>> = vec![Vec::new(); stories.len()];
    for (index, phase) in phases.iter().enumerate() {
        for story in &phase.stories {
            listed_by[*story].push(index + 1);
        }
    }
    match listed_by
        .iter()
        .enumerate()
        .find(|(_, phases)| phases.len() > 1)
    {
        Some((story, phases)) => Err(SubmissionRefusal::StoryDoubleAssigned {
            story: stories[story].label.clone(),
            phases: [phases[0], phases[1]],
        }),
        None => Ok(()),
    }
}

/// The 1-based positions of every phase on a dependency cycle, ascending.
///
/// A phase is on a cycle when its strongly connected component holds more
/// than one phase or it depends on itself. A phase that only depends on a
/// cycle is not on it. Tarjan's algorithm, kept iterative so a long chain
/// in an untrusted submission cannot exhaust the stack.
fn phases_on_cycles(phases: &[Phase]) -> Vec<usize> {
    let edges: Vec<Vec<usize>> = phases
        .iter()
        .map(|phase| {
            phase
                .depends_on
                .iter()
                .map(|position| position - 1)
                .collect()
        })
        .collect();
    let count = edges.len();
    let mut index = vec![usize::MAX; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut on_cycle = Vec::new();
    for root in 0..count {
        if index[root] != usize::MAX {
            continue;
        }
        let mut calls = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (node, ref mut edge)) = calls.last_mut() {
            if let Some(&to) = edges[node].get(*edge) {
                *edge += 1;
                if index[to] == usize::MAX {
                    index[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    calls.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(index[to]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                if component.len() > 1 || edges[node].contains(&node) {
                    on_cycle.extend(component.into_iter().map(|member| member + 1));
                }
            }
        }
    }
    on_cycle.sort_unstable();
    on_cycle
}

#[cfg(test)]
mod tests {
    //! Expected digests and bodies are the plan's, computed with `sha256sum`
    //! over the canonical bytes written out by hand.

    use serde_json::{Value, json};

    use super::*;

    fn s1() -> Value {
        json!({
            "description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]},
            "stories": [
                {"label": "record", "sentence": "Every step is recorded", "status": "active"},
                {"label": "approve", "sentence": "The owner approves scope", "status": "active"},
                {"label": "later", "sentence": "Teammates see the backlog", "status": "deferred"}
            ],
            "roadmap": [
                {"name": "Records", "goal": "Steps are recorded",
                    "detail": "The ledger and its views", "depends_on": [], "stories": ["record"]},
                {"name": "Approval", "goal": "The owner approves", "detail": "Approval by digest",
                    "depends_on": [1], "stories": ["approve"]}
            ]
        })
    }

    /// S1 with the member at `pointer` set to `to`.
    fn s1_with(changes: &[(&str, Value)]) -> Value {
        let mut value = s1();
        for (pointer, to) in changes {
            *value.pointer_mut(pointer).expect("pointer into S1") = to.clone();
        }
        value
    }

    /// S1 with a member added to the object at `pointer`.
    fn s1_adding(pointer: &str, member: &str, to: Value) -> Value {
        let mut value = s1();
        value
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .expect("object in S1")
            .insert(member.into(), to);
        value
    }

    fn malformed(section: &str, field: &str) -> SubmissionRefusal {
        SubmissionRefusal::Malformed {
            section: section.into(),
            field: field.into(),
        }
    }

    fn refusal(value: &Value) -> SubmissionRefusal {
        judge_submission(value).expect_err("refused")
    }

    fn labels(submission: &Submission, stories: &[usize]) -> Vec<String> {
        stories
            .iter()
            .map(|index| submission.stories[*index].label.clone())
            .collect()
    }

    #[test]
    fn a_digest_over_more_than_the_canonical_content_or_over_trimmed_text_is_caught() {
        let m = json!({"description": {"what": "w", "core_value": "v", "constraints": []},
            "stories": [{"label": "s", "sentence": "t", "status": "active"}],
            "roadmap": [{"name": "n", "goal": "g", "detail": "d", "depends_on": [],
                "stories": ["s"]}]});
        let reversed = json!({"roadmap": [{"stories": ["s"], "depends_on": [], "detail": "d",
                "goal": "g", "name": "n"}],
            "stories": [{"status": "active", "sentence": "t", "label": "s"}],
            "description": {"constraints": [], "core_value": "v", "what": "w"}});
        let m_bytes = br#"{"description":{"constraints":[],"core_value":"v","what":"w"},"roadmap":[{"depends_on":[],"detail":"d","goal":"g","name":"n","stories":["s"]}],"stories":[{"label":"s","sentence":"t","status":"active"}]}"#;
        for value in [&m, &reversed] {
            let submission = judge_submission(value).expect("M is accepted");
            assert_eq!(
                submission.digest().to_hex(),
                "1b11196b8f9f4ffd5c08a4d97643f80867194c5e1a6d8cc51d2929f8db38d68e"
            );
            assert_eq!(submission.body(), m_bytes);
        }
        let mut mw = m.clone();
        mw["description"]["what"] = json!(" w ");
        let submission = judge_submission(&mw).expect("MW is accepted");
        assert_eq!(
            submission.body(),
            br#"{"description":{"constraints":[],"core_value":"v","what":" w "},"roadmap":[{"depends_on":[],"detail":"d","goal":"g","name":"n","stories":["s"]}],"stories":[{"label":"s","sentence":"t","status":"active"}]}"#
        );
        assert_eq!(submission.description.what, " w ");
        assert_eq!(
            submission.digest().to_hex(),
            "ab01e27716476d572b2e167c78581b95089e1767caa067dbb69f1e8e52879a79"
        );
    }

    #[test]
    fn a_missing_blank_or_off_type_field_accepted_or_named_wrongly_is_caught() {
        let mut no_description = s1();
        no_description
            .as_object_mut()
            .unwrap()
            .remove("description");
        let cases = [
            (no_description, ("description", "description")),
            (
                s1_with(&[("/description/what", json!("  "))]),
                ("description", "description.what"),
            ),
            (
                s1_with(&[("/stories/1/status", json!("done"))]),
                ("stories", "stories[1].status"),
            ),
            (
                s1_with(&[("/roadmap/1/depends_on", json!(["1"]))]),
                ("roadmap", "roadmap[1].depends_on[0]"),
            ),
            (
                s1_with(&[("/roadmap/0/detail", json!(7))]),
                ("roadmap", "roadmap[0].detail"),
            ),
            (json!([]), ("submission", "submission")),
        ];
        for (value, (section, field)) in cases {
            assert_eq!(refusal(&value), malformed(section, field), "{value}");
        }
    }

    #[test]
    fn an_empty_story_list_or_roadmap_accepted_is_caught() {
        assert_eq!(
            refusal(&s1_with(&[("/stories", json!([]))])),
            malformed("stories", "stories")
        );
        assert_eq!(
            refusal(&s1_with(&[("/roadmap", json!([]))])),
            malformed("roadmap", "roadmap")
        );
    }

    #[test]
    fn an_unknown_member_dropped_instead_of_refused_is_caught() {
        let cases = [
            (
                s1_adding("/roadmap/0", "owner", json!("x")),
                ("roadmap", "roadmap[0].owner"),
            ),
            (s1_adding("", "kind", json!("start")), ("kind", "kind")),
            (
                s1_adding("/description", "owner", json!("x")),
                ("description", "description.owner"),
            ),
            (
                s1_adding("/stories/0", "owner", json!("x")),
                ("stories", "stories[0].owner"),
            ),
        ];
        for (value, (section, field)) in cases {
            assert_eq!(refusal(&value), malformed(section, field), "{value}");
        }
    }

    #[test]
    fn a_phase_naming_a_missing_deferred_excluded_or_repeated_story_accepted_is_caught() {
        let cases = [
            (
                s1_with(&[("/roadmap/0/stories", json!(["nope"]))]),
                "roadmap[0].stories[0]",
            ),
            (
                s1_with(&[("/roadmap/0/stories", json!(["later"]))]),
                "roadmap[0].stories[0]",
            ),
            (
                s1_with(&[("/roadmap/0/stories", json!(["record", "record"]))]),
                "roadmap[0].stories[1]",
            ),
            (
                s1_with(&[
                    ("/stories/2/status", json!("excluded")),
                    ("/roadmap/1/stories", json!(["approve", "later"])),
                ]),
                "roadmap[1].stories[1]",
            ),
        ];
        for (value, field) in cases {
            assert_eq!(refusal(&value), malformed("roadmap", field), "{value}");
        }
    }

    #[test]
    fn a_repeated_story_label_accepted_is_caught() {
        assert_eq!(
            refusal(&s1_with(&[("/stories/2/label", json!("record"))])),
            malformed("stories", "stories[2].label")
        );
    }

    #[test]
    fn a_dependency_outside_the_roadmap_accepted_or_a_self_dependency_called_malformed_is_caught() {
        let with = |depends_on: Value| s1_with(&[("/roadmap/1/depends_on", depends_on)]);
        assert_eq!(
            refusal(&with(json!([0]))),
            malformed("roadmap", "roadmap[1].depends_on[0]")
        );
        assert_eq!(
            refusal(&with(json!([3]))),
            malformed("roadmap", "roadmap[1].depends_on[0]")
        );
        assert_eq!(
            refusal(&with(json!([1, 1]))),
            malformed("roadmap", "roadmap[1].depends_on[1]")
        );
        assert_eq!(
            refusal(&with(json!([2]))),
            SubmissionRefusal::DependencyCycle { phases: vec![2] }
        );
    }

    #[test]
    fn a_longer_cycle_named_partly_a_blocked_phase_named_on_a_cycle_or_a_forward_dependency_refused_is_caught()
     {
        let with_reads = |first_depends_on: Value| {
            let mut value = s1_with(&[("/roadmap/0/depends_on", first_depends_on)]);
            value["roadmap"].as_array_mut().unwrap().push(json!(
                {"name": "Reads", "goal": "g", "detail": "d", "depends_on": [2], "stories": []}
            ));
            value
        };
        assert_eq!(
            refusal(&with_reads(json!([3]))),
            SubmissionRefusal::DependencyCycle {
                phases: vec![1, 2, 3]
            }
        );
        assert_eq!(
            refusal(&with_reads(json!([2]))),
            SubmissionRefusal::DependencyCycle { phases: vec![1, 2] }
        );
        let forward = s1_with(&[
            ("/roadmap/0/depends_on", json!([2])),
            ("/roadmap/1/depends_on", json!([])),
        ]);
        let submission = judge_submission(&forward).expect("a forward dependency is accepted");
        assert_eq!(submission.phases[0].depends_on, [2]);
    }

    #[test]
    fn a_story_listed_by_two_phases_named_by_the_wrong_phases_is_caught() {
        let mut value = s1();
        value["roadmap"].as_array_mut().unwrap().push(json!(
            {"name": "Reads", "goal": "g", "detail": "d", "depends_on": [], "stories": ["record"]}
        ));
        assert_eq!(
            refusal(&value),
            SubmissionRefusal::StoryDoubleAssigned {
                story: "record".into(),
                phases: [1, 3]
            }
        );
    }

    #[test]
    fn an_accepted_submission_resolving_positions_or_labels_wrongly_is_caught() {
        let s3 = s1_with(&[
            ("/roadmap/0/stories", json!(["approve"])),
            ("/roadmap/1/stories", json!(["record"])),
        ]);
        for (value, first, second) in [(s1(), "record", "approve"), (s3, "approve", "record")] {
            let submission = judge_submission(&value).expect("accepted");
            let story_labels: Vec<&str> = submission
                .stories
                .iter()
                .map(|story| story.label.as_str())
                .collect();
            assert_eq!(story_labels, ["record", "approve", "later"]);
            assert_eq!(submission.phases.len(), 2);
            assert!(submission.phases[0].depends_on.is_empty());
            assert_eq!(labels(&submission, &submission.phases[0].stories), [first]);
            assert_eq!(submission.phases[1].depends_on, [1]);
            assert_eq!(labels(&submission, &submission.phases[1].stories), [second]);
        }
    }
}
