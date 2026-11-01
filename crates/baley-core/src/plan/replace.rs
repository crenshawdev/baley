//! The judgement of a plan replacement and the phase facts the new plan is
//! judged under (design 0005 section 5 `plan replace`, PLN-R14, D-04).
//!
//! A replacement names the plan in force it replaces by number and the
//! digest that plan was approved at. Once accepted, the new plan is judged
//! as if the old one were gone: its tasks off the phase's size and its
//! evidence items off the approved items.

use baley_store::Hash;
use serde_json::{Value, json};

use super::judge::PlanFacts;
use super::submission::Replaces;

/// The refusal code for a replacement naming no plan in force by that
/// number, or naming one at a digest it was not approved at (PRJ-R18).
pub const NO_SUCH_PLAN: &str = "no-such-plan";
/// The refusal code for replacing a plan whose execution is admitted
/// (PLN-R14).
pub const ADMITTED_PLAN: &str = "admitted-plan";

/// Why a replacement is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacementRefusal {
    /// No plan in force has number `plan` at the digest named.
    NoSuchPlan { plan: u32 },
    /// Plan `plan`'s execution is admitted.
    AdmittedPlan { plan: u32 },
}

impl ReplacementRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSuchPlan { .. } => NO_SUCH_PLAN,
            Self::AdmittedPlan { .. } => ADMITTED_PLAN,
        }
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Value {
        match self {
            Self::NoSuchPlan { plan } | Self::AdmittedPlan { plan } => json!({"plan": plan}),
        }
    }
}

/// One approved plan of the phase that no replacement has replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanInForce {
    /// Its number.
    pub plan: u32,
    /// The digest it was approved at.
    pub digest: Hash,
    /// Its size: the number of its tasks.
    pub size: u32,
}

/// What a replacement is judged on.
#[derive(Debug, Clone, Copy)]
pub struct ReplacementFacts<'a> {
    /// The phase's plans in force.
    pub in_force: &'a [PlanInForce],
    /// Whether the named plan's execution is admitted. The binary reads it
    /// as false until Build 5 records admission.
    pub admitted: bool,
    /// The phase's facts as they stand, the replaced plan still counted.
    pub phase: &'a PlanFacts,
}

/// Judges a replacement of `replaces`: `no-such-plan` when no plan in force
/// has its number at its digest, then `admitted-plan`. Accepted, it gives
/// the phase's facts with the replaced plan's evidence items taken out of
/// `approved` and its size taken off `size`, never below 0, so the new plan
/// is judged as the phase would stand once the old one is gone.
pub fn judge_replacement(
    replaces: Replaces,
    facts: &ReplacementFacts,
) -> Result<PlanFacts, ReplacementRefusal> {
    let plan = replaces.plan;
    let Some(old) = facts
        .in_force
        .iter()
        .find(|old| old.plan == plan && old.digest == replaces.digest)
    else {
        return Err(ReplacementRefusal::NoSuchPlan { plan });
    };
    if facts.admitted {
        return Err(ReplacementRefusal::AdmittedPlan { plan });
    }
    let mut phase = facts.phase.clone();
    phase.approved.retain(|item| item.plan != plan);
    phase.size = phase.size.saturating_sub(old.size);
    Ok(phase)
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, D-04 and the
    //! plan's test data, never from running this code.

    use std::num::NonZeroU32;

    use super::*;
    use crate::plan::PlanRefusal;
    use crate::plan::fixtures::{accepted, approved, c1, c4, f1, p1, refusal};

    /// X: `5c` repeated 32 times.
    fn x() -> Hash {
        Hash([0x5c; 32])
    }

    /// Z: `7e` repeated 32 times.
    fn z() -> Hash {
        Hash([0x7e; 32])
    }

    fn in_force() -> Vec<PlanInForce> {
        vec![
            PlanInForce {
                plan: 1,
                digest: x(),
                size: 2,
            },
            PlanInForce {
                plan: 3,
                digest: z(),
                size: 1,
            },
        ]
    }

    #[test]
    fn a_replacement_of_a_plan_not_in_force_or_admitted_accepted_is_caught() {
        let plans = in_force();
        let phase = f1();
        let judged = |plan: u32, digest: Hash, admitted: bool| {
            judge_replacement(
                Replaces { plan, digest },
                &ReplacementFacts {
                    in_force: &plans,
                    admitted,
                    phase: &phase,
                },
            )
            .map_err(|refusal| (refusal.code(), refusal.details()))
        };

        assert_eq!(
            judged(2, x(), false),
            Err(("no-such-plan", json!({"plan": 2})))
        );
        assert_eq!(
            judged(1, z(), false),
            Err(("no-such-plan", json!({"plan": 1})))
        );
        assert_eq!(
            judged(1, x(), true),
            Err(("admitted-plan", json!({"plan": 1})))
        );
        assert!(judged(1, x(), false).is_ok());
    }

    #[test]
    fn a_replaced_plan_still_counted_in_the_phase_is_caught() {
        let mut phase = f1();
        phase.approved = vec![approved(1, c1()), approved(3, c4())];
        phase.size = 3;
        phase.capacity = NonZeroU32::new(2);
        let plans = in_force();

        let replaced = judge_replacement(
            Replaces {
                plan: 1,
                digest: x(),
            },
            &ReplacementFacts {
                in_force: &plans,
                admitted: false,
                phase: &phase,
            },
        )
        .expect("accepted");
        assert_eq!(replaced.approved, vec![approved(3, c4())]);
        assert_eq!(replaced.size, 1);
        accepted(&p1(), &replaced);
        assert_eq!(
            refusal(&p1(), &phase),
            PlanRefusal::OverCapacity {
                stories: vec![1],
                size: 4,
            }
        );

        let larger = [PlanInForce {
            plan: 1,
            digest: x(),
            size: 5,
        }];
        let shrunk = judge_replacement(
            Replaces {
                plan: 1,
                digest: x(),
            },
            &ReplacementFacts {
                in_force: &larger,
                admitted: false,
                phase: &phase,
            },
        )
        .expect("accepted");
        assert_eq!(shrunk.size, 0);
    }
}
