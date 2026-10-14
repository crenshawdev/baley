//! The judgement of an approval by digest (design 0004 section 5).
//!
//! An approval is judged on facts the binary read: whether the target draft
//! is recorded, whether a start is already approved, and which draft is the
//! newest. The order is fixed, so one approval always gets one answer.

use serde_json::Value;

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
}
