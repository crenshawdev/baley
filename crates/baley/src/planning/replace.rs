//! `plan replace` (design 0005 section 5, PLN-R14, D-04, D-07): a session
//! submits a planner's plan to replace an approved plan in force, named by
//! its number and the digest it was approved at.
//!
//! A replacement is checked as a submit is, then judged by the replacement
//! judge over the phase's plans in force, and the new plan is judged as if
//! the replaced one were gone. It is recorded as a submit records, on the
//! phase's one draft line, its body naming the plan it replaces. The
//! owner's approval judges the replacement again and records `plan.approved`
//! with a new number and `plan.replaced` together.

use std::num::NonZeroU32;

use baley_core::plan::{
    IN_FORCE, PlanFacts, PlanInForce, ReplacementFacts, ReplacementRefusal, Replaces,
    judge_replacement,
};
use baley_store::{Hash, ProjectId, Views};
use serde_json::Value;

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::facts::PhaseFacts;
use crate::planning::submit::{self, Checked};

/// The check of the plan `submitted` replacing `replaces`, answering work
/// order `work_order` under `capacity`, run before admission and recording
/// nothing (D-04): the submit's check in its order, the order and
/// `phase-not-active` first; then the replacement, `no-such-plan` or
/// `admitted-plan` at slot `replaces`; then the plan judged by `judge_plan`
/// with the replaced plan's tasks off the phase's size and its items off the
/// approved items. `submit::record` records what it accepts, its draft body
/// naming the plan replaced.
pub fn check<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    submitted: &Value,
    work_order: &str,
    replaces: Replaces,
    capacity: Option<NonZeroU32>,
) -> Result<Checked, Value> {
    submit::checked(
        store,
        project,
        submitted,
        work_order,
        Some(replaces),
        capacity,
    )
}

/// The facts the new plan of phase `phase` replacing `replaces` is judged
/// under: the replacement judged over the plans in force that `read`'s
/// `phase_plan` document lists and the admitted observation, a refusal at
/// slot `replaces` with its reason ending `not_done`. The submit's check and
/// the approval's first step both take them here, so an approval judges the
/// replacement again on the phase as it stands then.
pub(crate) fn facts_without<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    replaces: Replaces,
    read: &PhaseFacts,
    not_done: &str,
) -> Result<PlanFacts, Value> {
    let in_force = in_force(read.phase_plan.as_ref())?;
    judge_replacement(
        replaces,
        &ReplacementFacts {
            in_force: &in_force,
            admitted: admitted(store, project, phase, replaces.plan),
            phase: &read.facts,
        },
    )
    .map_err(|refusal| replacement_refused(&refusal, not_done))
}

/// Whether plan `plan` of phase `phase`'s execution is admitted. Nothing
/// records admission until Build 5, so this reads false; Build 5's execution
/// admission makes it a read of the ledger here, the one seam the
/// `admitted-plan` refusal waits on (decision 15 of the Build 4 plan).
fn admitted<S: Views + ?Sized>(_store: &S, _project: &ProjectId, _phase: u32, _plan: u32) -> bool {
    false
}

/// The plans in force a `phase_plan` document lists, none before anything
/// was projected for the phase. A document that cannot be read is a ledger
/// fault.
fn in_force(phase_plan: Option<&Value>) -> Result<Vec<PlanInForce>, Value> {
    let Some(body) = phase_plan else {
        return Ok(Vec::new());
    };
    let plans = body
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?;
    let mut held = Vec::new();
    for plan in plans {
        if plan.get("state").and_then(Value::as_str) != Some(IN_FORCE) {
            continue;
        }
        let whole = |name: &str| {
            plan.get(name)
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
        };
        let digest = plan
            .get("digest")
            .and_then(Value::as_str)
            .and_then(Hash::from_hex);
        let (Some(number), Some(digest), Some(size)) = (whole("plan"), digest, whole("size"))
        else {
            return Err(unreadable());
        };
        held.push(PlanInForce {
            plan: number,
            digest,
            size,
        });
    }
    Ok(held)
}

/// The refusal for a replacement, at slot `replaces`, naming the plan.
fn replacement_refused(refusal: &ReplacementRefusal, not_done: &str) -> Value {
    let reason = match refusal {
        ReplacementRefusal::NoSuchPlan { plan } => format!(
            "no plan {plan} of the phase is in force at that digest: it was replaced, never approved, or approved at another digest, so {not_done}"
        ),
        ReplacementRefusal::AdmittedPlan { plan } => format!(
            "plan {plan} is admitted to execution and can no longer be replaced, so {not_done}"
        ),
    };
    Refusal::new(refusal.code(), reason)
        .slot("replaces")
        .details(refusal.details())
        .value()
}

fn unreadable() -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        "the phase_plan view document cannot be read, so nothing was recorded",
        PLACE_LEDGER,
    )
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected answers are written out from design 0005
    //! sections 5 and 6, D-04, D-07 and the plan's data store C, P0, PL1,
    //! PL1r, PL1rr, PL1d and PL3, never from running this code.

    use baley_core::plan::{EVIDENCE_MAP_VIEW, PHASE_PLAN_VIEW, PLAN_VIEW, phase_plan_key};
    use baley_core::plan::{evidence_map_key, plan_key};
    use baley_store::{Hash, Views};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::planning::approve::tests::{approved, digest_of};
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit::tests::{order_o, plan_submitted};
    use crate::scope::fixtures::*;

    /// The request ids of the second test, apart from store C's and R1 to
    /// R9.
    const R10: &str = "6a7b8c9d-0e1f-4a2b-8c3d-4e5f6a7b8c9d";
    const R11: &str = "7b8c9d0e-1f2a-4b3c-9d4e-5f6a7b8c9d0e";

    /// PL1r: PL1 with goal `Record every step, revised`.
    fn pl1r() -> Value {
        let mut plan = pl1();
        plan["goal"] = json!("Record every step, revised");
        plan
    }

    /// PL1rr: PL1 with goal `Record every step, third`.
    fn pl1rr() -> Value {
        let mut plan = pl1();
        plan["goal"] = json!("Record every step, third");
        plan
    }

    /// PL1d: PL1r with a second task `k3`, as K with id `k3`.
    fn pl1d() -> Value {
        let mut plan = pl1r();
        let mut k3 = k();
        k3["id"] = json!("k3");
        plan["tasks"] = json!([k(), k3]);
        plan
    }

    /// Submits `plan` replacing plan `number` at `digest`, answering order
    /// `order` under `capacity`, through the check and the submit's
    /// recording, as `plan-replace` runs them below the session.
    fn plan_replaced(
        store: &SqliteStore,
        plan: &Value,
        number: u32,
        digest: Hash,
        capacity: Option<u32>,
        request_id: &str,
        at: &str,
    ) -> Value {
        let capacity = capacity.map(|capacity| NonZeroU32::new(capacity).unwrap());
        let replaces = Replaces {
            plan: number,
            digest,
        };
        let checked = match check(store, &project(), plan, O, replaces, capacity) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.replace", request_id, 8, at);
        let on = caller()
            .with_work_order(O)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(baley_store::Caller::Server(on));
        crate::scope::submit::answer(submit::record(store, &command, Some(&checked)), store)
    }

    /// The approval of `digest` under P0, which must be `ok`. Gives the
    /// answer.
    fn approved_ok(store: &SqliteStore, digest: Hash, request_id: &str, at: &str) -> Value {
        let answer = approved(store, digest, &p0(), request_id, at);
        assert_eq!(answer["status"], "ok", "{answer}");
        answer
    }

    #[test]
    fn a_plan_number_reused_or_a_replacement_keeping_the_old_number_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        assert_eq!(approved_ok(&store, x1, R3, T5)["plan"], 1);
        let x3 = digest_of(&plan_submitted(&store, &pl3(), O, None, R4, T6));
        assert_eq!(approved_ok(&store, x3, R5, T6)["plan"], 2);

        let x1r = digest_of(&plan_replaced(&store, &pl1r(), 1, x1, None, R6, T7));
        let third = approved_ok(&store, x1r, R7, T7);
        assert_eq!(third["plan"], 3, "{third}");
        let events = history(&store);
        let recorded: Vec<_> = events
            .iter()
            .filter(|event| event.request_id.0 == R7)
            .filter(|event| event.type_name.starts_with("plan."))
            .collect();
        assert_eq!(recorded.len(), 2, "{recorded:?}");
        assert_eq!(recorded[0].type_name, "plan.approved");
        assert_eq!(recorded[0].payload["plan"], 3);
        assert_eq!(recorded[1].type_name, "plan.replaced");
        assert_eq!(recorded[1].stream, "phase/1");
        assert_eq!(
            recorded[1].payload,
            json!({"phase": 1, "plan": 1, "old_digest": x1.to_hex(),
                "new_digest": x1r.to_hex(), "owner": "Ada Owner", "at": T7})
        );

        let x1rr = digest_of(&plan_replaced(&store, &pl1rr(), 3, x1r, None, R8, T8));
        assert_eq!(approved_ok(&store, x1rr, R9, T8)["plan"], 4);

        let numbers: Vec<Value> = history(&store)
            .iter()
            .filter(|event| event.type_name == "plan.approved")
            .map(|event| event.payload["plan"].clone())
            .collect();
        assert_eq!(numbers, [json!(1), json!(2), json!(3), json!(4)]);

        let phase_plan = store
            .get(&project(), PHASE_PLAN_VIEW, &phase_plan_key(1))
            .unwrap()
            .unwrap()
            .body;
        assert_eq!(phase_plan["highest"], 4);
        assert_eq!(phase_plan["size"], 2);
        let in_force: Vec<Value> = phase_plan["plans"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|held| held["state"] == "in-force")
            .map(|held| held["plan"].clone())
            .collect();
        assert_eq!(in_force, [json!(2), json!(4)]);

        for (number, digest) in [(1, x1), (3, x1r)] {
            let plan = store
                .get(&project(), PLAN_VIEW, &plan_key(1, number))
                .unwrap()
                .unwrap()
                .body;
            assert_eq!(plan["state"], "replaced", "{plan}");
            assert_eq!(plan["digest"], digest.to_hex(), "{plan}");
            let map = store
                .get(&project(), EVIDENCE_MAP_VIEW, &evidence_map_key(1, number))
                .unwrap()
                .unwrap()
                .body;
            assert_eq!(map["state"], "replaced", "{map}");
            assert_eq!(map["items"], json!([e1(), e2()]), "{map}");
        }
    }

    #[test]
    fn a_replaced_plan_replaced_again_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        approved_ok(&store, x1, R3, T5);
        let x1r = digest_of(&plan_replaced(&store, &pl1r(), 1, x1, None, R4, T6));
        assert_eq!(approved_ok(&store, x1r, R5, T6)["plan"], 2);

        for (number, request_id) in [(1, R6), (2, R7)] {
            let before = head(&store);
            let refusal = plan_replaced(&store, &pl1rr(), number, x1, None, request_id, T7);
            assert_eq!(refusal["status"], "refused", "{refusal}");
            assert_eq!(refusal["code"], "no-such-plan", "{refusal}");
            assert_eq!(refusal["slot"], "replaces", "{refusal}");
            assert_eq!(refusal["details"], json!({"plan": number}), "{refusal}");
            assert_eq!(head(&store), before);
        }
    }

    #[test]
    fn a_replacement_judged_with_the_replaced_plan_counted_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = digest_of(&plan_submitted(&store, &pl1(), O, None, R2, T5));
        approved_ok(&store, x1, R3, T5);
        let x3 = digest_of(&plan_submitted(&store, &pl3(), O, None, R4, T6));
        approved_ok(&store, x3, R5, T6);

        let accepted = plan_replaced(&store, &pl1r(), 1, x1, Some(2), R10, T7);
        assert_eq!(accepted["status"], "ok", "{accepted}");
        let before = head(&store);
        let over = plan_replaced(&store, &pl1d(), 1, x1, Some(2), R11, T8);
        assert_eq!(over["status"], "refused", "{over}");
        assert_eq!(over["code"], "over-capacity", "{over}");
        assert_eq!(over["details"], json!({"stories": [1], "size": 3}));
        assert_eq!(head(&store), before);
    }
}
