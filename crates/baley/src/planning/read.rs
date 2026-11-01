//! A phase's approved plans read back with their evidence maps (design 0005
//! section 6 Views, design 0001 Appendix B `plan-read`, PLN-R4, PLN-R14,
//! PLN-R15, D-07, D-17). `plan-read` over MCP and `baley plan show` both
//! read through here, so the session and the owner see the same thing.
//!
//! Only the views an approval writes are read, `phase_plan`, `plan` and
//! `evidence_map`, so a draft's map never appears. A replaced plan is read
//! with the rest, its map unchanged. Readiness is worked out here from the
//! `truths` view each time and never stored, so a truth revised after an
//! approval shows at once. Nothing is appended.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use baley_core::plan::{
    EVIDENCE_MAP_VIEW, PHASE_PLAN_VIEW, PLAN_VIEW, approved_items, evidence_map_key,
    phase_plan_key, plan_key,
};
use baley_core::truth::{TRUTHS_VIEW, truths_key};
use baley_store::{DocKey, ProjectId, Views};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scope::read::{ReadError, roadmap};

/// A phase's approved plans, in plan-number order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhasePlans {
    /// The phase's number.
    pub phase: u32,
    /// Every plan ever approved in the phase, replaced ones included.
    pub plans: Vec<ApprovedPlan>,
}

/// One approved plan joined with its evidence map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovedPlan {
    /// Its number in the phase, never reused.
    pub plan: u32,
    /// The digest the owner approved.
    pub digest: String,
    /// The stories it serves.
    pub stories: Vec<u32>,
    /// Its size in tasks.
    pub size: u64,
    /// The suite command it was approved with, or none.
    pub suite: Option<String>,
    /// The id of its draft's question set.
    pub set: String,
    /// The owner who approved it.
    pub owner: String,
    /// When it was approved.
    pub at: String,
    /// `approved` or `replaced`.
    pub state: String,
    /// The digest of the plan that replaced it, or none.
    pub replaced_by: Option<String>,
    /// Whether its items still name their truths' current versions.
    pub readiness: Readiness,
    /// Its evidence items, whole, as approved.
    pub items: Vec<Value>,
}

/// A plan's readiness, worked out when read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Readiness {
    /// True when every item names its truth's current version.
    pub ready: bool,
    /// The ids of the items bound to a version that is not current.
    pub not_ready: Vec<String>,
}

/// Reads phase `phase` of `project`'s approved plans, recording nothing: a
/// phase the roadmap does not hold is `ReadError::NoSuchPhase`; its plans
/// are the ones its `phase_plan` document lists, in plan-number order,
/// replaced ones included, each joined from its `plan` and `evidence_map`
/// documents; each plan's readiness is worked out from the current truths
/// of the stories its items name. A plan listed with a document that is
/// missing or cannot be read is a ledger fault, never skipped.
pub fn read<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
) -> Result<PhasePlans, ReadError> {
    if !roadmap(store, project)?
        .phases
        .iter()
        .any(|held| held.number == phase)
    {
        return Err(ReadError::NoSuchPhase);
    }
    let listed = match document::<PhasePlanDocument, _>(
        store,
        project,
        PHASE_PLAN_VIEW,
        &phase_plan_key(phase),
    )? {
        Some(document) => document.plans,
        None => Vec::new(),
    };
    let mut numbers: Vec<u32> = listed.into_iter().map(|listed| listed.plan).collect();
    numbers.sort_unstable();
    let mut current = BTreeMap::new();
    let mut plans = Vec::with_capacity(numbers.len());
    for number in numbers {
        let held =
            document::<PlanDocument, _>(store, project, PLAN_VIEW, &plan_key(phase, number))?
                .ok_or_else(|| unreadable(PLAN_VIEW))?;
        let map = store
            .get(project, EVIDENCE_MAP_VIEW, &evidence_map_key(phase, number))
            .map_err(ReadError::Store)?
            .ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?
            .body;
        let bound = approved_items(&map).ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?;
        let mut not_ready = Vec::new();
        for item in &bound {
            let truths = match current.entry(item.story) {
                Entry::Occupied(held) => held.into_mut(),
                Entry::Vacant(empty) => empty.insert(versions(store, project, item.story)?),
            };
            if truths.get(&item.truth) != Some(&item.truth_version) {
                not_ready.push(item.id.clone());
            }
        }
        plans.push(ApprovedPlan {
            plan: number,
            digest: held.digest,
            stories: held.stories,
            size: held.size,
            suite: held.suite,
            set: held.question_set,
            owner: held.owner,
            at: held.at,
            state: held.state,
            replaced_by: held.replaced_by,
            readiness: Readiness {
                ready: not_ready.is_empty(),
                not_ready,
            },
            items: bound.into_iter().map(|item| item.definition).collect(),
        });
    }
    Ok(PhasePlans { phase, plans })
}

/// The `phase_plan` document as its projector writes it; only its plan
/// list is read.
#[derive(Deserialize)]
struct PhasePlanDocument {
    plans: Vec<ListedPlan>,
}

/// One plan as `phase_plan` lists it.
#[derive(Deserialize)]
struct ListedPlan {
    plan: u32,
}

/// The `plan` document as its projector writes it; the members a read does
/// not need are left out.
#[derive(Deserialize)]
struct PlanDocument {
    digest: String,
    stories: Vec<u32>,
    size: u64,
    suite: Option<String>,
    question_set: String,
    owner: String,
    at: String,
    state: String,
    replaced_by: Option<String>,
}

/// The `truths` document as its projector writes it; only the current
/// truths' ids and versions are read.
#[derive(Deserialize)]
struct TruthsDocument {
    truths: Vec<TruthVersion>,
}

/// One current truth's id and version.
#[derive(Deserialize)]
struct TruthVersion {
    id: String,
    version: u32,
}

/// Story `story`'s current truth versions by id; none before its first
/// refinement.
fn versions<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    story: u32,
) -> Result<BTreeMap<String, u32>, ReadError> {
    Ok(
        document::<TruthsDocument, _>(store, project, TRUTHS_VIEW, &truths_key(story))?
            .map(|document| {
                document
                    .truths
                    .into_iter()
                    .map(|truth| (truth.id, truth.version))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// One view document read as `T`, or none when the view holds no such key.
fn document<T: for<'de> Deserialize<'de>, S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    view: &str,
    key: &DocKey,
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
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0001
    //! Appendix B `plan-read`, design 0005 section 6 Views, D-07, D-17 and
    //! the plan's data store C, O, PL1 and PL2, with PL3 and PL1r given maps
    //! of their own here, never from running this code.

    use baley_store::Hash;
    use serde_json::json;

    use super::*;
    use crate::planning::approve::tests::{approved, digest_of};
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit::tests::{order_o, plan_submitted};
    use crate::refine::submit::tests::{t1, t2};
    use crate::scope::fixtures::*;

    /// The approval of `digest` under P0, which must be `ok`.
    fn approved_ok(store: &baley_store_sqlite::SqliteStore, digest: Hash, request: &str, at: &str) {
        let answer = approved(store, digest, &p0(), request, at);
        assert_eq!(answer["status"], "ok", "{answer}");
    }

    /// E3, a unit check of story 1's `t2` at version 1, where E2 checks it
    /// at the integration level.
    fn e3() -> Value {
        json!({"id": "e3", "story": 1, "truth": "t2", "truth_version": 1, "kind": "check",
            "spec": {"command": "cargo test round_unit",
                "expected": {"kind": "literal", "value": "one round"},
                "test": {"file": "src/ledger.rs", "function": "one_round"},
                "setup": "a set answered once", "call": "read the round",
                "level": "unit", "boundary": "the round read", "fakes": []},
            "reason": "a round read wrongly"})
    }

    /// PL3 with E1 alone, so plan 2's map differs from plan 1's. Plan 1's
    /// E2 still checks `t2`, and a truth takes one check id in a phase.
    fn pl3_e1() -> Value {
        let mut plan = pl3();
        plan["evidence_map"] = json!([e1()]);
        plan
    }

    /// PL1r with E1 and E3, so the replacement checks `t2` at another level
    /// than the plan it replaces. E2 leaves the phase with plan 1.
    fn pl1r_e3() -> Value {
        let mut plan = pl1r();
        plan["evidence_map"] = json!([e1(), e3()]);
        plan
    }

    /// The plan read expects for plan `plan` at `digest` with set `set`,
    /// approved at `at`, carrying `items`, ready.
    fn expected(plan: u32, digest: Hash, set: &Value, at: &str, items: Vec<Value>) -> ApprovedPlan {
        ApprovedPlan {
            plan,
            digest: digest.to_hex(),
            stories: vec![1],
            size: 1,
            suite: None,
            set: set.as_str().expect("a set id").to_owned(),
            owner: "Ada Owner".into(),
            at: at.into(),
            state: "approved".into(),
            replaced_by: None,
            readiness: Readiness {
                ready: true,
                not_ready: Vec::new(),
            },
            items,
        }
    }

    #[test]
    fn an_evidence_map_read_unapproved_out_of_order_or_changed_by_a_rebuild_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let first = plan_submitted(&store, &pl1(), O, None, R2, T5);
        let x1 = digest_of(&first);
        approved_ok(&store, x1, R3, T5);
        let second = plan_submitted(&store, &pl3_e1(), O, None, R4, T6);
        let x3 = digest_of(&second);
        approved_ok(&store, x3, R5, T6);
        let x2 = digest_of(&plan_submitted(&store, &pl2(), O, None, R6, T7));

        // Each plan's map is its own: one read under another plan's key fails.
        let one = expected(1, x1, &first["set"], T5, vec![e1(), e2()]);
        let two = expected(2, x3, &second["set"], T6, vec![e1()]);
        let read_1 = read(&store, &project(), 1).expect("phase 1 is read");
        assert_eq!(
            read_1,
            PhasePlans {
                phase: 1,
                plans: vec![one.clone(), two.clone()]
            }
        );
        assert!(
            read_1.plans.iter().all(|plan| plan.digest != x2.to_hex()),
            "a draft's map read as approved"
        );
        // Each item keeps its truth's version and its check's level.
        assert_eq!(read_1.plans[0].items[0]["truth"], "t1");
        assert_eq!(read_1.plans[0].items[0]["truth_version"], 1);
        assert_eq!(read_1.plans[0].items[0]["spec"]["level"], "unit");
        assert_eq!(read_1.plans[0].items[1]["truth"], "t2");
        assert_eq!(read_1.plans[0].items[1]["spec"]["level"], "integration");

        store.rebuild(&project()).expect("the views rebuild");
        assert_eq!(read(&store, &project(), 1).expect("read again"), read_1);

        let replaced = plan_replaced(&store, &pl1r_e3(), O, 1, x1, R7, T7);
        let x1r = digest_of(&replaced);
        approved_ok(&store, x1r, R8, T8);
        let mut one_replaced = one.clone();
        one_replaced.state = "replaced".into();
        one_replaced.replaced_by = Some(x1r.to_hex());
        let three = expected(3, x1r, &replaced["set"], T8, vec![e1(), e3()]);
        let after_replacement = read(&store, &project(), 1).expect("read after the replacement");
        assert_eq!(
            after_replacement,
            PhasePlans {
                phase: 1,
                plans: vec![one_replaced.clone(), two.clone(), three.clone()]
            }
        );
        // The replaced plan keeps E2 at integration, its replacement has E3
        // at unit.
        assert_eq!(after_replacement.plans[0].items[1]["id"], "e2");
        assert_eq!(
            after_replacement.plans[0].items[1]["spec"]["level"],
            "integration"
        );
        assert_eq!(after_replacement.plans[2].items[1]["id"], "e3");
        assert_eq!(after_replacement.plans[2].items[1]["spec"]["level"], "unit");

        let revised = with_version(t1(), 2);
        append(
            &store,
            R9,
            T9,
            vec![refined(
                1,
                "1a1a1a1a-1a1a-4a1a-8a1a-1a1a1a1a1a1a",
                vec![revised, versioned(t2())],
            )],
        );
        let stale = Readiness {
            ready: false,
            not_ready: vec!["e1".into()],
        };
        let after = read(&store, &project(), 1).expect("read after the revision");
        for (plan, before) in after.plans.iter().zip([&one_replaced, &two, &three]) {
            assert_eq!(plan.readiness, stale, "plan {}", plan.plan);
            assert_eq!(plan.items, before.items, "plan {}", plan.plan);
            assert_eq!(plan.items[0]["truth_version"], 1, "plan {}", plan.plan);
        }
        assert_eq!(after.plans.len(), 3);
    }

    /// `truth` at `version`.
    fn with_version(truth: Value, version: u32) -> Value {
        let mut truth = truth;
        truth["version"] = json!(version);
        truth
    }
}
