//! The judgement of one submitted plan whole (design 0005 sections 3 and 6,
//! PLN-R11 to PLN-R13).
//!
//! A plan arrives as plain JSON and the phase's facts as plain values the
//! binary gathered from its views. The plan is judged in one fixed order,
//! the first fault answering, so one plan always gets one answer: (1) the
//! plan's own members, task by task within `tasks` and item by item within
//! `evidence_map`; (2) the `questions` member through the question set
//! judge; (3) each task file inside the lease; (4) no check field with a
//! code blank; (5) a committed story holding a truth; (6) every story the
//! plan serves committed to the phase; (7) each evidence item bound to its
//! own story's current truth; (8) each item id holding one definition
//! across the phase; (9) each link's value named by its truth; (10) every
//! truth the plan serves named by an item of the phase; (11) each such
//! truth holding a check; (12) each such truth a person must prove holding
//! an observation; (13) each such truth holding one check at most;
//! (14) the plan's tasks within the phase's capacity. The judge trusts the
//! facts and judges the plan. Nothing here records anything.

use std::num::NonZeroU32;

use serde_json::{Map, Value, json};

use super::evidence::{self, EvidenceItem};
use super::task::{self, Task};
use super::{binding, capacity, content, coverage, link, stories};
use crate::questions::{QuestionSet, SetRefusal};

/// The refusal code for a plan out of its grammar: not an object, or a
/// member unknown, missing, of another type or out of range. It names the
/// path of the first fault.
pub const TYPED_CONTENT: &str = "typed-content";
/// The refusal code for a task naming a file outside the plan's lease.
pub const LEASE: &str = "lease";
/// The refusal code for a check whose command is blank.
pub const CHECK_COMMAND: &str = "check-command";
/// The refusal code for a check whose expected value is blank.
pub const CHECK_EXPECTED: &str = "check-expected";
/// The refusal code for a check whose test file is blank.
pub const CHECK_TEST_FILE: &str = "check-test-file";
/// The refusal code for a check whose boundary is blank, at either level
/// (D-07).
pub const CHECK_BOUNDARY: &str = "check-boundary";
/// The refusal code for a plan for a phase none of whose committed stories
/// holds a current truth.
pub const NO_TRUTHS: &str = "no-truths";
/// The refusal code for a plan serving a story not committed to the phase.
pub const STORY_NOT_IN_PHASE: &str = "story-not-in-phase";
/// The refusal code for an evidence item naming no current truth of a story
/// the plan serves.
pub const EVIDENCE_ITEM_TRUTH: &str = "evidence-item-truth";
/// The refusal code for an evidence item naming its truth at a version
/// other than the current one.
pub const TRUTH_VERSION_MISMATCH: &str = "truth-version-mismatch";
/// The refusal code for an evidence item whose id an approved plan of the
/// phase holds with another definition.
pub const EVIDENCE_ITEM_CONFLICT: &str = "evidence-item-conflict";
/// The refusal code for a link whose value its truth does not name.
pub const LINK_VALUE_NOT_NAMED: &str = "link-value-not-named";
/// The refusal code for a truth the plan serves that no evidence item of
/// the phase names.
pub const UNCOVERED_TRUTH: &str = "uncovered-truth";
/// The refusal code for a truth the plan serves whose evidence items across
/// the phase hold no check.
pub const TRUTH_WITHOUT_CHECK: &str = "truth-without-check";
/// The refusal code for a truth the plan serves that a person must prove
/// live (`human_verify`) and whose evidence items across the phase hold no
/// observation (D-04). Its one check is still required.
pub const TRUTH_WITHOUT_OBSERVATION: &str = "truth-without-observation";
/// The refusal code for a truth the plan serves whose evidence items across
/// the phase hold more than one distinct check for its current version.
pub const TRUTH_CHECK_LIMIT: &str = "truth-check-limit";
/// The refusal code for a plan that takes the phase over its capacity.
pub const OVER_CAPACITY: &str = "over-capacity";

/// Why a plan is refused. Positions count from 0 in the list as submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRefusal {
    /// The plan breaks its grammar at `path`: member names joined by `.`
    /// and a list position written `[<n>]`, such as `tasks[0].verify[1]`.
    /// `None` when the plan is not an object.
    TypedContent { path: Option<String> },
    /// The task at `position`, whose id is `id`, names `file`, which lies
    /// outside the plan's lease.
    Lease {
        position: usize,
        id: String,
        file: String,
    },
    /// The question set judge refused the `questions` member: its refusal
    /// unchanged, positions counting within `questions` (D-10).
    Questions(SetRefusal),
    /// The check at `position`, whose id is `id`, has a blank command.
    CheckCommand { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank expected
    /// value.
    CheckExpected { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank test file.
    CheckTestFile { position: usize, id: String },
    /// The check at `position`, whose id is `id`, has a blank boundary.
    CheckBoundary { position: usize, id: String },
    /// No committed story of the phase holds a current truth.
    NoTruths,
    /// The plan serves `story`, which is not committed to the phase.
    StoryNotInPhase { story: u32 },
    /// The item at `position`, whose id is `id`, names no current truth of
    /// a story the plan serves.
    EvidenceItemTruth { position: usize, id: String },
    /// The item at `position`, whose id is `id`, names its truth at a
    /// version other than `current`, the truth's current version.
    TruthVersionMismatch {
        position: usize,
        id: String,
        current: u32,
    },
    /// The item at `position`, whose id is `id`, differs from the item of
    /// approved plan `plan` that holds the same id.
    EvidenceItemConflict {
        position: usize,
        id: String,
        plan: u32,
    },
    /// The link at `position`, whose id is `id`, carries a value its truth
    /// does not name.
    LinkValueNotNamed { position: usize, id: String },
    /// No item of the phase names `truth` of `story` at `version`, its
    /// current version.
    UncoveredTruth {
        story: u32,
        truth: String,
        version: u32,
    },
    /// The items of the phase naming `truth` of `story` at `version`, its
    /// current version, hold no check.
    TruthWithoutCheck {
        story: u32,
        truth: String,
        version: u32,
    },
    /// `truth` of `story` at `version`, its current version, needs a person
    /// to prove it, and the items of the phase naming it hold no
    /// observation.
    TruthWithoutObservation {
        story: u32,
        truth: String,
        version: u32,
    },
    /// The items of the phase naming `truth` of `story` at `version`, its
    /// current version, hold the distinct checks `checks`, more than one,
    /// in the order the approved plans and then this plan list them.
    TruthCheckLimit {
        story: u32,
        truth: String,
        version: u32,
        checks: Vec<String>,
    },
    /// The plan, serving `stories` in its order, would take the phase to
    /// `size` tasks, over its capacity. Which story waits is the owner's
    /// choice (PLN-R10).
    OverCapacity { stories: Vec<u32>, size: u64 },
}

impl PlanRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypedContent { .. } => TYPED_CONTENT,
            Self::Lease { .. } => LEASE,
            Self::Questions(refusal) => refusal.code(),
            Self::CheckCommand { .. } => CHECK_COMMAND,
            Self::CheckExpected { .. } => CHECK_EXPECTED,
            Self::CheckTestFile { .. } => CHECK_TEST_FILE,
            Self::CheckBoundary { .. } => CHECK_BOUNDARY,
            Self::NoTruths => NO_TRUTHS,
            Self::StoryNotInPhase { .. } => STORY_NOT_IN_PHASE,
            Self::EvidenceItemTruth { .. } => EVIDENCE_ITEM_TRUTH,
            Self::TruthVersionMismatch { .. } => TRUTH_VERSION_MISMATCH,
            Self::EvidenceItemConflict { .. } => EVIDENCE_ITEM_CONFLICT,
            Self::LinkValueNotNamed { .. } => LINK_VALUE_NOT_NAMED,
            Self::UncoveredTruth { .. } => UNCOVERED_TRUTH,
            Self::TruthWithoutCheck { .. } => TRUTH_WITHOUT_CHECK,
            Self::TruthWithoutObservation { .. } => TRUTH_WITHOUT_OBSERVATION,
            Self::TruthCheckLimit { .. } => TRUTH_CHECK_LIMIT,
            Self::OverCapacity { .. } => OVER_CAPACITY,
        }
    }

    /// What the refusal names, as its answer's details, under the names the
    /// design's refusal rows use, or none for `no-truths`, which names
    /// nothing beyond the phase the caller already sent.
    pub fn details(&self) -> Option<Value> {
        let item = |position: &usize, id: &String| json!({"position": position, "id": id});
        let truth = |story: &u32, truth: &String, version: &u32| json!({"story": story, "truth": truth, "version": version});
        Some(match self {
            Self::TypedContent { path } => json!({"path": path}),
            Self::Lease { position, id, file } => {
                json!({"position": position, "id": id, "file": file})
            }
            Self::Questions(refusal) => match refusal {
                SetRefusal::TypedContent { position } => json!({"position": position}),
                SetRefusal::QuestionId { position, id }
                | SetRefusal::QuestionBlank { position, id }
                | SetRefusal::QuestionDependency { position, id } => item(position, id),
                SetRefusal::QuestionCycle { questions } => {
                    let ids: Vec<&str> = questions.iter().map(|(_, id)| id.as_str()).collect();
                    json!({"questions": ids})
                }
            },
            Self::CheckCommand { position, id }
            | Self::CheckExpected { position, id }
            | Self::CheckTestFile { position, id }
            | Self::CheckBoundary { position, id }
            | Self::EvidenceItemTruth { position, id }
            | Self::LinkValueNotNamed { position, id } => item(position, id),
            Self::NoTruths => return None,
            Self::StoryNotInPhase { story } => json!({"story": story}),
            Self::TruthVersionMismatch {
                position,
                id,
                current,
            } => json!({"position": position, "id": id, "current": current}),
            Self::EvidenceItemConflict { position, id, plan } => {
                json!({"position": position, "id": id, "plan": plan})
            }
            Self::UncoveredTruth {
                story,
                truth: id,
                version,
            }
            | Self::TruthWithoutCheck {
                story,
                truth: id,
                version,
            }
            | Self::TruthWithoutObservation {
                story,
                truth: id,
                version,
            } => truth(story, id, version),
            Self::TruthCheckLimit {
                story,
                truth: id,
                version,
                checks,
            } => {
                let mut named = truth(story, id, version);
                named["checks"] = json!(checks);
                named
            }
            Self::OverCapacity { stories, size } => json!({"stories": stories, "size": size}),
        })
    }

    /// A `typed-content` refusal naming `path`.
    pub(super) fn typed(path: impl Into<String>) -> Self {
        Self::TypedContent {
            path: Some(path.into()),
        }
    }
}

/// The kind of an evidence item (design 0005 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// A file or record that must exist.
    Artifact,
    /// A value that must pass from a caller to a callee.
    Link,
    /// A command whose result proves the truth.
    Check,
    /// Something a named person must see.
    Observation,
}

/// What a plan is judged on besides itself. The binary gathers every value
/// from its views (D-09) and the judge trusts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFacts {
    /// The phase's committed stories, in commitment order.
    pub stories: Vec<CommittedStory>,
    /// The evidence items of the phase's approved plans.
    pub approved: Vec<ApprovedItem>,
    /// The phase's current size: the tasks of its approved plans, each
    /// plan's counted once.
    pub size: u32,
    /// The phase's capacity in tasks, or `None` when none is set.
    pub capacity: Option<NonZeroU32>,
}

/// One story committed to the phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedStory {
    /// The story's id.
    pub id: u32,
    /// Its current truths, in the story's order. Empty when the story holds
    /// no approved truth.
    pub truths: Vec<CurrentTruth>,
}

/// One current truth of a committed story, its slot texts as approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTruth {
    /// Its id, unique within its story.
    pub id: String,
    /// Its current version.
    pub version: u32,
    /// The one trigger, or a `for-any` truth's input.
    pub trigger: String,
    /// The one observer.
    pub observer: String,
    /// The one outcome.
    pub outcome: String,
    /// The tool or live service a person needs to prove it, if any.
    pub human_verify: Option<String>,
}

/// One evidence item of an approved plan of the phase, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedItem {
    /// The number of the approved plan that holds it.
    pub plan: u32,
    /// Its id.
    pub id: String,
    /// The story of the truth it serves.
    pub story: u32,
    /// The id of the truth it serves.
    pub truth: String,
    /// The version of the truth it serves.
    pub truth_version: u32,
    /// Its kind.
    pub kind: EvidenceKind,
    /// Its whole definition as recorded.
    pub definition: Value,
}

/// A plan judged whole and accepted, every member kept exactly as
/// submitted. Only [`judge_plan`] builds one, so nothing unjudged can be
/// recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub(super) phase: u32,
    pub(super) stories: Vec<u32>,
    pub(super) files: Vec<String>,
    pub(super) directories: Vec<String>,
    pub(super) goal: String,
    pub(super) context: String,
    pub(super) notes: String,
    pub(super) tasks: Vec<Task>,
    pub(super) evidence_map: Vec<EvidenceItem>,
    pub(super) questions: QuestionSet,
    // The plan object as submitted, kept so its draft body is exactly what
    // was judged (D-05).
    pub(super) submitted: Map<String, Value>,
}

impl Plan {
    /// The phase the plan is for.
    pub fn phase(&self) -> u32 {
        self.phase
    }

    /// The stories the plan serves, in submitted order.
    pub fn stories(&self) -> &[u32] {
        &self.stories
    }

    /// The lease's files, in submitted order.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// The lease's directories, in submitted order.
    pub fn directories(&self) -> &[String] {
        &self.directories
    }

    /// The plan's goal.
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// The plan's context.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// The plan's notes.
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// The tasks, in submitted order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The evidence items, in submitted order.
    pub fn evidence_map(&self) -> &[EvidenceItem] {
        &self.evidence_map
    }

    /// The plan's accepted question set, its questions as written.
    pub fn questions(&self) -> &QuestionSet {
        &self.questions
    }

    /// The plan's size: the number of its tasks.
    pub fn size(&self) -> usize {
        self.tasks.len()
    }
}

/// Judges the submitted plan `submitted` against the phase's `facts` in the
/// fixed order, the first fault answering: (1) the plan's own members;
/// (2) the `questions` member through `judge_set`; (3) `lease`, task by
/// task and file by file; (4) the blank check fields, item by item;
/// (5) `no-truths`; (6) `story-not-in-phase`, in the plan's story order;
/// (7) `evidence-item-truth` and `truth-version-mismatch`, item by item;
/// (8) `evidence-item-conflict`, item by item;
/// (9) `link-value-not-named`, link by link; then over the served truths
/// in order, with the approved plans' items, (10) `uncovered-truth`,
/// (11) `truth-without-check`, (12) `truth-without-observation` and
/// (13) `truth-check-limit`; last, (14) `over-capacity`.
pub fn judge_plan(submitted: &Value, facts: &PlanFacts) -> Result<Plan, PlanRefusal> {
    let plan = content::read(submitted)?;
    task::within_lease(&plan.tasks, &plan.files, &plan.directories)?;
    evidence::blank_fields(&plan.evidence_map)?;
    stories::holds_truths(facts)?;
    stories::committed(&plan.stories, facts)?;
    let truths = binding::bind(&plan.stories, &plan.evidence_map, facts)?;
    // The member walk has read `evidence_map` as a list by now.
    let definitions = submitted["evidence_map"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    coverage::consistent(definitions, &plan.evidence_map, &facts.approved)?;
    link::named(&plan.evidence_map, &truths)?;
    let phase = coverage::Phase::new(&plan.stories, &plan.evidence_map, facts);
    phase.covered()?;
    phase.checked()?;
    phase.observed()?;
    phase.limited()?;
    capacity::within(&plan.stories, plan.tasks.len(), facts)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;
    use crate::plan::fixtures::{
        a1, a2, accepted, ar, at, c3, c5, dropping, f0, f1, f3, l1, p1, p2, plus, qa, qb, refusal,
        replacing, typed, under, with, without,
    };
    use crate::questions::Question;

    #[test]
    fn a_question_refusal_renamed_or_a_plan_question_rule_added_is_caught() {
        let facts = f1();
        let plan = accepted(&with(p1(), "", "questions", json!([qa(), qb()])), &facts);
        assert_eq!(
            plan.questions().questions(),
            [
                Question {
                    id: "q1".into(),
                    question: "Which store backs the ledger?".into(),
                    recommended: "SQLite".into(),
                    depends_on: Vec::new(),
                },
                Question {
                    id: "q2".into(),
                    question: "Where does the ledger live?".into(),
                    recommended: "Outside the checkout".into(),
                    depends_on: vec!["q1".into()],
                },
            ]
        );
        let twice = refusal(&with(p1(), "", "questions", json!([qa(), qa()])), &facts);
        assert_eq!(
            twice,
            PlanRefusal::Questions(SetRefusal::QuestionId {
                position: 1,
                id: "q1".into()
            })
        );
        assert_eq!(twice.code(), "question-id");
        let waiting = refusal(&with(p1(), "", "questions", json!([qb()])), &facts);
        assert_eq!(
            waiting,
            PlanRefusal::Questions(SetRefusal::QuestionDependency {
                position: 0,
                id: "q2".into()
            })
        );
        assert_eq!(waiting.code(), "question-dependency");
    }

    #[test]
    fn a_plan_refusal_answered_without_what_it_names_is_caught() {
        let cases = [
            (
                typed("tasks[0].verify[1]"),
                json!({"path": "tasks[0].verify[1]"}),
            ),
            (
                lease(0, "k1", "src/x.rs"),
                json!({"position": 0, "id": "k1", "file": "src/x.rs"}),
            ),
            (
                PlanRefusal::TruthVersionMismatch {
                    position: 0,
                    id: "c1".into(),
                    current: 2,
                },
                json!({"position": 0, "id": "c1", "current": 2}),
            ),
            (
                PlanRefusal::EvidenceItemConflict {
                    position: 0,
                    id: "c1".into(),
                    plan: 1,
                },
                json!({"position": 0, "id": "c1", "plan": 1}),
            ),
            (
                PlanRefusal::TruthCheckLimit {
                    story: 1,
                    truth: "t1".into(),
                    version: 1,
                    checks: vec!["c1".into(), "c3".into()],
                },
                json!({"story": 1, "truth": "t1", "version": 1, "checks": ["c1", "c3"]}),
            ),
            (
                PlanRefusal::OverCapacity {
                    stories: vec![1, 2],
                    size: 4,
                },
                json!({"stories": [1, 2], "size": 4}),
            ),
        ];
        for (refusal, details) in cases {
            assert_eq!(refusal.details(), Some(details), "{}", refusal.code());
        }
    }

    fn lease(position: usize, id: &str, file: &str) -> PlanRefusal {
        PlanRefusal::Lease {
            position,
            id: id.into(),
            file: file.into(),
        }
    }

    #[test]
    fn a_plan_with_bad_content_a_bad_lease_or_an_uncommitted_story_accepted_or_named_wrongly_is_caught()
     {
        let facts = f1();
        assert_eq!(accepted(&p1(), &facts).size(), 1);
        let mut over = vec![json!("docs")];
        over.extend((0..=254).map(|n| json!(format!("d/{n}"))));
        let cases = [
            (with(p1(), "", "plan", json!(1)), typed("plan")),
            (
                with(
                    p1(),
                    "",
                    "question_set",
                    json!("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d"),
                ),
                typed("question_set"),
            ),
            (with(p1(), "", "size", json!(1)), typed("size")),
            (with(p1(), "", "suite", json!("cargo test")), typed("suite")),
            (with(p1(), "", "owner", json!("Ada")), typed("owner")),
            (without(p1(), "", "goal"), typed("goal")),
            (with(p1(), "", "phase", json!("8")), typed("phase")),
            (
                with(p1(), "", "stories", json!([1, 1])),
                typed("stories[1]"),
            ),
            (
                with(p1(), "", "directories", json!(["docs/"])),
                typed("directories[0]"),
            ),
            (
                with(p1(), "", "files", json!(["/src/judge.rs"])),
                typed("files[0]"),
            ),
            (
                with(p1(), "", "files", json!(["src/../judge.rs"])),
                typed("files[0]"),
            ),
            (
                with(p1(), "", "files", json!(["src/judge.rs", "src/judge.rs"])),
                typed("files[1]"),
            ),
            (
                with(
                    with(p1(), "", "files", json!([])),
                    "",
                    "directories",
                    json!([]),
                ),
                typed("files"),
            ),
            (
                with(p1(), "", "directories", Value::Array(over)),
                typed("directories[255]"),
            ),
            (
                with(
                    p1(),
                    "/tasks/0",
                    "files",
                    json!(["src/judge.rs", "src/other.rs"]),
                ),
                lease(0, "k1", "src/other.rs"),
            ),
            (
                with(p1(), "", "stories", json!([1, 4])),
                PlanRefusal::StoryNotInPhase { story: 4 },
            ),
            (
                with(
                    p1(),
                    "",
                    "questions",
                    json!([{"id": "q1", "question": "Which store backs the ledger?",
                        "recommended": "SQLite", "depends_on": ["q9"]}]),
                ),
                PlanRefusal::Questions(SetRefusal::QuestionDependency {
                    position: 0,
                    id: "q1".into(),
                }),
            ),
            (
                with(p1(), "", "questions", json!("none")),
                PlanRefusal::Questions(SetRefusal::TypedContent { position: None }),
            ),
        ];
        for (plan, refused) in cases {
            assert_eq!(refusal(&plan, &facts), refused, "{plan}");
        }
        assert_eq!(refusal(&p1(), &f0()), PlanRefusal::NoTruths);
    }

    #[test]
    fn faults_judged_in_another_order_are_caught() {
        let facts = f1();
        let other = |plan| with(plan, "/tasks/0", "files", json!(["src/other.rs"]));
        let blank = |plan| with(plan, "/evidence_map/0/spec", "command", json!(""));
        let waiting = |plan| with(plan, "", "questions", json!([qb()]));
        let owner = with(p1(), "", "owner", json!("Ada"));
        let cases = [
            (
                with(owner, "", "directories", json!(["docs/"])),
                &facts,
                typed("owner"),
            ),
            (waiting(without(p1(), "", "goal")), &facts, typed("goal")),
            (
                waiting(other(p1())),
                &facts,
                PlanRefusal::Questions(SetRefusal::QuestionDependency {
                    position: 0,
                    id: "q2".into(),
                }),
            ),
            (blank(other(p1())), &facts, lease(0, "k1", "src/other.rs")),
        ];
        for (plan, facts, refused) in cases {
            assert_eq!(refusal(&plan, facts), refused, "{plan}");
        }
        assert_eq!(
            refusal(&blank(p1()), &f0()),
            PlanRefusal::CheckCommand {
                position: 0,
                id: "c1".into()
            }
        );
        assert_eq!(
            refusal(&with(p1(), "", "stories", json!([1, 4])), &f0()),
            PlanRefusal::NoTruths
        );
    }

    fn served(story: u32, truth: &str, version: u32) -> (u32, String, u32) {
        (story, truth.into(), version)
    }

    #[test]
    fn an_evidence_map_judged_per_plan_or_by_truth_id_alone_is_caught() {
        let facts = f1();
        accepted(&p1(), &facts);
        accepted(&p2(), &facts);
        let (story, truth, version) = served(1, "t1", 1);
        assert_eq!(
            refusal(&dropping(p1(), &["c1"]), &facts),
            PlanRefusal::UncoveredTruth {
                story,
                truth,
                version
            }
        );
        accepted(&dropping(p1(), &["c1"]), &under(a2()));
        let (story, truth, version) = served(1, "t2", 2);
        assert_eq!(
            refusal(&dropping(p1(), &["c2"]), &facts),
            PlanRefusal::TruthWithoutCheck {
                story,
                truth,
                version
            }
        );
        let (story, truth, version) = served(1, "t2", 2);
        assert_eq!(
            refusal(&dropping(p1(), &["o1"]), &facts),
            PlanRefusal::TruthWithoutObservation {
                story,
                truth,
                version
            }
        );
        let (story, truth, version) = served(1, "t1", 1);
        assert_eq!(
            refusal(&plus(p1(), c3()), &facts),
            PlanRefusal::TruthCheckLimit {
                story,
                truth,
                version,
                checks: vec!["c1".into(), "c3".into()]
            }
        );
        accepted(&p1(), &under(a2()));
        let untrue = PlanRefusal::EvidenceItemTruth {
            position: 0,
            id: "c1".into(),
        };
        let first = "/evidence_map/0";
        assert_eq!(
            refusal(&with(p1(), first, "truth", json!("t9")), &facts),
            untrue
        );
        assert_eq!(
            refusal(&with(p1(), first, "story", json!(2)), &facts),
            untrue
        );
        let stale = PlanRefusal::TruthVersionMismatch {
            position: 0,
            id: "c1".into(),
            current: 1,
        };
        assert_eq!(
            refusal(&with(p1(), first, "truth_version", json!(2)), &facts),
            stale
        );
        assert_eq!(
            refusal(&with(p2(), first, "truth_version", json!(3)), &facts),
            stale
        );
        assert_eq!(
            refusal(&p1(), &under(a1())),
            PlanRefusal::EvidenceItemConflict {
                position: 0,
                id: "c1".into(),
                plan: 1
            }
        );
        accepted(&plus(p1(), l1()), &facts);
        assert_eq!(
            refusal(
                &plus(p1(), with(l1(), "/spec", "value", json!("id"))),
                &facts
            ),
            PlanRefusal::LinkValueNotNamed {
                position: 3,
                id: "l1".into()
            }
        );
        accepted(&replacing(p2(), "c4", c5()), &f3());
        let (story, truth, version) = served(2, "t1", 1);
        assert_eq!(
            refusal(&dropping(p2(), &["c4"]), &f3()),
            PlanRefusal::UncoveredTruth {
                story,
                truth,
                version
            }
        );
    }

    #[test]
    fn phase_faults_judged_in_another_order_are_caught() {
        let unnamed = || with(l1(), "/spec", "value", json!("id"));
        let artifact = with(
            with(with(ar(), "", "id", json!("a2")), "", "truth", json!("t2")),
            "",
            "truth_version",
            json!(2),
        );
        let codes = [
            (
                with(p1(), "/evidence_map/0", "truth", json!("t9")),
                under(a1()),
                "evidence-item-truth",
            ),
            (plus(p1(), unnamed()), under(a1()), "evidence-item-conflict"),
            (
                plus(dropping(p1(), &["c2"]), unnamed()),
                f1(),
                "link-value-not-named",
            ),
            (
                plus(dropping(p1(), &["c2", "o1"]), unnamed()),
                f1(),
                "link-value-not-named",
            ),
            (plus(p1(), c3()), at(1, 1), "truth-check-limit"),
            (
                with(p1(), "/evidence_map/0/spec", "command", json!("")),
                at(1, 5),
                "check-command",
            ),
        ];
        for (plan, facts, code) in codes {
            assert_eq!(refusal(&plan, &facts).code(), code, "{plan}");
        }
        let facts = f1();
        let cases = [
            (
                dropping(p1(), &["c1", "c2"]),
                PlanRefusal::UncoveredTruth {
                    story: 1,
                    truth: "t1".into(),
                    version: 1,
                },
            ),
            (
                dropping(replacing(p1(), "c2", artifact), &["o1"]),
                PlanRefusal::TruthWithoutCheck {
                    story: 1,
                    truth: "t2".into(),
                    version: 2,
                },
            ),
            (
                dropping(plus(p1(), c3()), &["o1"]),
                PlanRefusal::TruthWithoutObservation {
                    story: 1,
                    truth: "t2".into(),
                    version: 2,
                },
            ),
        ];
        for (plan, refused) in cases {
            assert_eq!(refusal(&plan, &facts), refused, "{plan}");
        }
    }
}
