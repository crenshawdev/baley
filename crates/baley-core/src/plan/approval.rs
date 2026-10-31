//! The judgement of an approval of a plan draft by digest (design 0005
//! section 5 `plan approve`, PLN-R15, PLN-R26, PLN-R27, D-16).
//!
//! An approval is judged on facts the binary read, in one fixed order, so
//! one approval always gets one answer: the draft, its staleness, an earlier
//! approval, then the draft's question set. Judging the plan again against
//! the phase as it stands is the binary's next step, with `judge_plan`.

use std::collections::HashSet;

use baley_store::Hash;
use serde_json::{Value, json};

use super::submission::{RecordedPlan, Replaces};
use crate::questions::SetFacts;
use crate::scope::{DRAFT_APPROVED, PriorApproval, STALE_DRAFT, UNKNOWN_DRAFT};
use crate::truth::QUESTION_OPEN;

/// The refusal code for approving a plan draft while an answer of its
/// question set rejected the recommended answer, so the plan does not yet
/// apply it (PLN-R27).
pub const ANSWER_NOT_APPLIED: &str = "answer-not-applied";

// The members in the order the plan judge reads them, which is the order a
// stale path is looked for in.
const PLAN: [&str; 10] = [
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
const TASK: [&str; 5] = ["id", "title", "files", "action", "verify"];
const ITEM: [&str; 7] = [
    "id",
    "story",
    "truth",
    "truth_version",
    "kind",
    "spec",
    "reason",
];
const ARTIFACT: [&str; 2] = ["path", "record"];
const LINK: [&str; 3] = ["caller", "callee", "value"];
const OBSERVATION: [&str; 2] = ["what", "by"];
const CHECK: [&str; 8] = [
    "command", "expected", "test", "setup", "call", "level", "boundary", "fakes",
];
const EXPECTED: [&str; 2] = ["kind", "value"];
const TEST: [&str; 2] = ["file", "function"];
const QUESTION: [&str; 4] = ["id", "question", "recommended", "depends_on"];

/// Why an approval of a plan draft is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanApprovalRefusal {
    /// No recorded plan draft has the digest.
    UnknownDraft,
    /// A newer plan draft of the phase was submitted. `path` names the
    /// first part that differs from the newest, such as `tasks[0].title`.
    StaleDraft { path: String },
    /// The draft was approved by `owner` at `at` after its latest
    /// submission.
    DraftApproved { owner: String, at: String },
    /// These questions of the draft's set are neither answered nor
    /// deferred, in submitted order.
    QuestionOpen { questions: Vec<String> },
    /// These questions' answers rejected the recommended answer, in
    /// submitted order.
    AnswerNotApplied { questions: Vec<String> },
}

impl PlanApprovalRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownDraft => UNKNOWN_DRAFT,
            Self::StaleDraft { .. } => STALE_DRAFT,
            Self::DraftApproved { .. } => DRAFT_APPROVED,
            Self::QuestionOpen { .. } => QUESTION_OPEN,
            Self::AnswerNotApplied { .. } => ANSWER_NOT_APPLIED,
        }
    }

    /// What the refusal names, as its answer's details, or none.
    pub fn details(&self) -> Option<Value> {
        match self {
            Self::UnknownDraft => None,
            Self::StaleDraft { path } => Some(json!({"path": path})),
            Self::DraftApproved { owner, at } => Some(json!({"owner": owner, "at": at})),
            Self::QuestionOpen { questions } | Self::AnswerNotApplied { questions } => {
                Some(json!({"questions": questions}))
            }
        }
    }
}

/// A recorded plan draft: its digest and its body read back.
#[derive(Debug, Clone, Copy)]
pub struct DraftRead<'a> {
    /// The draft's digest.
    pub digest: Hash,
    /// The draft's body read back.
    pub plan: &'a RecordedPlan,
}

/// What an approval of one plan draft is judged on.
#[derive(Debug, Clone, Copy)]
pub struct PlanApprovalFacts<'a> {
    /// The target draft, or `None` when no `draft` document has its
    /// digest.
    pub target: Option<DraftRead<'a>>,
    /// Whether the target's `scope.submitted` was found among the events.
    pub recorded: bool,
    /// The newest plan draft of the phase, or `None` when none is listed.
    pub newest: Option<DraftRead<'a>>,
    /// The `plan.approved` of this digest recorded after its latest
    /// submission, or `None`.
    pub approved: Option<PriorApproval<'a>>,
    /// Everything recorded about the target draft's question set.
    pub set: &'a SetFacts,
}

/// Judges an approval of a plan draft in D-16's order: `unknown-draft`
/// when the draft or its event is missing, or no newest draft is listed;
/// `stale-draft` when the newest plan draft of the phase is not this one,
/// with the first differing path; `draft-approved` naming the earlier
/// approval; `question-open` naming every question neither answered nor
/// deferred; and `answer-not-applied` naming every question whose answer
/// rejected its recommendation. A deferral leaves the plan on the
/// recommended answer, so it is never taken for a rejection (PLN-R27).
pub fn judge_plan_approval(facts: &PlanApprovalFacts) -> Result<(), PlanApprovalRefusal> {
    let Some(target) = facts.target.filter(|_| facts.recorded) else {
        return Err(PlanApprovalRefusal::UnknownDraft);
    };
    let Some(newest) = facts.newest else {
        return Err(PlanApprovalRefusal::UnknownDraft);
    };
    if newest.digest != target.digest {
        // A digest is taken over the plan, its kind and its binding, and
        // both are plan drafts, so they differ in the plan or the binding.
        let path =
            difference(target.plan, newest.plan).expect("differing digests differ in content");
        return Err(PlanApprovalRefusal::StaleDraft { path });
    }
    if let Some(approved) = facts.approved {
        return Err(PlanApprovalRefusal::DraftApproved {
            owner: approved.owner.into(),
            at: approved.at.into(),
        });
    }
    let set = facts.set;
    let answered: HashSet<&str> = set.answers.iter().map(|a| a.question.as_str()).collect();
    let deferred: HashSet<&str> = set.deferrals.iter().map(|d| d.question.as_str()).collect();
    let open: Vec<String> = set
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .filter(|id| !answered.contains(id) && !deferred.contains(id))
        .map(str::to_owned)
        .collect();
    if !open.is_empty() {
        return Err(PlanApprovalRefusal::QuestionOpen { questions: open });
    }
    let rejected: HashSet<&str> = set
        .answers
        .iter()
        .filter(|answer| !answer.accepted)
        .map(|answer| answer.question.as_str())
        .collect();
    let rejected: Vec<String> = set
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .filter(|id| rejected.contains(id))
        .map(str::to_owned)
        .collect();
    if !rejected.is_empty() {
        return Err(PlanApprovalRefusal::AnswerNotApplied {
            questions: rejected,
        });
    }
    Ok(())
}

/// The first part of two plan drafts that differs: `replaces`, then the
/// plan's members in the judge's order. `None` when they are equal.
fn difference(a: &RecordedPlan, b: &RecordedPlan) -> Option<String> {
    replaces(a.replaces, b.replaces).or_else(|| {
        PLAN.iter().find_map(|&name| {
            let (x, y) = (&a.plan[name], &b.plan[name]);
            match name {
                "tasks" => list(name, x, y, |at, x, y| members(&at, x, y, &TASK)),
                "evidence_map" => list(name, x, y, |at, x, y| item(&at, x, y)),
                "questions" => list(name, x, y, |at, x, y| members(&at, x, y, &QUESTION)),
                _ => leaf(name.into(), x, y),
            }
        })
    })
}

/// The replacement binding's path when the two differ: `replaces` when only
/// one draft holds it, its member otherwise.
fn replaces(a: Option<Replaces>, b: Option<Replaces>) -> Option<String> {
    match (a, b) {
        (None, None) => None,
        (Some(a), Some(b)) if a.plan != b.plan => Some("replaces.plan".into()),
        (Some(a), Some(b)) if a.digest != b.digest => Some("replaces.digest".into()),
        (Some(_), Some(_)) => None,
        _ => Some("replaces".into()),
    }
}

/// The first differing member of two evidence items, looking inside the
/// spec in the order of its kind. The kinds are equal by the time the spec
/// is compared, since `kind` is compared first.
fn item(path: &str, a: &Value, b: &Value) -> Option<String> {
    ITEM.iter().find_map(|&name| {
        let at = format!("{path}.{name}");
        let (x, y) = (&a[name], &b[name]);
        if name != "spec" {
            return leaf(at, x, y);
        }
        match a["kind"].as_str() {
            Some("artifact") => members(&at, x, y, &ARTIFACT),
            Some("link") => members(&at, x, y, &LINK),
            Some("observation") => members(&at, x, y, &OBSERVATION),
            Some("check") => CHECK.iter().find_map(|&member| {
                let inner = format!("{at}.{member}");
                let (x, y) = (&x[member], &y[member]);
                match member {
                    "expected" => members(&inner, x, y, &EXPECTED),
                    "test" => members(&inner, x, y, &TEST),
                    _ => leaf(inner, x, y),
                }
            }),
            _ => leaf(at, x, y),
        }
    })
}

/// The first of `names`, in that order, whose values differ under `path`.
/// A member one value lacks differs from one the other holds.
fn members(path: &str, a: &Value, b: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| leaf(format!("{path}.{name}"), &a[*name], &b[*name]))
}

/// `path` when two values differ, looking inside lists entry by entry.
fn leaf(path: String, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Array(_), Value::Array(_)) => list(&path, a, b, leaf),
        _ => (a != b).then_some(path),
    }
}

/// The first entry `each` finds differing, or the first position only one
/// list holds, or `path` itself when either value is not a list.
fn list(
    path: &str,
    a: &Value,
    b: &Value,
    each: impl Fn(String, &Value, &Value) -> Option<String>,
) -> Option<String> {
    let (Value::Array(a), Value::Array(b)) = (a, b) else {
        return (a != b).then(|| path.to_owned());
    };
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
    //! Expected answers are written out from D-16, design 0005's refusal
    //! table and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;
    use crate::plan::fixtures::{c4, p1, plus, qa, qb, with};
    use crate::questions::{Answer, Deferral, Question};

    const T: &str = "2026-10-31T09:00:03Z";

    fn x() -> Hash {
        Hash([0x5c; 32])
    }

    fn y() -> Hash {
        Hash([0x6d; 32])
    }

    fn p1q() -> Value {
        with(p1(), "", "questions", json!([qa(), qb()]))
    }

    fn recorded(plan: Value) -> RecordedPlan {
        RecordedPlan {
            plan,
            replaces: None,
        }
    }

    fn question(value: Value) -> Question {
        Question {
            id: value["id"].as_str().unwrap().into(),
            question: value["question"].as_str().unwrap().into(),
            recommended: value["recommended"].as_str().unwrap().into(),
            depends_on: value["depends_on"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().into())
                .collect(),
        }
    }

    fn answer(id: &str, accepted: bool) -> Answer {
        Answer {
            question: id.into(),
            answer: if accepted { "Yes" } else { "No" }.into(),
            accepted,
            round: 1,
            owner: "Ada Owner".into(),
            at: T.into(),
        }
    }

    fn deferral(id: &str) -> Deferral {
        Deferral {
            question: id.into(),
            reason: "Decide once the store is chosen.".into(),
            round: 1,
            owner: "Ada Owner".into(),
            at: T.into(),
        }
    }

    fn set(answers: Vec<Answer>, deferrals: Vec<Deferral>) -> SetFacts {
        SetFacts {
            questions: vec![question(qa()), question(qb())],
            answers,
            deferrals,
            abandoned: None,
        }
    }

    fn answer_of(facts: &PlanApprovalFacts) -> Result<(), (&'static str, Option<Value>)> {
        judge_plan_approval(facts).map_err(|refusal| (refusal.code(), refusal.details()))
    }

    #[test]
    fn a_plan_approval_judged_in_another_order_is_caught() {
        let target = recorded(p1q());
        let other = recorded(p1());
        let q2_open = set(vec![answer("q1", false)], Vec::new());
        let rejected = set(vec![answer("q1", false), answer("q2", true)], Vec::new());
        let applied = set(vec![answer("q1", true), answer("q2", true)], Vec::new());
        let mut facts = PlanApprovalFacts {
            target: Some(DraftRead {
                digest: x(),
                plan: &target,
            }),
            recorded: false,
            newest: Some(DraftRead {
                digest: y(),
                plan: &other,
            }),
            approved: Some(PriorApproval {
                owner: "Ada Owner",
                at: T,
            }),
            set: &q2_open,
        };

        assert_eq!(answer_of(&facts), Err(("unknown-draft", None)));
        facts.recorded = true;
        assert_eq!(
            answer_of(&facts),
            Err(("stale-draft", Some(json!({"path": "questions[0]"}))))
        );
        facts.newest = facts.target;
        assert_eq!(
            answer_of(&facts),
            Err((
                "draft-approved",
                Some(json!({"owner": "Ada Owner", "at": T}))
            ))
        );
        facts.approved = None;
        assert_eq!(
            answer_of(&facts),
            Err(("question-open", Some(json!({"questions": ["q2"]}))))
        );

        facts.set = &rejected;
        assert_eq!(
            answer_of(&facts),
            Err(("answer-not-applied", Some(json!({"questions": ["q1"]}))))
        );

        facts.set = &applied;
        assert_eq!(answer_of(&facts), Ok(()));
    }

    #[test]
    fn a_stale_plan_draft_named_at_the_wrong_path_is_caught() {
        let target = recorded(p1());
        let empty = set(Vec::new(), Vec::new());
        let stale = |newest: &RecordedPlan| {
            judge_plan_approval(&PlanApprovalFacts {
                target: Some(DraftRead {
                    digest: x(),
                    plan: &target,
                }),
                recorded: true,
                newest: Some(DraftRead {
                    digest: y(),
                    plan: newest,
                }),
                approved: None,
                set: &empty,
            })
        };
        let path = |path: &str| Err(PlanApprovalRefusal::StaleDraft { path: path.into() });

        let retitled = recorded(with(p1(), "/tasks/0", "title", json!("Another")));
        assert_eq!(stale(&retitled), path("tasks[0].title"));
        // `goal` comes before `tasks` in the judge's order.
        let regoaled = recorded(with(
            with(p1(), "", "goal", json!("Another goal.")),
            "/tasks/0",
            "title",
            json!("Another"),
        ));
        assert_eq!(stale(&regoaled), path("goal"));
        let extended = recorded(plus(p1(), c4()));
        assert_eq!(stale(&extended), path("evidence_map[3]"));
        let replacing = RecordedPlan {
            plan: p1(),
            replaces: Some(Replaces {
                plan: 1,
                digest: x(),
            }),
        };
        assert_eq!(stale(&replacing), path("replaces"));
    }

    #[test]
    fn a_deferral_taken_for_a_rejection_or_an_open_question_missed_is_caught() {
        let target = recorded(p1q());
        let draft = DraftRead {
            digest: x(),
            plan: &target,
        };
        let judged = |set: &SetFacts| {
            answer_of(&PlanApprovalFacts {
                target: Some(draft),
                recorded: true,
                newest: Some(draft),
                approved: None,
                set,
            })
        };

        assert_eq!(
            judged(&set(vec![answer("q1", true)], vec![deferral("q2")])),
            Ok(())
        );
        assert_eq!(
            judged(&set(vec![answer("q1", false)], vec![deferral("q2")])),
            Err(("answer-not-applied", Some(json!({"questions": ["q1"]}))))
        );
        assert_eq!(
            judged(&set(Vec::new(), Vec::new())),
            Err(("question-open", Some(json!({"questions": ["q1", "q2"]}))))
        );
    }
}
