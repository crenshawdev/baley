//! What one `phase-plan` call does with a selection, and whether a plan may
//! be submitted for its phase (design 0005 section 5 `phase plan` and `plan
//! submit`, PLN-R7, PRJ-R4, PRJ-R18, D-01, D-02, D-10, D-11, D-15).
//!
//! The selection's readiness is judged first, then the path: hand over an
//! approved continuation's order, refuse beside a pending planner order,
//! draft a changed selection, issue a due revision, or issue a planning
//! order. The binary reads every fact; nothing here reads a store.

use serde_json::{Value, json};

/// The refusal code for planning a phase while another phase is active; it
/// names that phase (PLN-R7).
pub const PHASE_ACTIVE: &str = "phase-active";
/// The refusal code for submitting a plan while a phase other than the
/// plan's is active; it names the active phase (D-02).
pub const PHASE_NOT_ACTIVE: &str = "phase-not-active";
/// The refusal code for a selected story with no approved truth; it names
/// the story (D-10).
pub const STORY_UNREFINED: &str = "story-unrefined";
/// The refusal code for a selected story committed to another phase; it
/// names the story and that phase (PRJ-R4).
pub const STORY_IN_PHASE: &str = "story-in-phase";
/// The refusal code for a `phase-plan` while a planner order of the phase
/// is pending; it names that order, since a phase has one live planner
/// order (D-11).
pub const PLANNER_ORDER_PENDING: &str = "planner-order-pending";

/// Why a `phase-plan` call is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitmentRefusal {
    /// Phase `phase`, another phase, is active.
    PhaseActive { phase: u32 },
    /// The roadmap does not hold phase `phase`, or it is withdrawn.
    NoSuchPhase { phase: u32 },
    /// The roadmap does not hold story `story`, or it is dropped.
    NoSuchStory { story: u32 },
    /// Story `story` holds no approved truth.
    StoryUnrefined { story: u32 },
    /// Story `story` is committed to phase `phase`, another phase.
    StoryInPhase { story: u32, phase: u32 },
    /// Planner order `work_order` of the phase is pending.
    PlannerOrderPending { work_order: String },
}

impl CommitmentRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PhaseActive { .. } => PHASE_ACTIVE,
            Self::NoSuchPhase { .. } => crate::capture::NO_SUCH_PHASE,
            Self::NoSuchStory { .. } => crate::scope::NO_SUCH_STORY,
            Self::StoryUnrefined { .. } => STORY_UNREFINED,
            Self::StoryInPhase { .. } => STORY_IN_PHASE,
            Self::PlannerOrderPending { .. } => PLANNER_ORDER_PENDING,
        }
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Value {
        match self {
            Self::PhaseActive { phase } | Self::NoSuchPhase { phase } => json!({"phase": phase}),
            Self::NoSuchStory { story } | Self::StoryUnrefined { story } => {
                json!({"story": story})
            }
            Self::StoryInPhase { story, phase } => json!({"story": story, "phase": phase}),
            Self::PlannerOrderPending { work_order } => json!({"work_order": work_order}),
        }
    }
}

/// One selected story as the roadmap and the `truths` view hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedStory {
    /// Its id.
    pub id: u32,
    /// Whether the roadmap holds it and it is not dropped.
    pub held: bool,
    /// The phase it is committed to, `None` when none.
    pub phase: Option<u32>,
    /// Whether it holds an approved truth.
    pub refined: bool,
}

/// What a selection's readiness is judged on.
#[derive(Debug, Clone, Copy)]
pub struct Readiness<'a> {
    /// The phase being planned.
    pub phase: u32,
    /// Whether the roadmap holds the phase and it is not withdrawn.
    pub phase_held: bool,
    /// The active phases. The phase being planned may be among them.
    pub active: &'a [u32],
    /// The selection, in order.
    pub stories: &'a [SelectedStory],
}

/// A phase's latest planning continuation.
#[derive(Debug, Clone, Copy)]
pub struct Continuation<'a> {
    /// The selection it binds, in order.
    pub stories: &'a [u32],
    /// The planner order its approval issues.
    pub work_order: &'a str,
    /// Whether that order is pending.
    pub pending: bool,
    /// Whether that order was already handed to a session.
    pub followed: bool,
}

/// What one `phase-plan` call is judged on.
#[derive(Debug, Clone, Copy)]
pub struct CommitmentFacts<'a> {
    /// The selection and what its readiness needs.
    pub readiness: Readiness<'a>,
    /// The phase's committed stories, in order.
    pub committed: &'a [u32],
    /// The phase's pending planner order, if any.
    pub pending_order: Option<&'a str>,
    /// The phase's latest continuation, if any.
    pub continuation: Option<Continuation<'a>>,
    /// Whether a revision is due: the phase's current unapproved draft's
    /// question set closed with an answer that rejected a recommendation.
    pub revision_due: bool,
}

/// What a ready `phase-plan` call does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhasePlanPath {
    /// Hand over the approved continuation's order `work_order` (D-01).
    HandOver { work_order: String },
    /// Draft the selection as a `phase edit` of the phase's stories (D-09).
    Draft,
    /// Issue the due planner revision order (D-15).
    Revision,
    /// Issue a planning order for the phase's stories.
    Planning,
}

/// Judges a selection's readiness, the first fault answering in design
/// 0005's order: `phase-active` naming the lowest active phase other than
/// this one, `no-such-phase`, `no-such-story`, `story-unrefined`, then
/// `story-in-phase`, each naming the first selected story at fault. The
/// commitment approval rechecks a continuation with this same rule (D-01).
pub fn judge_readiness(readiness: &Readiness) -> Result<(), CommitmentRefusal> {
    let phase = readiness.phase;
    if let Some(other) = readiness
        .active
        .iter()
        .copied()
        .filter(|p| *p != phase)
        .min()
    {
        return Err(CommitmentRefusal::PhaseActive { phase: other });
    }
    if !readiness.phase_held {
        return Err(CommitmentRefusal::NoSuchPhase { phase });
    }
    let stories = readiness.stories;
    if let Some(story) = stories.iter().find(|story| !story.held) {
        return Err(CommitmentRefusal::NoSuchStory { story: story.id });
    }
    if let Some(story) = stories.iter().find(|story| !story.refined) {
        return Err(CommitmentRefusal::StoryUnrefined { story: story.id });
    }
    for story in stories {
        if let Some(other) = story.phase.filter(|other| *other != phase) {
            return Err(CommitmentRefusal::StoryInPhase {
                story: story.id,
                phase: other,
            });
        }
    }
    Ok(())
}

/// Judges one `phase-plan` call: readiness first, then the path. When the
/// selection equals both the phase's stories and the latest continuation's,
/// and that continuation's order is pending and not yet followed, the order
/// is handed over. Otherwise a pending planner order is refused
/// `planner-order-pending`; a selection that differs from the phase's
/// stories, in content or in order, is drafted; a due revision is issued;
/// and otherwise a planning order is.
pub fn judge_commitment(facts: &CommitmentFacts) -> Result<PhasePlanPath, CommitmentRefusal> {
    judge_readiness(&facts.readiness)?;
    let selection: Vec<u32> = facts.readiness.stories.iter().map(|s| s.id).collect();
    let unchanged = selection == facts.committed;
    if let Some(continuation) = facts.continuation
        && unchanged
        && continuation.stories == selection
        && continuation.pending
        && !continuation.followed
    {
        return Ok(PhasePlanPath::HandOver {
            work_order: continuation.work_order.to_owned(),
        });
    }
    if let Some(order) = facts.pending_order {
        return Err(CommitmentRefusal::PlannerOrderPending {
            work_order: order.to_owned(),
        });
    }
    if !unchanged {
        return Ok(PhasePlanPath::Draft);
    }
    if facts.revision_due {
        return Ok(PhasePlanPath::Revision);
    }
    Ok(PhasePlanPath::Planning)
}

/// A plan submitted while another phase is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseNotActive {
    /// The active phase.
    pub phase: u32,
}

impl PhaseNotActive {
    /// Always `phase-not-active`.
    pub fn code(&self) -> &'static str {
        PHASE_NOT_ACTIVE
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Value {
        json!({"phase": self.phase})
    }
}

/// Judges a plan's phase at submit: refused naming the lowest active phase
/// other than `phase`, and let through when none is active or only `phase`
/// is, so a phase's first plan can be submitted (D-02).
pub fn judge_submit_phase(phase: u32, active: &[u32]) -> Result<(), PhaseNotActive> {
    match active.iter().copied().filter(|p| *p != phase).min() {
        Some(other) => Err(PhaseNotActive { phase: other }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005 section 5, D-01,
    //! D-02, D-10, D-11, D-15 and the plan's test data, never from running
    //! this code.

    use super::*;

    const O: &str = "6a1f0c3e-2b4d-4e5f-8a7b-9c0d1e2f3a4b";
    const O2: &str = "7b2e1d4f-3c5e-4f60-9b8c-0d1e2f3a4b5c";

    /// A story the roadmap holds, refined and in no phase.
    fn ready(id: u32) -> SelectedStory {
        SelectedStory {
            id,
            held: true,
            phase: None,
            refined: true,
        }
    }

    /// Phase 1, held, with nothing active, nothing pending, no continuation
    /// and no revision due.
    fn facts<'a>(
        committed: &'a [u32],
        stories: &'a [SelectedStory],
        active: &'a [u32],
    ) -> CommitmentFacts<'a> {
        CommitmentFacts {
            readiness: Readiness {
                phase: 1,
                phase_held: true,
                active,
                stories,
            },
            committed,
            pending_order: None,
            continuation: None,
            revision_due: false,
        }
    }

    #[test]
    fn a_selection_judged_in_another_order_is_caught() {
        let mut stories = [
            SelectedStory {
                held: false,
                ..ready(4)
            },
            SelectedStory {
                held: false,
                ..ready(5)
            },
            SelectedStory {
                refined: false,
                ..ready(3)
            },
            SelectedStory {
                phase: Some(2),
                ..ready(2)
            },
        ];
        let mut active = vec![2];
        let mut held = false;
        let judged = |stories: &[SelectedStory], active: &[u32], held: bool| {
            let mut facts = facts(&[1], stories, active);
            facts.readiness.phase_held = held;
            judge_commitment(&facts)
        };
        assert_eq!(
            judged(&stories, &active, held),
            Err(CommitmentRefusal::PhaseActive { phase: 2 })
        );
        active.clear();
        let refusal = judged(&stories, &active, held).unwrap_err();
        assert_eq!(refusal, CommitmentRefusal::NoSuchPhase { phase: 1 });
        assert_eq!(refusal.code(), "no-such-phase");
        held = true;
        let refusal = judged(&stories, &active, held).unwrap_err();
        assert_eq!(refusal, CommitmentRefusal::NoSuchStory { story: 4 });
        assert_eq!(refusal.code(), "no-such-story");
        stories[0].held = true;
        assert_eq!(
            judged(&stories, &active, held),
            Err(CommitmentRefusal::NoSuchStory { story: 5 })
        );
        stories[1].held = true;
        assert_eq!(
            judged(&stories, &active, held),
            Err(CommitmentRefusal::StoryUnrefined { story: 3 })
        );
        stories[2].refined = true;
        let refusal = judged(&stories, &active, held).unwrap_err();
        assert_eq!(
            refusal,
            CommitmentRefusal::StoryInPhase { story: 2, phase: 2 }
        );
        assert_eq!(refusal.details(), json!({"story": 2, "phase": 2}));
        stories[3].phase = None;
        assert_eq!(judged(&stories, &active, held), Ok(PhasePlanPath::Draft));
        // The missing story comes later, so judging story by story would
        // answer story-unrefined first.
        let mixed = [
            SelectedStory {
                refined: false,
                ..ready(3)
            },
            SelectedStory {
                held: false,
                ..ready(4)
            },
        ];
        assert_eq!(
            judged(&mixed, &active, held),
            Err(CommitmentRefusal::NoSuchStory { story: 4 })
        );
    }

    #[test]
    fn a_reordered_selection_issued_without_a_draft_is_caught() {
        for (selection, expected) in [
            (vec![2, 1], PhasePlanPath::Draft),
            (vec![1], PhasePlanPath::Draft),
            (vec![1, 2], PhasePlanPath::Planning),
        ] {
            let stories: Vec<SelectedStory> = selection.iter().copied().map(ready).collect();
            assert_eq!(
                judge_commitment(&facts(&[1, 2], &stories, &[])),
                Ok(expected),
                "{selection:?}"
            );
        }
    }

    #[test]
    fn a_second_planner_order_or_a_continuation_order_handed_over_twice_is_caught() {
        let judged = |selection: &[u32], continuation: Continuation| {
            let stories: Vec<SelectedStory> = selection.iter().copied().map(ready).collect();
            let mut facts = facts(&[1, 4], &stories, &[]);
            facts.pending_order = Some(O);
            facts.continuation = Some(continuation);
            judge_commitment(&facts)
        };
        let latest = Continuation {
            stories: &[1, 4],
            work_order: O,
            pending: true,
            followed: false,
        };
        let pending = Err(CommitmentRefusal::PlannerOrderPending {
            work_order: O.into(),
        });
        assert_eq!(
            judged(&[1, 4], latest),
            Ok(PhasePlanPath::HandOver {
                work_order: O.into()
            })
        );
        let followed = Continuation {
            followed: true,
            ..latest
        };
        assert_eq!(judged(&[1, 4], followed), pending);
        assert_eq!(judged(&[4, 1], latest), pending);
        let other = Continuation {
            stories: &[1],
            work_order: O2,
            pending: false,
            followed: false,
        };
        assert_eq!(judged(&[1, 4], other), pending);
    }

    #[test]
    fn a_due_revision_skipped_or_issued_beside_a_pending_order_is_caught() {
        let stories = [ready(1)];
        let judged = |due: bool, order: Option<&str>| {
            let mut facts = facts(&[1], &stories, &[]);
            facts.revision_due = due;
            facts.pending_order = order;
            judge_commitment(&facts)
        };
        assert_eq!(judged(true, None), Ok(PhasePlanPath::Revision));
        let refusal = judged(true, Some(O)).unwrap_err();
        assert_eq!(
            refusal,
            CommitmentRefusal::PlannerOrderPending {
                work_order: O.into()
            }
        );
        assert_eq!(refusal.code(), "planner-order-pending");
        assert_eq!(judged(false, None), Ok(PhasePlanPath::Planning));
    }

    #[test]
    fn a_submit_refused_for_its_own_active_phase_or_let_through_beside_another_is_caught() {
        assert_eq!(judge_submit_phase(1, &[]), Ok(()));
        assert_eq!(judge_submit_phase(1, &[1]), Ok(()));
        let refusal = judge_submit_phase(1, &[2]).unwrap_err();
        assert_eq!(refusal, PhaseNotActive { phase: 2 });
        assert_eq!(refusal.code(), "phase-not-active");
    }
}
