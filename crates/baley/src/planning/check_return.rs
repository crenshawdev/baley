//! The plan checker's return (design 0005 PLN-R16 and section 6, D-06,
//! D-17): a session relays the findings its checker returned for a checker
//! order of a phase. The findings are relayed unchanged: neither the
//! session nor Baley edits, drops or adjudicates them, since adjudication
//! belongs to reviews (ADR 0019).
//!
//! The check runs before admission and records nothing: the order, the
//! findings' shape, then the round's binding read back from the order
//! itself, never from the files or settings current at the return.

use std::io::Read;

use baley_core::plan::{
    CheckRefusal, Findings, RecordedPlan, TruthVersion, judge_findings, read_plan_draft,
};
use baley_core::policy::{RecheckScope, Role};
use baley_core::work_order::{
    NO_SUCH_WORK_ORDER, OrderInput, PENDING, WORK_ORDER_NOT_PENDING, WORK_ORDER_VIEW,
    WRONG_WORK_ORDER, phase_subject, work_order_key,
};
use baley_store::{
    Hash, InstructionEvidence, ObservedDocument, PayloadBody, PayloadRef, Payloads, ProjectId,
    StoreError, Views,
};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::checker::{CHECK_INPUT, PLAN_INPUT, TRUTHS_INPUT};
use crate::refine::order::order_evidence;
use crate::scope::submit::NOT_RECORDED_BUSY;
use crate::work_order::read::order_document;

/// What the return's check read and judged, which the recording records.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedReturn {
    /// The checker order's instruction evidence, which the return's caller
    /// carries.
    pub evidence: Vec<InstructionEvidence>,
    /// The checker order returned.
    work_order: String,
    /// The phase of the plan checked.
    phase: u32,
    /// The round's binding, copied from the order.
    binding: Binding,
    /// The findings, judged and exactly as returned.
    findings: Findings,
    /// The draft's body as recorded.
    body: Value,
    /// The draft's body read back as a plan draft.
    draft: RecordedPlan,
    /// The order's `work_order` document at the sequence that produced it.
    documents: Vec<ObservedDocument>,
}

/// A round's binding as the checker order holds it (design 0005 section 6).
#[derive(Debug, Clone, PartialEq)]
struct Binding {
    /// The round, 1 here.
    round: u64,
    /// The digest of the draft checked.
    plan_digest: Hash,
    /// The scope the round reads.
    scope: RecheckScope,
    /// The truths the round is bound to.
    truth_versions: Vec<TruthVersion>,
    /// The policy version that selected the scope.
    scope_policy_version: u64,
    /// The order's `plan` and `truths` inputs' references.
    material: Vec<OrderInput>,
}

/// The check of a return of checker order `work_order` with `findings` as
/// relayed, run before admission and recording nothing. In order, each at
/// slot `work_order`: `no-such-work-order` when the project holds no order
/// with that id; `wrong-work-order` when it is not a checker order of a
/// phase; `work-order-not-pending` naming its state when it failed or
/// already returned. Then the findings judged by `judge_findings`,
/// `finding-shape` at slot `findings` naming the first entry that is not a
/// finding. Then the round's binding, read back from the order's `check`
/// input, with its `plan` and `truths` inputs' references as the material,
/// and the draft read back through its `plan` input, so the result is bound
/// to what the order bound and never to the truths or settings current now
/// (design 0005 section 6). A binding or body that cannot be read is a
/// ledger fault; a read fault is the `failed` answer.
pub fn check<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    work_order: &str,
    findings: Vec<Value>,
) -> Result<CheckedReturn, Value> {
    let busy = |error: StoreError| store_failed(&error, NOT_RECORDED_BUSY);
    let order = order_document(store, project, work_order)
        .map_err(busy)?
        .ok_or_else(no_such_work_order)?;
    let body = &order.body;
    let text = |field: &str| body.get(field).and_then(Value::as_str);
    let phase = (text("role") == Some(Role::Checker.name()))
        .then(|| text("subject").and_then(phase_of_subject))
        .flatten()
        .ok_or_else(wrong_work_order)?;
    match text("state") {
        Some(PENDING) => {}
        Some(state) => return Err(not_pending(work_order, state)),
        None => return Err(unreadable("the checker order's state cannot be read")),
    }
    let findings = judge_findings(findings).map_err(|refusal| finding_refused(&refusal))?;
    let evidence = order_evidence(body)
        .ok_or_else(|| unreadable("the checker order's instructions cannot be read"))?;
    let binding_body = input_body(store, body, CHECK_INPUT)?;
    let material = [PLAN_INPUT, TRUTHS_INPUT]
        .into_iter()
        .map(|name| {
            input_ref(body, name).map(|body| OrderInput {
                name: name.into(),
                body,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| unreadable("the checker order's material cannot be read"))?;
    let binding = binding(&binding_body, phase, material)
        .ok_or_else(|| unreadable("the checker order's binding cannot be read"))?;
    let draft_body = input_body(store, body, PLAN_INPUT)?;
    let draft = read_plan_draft(&draft_body)
        .ok_or_else(|| unreadable("the checked draft no longer reads as a plan draft"))?;
    Ok(CheckedReturn {
        evidence,
        work_order: work_order.into(),
        phase,
        binding,
        findings,
        body: draft_body,
        draft,
        documents: vec![ObservedDocument {
            view: WORK_ORDER_VIEW.into(),
            key: work_order_key(work_order),
            produced_seq: Some(order.produced_seq),
        }],
    })
}

/// The phase `subject` names when it is `phase/<n>` exactly, as
/// `phase_subject` spells it.
fn phase_of_subject(subject: &str) -> Option<u32> {
    let phase = subject.strip_prefix("phase/")?.parse::<u32>().ok()?;
    (phase > 0 && phase_subject(phase) == subject).then_some(phase)
}

/// The reference of the input `name` an order's document lists.
fn input_ref(order: &Value, name: &str) -> Option<PayloadRef> {
    order
        .get("inputs")?
        .as_array()?
        .iter()
        .find(|input| input.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|input| PayloadRef::from_value(input.get("body")?))
}

/// The input `name` an order's document lists, its body read and parsed as
/// JSON. A body not listed, no longer held or not JSON is a ledger fault.
fn input_body<P: Payloads + ?Sized>(
    payloads: &P,
    order: &Value,
    name: &str,
) -> Result<Value, Value> {
    let missing = || unreadable(&format!("the checker order's {name} input cannot be read"));
    let reference = input_ref(order, name).ok_or_else(missing)?;
    let mut reader = match payloads
        .open(&reference.hash)
        .map_err(|error| store_failed(&error, NOT_RECORDED_BUSY))?
    {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => return Err(missing()),
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|_| missing())?;
    serde_json::from_slice(&bytes).map_err(|_| missing())
}

/// The round-1 binding a `check` input's body holds for phase `phase`, with
/// `material` as its material, or none when a member cannot be read, the
/// phase differs or the round is not 1.
fn binding(check: &Value, phase: u32, material: Vec<OrderInput>) -> Option<Binding> {
    let whole = |value: &Value, name: &str| value.get(name).and_then(Value::as_u64);
    let small = |value: &Value, name: &str| whole(value, name).and_then(|n| u32::try_from(n).ok());
    // Round 2's binding is the one `plan.check_issued` records, read when
    // round 2 returns, so an order holding another round is not read here.
    let round = whole(check, "round").filter(|round| *round == 1)?;
    if small(check, "phase")? != phase {
        return None;
    }
    let plan_digest = check
        .get("plan_digest")
        .and_then(Value::as_str)
        .and_then(Hash::from_hex)?;
    let scope = check
        .get("scope")
        .and_then(Value::as_str)
        .and_then(RecheckScope::parse)?;
    let truth_versions = check
        .get("truth_versions")?
        .as_array()?
        .iter()
        .map(|truth| {
            Some(TruthVersion {
                story: small(truth, "story")?,
                truth: truth.get("truth")?.as_str()?.into(),
                version: small(truth, "version")?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Binding {
        round,
        plan_digest,
        scope,
        truth_versions,
        scope_policy_version: whole(check, "scope_policy_version")?,
        material,
    })
}

fn no_such_work_order() -> Value {
    Refusal::new(
        NO_SUCH_WORK_ORDER,
        "this project holds no work order with that id, so nothing was recorded",
    )
    .slot("work_order")
    .value()
}

fn wrong_work_order() -> Value {
    Refusal::new(
        WRONG_WORK_ORDER,
        "that work order is not a checker order of a phase, so these findings cannot answer it, and nothing was recorded",
    )
    .slot("work_order")
    .value()
}

/// The refusal for checker order `work_order`, which is `state` rather than
/// pending.
fn not_pending(work_order: &str, state: &str) -> Value {
    Refusal::new(
        WORK_ORDER_NOT_PENDING,
        format!(
            "checker work order {work_order} is {state}, so these findings were not recorded. A failed order is retried with `work-order-retry`, and an answered one has already returned"
        ),
    )
    .slot("work_order")
    .details(json!({"state": state}))
    .value()
}

/// The refusal for a findings list `judge_findings` refused.
fn finding_refused(refusal: &CheckRefusal) -> Value {
    let position = match refusal {
        CheckRefusal::FindingShape { position } => *position,
        // The findings judge refuses nothing else.
        _ => 0,
    };
    Refusal::new(
        refusal.code(),
        format!(
            "the entry at position {position}, counted from 0, is not a finding: a finding needs a `severity` of `blocker` or `warning`, a `location`, a `claim` and a `fix`, and no other member, so nothing was recorded"
        ),
    )
    .slot("findings")
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
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005
    //! PLN-R16 and sections 5 to 7, design 0008 REV-R6, D-04, D-06 to D-08,
    //! D-14, D-17 and the plan's data store C, O, PL1, PL2, P0, POFF, PB,
    //! PDF, PAJ, N0, C1, FB, FW and FN, never from running this code.

    use baley_core::policy::{EffectivePolicy, Host};
    use baley_store::Caller;
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;
    use crate::planning::checker::pending_checker;
    use crate::planning::checker::tests::C1;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit;
    use crate::planning::submit::tests::order_o;
    use crate::refine::order::tests::issued;
    use crate::scope::fixtures::*;

    /// V, the planner revision order a round-1 blocker issues.
    pub(crate) const V: &str = "2d3e4f50-6172-4c83-9d94-aea5f6071823";

    /// FB, a blocker.
    pub(crate) fn fb() -> Value {
        json!({"severity": "blocker", "location": "tasks[0]",
            "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    /// FW, a warning.
    pub(crate) fn fw() -> Value {
        json!({"severity": "warning", "location": "tasks[0].verify[0]",
            "claim": "The verify runs every ledger test", "fix": "Name the round test"})
    }

    /// FN, FB without its severity.
    pub(crate) fn fn_() -> Value {
        json!({"location": "tasks[0]", "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    /// Submits `plan` answering `order` with no capacity through the check
    /// and the gated recording under `policy` for Claude Code with N0, its
    /// checker order, if any, as `id`, under a command at policy version 5
    /// whose caller carries the order and its instructions, under
    /// `request_id` at `at`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gated(
        store: &SqliteStore,
        plan: &Value,
        order: &str,
        policy: &EffectivePolicy,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        let checked = match submit::check(store, &project(), plan, order, None) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let mut command = command("plan.submit", request_id, 8, at);
        command.policy_version = 5;
        let on = caller()
            .with_work_order(order)
            .unwrap()
            .with_instructions(checked.evidence.clone())
            .unwrap();
        command.caller = Some(Caller::Server(on));
        submit::record_gated(
            store,
            &command,
            Some(&checked),
            policy,
            Host::ClaudeCode,
            &n0(),
            id,
        )
    }

    #[test]
    fn a_finding_without_severity_recorded_or_the_plan_gate_read_wrongly_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let off = gated(&store, &pl1(), O, &poff(), C1, R2, T5);
        assert_eq!(off["status"], "ok", "{off}");
        assert!(off.get("preview").is_some(), "{off}");
        assert!(off["set"].is_string(), "{off}");
        assert!(off.get("check").is_none(), "{off}");
        assert!(
            issued(&store)
                .iter()
                .all(|event| event.payload["role"] != "checker")
        );

        for policy in [pb(), pdf(), paj()] {
            let (_dir, store) = store_c();
            order_o(&store, R1);
            let receipt = gated(&store, &pl1(), O, &policy, C1, R2, T5);
            assert_eq!(receipt["status"], "ok", "{receipt}");
            assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
            let checks: Vec<_> = issued(&store)
                .into_iter()
                .filter(|event| event.payload["id"] == C1)
                .collect();
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].payload["role"], "checker");
        }

        let (_dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl1(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        let before = head(&store);
        let refusal = check(&store, &project(), C1, vec![fw(), fn_()]).expect_err("FN is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "finding-shape", "{refusal}");
        assert_eq!(refusal["slot"], "findings", "{refusal}");
        assert_eq!(refusal["details"], json!({"position": 1}));
        assert_eq!(head(&store), before);
        assert_eq!(
            pending_checker(&store, &project(), 1).unwrap().as_deref(),
            Some(C1)
        );
    }

    /// An id no order has.
    const UNKNOWN: &str = "5d6e7f80-91a2-4bc3-8d4e-5f6071829304";

    /// Store C with O, then PL2 through the gated recording under P0 as C1.
    /// Gives the store and X2.
    fn pl2_checking() -> (tempfile::TempDir, SqliteStore, String) {
        let (dir, store) = store_c();
        order_o(&store, R1);
        let receipt = gated(&store, &pl2(), O, &p0(), C1, R2, T5);
        assert_eq!(receipt["check"]["work_order"], C1, "{receipt}");
        let x2 = receipt["digest"].as_str().unwrap().to_owned();
        (dir, store, x2)
    }

    /// The return of `work_order` with `[FW]`, which must refuse with
    /// `code` at slot `work_order` and record nothing. Gives the refusal.
    fn refused_with(store: &SqliteStore, work_order: &str, code: &str) -> Value {
        let before = head(store);
        let refusal =
            check(store, &project(), work_order, vec![fw()]).expect_err("the return is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], code, "{refusal}");
        assert_eq!(refusal["slot"], "work_order", "{refusal}");
        assert_eq!(head(store), before);
        refusal
    }

    /// A round-1 `plan.checked` of C1 for the draft `digest` with `[FW]` and
    /// no revision, appended by hand under `request_id`.
    fn returned_by_hand(store: &SqliteStore, digest: &str, request_id: &str) {
        use baley_core::plan::{CheckResult, PLAN_CHECKED, PLAN_CHECKED_VERSION};
        use baley_store::{NewEvent, StreamName};

        let findings = [fw()];
        let payload = CheckResult {
            phase: 1,
            plan_digest: Hash::from_hex(digest).unwrap(),
            findings: &findings,
            work_order: C1,
            scope: RecheckScope::Full,
            previous_check: None,
            truth_versions: &[],
            material: &[],
            scope_policy_version: 5,
            revision: None,
        }
        .payload();
        append(
            store,
            request_id,
            T6,
            vec![NewEvent {
                stream: StreamName("phase/1".into()),
                type_name: PLAN_CHECKED.into(),
                type_version: PLAN_CHECKED_VERSION,
                git: None,
                payload,
                attachments: Vec::new(),
            }],
        );
    }

    #[test]
    fn a_return_for_a_planner_order_or_a_returned_check_accepted_is_caught() {
        use crate::planning::order::tests::fail_by_hand;

        let (_dir, store, _) = pl2_checking();
        refused_with(&store, UNKNOWN, "no-such-work-order");
        refused_with(&store, O, "wrong-work-order");
        fail_by_hand(&store, C1, R3);
        let failed = refused_with(&store, C1, "work-order-not-pending");
        assert_eq!(failed["details"], json!({"state": "failed"}));

        let (_dir, store, x2) = pl2_checking();
        returned_by_hand(&store, &x2, R3);
        let answered = refused_with(&store, C1, "work-order-not-pending");
        assert_eq!(answered["details"], json!({"state": "answered"}));
    }

    /// The references C1's issued order holds for its `plan` and `truths`
    /// inputs, in that order.
    pub(crate) fn c1_material(store: &SqliteStore) -> Vec<OrderInput> {
        let order = issued(store)
            .into_iter()
            .find(|event| event.payload["id"] == C1)
            .expect("C1 is issued");
        ["plan", "truths"]
            .into_iter()
            .map(|name| {
                let input = order.payload["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|input| input["name"] == name)
                    .unwrap();
                OrderInput {
                    name: name.into(),
                    body: PayloadRef::from_value(&input["body"]).unwrap(),
                }
            })
            .collect()
    }

    /// TV: story 1's `t1` and `t2`, each at version 1.
    pub(crate) fn tv() -> Vec<TruthVersion> {
        ["t1", "t2"]
            .into_iter()
            .map(|truth| TruthVersion {
                story: 1,
                truth: truth.into(),
                version: 1,
            })
            .collect()
    }

    #[test]
    fn a_round_one_binding_read_from_the_current_truths_instead_of_the_order_is_caught() {
        use crate::planning::submit::tests::as_drafted;
        use crate::refine::submit::tests::{t1, t2};

        let (_dir, store, x2) = pl2_checking();
        let mut t2_again = versioned(t2());
        t2_again["outcome"] = json!("the same round, whatever the order");
        t2_again["version"] = json!(2);
        append(
            &store,
            R3,
            T6,
            vec![refined(
                1,
                "1b1b1b1b-1b1b-4b1b-8b1b-1b1b1b1b1b1b",
                vec![versioned(t1()), t2_again],
            )],
        );

        let checked = check(&store, &project(), C1, vec![fw()]).expect("the return is checked");
        assert_eq!(
            checked.binding,
            Binding {
                round: 1,
                plan_digest: Hash::from_hex(&x2).unwrap(),
                scope: RecheckScope::Full,
                truth_versions: tv(),
                scope_policy_version: 5,
                material: c1_material(&store),
            }
        );
        assert_eq!(checked.phase, 1);
        assert_eq!(checked.draft.plan, pl2());
        assert_eq!(checked.body, as_drafted(pl2()));
    }
}
