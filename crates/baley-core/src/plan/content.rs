//! Step (1) of the plan judge: the plan's own members read in a fixed order
//! (PLN-R11, D-03), the first fault answering `typed-content` with its
//! path. No text is trimmed or rewritten.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::judge::{Plan, PlanRefusal};
use super::{evidence, lease, task};
use crate::questions::judge_set;

/// The plan's members, in the order they are judged. The plan number, the
/// question set, the size and the suite are written by Baley at approval,
/// so a plan naming one is refused as naming an unknown member (D-03).
const MEMBERS: [&str; 10] = [
    "phase",
    "stories",
    "files",
    "directories",
    "goal",
    "context",
    "notes",
    "tasks",
    "evidence_map",
    "questions",
];

/// Reads a plan into the accepted plan: an unknown member first, the first
/// by name, then each member in the judge's order: `phase`, `stories`,
/// `files`, `directories` with the lease rules, `goal`, `context`, `notes`,
/// `tasks`, `evidence_map`, then the presence of `questions`; then step (2),
/// the `questions` value through the question set judge.
pub(super) fn read(value: &Value) -> Result<Plan, PlanRefusal> {
    let Value::Object(plan) = value else {
        return Err(PlanRefusal::TypedContent { path: None });
    };
    if let Some(unknown) = first_unknown(plan, &MEMBERS) {
        return Err(PlanRefusal::typed(unknown));
    }
    let phase = counter(plan.get("phase")).ok_or_else(|| PlanRefusal::typed("phase"))?;
    // A repeated story is named at its later position.
    let mut seen = HashSet::new();
    let stories = list(plan, "stories")?
        .iter()
        .enumerate()
        .map(|(n, story)| {
            counter(Some(story))
                .filter(|id| seen.insert(*id))
                .ok_or_else(|| PlanRefusal::typed(format!("stories[{n}]")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let files = list(plan, "files")?;
    let directories = list(plan, "directories")?;
    let (files, directories) = lease::read(files, directories)?;
    let goal = text(plan, "goal")?;
    let context = text(plan, "context")?;
    let notes = text(plan, "notes")?;
    let tasks = task::read(list(plan, "tasks")?)?;
    let evidence_map = evidence::read(list(plan, "evidence_map")?)?;
    let questions = plan
        .get("questions")
        .ok_or_else(|| PlanRefusal::typed("questions"))?;
    // Step (2) follows at once: the value through the question set judge,
    // its refusal passed on unchanged (D-10).
    let questions = judge_set(questions).map_err(PlanRefusal::Questions)?;
    Ok(Plan {
        phase,
        stories,
        files,
        directories,
        goal,
        context,
        notes,
        tasks,
        evidence_map,
        questions,
        submitted: plan.clone(),
    })
}

/// The first member of `object`, by name, that is not among `members`.
pub(super) fn first_unknown<'a>(
    object: &'a Map<String, Value>,
    members: &[&str],
) -> Option<&'a str> {
    // Key order depends on serde_json's features, so the first is chosen.
    object
        .keys()
        .map(String::as_str)
        .filter(|name| !members.contains(name))
        .min()
}

/// A whole number from 1 to `u32::MAX`, as phase, story and version
/// counters run,
/// or `None` when the value is missing or anything else.
pub(super) fn counter(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .filter(|number| *number >= 1)
        .and_then(|number| u32::try_from(number).ok())
}

/// The list member `name`, refused naming it when missing or not a list.
fn list<'a>(plan: &'a Map<String, Value>, name: &str) -> Result<&'a Vec<Value>, PlanRefusal> {
    match plan.get(name) {
        Some(Value::Array(entries)) => Ok(entries),
        _ => Err(PlanRefusal::typed(name)),
    }
}

/// The text member `name` as written, blank allowed, refused naming it when
/// missing or not text.
fn text(plan: &Map<String, Value>, name: &str) -> Result<String, PlanRefusal> {
    match plan.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(PlanRefusal::typed(name)),
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, f1, p0, p1, refusal, typed, with, without};

    #[test]
    fn a_plan_member_missing_or_of_another_type_accepted_is_caught() {
        let facts = f1();
        assert_eq!(
            refusal(&json!([p1()]), &facts),
            PlanRefusal::TypedContent { path: None }
        );
        for member in [
            "phase",
            "stories",
            "files",
            "directories",
            "goal",
            "context",
            "notes",
            "tasks",
            "evidence_map",
            "questions",
        ] {
            assert_eq!(
                refusal(&without(p1(), "", member), &facts),
                typed(member),
                "without {member}"
            );
        }
        let cases = [
            (with(p1(), "", "phase", json!(8.5)), "phase"),
            (with(p1(), "", "stories", json!("1")), "stories"),
            (with(p1(), "", "stories", json!(["1"])), "stories[0]"),
            (with(p1(), "", "goal", json!(3)), "goal"),
            (with(p1(), "", "tasks", json!({})), "tasks"),
            (with(p1(), "", "evidence_map", json!(null)), "evidence_map"),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn an_accepted_plan_trimmed_or_an_empty_member_refused_is_caught() {
        let facts = f1();
        let plan = with(
            with(p1(), "", "goal", json!(" Judge a plan whole. ")),
            "",
            "context",
            json!(""),
        );
        let plan = accepted(&plan, &facts);
        assert_eq!(plan.goal(), " Judge a plan whole. ");
        assert_eq!(plan.context(), "");
        assert!(accepted(&p0(), &facts).stories().is_empty());
    }

    #[test]
    fn a_phase_or_story_number_out_of_range_or_repeated_accepted_is_caught() {
        let facts = f1();
        let cases = [
            (with(p1(), "", "phase", json!(0)), "phase"),
            (with(p1(), "", "phase", json!(4_294_967_296_u64)), "phase"),
            (with(p1(), "", "stories", json!([0])), "stories[0]"),
            (
                with(p1(), "", "stories", json!([4_294_967_297_u64])),
                "stories[0]",
            ),
            (with(p1(), "", "stories", json!([1, 2, 1])), "stories[2]"),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }

    #[test]
    fn faults_in_the_plan_members_judged_in_another_order_are_caught() {
        let facts = f1();
        let owner = with(p1(), "", "owner", json!("Ada"));
        let both = with(with(p1(), "", "zeta", json!(1)), "", "alpha", json!(1));
        let cases = [
            (without(owner, "", "goal"), "owner"),
            (both, "alpha"),
            (
                without(with(p1(), "", "phase", json!("8")), "", "goal"),
                "phase",
            ),
            (
                without(with(p1(), "", "stories", json!(["x"])), "", "goal"),
                "stories[0]",
            ),
        ];
        for (plan, path) in cases {
            assert_eq!(refusal(&plan, &facts), typed(path), "{plan}");
        }
    }
}
