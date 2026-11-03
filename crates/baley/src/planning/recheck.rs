//! The re-check at submit (design 0005 PLN-R16 and section 5, ADR 0037):
//! after a plan submit's or replacement's check, the submit's place in its
//! check lineage. A lineage is keyed by the planner order a submit answers
//! (D-02): a submit under the chain of a lineage's revision order is that
//! lineage's round 2, and Baley finds the first check from the record. The
//! caller never names it.
//!
//! The step records nothing. It takes the check's result and gives it back,
//! with the round-2 binding attached for a re-check, so the gated recording
//! records round 2 from what the step found.

use baley_core::plan::{CheckRefusal, CheckRound, Lineage, PreviousCheck, judge_check_round};
use baley_core::policy::{EffectivePolicy, PlanGate};
use baley_store::Hash;
use serde_json::Value;

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::submit::Checked;

/// The round-2 binding a submit's check carries once the lineage step
/// finds the submit is a re-check.
#[derive(Debug, Clone, PartialEq)]
pub struct Recheck {
    /// The lineage's planner revision order, the head of the submit's chain.
    revision: String,
    /// The lineage's round-1 result.
    first: PreviousCheck,
}

/// One lineage `phase_plan` holds.
struct Held {
    revision: String,
    first: PreviousCheck,
    rechecked: bool,
}

/// Judges the place of `checked`, a submit's or a replacement's accepted
/// check, in its check lineage under `policy`, recording nothing, and gives
/// `checked` back (D-01, D-02). With the plan gate `off` nothing is judged.
/// Otherwise the phase's lineages, `phase_plan`'s `checks` as the check
/// read them, and the head of the answered order's chain, its `request`,
/// go to `judge_check_round`: a fresh lineage gives `checked` back as it
/// came; round 2 attaches the lineage's revision order and first check; a
/// lineage whose round 2 was issued is refused `third-round` at slot
/// `work_order` naming its revision order. The documents the check read are
/// the ones the recording observes, so a lineage moved since makes the call
/// `ledger-busy`. A lineage or chain that cannot be read is a ledger fault.
pub fn judge(mut checked: Checked, policy: &EffectivePolicy) -> Result<Checked, Value> {
    if policy.plan_gate().0 == PlanGate::Off {
        return Ok(checked);
    }
    let held = lineages(checked.phase_plan.as_ref())?;
    let chain = checked
        .chain
        .as_deref()
        .ok_or_else(|| unreadable("the answered work order's chain cannot be read"))?;
    let judged: Vec<Lineage<'_>> = held
        .iter()
        .map(|lineage| Lineage {
            revision: &lineage.revision,
            rechecked: lineage.rechecked,
        })
        .collect();
    let revision = match judge_check_round(&judged, chain) {
        Ok(CheckRound::Fresh) => return Ok(checked),
        Ok(CheckRound::Recheck { revision }) => revision,
        Err(refusal) => return Err(third_round(&refusal)),
    };
    let first = held
        .iter()
        .find(|lineage| lineage.revision == revision)
        .map(|lineage| lineage.first)
        .ok_or_else(|| unreadable("the phase's check lineage cannot be read"))?;
    checked.recheck = Some(Recheck {
        revision: revision.to_owned(),
        first,
    });
    Ok(checked)
}

/// The lineages a `phase_plan` document holds in its `checks`, none before
/// the phase has a document.
fn lineages(phase_plan: Option<&Value>) -> Result<Vec<Held>, Value> {
    let Some(body) = phase_plan else {
        return Ok(Vec::new());
    };
    let read = |lineage: &Value| {
        let first = lineage.get("first")?;
        Some(Held {
            revision: lineage.get("revision")?.as_str()?.to_owned(),
            first: PreviousCheck {
                seq: first.get("seq")?.as_u64()?,
                plan_digest: Hash::from_hex(first.get("plan_digest")?.as_str()?)?,
            },
            rechecked: !lineage.get("recheck")?.is_null(),
        })
    };
    body.get("checks")
        .and_then(Value::as_array)
        .and_then(|checks| checks.iter().map(read).collect::<Option<Vec<_>>>())
        .ok_or_else(|| unreadable("the phase's check lineages cannot be read"))
}

/// The refusal for a submit under a revision order whose round 2 was
/// already issued.
fn third_round(refusal: &CheckRefusal) -> Value {
    let revision = match refusal {
        CheckRefusal::ThirdRound { work_order } => work_order.as_str(),
        // The lineage judge refuses nothing else.
        _ => "",
    };
    Refusal::new(
        refusal.code(),
        format!(
            "the one re-check the lineage of revision work order {revision} buys was already issued, so this plan was not checked again and nothing was recorded. A fresh `phase-plan` order starts a new lineage, whose submit gets a full first check"
        ),
    )
    .slot("work_order")
    .details(refusal.details())
    .value()
}

fn unreadable(reason: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("{reason}, so nothing was recorded"),
        PLACE_LEDGER,
    )
}

#[cfg(test)]
mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005
    //! sections 5 and 6, ADR 0037, D-02, D-09, D-11 and the plan's data
    //! store C, O, PL2, PL2r, P0, POFF, N0, C1, V, FB and FW, never from
    //! running this code.

    use baley_core::plan::{CheckIssue, PLAN_CHECK_ISSUED, PLAN_CHECK_ISSUED_VERSION};
    use baley_core::policy::RecheckScope;
    use baley_store::{NewEvent, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::planning::check_return::tests::{V, fb, fw, gated, returned};
    use crate::planning::checker::tests::{C1, issued_alone};
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::{O, fail_by_hand};
    use crate::planning::submit::check;
    use crate::scope::fixtures::*;

    /// V9, V's retry.
    const V9: &str = "9e8d7c6b-5a49-4382-9716-a5b4c3d2e1f0";
    /// W2, a checker order of phase 1 issued by hand.
    const W2: &str = "a2b3c4d5-e6f7-4a8b-9c0d-e1f2a3b4c5d6";

    /// Store C with O, PL2 through the gated recording under P0 as C1 at
    /// policy version 5 under R2 at T5, then C1's return of `findings`
    /// recorded under R3 at T6 with V its revision. Gives the store, X2 and
    /// K, the round-1 result's sequence.
    pub(crate) fn lineage_v(findings: Vec<Value>) -> (tempfile::TempDir, SqliteStore, Hash, u64) {
        let (dir, store) = store_c();
        crate::planning::submit::tests::order_o(&store, R1);
        let receipt = gated(&store, &pl2(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        let x2 = Hash::from_hex(receipt["digest"].as_str().unwrap()).unwrap();
        let back = returned(&store, C1, findings, &p0(), V, R3, T6, 5);
        assert_eq!(back["revision"]["work_order"], V, "{back}");
        let events = history(&store);
        let k = of_type(&events, "plan.checked")[0].seq;
        (dir, store, x2, k)
    }

    /// `plan` checked under `order` with no capacity, then its lineage
    /// judged under `policy`.
    pub(crate) fn judged(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
    ) -> Result<Checked, Value> {
        let checked = check(store, &project(), plan, order, None)?;
        judge(checked, policy)
    }

    /// The round-2 binding `plan` under `order` gets under `policy`.
    fn binding(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
    ) -> Option<Recheck> {
        judged(store, plan, order, policy)
            .expect("the plan is checked and judged")
            .recheck
    }

    #[test]
    fn a_revision_submit_taken_for_a_fresh_check_or_a_retried_revision_buying_a_new_lineage_is_caught()
     {
        let (_dir, store, x2, k) = lineage_v(vec![fb(), fw()]);
        let first = PreviousCheck {
            seq: k,
            plan_digest: x2,
        };

        let round_two = binding(&store, &pl2r(), V, &p0()).expect("round 2");
        assert_eq!(round_two.revision, V);
        assert_eq!(round_two.first, first);

        assert_eq!(binding(&store, &pl2r(), V, &poff()), None);
        assert_eq!(binding(&store, &pl2r(), O, &p0()), None);

        crate::mcp::work_order::tests::fail_order(&store, &project(), V, R4, T7);
        let retry = crate::mcp::work_order::retry_check(&store, &project(), V)
            .expect("V failed and nothing of phase 1 is pending");
        let command = command(crate::mcp::work_order::RETRY_COMMAND, R5, 9, T8);
        let receipt = crate::mcp::work_order::issue_retry(
            &store,
            &command,
            &retry,
            &p0(),
            baley_core::policy::Host::ClaudeCode,
            &n0(),
            V9,
        );
        assert_eq!(receipt["work_order"], V9, "{receipt}");
        let again = binding(&store, &pl2r(), V9, &p0()).expect("round 2 again");
        assert_eq!(again.revision, V);
        assert_eq!(again.first, first);
    }

    #[test]
    fn a_resubmit_after_round_two_issued_accepted_is_caught() {
        let (_dir, store, x2, k) = lineage_v(vec![fb(), fw()]);
        let w2 = issued_alone(&store, W2, R4, T7);
        assert_eq!(w2["status"], "ok", "{w2}");
        fail_by_hand(&store, W2, R5);
        let issue = CheckIssue {
            phase: 1,
            work_order: W2,
            previous_check: PreviousCheck {
                seq: k,
                plan_digest: x2,
            },
            original_plan_digest: x2,
            plan_digest: x2,
            truth_versions: &[],
            scope: RecheckScope::Full,
            scope_policy_version: 5,
            material: &[],
        };
        append(
            &store,
            R6,
            T8,
            vec![NewEvent {
                stream: StreamName("phase/1".into()),
                type_name: PLAN_CHECK_ISSUED.into(),
                type_version: PLAN_CHECK_ISSUED_VERSION,
                git: None,
                payload: issue.payload(),
                attachments: Vec::new(),
            }],
        );

        let before = head(&store);
        let refusal = judged(&store, &pl2r(), V, &p0()).expect_err("a third round is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "third-round", "{refusal}");
        assert_eq!(refusal["slot"], "work_order", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": V}));
        assert_eq!(head(&store), before);
    }
}
