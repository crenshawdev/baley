//! The re-check at submit (design 0005 PLN-R16 and section 5, ADR 0037):
//! after a plan submit's or replacement's check, the submit's place in its
//! check lineage. A lineage is keyed by the planner order a submit answers
//! (D-02): a submit under the chain of a lineage's revision order is that
//! lineage's round 2, and Baley finds the first check from the record. The
//! caller never names it.
//!
//! The step records nothing. It takes the check's result and gives it back,
//! with the round-2 binding attached for a re-check, so the gated recording
//! records round 2 from what the step found: in one transaction with the
//! revised draft, `plan.check_issued` binding the first check, both
//! digests, round 1's truth versions, the scope the command's own policy
//! selects with that policy's version, and the material, then the round-2
//! checker order whose inputs are exactly that material (D-09).

use std::io::Read;

use baley_core::plan::{
    CheckIssue, CheckRefusal, CheckRound, Lineage, PLAN_CHECK_ISSUED, PLAN_CHECK_ISSUED_VERSION,
    PLAN_CHECKED, PlanDraft, PreviousCheck, RecordedPlan, TruthVersion, judge_check_round,
    judge_findings, judge_recheck_truths, phase_stream, plan_delta, read_plan_draft,
};
use baley_core::policy::{EffectivePolicy, PlanGate, RecheckScope, Value as Setting};
use baley_core::work_order::{MATERIAL_UNAVAILABLE, OrderInput};
use baley_store::{
    Hash, HistoryFilter, Ledger, NewEvent, PageRequest, PayloadBody, PayloadRef, PayloadStatus,
    Payloads, ProjectId, RetentionClass, StoreError, StreamName, Transaction, canonical_json,
    request_digest,
};
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::checker::{
    BLOCKERS_INPUT, DELTA_INPUT, ORIGINAL_INPUT, PLAN_INPUT, PreparedRecheck, TRUTHS_INPUT,
};
use crate::planning::submit::Checked;
use crate::scope::submit::NOT_RECORDED_BUSY;

/// The setting the re-check scope is read from.
const SCOPE_SETTING: &str = "review.triggers.plan.recheck";

/// The round-2 binding a submit's check carries once the lineage step
/// finds the submit is a re-check.
#[derive(Debug, Clone, PartialEq)]
pub struct Recheck {
    /// The lineage's planner revision order, the head of the submit's chain.
    revision: String,
    /// The lineage's round-1 result, whose draft is the original.
    first: PreviousCheck,
    /// The reference of the original draft's body as round 1 held it.
    original: PayloadRef,
    /// The original draft's body read back.
    original_plan: RecordedPlan,
    /// The reference of round 1's truths snapshot.
    truths: PayloadRef,
    /// The truths round 1 was bound to, which round 2 binds unchanged.
    truth_versions: Vec<TruthVersion>,
    /// Every round-1 blocker as the checker returned it.
    blockers: Vec<Value>,
}

impl Recheck {
    /// Records round 2 of phase `phase`'s check inside `tx` at `at`, after
    /// `draft`, the revision, was recorded in it: the revision's body, the
    /// delta under `diff` and the blockers as `record` payloads, then
    /// `plan.check_issued` on `phase/<n>` binding the first check, both
    /// digests, round 1's truth versions, `scope`, `scope_policy_version`
    /// and the material, then `order` issued with that material as its
    /// inputs. The material is, in order, `original`, `plan`, `truths`,
    /// `delta` only under `diff`, and `blockers`, each body canonical JSON.
    /// Gives the order's receipt members.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_within(
        &self,
        tx: &mut dyn Transaction,
        phase: u32,
        draft: &PlanDraft,
        scope: RecheckScope,
        scope_policy_version: u64,
        order: &PreparedRecheck,
        at: &str,
    ) -> Result<Map<String, Value>, StoreError> {
        let input = |name: &str, body: PayloadRef| OrderInput {
            name: name.into(),
            body,
        };
        let mut material = vec![
            input(ORIGINAL_INPUT, self.original.clone()),
            input(
                PLAN_INPUT,
                tx.put_payload(draft.body(), RetentionClass::Record)?,
            ),
            input(TRUTHS_INPUT, self.truths.clone()),
        ];
        if scope == RecheckScope::Diff {
            let revision = serde_json::from_slice(draft.body())
                .ok()
                .as_ref()
                .and_then(read_plan_draft)
                .ok_or_else(|| unrecordable("the revised draft"))?;
            let delta = canonical(&Value::Array(plan_delta(&self.original_plan, &revision)))?;
            material.push(input(
                DELTA_INPUT,
                tx.put_payload(&delta, RetentionClass::Record)?,
            ));
        }
        let blockers = canonical(&Value::Array(self.blockers.clone()))?;
        material.push(input(
            BLOCKERS_INPUT,
            tx.put_payload(&blockers, RetentionClass::Record)?,
        ));
        let issue = CheckIssue {
            phase,
            work_order: order.id(),
            previous_check: self.first,
            original_plan_digest: self.first.plan_digest,
            plan_digest: draft.digest(),
            truth_versions: &self.truth_versions,
            scope,
            scope_policy_version,
            material: &material,
        };
        tx.append(NewEvent {
            stream: StreamName(phase_stream(phase)),
            type_name: PLAN_CHECK_ISSUED.into(),
            type_version: PLAN_CHECK_ISSUED_VERSION,
            git: None,
            payload: issue.payload(),
            attachments: material.iter().map(|input| input.body.clone()).collect(),
        })?;
        order.issue_within(tx, at, &material)
    }
}

/// The re-check scope `policy` selects, `review.triggers.plan.recheck`,
/// `full` when it holds none.
pub fn scope(policy: &EffectivePolicy) -> RecheckScope {
    match policy
        .settings
        .get(SCOPE_SETTING)
        .and_then(|effective| effective.value.as_ref())
    {
        Some(Setting::RecheckScope(scope)) => *scope,
        _ => RecheckScope::Full,
    }
}

/// `value` as canonical JSON, the form every material body takes.
fn canonical(value: &Value) -> Result<Vec<u8>, StoreError> {
    canonical_json(value).map_err(|_| unrecordable("a re-check's material"))
}

/// The fault of round-2 material that has no canonical form.
fn unrecordable(what: &str) -> StoreError {
    StoreError::Unavailable(format!("{what} cannot be recorded as canonical JSON"))
}

/// One lineage `phase_plan` holds.
struct Held {
    revision: String,
    first: PreviousCheck,
    rechecked: bool,
}

/// Judges the place of `checked`, a submit's or a replacement's accepted
/// check of `project`, in its check lineage under `policy`, recording
/// nothing, and gives `checked` back (D-01, D-02). With the plan gate `off`
/// nothing is judged. Otherwise the phase's lineages, `phase_plan`'s
/// `checks` as the check read them, and the head of the answered order's
/// chain, its `request`, go to `judge_check_round`: a fresh lineage gives
/// `checked` back as it came; round 2 attaches the lineage's revision order
/// and first check; a lineage whose round 2 was issued is refused
/// `third-round` at slot `work_order` naming its revision order.
///
/// For round 2 the lineage's round-1 `plan.checked` is read by its sequence
/// on `phase/<n>`, its draft the lineage's first, and each body it bound
/// must be held whole as recorded: an original draft purged, reduced, not
/// read back as a plan draft or at another digest is refused
/// `material-unavailable` at slot `work_order` with `{"input": "original"}`,
/// and a purged or reduced truths snapshot the same with `{"input":
/// "truths"}`, since a re-check never substitutes the current draft or
/// truths (ADR 0037). Then the truths the check judged the plan's stories
/// under go with round 1's versions to `judge_recheck_truths`: a truth
/// added, removed or re-versioned since is refused `recheck-truths-changed`
/// at slot `plan` naming each, while an approval that left a truth's text
/// alone keeps its version and passes (D-11). Round 2 attaches round 1's
/// truth versions with the lineage.
///
/// The documents the check read are the ones the recording observes, so a
/// lineage moved since makes the call `ledger-busy`. A lineage, chain or
/// round-1 result that cannot be read is a ledger fault.
pub fn judge<S: Ledger + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    mut checked: Checked,
    policy: &EffectivePolicy,
) -> Result<Checked, Value> {
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
    let round_one = round_one(store, project, checked.plan.phase(), first)?;
    let unavailable = |input: &str| material_unavailable(input, revision);
    let Some(original_plan) = original_held(store, &round_one.plan, first.plan_digest)? else {
        return Err(unavailable(ORIGINAL_INPUT));
    };
    if !held_whole(store, &round_one.truths)? {
        return Err(unavailable(TRUTHS_INPUT));
    }
    let recheck = Recheck {
        revision: revision.to_owned(),
        first,
        original: round_one.plan,
        original_plan,
        truths: round_one.truths,
        truth_versions: round_one.truth_versions,
        blockers: round_one.blockers,
    };
    judge_recheck_truths(&recheck.truth_versions, &checked.truths)
        .map_err(|refusal| truths_changed(&refusal, revision))?;
    checked.recheck = Some(recheck);
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

/// What a lineage's round-1 result bound that round 2 reads again.
struct RoundOne {
    /// The reference of round 1's draft body, its `plan` material.
    plan: PayloadRef,
    /// The reference of round 1's truths snapshot.
    truths: PayloadRef,
    /// The truths round 1 was bound to.
    truth_versions: Vec<TruthVersion>,
    /// Round 1's blockers as returned.
    blockers: Vec<Value>,
}

/// The round-1 `plan.checked` at `first`'s sequence on phase `phase`'s
/// stream, checked against `first`'s draft. A result not there, at another
/// draft, or whose material cannot be read is a ledger fault.
fn round_one<L: Ledger + ?Sized>(
    store: &L,
    project: &ProjectId,
    phase: u32,
    first: PreviousCheck,
) -> Result<RoundOne, Value> {
    let filter = HistoryFilter {
        types: vec![PLAN_CHECKED.into()],
        git_commit: None,
    };
    let page = PageRequest {
        limit: 1,
        after: None,
    };
    let digest = json!(first.plan_digest.to_hex());
    let stream = phase_stream(phase);
    let event = store
        .history(project, first.seq..=first.seq, &filter, page)
        .map_err(busy)?
        .items
        .into_iter()
        .find(|event| event.stream == stream && event.payload.get("plan_digest") == Some(&digest))
        .ok_or_else(|| unreadable("the lineage's first check cannot be read"))?;
    let material = |name: &str| {
        event
            .payload
            .get("material")?
            .as_array()?
            .iter()
            .find(|input| input.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|input| PayloadRef::from_value(input.get("body")?))
    };
    let small = |value: &Value, name: &str| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let truth_versions = event
        .payload
        .get("truth_versions")
        .and_then(Value::as_array)
        .and_then(|truths| {
            truths
                .iter()
                .map(|truth| {
                    Some(TruthVersion {
                        story: small(truth, "story")?,
                        truth: truth.get("truth")?.as_str()?.to_owned(),
                        version: small(truth, "version")?,
                    })
                })
                .collect::<Option<Vec<_>>>()
        });
    // Recorded findings were judged at the return, so they judge again.
    let blockers = event
        .payload
        .get("findings")
        .and_then(Value::as_array)
        .and_then(|findings| judge_findings(findings.clone()).ok())
        .map(|findings| findings.blockers().into_iter().cloned().collect());
    match (
        material(PLAN_INPUT),
        material(TRUTHS_INPUT),
        truth_versions,
        blockers,
    ) {
        (Some(plan), Some(truths), Some(truth_versions), Some(blockers)) => Ok(RoundOne {
            plan,
            truths,
            truth_versions,
            blockers,
        }),
        _ => Err(unreadable("the lineage's first check cannot be read")),
    }
}

/// Whether the body `reference` names is held whole, at the length it was
/// recorded with.
fn held_whole<P: Payloads + ?Sized>(store: &P, reference: &PayloadRef) -> Result<bool, Value> {
    Ok(match store.status(&reference.hash).map_err(busy)? {
        PayloadStatus::Present { bytes } => bytes == reference.bytes,
        PayloadStatus::Reduced { .. } | PayloadStatus::Purged { .. } => false,
    })
}

/// Round 1's draft body `reference` read back as a plan draft, or none when
/// it is not held whole, does not read back as one, or is at a digest other
/// than `digest`.
fn original_held<P: Payloads + ?Sized>(
    store: &P,
    reference: &PayloadRef,
    digest: Hash,
) -> Result<Option<RecordedPlan>, Value> {
    if !held_whole(store, reference)? {
        return Ok(None);
    }
    let mut reader = match store.open(&reference.hash).map_err(busy)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => return Ok(None),
    };
    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).is_err() {
        return Ok(None);
    }
    let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    if !request_digest(&body).is_ok_and(|read| read == digest) {
        return Ok(None);
    }
    Ok(read_plan_draft(&body))
}

/// The refusal for a re-check of the lineage of revision order `revision`
/// whose retained `input` is gone.
fn material_unavailable(input: &str, revision: &str) -> Value {
    let what = match input {
        ORIGINAL_INPUT => "the draft round 1 checked",
        _ => "the truths round 1 was bound to",
    };
    Refusal::new(
        MATERIAL_UNAVAILABLE,
        format!(
            "the retained copy of {what} is no longer held as recorded, so no re-check can run and nothing was recorded, since a re-check reads the retained copies and never the current draft or truths. Fail revision work order {revision} with `work-order-fail`, then a fresh `phase-plan` order starts a new lineage, whose submit gets a full first check"
        ),
    )
    .slot("work_order")
    .details(json!({"input": input}))
    .value()
}

/// The refusal for a re-check of the lineage of revision order `revision`
/// whose truths differ from round 1's.
fn truths_changed(refusal: &CheckRefusal, revision: &str) -> Value {
    Refusal::new(
        refusal.code(),
        format!(
            "the truths of the plan's stories are not the ones round 1 was bound to, as the details list, so no re-check can run and nothing was recorded, since changed truths need a new plan submission and a full first check. Fail revision work order {revision} with `work-order-fail`, then a fresh `phase-plan` order starts a new lineage, whose submit gets a full first check"
        ),
    )
    .slot("plan")
    .details(refusal.details())
    .value()
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

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
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
        judge(store, &project(), checked, policy)
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

    /// The state of work order `id` as the `work_order` view holds it.
    fn state(store: &SqliteStore, id: &str) -> Value {
        crate::work_order::read::order(store, &project(), id)
            .unwrap()
            .unwrap()["state"]
            .clone()
    }

    #[test]
    fn a_recheck_over_a_purged_original_substituting_the_current_draft_is_caught() {
        use crate::planning::check_return::tests::c1_material;

        for (input, name) in [(0, "original"), (1, "truths")] {
            let (_dir, store, _, _) = lineage_v(vec![fb(), fw()]);
            let gone = c1_material(&store)[input].body.hash;
            crate::scope::fixtures::purge_in(&store, &project(), gone, "the owner purged it");

            let before = head(&store);
            let refusal = judged(&store, &pl2r(), V, &p0()).expect_err("the re-check is refused");
            assert_eq!(refusal["status"], "refused", "{refusal}");
            assert_eq!(refusal["code"], "material-unavailable", "{refusal}");
            assert_eq!(refusal["slot"], "work_order", "{refusal}");
            assert_eq!(refusal["details"], json!({"input": name}));
            assert_eq!(head(&store), before);
            assert_eq!(state(&store, V), "pending");
        }
    }

    #[test]
    fn a_truth_approved_unchanged_refusing_the_recheck_is_caught() {
        use crate::planning::check_return::tests::tv;
        use crate::refine::submit::tests::{t1, t2};

        let (_dir, store, _, _) = lineage_v(vec![fb(), fw()]);
        append(
            &store,
            R4,
            T7,
            vec![refined(
                1,
                "1c1c1c1c-1c1c-4c1c-8c1c-1c1c1c1c1c1c",
                vec![versioned(t1()), versioned(t2())],
            )],
        );

        let round_two = binding(&store, &pl2r(), V, &p0()).expect("round 2");
        assert_eq!(round_two.truth_versions, tv());
    }

    /// C2, the round-2 checker order.
    const C2: &str = "c2c2c2c2-2a2b-4c2d-8e2f-303132333435";
    /// C3 and C4, the round-1 checker orders of fresh lineages.
    const C3: &str = "c3c3c3c3-3a3b-4c3d-9e3f-404142434445";
    const C4: &str = "c4c4c4c4-4a4b-4c4d-8e4f-505152535455";
    /// O3 and O4, fresh planner orders of phase 1.
    const O3: &str = "3f4e5d6c-7b8a-4990-8a1b-2c3d4e5f6071";
    const O4: &str = "4a5b6c7d-8e9f-4a0b-9c1d-2e3f4a5b6c7d";

    /// `plan` answering `order` through the check, the lineage step under
    /// `policy` and the gated recording under the same policy for Claude
    /// Code with N0, its checker order as `id`, under a command at policy
    /// version `version` whose caller carries the order and its
    /// instructions, under `request_id` at `at`. Gives the answer.
    #[allow(clippy::too_many_arguments)]
    fn through(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
        version: u64,
    ) -> Value {
        let checked = match judged(store, plan, order, policy) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.submit", request_id, 8, at);
        command.policy_version = version;
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(baley_store::Caller::Server(on));
        crate::planning::submit::record_gated(
            store,
            &command,
            Some(&checked),
            policy,
            baley_core::policy::Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    /// The issue of order `id`.
    fn issue_of(store: &SqliteStore, id: &str) -> baley_store::Event {
        crate::refine::order::tests::issued(store)
            .into_iter()
            .find(|event| event.payload["id"] == id)
            .expect("the order is issued")
    }

    #[test]
    fn a_changed_truth_rechecked_a_third_round_bought_or_the_lineage_left_stuck_is_caught() {
        use crate::planning::phase_plan::tests::planned;
        use crate::refine::order::tests::{input_body, input_names};
        use crate::refine::submit::tests::{t1, t2};

        // Truths: t2 changed since round 1.
        let (_dir, store, _, _) = lineage_v(vec![fb()]);
        let mut t2_again = versioned(t2());
        t2_again["outcome"] = json!("the same round, whatever the order");
        t2_again["version"] = json!(2);
        append(
            &store,
            R4,
            T7,
            vec![refined(
                1,
                "1b1b1b1b-1b1b-4b1b-8b1b-1b1b1b1b1b1b",
                vec![versioned(t1()), t2_again],
            )],
        );
        let before = head(&store);
        let refusal = judged(&store, &pl2v(), V, &p0()).expect_err("the re-check is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "recheck-truths-changed", "{refusal}");
        assert_eq!(refusal["slot"], "plan", "{refusal}");
        assert_eq!(
            refusal["details"],
            json!({"truths": [{"story": 1, "truth": "t2", "bound": 1, "current": 2}]})
        );
        assert_eq!(head(&store), before);
        assert_eq!(state(&store, V), "pending");
        crate::mcp::work_order::tests::fail_order(&store, &project(), V, R5, T8);
        let o3 = planned(&store, &[1], R6, T8, O3);
        assert_eq!(o3["work_order"], O3, "{o3}");
        let fresh = through(&store, &pl2v(), O3, &p0(), C3, R7, T9, 5);
        assert_eq!(fresh["check"]["work_order"], C3, "{fresh}");
        let check = input_body(&store, &issue_of(&store, C3), "check");
        assert_eq!(check["round"], 1, "{check}");
        assert_eq!(check["scope"], "full", "{check}");
        assert_eq!(
            check["truth_versions"],
            json!([{"story": 1, "truth": "t1", "version": 1},
                {"story": 1, "truth": "t2", "version": 2}])
        );

        // Third round: round 2 issued and failed, then V resubmitted.
        let (_dir, store, _, _) = lineage_v(vec![fb()]);
        let round_two = through(&store, &pl2r(), V, &p0(), C2, R4, T7, 5);
        assert_eq!(round_two["check"]["work_order"], C2, "{round_two}");
        assert_eq!(
            input_names(&issue_of(&store, C2)),
            ["original", "plan", "truths", "blockers"]
        );
        fail_by_hand(&store, C2, R5);
        let before = head(&store);
        let refusal = judged(&store, &pl2r(), V, &p0()).expect_err("a third round is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "third-round", "{refusal}");
        assert_eq!(refusal["slot"], "work_order", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": V}));
        assert_eq!(head(&store), before);
        let answered = crate::mcp::work_order::fail_check(&store, &project(), V)
            .expect_err("V is answered by its round-2 draft");
        assert_eq!(answered["code"], "work-order-not-pending", "{answered}");
        assert_eq!(answered["details"], json!({"state": "answered"}));
        let o4 = planned(&store, &[1], R6, T8, O4);
        assert_eq!(o4["work_order"], O4, "{o4}");
        let fresh = through(&store, &pl3(), O4, &p0(), C4, R7, T9, 5);
        assert_eq!(fresh["check"]["work_order"], C4, "{fresh}");
        let check = input_body(&store, &issue_of(&store, C4), "check");
        assert_eq!(check["round"], 1, "{check}");
    }

    /// The names of `material`, a list of `{name, body}`, in order.
    fn names(material: &Value) -> Vec<&str> {
        material
            .as_array()
            .unwrap()
            .iter()
            .map(|input| input["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_round_two_binding_recorded_after_its_order_or_reselected_from_current_files_is_caught() {
        use crate::planning::check_return::tests::c1_material;
        use crate::refine::order::tests::input_body;

        // Under PD at policy version 6: the `diff` scope.
        let (_dir, store, x2, k) = lineage_v(vec![fb(), fw()]);
        let receipt = through(&store, &pl2r(), V, &pd(), C2, R4, T7, 6);
        assert_eq!(receipt["status"], "ok", "{receipt}");
        assert_eq!(receipt["check"]["work_order"], C2, "{receipt}");
        let x2r = receipt["digest"].clone();

        let events = history(&store);
        // The store's own `command.completed` closes every transaction.
        let mine: Vec<_> = events
            .iter()
            .filter(|e| e.request_id.0 == R4 && e.type_name != "command.completed")
            .collect();
        let types: Vec<&str> = mine.iter().map(|e| e.type_name.as_str()).collect();
        assert_eq!(
            types,
            ["scope.submitted", "plan.check_issued", "work_order.issued"]
        );
        let (draft, issue, order) = (mine[0], mine[1], mine[2]);
        assert_eq!(draft.payload["digest"], x2r);
        assert_eq!(issue.stream, "phase/1");
        let mut binding = issue.payload.clone();
        let material = binding.as_object_mut().unwrap().remove("material").unwrap();
        assert_eq!(
            binding,
            json!({"phase": 1, "work_order": C2, "round": 2,
                "previous_check": {"seq": k, "plan_digest": x2.to_hex()},
                "original_plan_digest": x2.to_hex(), "plan_digest": x2r,
                "truth_versions": [{"story": 1, "truth": "t1", "version": 1},
                    {"story": 1, "truth": "t2", "version": 1}],
                "scope": "diff", "scope_policy_version": 6})
        );
        assert_eq!(
            names(&material),
            ["original", "plan", "truths", "delta", "blockers"]
        );
        let c1 = c1_material(&store);
        assert_eq!(material[0]["body"], c1[0].body.to_value());
        assert_eq!(material[1]["body"], draft.payload["body"]);
        assert_eq!(material[2]["body"], c1[1].body.to_value());
        assert_eq!(order.payload["id"], C2);
        assert_eq!(order.payload["inputs"], material);
        assert_eq!(
            input_body(&store, order, "delta"),
            json!([{"change": "modified", "member": "goal", "before": "Record every step",
                "after": "Record every step, revised"}])
        );
        assert_eq!(input_body(&store, order, "blockers"), json!([fb()]));

        // Under P0: the `full` scope, no delta.
        let (_dir, store, _, _) = lineage_v(vec![fb(), fw()]);
        let receipt = through(&store, &pl2r(), V, &p0(), C2, R4, T7, 5);
        assert_eq!(receipt["check"]["work_order"], C2, "{receipt}");
        let events = history(&store);
        let issue = of_type(&events, "plan.check_issued")[0];
        assert_eq!(issue.payload["scope"], "full");
        assert_eq!(
            names(&issue.payload["material"]),
            ["original", "plan", "truths", "blockers"]
        );
        assert_eq!(
            issue_of(&store, C2).payload["inputs"],
            issue.payload["material"]
        );
    }
}
