//! The plan checker's work order (design 0005 PLN-R16, sections 4 and 6,
//! D-01, D-03, D-16): round 1 of a submitted plan's check, prepared outside
//! any transaction and issued on the phase's stream, `phase/<n>`, with
//! subject `phase/<n>`, inside the transaction that records the draft it
//! checks.
//!
//! The order runs the `checker` role at attempt 1 of a new chain and lists
//! `bal-phase-checker`. Its inputs bind what the check judges: `check`, the
//! round's binding, which the result copies; `plan`, the draft's own
//! recorded body; and `truths`, the plan's stories with the truths it was
//! judged under. A phase has one live checker order: an issue declares that
//! no checker order of the phase is pending, so a second one recorded since
//! the caller looked makes the call `ledger-busy` with nothing recorded.

use std::num::NonZeroU32;

use baley_core::plan::{CHECKER_ORDER_PENDING, Plan, PlanDraft};
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, RecheckScope, Role};
use baley_core::work_order::{PENDING, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, phase_subject};
use baley_store::{
    Absence, Command, IndexQuery, InstructionEvidence, KeyValue, ObservedDocument, PageRequest,
    ProjectId, StoreError, Transaction, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::instruction::checker::IDENTITY;
use crate::planning::inputs::{StoriesRead, read_stories};
use crate::refine::inputs::bytes;
use crate::scope::submit::scope_unread;
use crate::work_order::issue::{self, Input, Issue};
use crate::work_order::route::{IssuedRoute, compose};

/// The input holding the round's binding.
pub const CHECK_INPUT: &str = "check";
/// The input holding the draft's body as recorded.
pub const PLAN_INPUT: &str = "plan";
/// The input holding the plan's stories with their truths.
pub const TRUTHS_INPUT: &str = "truths";

/// A round-1 checker order prepared outside any transaction: everything it
/// is issued with.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCheck {
    /// The order's id, also its request.
    id: String,
    /// The phase of the plan it checks.
    phase: u32,
    /// The route it runs under.
    route: IssuedRoute,
    /// The evidence of the checker's text.
    instructions: Vec<InstructionEvidence>,
    /// The inputs, in order.
    inputs: Vec<(&'static str, Vec<u8>)>,
    /// The documents the inputs were read from, at the sequence that
    /// produced each.
    documents: Vec<ObservedDocument>,
}

impl PreparedCheck {
    /// The order's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The documents its inputs were read from, which the recording's
    /// decision observes.
    pub fn documents(&self) -> &[ObservedDocument] {
        &self.documents
    }

    /// Issues the order inside `tx`, a transaction its caller runs and in
    /// which the draft was recorded, at `recorded_at`. The caller declares
    /// [`checker_absences`] and observes [`Self::documents`] in its own
    /// decision. The answer is the receipt's members other than `status`.
    pub fn issue_within(
        &self,
        tx: &mut dyn Transaction,
        recorded_at: &str,
    ) -> Result<Map<String, Value>, StoreError> {
        let inputs: Vec<Input<'_>> = self
            .inputs
            .iter()
            .map(|(name, bytes)| Input::Bytes { name, bytes })
            .collect();
        let subject = phase_subject(self.phase);
        let issue = Issue {
            id: &self.id,
            request: &self.id,
            parent: None,
            subject: &subject,
            route: &self.route,
            instructions: &self.instructions,
            inputs: &inputs,
            absences: &[],
            streams: &[],
            documents: &[],
            further: &Map::new(),
        };
        issue::issue_within(tx, &issue, recorded_at)
    }
}

/// Prepares round 1 of the check of `plan`, drafted as `draft`, as order
/// `id` for `command`'s project: the route for `checker` at attempt 1 under
/// `policy`, the host's effective policy, at the command's policy version,
/// with the catalog `names`; the evidence of `bal-phase-checker`; and the
/// inputs `check`, `plan` and `truths`. The `plan` input is the draft's body,
/// so its reference is the one the draft's `scope.submitted` carries. The
/// `truths` input holds the plan's stories in its order, each as the
/// planner order's `stories` input holds one, and `check` binds round 1,
/// the draft's digest, the `full` scope, each of those truths' versions in
/// the same order and the command's policy version as the scope policy
/// version (design 0005 section 6). Nothing is recorded. A fault is the
/// answer to give.
#[allow(clippy::too_many_arguments)]
pub fn prepare<S: Views + ?Sized>(
    store: &S,
    command: &Command,
    plan: &Plan,
    draft: &PlanDraft,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Result<PreparedCheck, Value> {
    let instructions = vec![crate::instruction::evidence(IDENTITY)?];
    let route = compose(
        policy,
        command.policy_version,
        host,
        Role::Checker,
        NonZeroU32::MIN,
        names,
    )
    .map_err(|refusal| Refusal::new(refusal.code(), refusal.to_string()).value())?;
    let StoriesRead {
        entries, documents, ..
    } = read_stories(store, &command.project, plan.stories())?;
    let check = json!({
        "round": 1,
        "phase": plan.phase(),
        "plan_digest": draft.digest().to_hex(),
        "scope": RecheckScope::Full.name(),
        "truth_versions": truth_versions(&entries),
        "scope_policy_version": command.policy_version,
    });
    let encoded = |value: &Value| bytes(value).map_err(|error| scope_unread(&error));
    Ok(PreparedCheck {
        id: id.into(),
        phase: plan.phase(),
        route,
        instructions,
        inputs: vec![
            (CHECK_INPUT, encoded(&check)?),
            (PLAN_INPUT, draft.body().to_vec()),
            (TRUTHS_INPUT, encoded(&Value::Array(entries))?),
        ],
        documents,
    })
}

/// Each truth `entries` hold, `{story, truth, version}`, in the entries'
/// order and each story's truth order.
fn truth_versions(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .flat_map(|entry| {
            let story = entry["id"].clone();
            entry["truths"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |truth| {
                    json!({"story": story, "truth": truth["id"], "version": truth["version"]})
                })
        })
        .collect()
}

/// The fact a checker order of phase `phase` relies on being absent: a
/// pending checker order of the same phase on the `live` index (D-03).
pub fn checker_absences(phase: u32) -> Vec<Absence> {
    vec![Absence::Documents {
        view: WORK_ORDER_VIEW.into(),
        index: WORK_ORDER_LIVE_INDEX.into(),
        equals: live(phase),
    }]
}

/// The id of phase `phase`'s pending checker order in `project`, the newest
/// the `live` index gives for `checker`, the phase's subject and `pending`,
/// or none.
pub fn pending_checker<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
) -> Result<Option<String>, StoreError> {
    let query = IndexQuery {
        index: WORK_ORDER_LIVE_INDEX.into(),
        equals: live(phase),
        page: PageRequest {
            limit: 1,
            after: None,
        },
    };
    Ok(store
        .find(project, WORK_ORDER_VIEW, &query)?
        .items
        .into_iter()
        .next()
        .map(|document| {
            document
                .body
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }))
}

/// The operation a pending checker order refused, which words the refusal,
/// so its caller is told to repeat what it was doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A `plan-submit` or `plan-replace`.
    Submit,
    /// A `phase-plan`.
    PhasePlan,
    /// The owner's approval of a phase edit whose continuation issues a
    /// planner order.
    Approval,
}

/// The refusal for phase `phase`'s pending checker order `pending`, worded
/// for the `refused` operation, at slot `digest` for an approval and
/// `operation` otherwise (D-03).
pub fn checker_order_pending(phase: u32, pending: &str, refused: Refused) -> Value {
    let (not_done, cause, again) = match refused {
        Refused::Submit => (
            "nothing was recorded",
            "a phase has one live checker order and its return must find the draft it checks",
            "submit again",
        ),
        Refused::PhasePlan => (
            "no planner order was issued and nothing was recorded",
            "a check's revision must be the phase's only live planner order",
            "run `phase-plan` again",
        ),
        Refused::Approval => (
            "nothing was approved",
            "a check's revision must be the phase's only live planner order",
            "run `baley scope approve` again",
        ),
    };
    Refusal::new(
        CHECKER_ORDER_PENDING,
        format!(
            "phase {phase}'s checker work order {pending} is still checking a plan draft, so {not_done}, since {cause}. Wait for that order's return, or fail it with `work-order-fail`, then {again}"
        ),
    )
    .slot(match refused {
        Refused::Approval => "digest",
        Refused::Submit | Refused::PhasePlan => "operation",
    })
    .details(json!({"work_order": pending}))
    .value()
}

/// The `live` index prefix of phase `phase`'s pending checker orders.
fn live(phase: u32) -> Vec<KeyValue> {
    let text = |value: &str| KeyValue::Text(value.into());
    vec![
        text(Role::Checker.name()),
        text(&phase_subject(phase)),
        text(PENDING),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005
    //! sections 4 to 6, D-03, D-16 and the plan's data store C, order O, PL1,
    //! P0 and N0, never from running this code.

    use baley_core::plan::PlanDraft;
    use baley_core::policy::Host;
    use baley_core::scope::{
        PROJECT_STREAM, SCOPE_SUBMITTED, SCOPE_SUBMITTED_VERSION, submitted_payload,
    };
    use baley_store::{Decision, NewEvent, Observed, OutcomeKind, RetentionClass, StreamName};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::planning::order::tests::O;
    use crate::planning::submit::check;
    use crate::planning::submit::tests::order_o;
    use crate::refine::order::tests::{input_body, input_names, issued};
    use crate::scope::fixtures::*;

    /// C1, a checker order of phase 1.
    pub(crate) const C1: &str = "c1c1c1c1-0a0b-4c0d-8e0f-101112131415";
    /// C9, another checker order of phase 1.
    pub(crate) const C9: &str = "c9c9c9c9-1a1b-4c1d-9e1f-202122232425";

    /// `plan` checked under O, then recorded as `record` records a draft
    /// answering O, with checker order `id` prepared under P0 for Claude
    /// Code with N0 and issued beside it, in one transaction under
    /// `request_id` at `at` and policy version 5. Gives the draft's digest.
    pub(crate) fn drafted_with_check(
        store: &SqliteStore,
        plan: &Value,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> String {
        let checked = check(store, &project(), plan, O, None).expect("the plan is accepted");
        let draft = PlanDraft::new(&checked.plan);
        let mut command = command("plan.submit", request_id, 8, at);
        command.policy_version = 5;
        command.caller = Some(baley_store::Caller::Server(
            caller().with_work_order(O).unwrap(),
        ));
        let prepared = prepare(
            store,
            &command,
            &checked.plan,
            &draft,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            id,
        )
        .expect("the check is prepared");
        store
            .transact(&command, &mut |tx| {
                let body = tx.put_payload(draft.body(), RetentionClass::Record)?;
                tx.append(NewEvent {
                    stream: StreamName(PROJECT_STREAM.into()),
                    type_name: SCOPE_SUBMITTED.into(),
                    type_version: SCOPE_SUBMITTED_VERSION,
                    git: None,
                    payload: submitted_payload(draft.digest(), "plan", draft.target(), &body),
                    attachments: vec![body],
                })?;
                prepared.issue_within(tx, at)?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .expect("the draft and its check are recorded");
        draft.digest().to_hex()
    }

    /// PL1's check prepared as `id` under P0 and N0 and issued alone in its
    /// own transaction under `request_id` at `at`, declaring that no checker
    /// order of phase 1 is pending. PL1 is judged over phase 1's facts
    /// directly, since a submit's check refuses beside a pending checker
    /// order. Gives the answer.
    pub(crate) fn issued_alone(store: &SqliteStore, id: &str, request_id: &str, at: &str) -> Value {
        let facts = crate::planning::facts::read(store, &project(), 1, None)
            .expect("phase 1's facts are read")
            .facts;
        let plan = baley_core::plan::judge_plan(&pl1(), &facts).expect("PL1 is accepted");
        let draft = PlanDraft::new(&plan);
        let command = command("plan.submit", request_id, 8, at);
        let prepared = prepare(
            store,
            &command,
            &plan,
            &draft,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            id,
        )
        .expect("the check is prepared");
        let result = store.transact(&command, &mut |tx| {
            let mut receipt = Map::new();
            receipt.insert("status".into(), json!("ok"));
            receipt.extend(prepared.issue_within(tx, at)?);
            Ok(Decision {
                kind: OutcomeKind::Done,
                answer: Value::Object(receipt),
                sensitive: false,
                observed: Observed {
                    absences: checker_absences(1),
                    ..Observed::default()
                },
                git: None,
            })
        });
        issue::answer(result, store)
    }

    #[test]
    fn a_checker_order_bound_to_a_copy_of_the_draft_or_off_its_phase_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        let x1 = drafted_with_check(&store, &pl1(), C1, R2, T5);

        let checks: Vec<_> = issued(&store)
            .into_iter()
            .filter(|event| event.payload["role"] == "checker")
            .collect();
        assert_eq!(checks.len(), 1);
        let order = &checks[0];
        assert_eq!(order.stream, "phase/1");
        let payload = &order.payload;
        assert_eq!(payload["subject"], "phase/1");
        assert_eq!(payload["attempt"], 1);
        assert_eq!(payload["parent"], Value::Null);
        assert_eq!(payload["request"], C1);
        assert_eq!(payload["agent"], "bal-checker-low");
        let identities: Vec<&str> = payload["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["identity"].as_str().unwrap())
            .collect();
        assert_eq!(identities, ["bal-phase-checker"]);
        assert_eq!(input_names(order), ["check", "plan", "truths"]);

        let events = history(&store);
        let draft: Vec<_> = of_type(&events, "scope.submitted")
            .into_iter()
            .filter(|event| event.payload["kind"] == "plan")
            .collect();
        assert_eq!(draft.len(), 1);
        assert_eq!(draft[0].payload["digest"], x1.as_str());
        let plan_input = payload["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["name"] == "plan")
            .unwrap();
        assert_eq!(plan_input["body"], draft[0].payload["body"]);

        assert_eq!(
            input_body(&store, order, "check"),
            json!({"round": 1, "phase": 1, "plan_digest": x1, "scope": "full",
                "truth_versions": [{"story": 1, "truth": "t1", "version": 1},
                    {"story": 1, "truth": "t2", "version": 1}],
                "scope_policy_version": 5})
        );
        let t1 = json!({"id": "t1", "version": 1, "form": "when",
            "trigger": "the owner approves a draft by a stale digest",
            "observer": "the owner", "verb": "is refused",
            "outcome": "stale-draft naming the first differing part", "kind": "literal"});
        let t2 = json!({"id": "t2", "version": 1, "form": "for-any",
            "trigger": "order the answers of a set are recorded in",
            "observer": "a ledger reader", "verb": "gets", "outcome": "the same round",
            "kind": "property"});
        assert_eq!(
            input_body(&store, order, "truths"),
            json!([{"id": 1, "sentence": "Every step is recorded", "truths": [t1, t2]}])
        );
    }

    #[test]
    fn a_second_checker_order_recorded_beside_a_pending_one_is_caught() {
        let (_dir, store) = store_c();
        order_o(&store, R1);
        drafted_with_check(&store, &pl1(), C1, R2, T5);
        assert_eq!(
            pending_checker(&store, &project(), 1).unwrap().as_deref(),
            Some(C1)
        );
        assert_eq!(pending_checker(&store, &project(), 2).unwrap(), None);

        let before = head(&store);
        let busy = issued_alone(&store, C9, R3, T6);
        assert_eq!(busy["status"], "failed", "{busy}");
        assert_eq!(busy["code"], "ledger-busy", "{busy}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_checker_pending_refusal_telling_a_phase_plan_to_submit_is_caught() {
        let reason = |refused: Refused| {
            let refusal = checker_order_pending(1, C1, refused);
            assert_eq!(refusal["code"], "checker-order-pending", "{refusal}");
            assert_eq!(refusal["slot"], "operation", "{refusal}");
            assert_eq!(refusal["details"], json!({"work_order": C1}));
            refusal["reason"].as_str().unwrap().to_owned()
        };

        let phase_plan = reason(Refused::PhasePlan);
        assert!(phase_plan.contains(C1), "{phase_plan}");
        assert!(
            phase_plan.ends_with("then run `phase-plan` again"),
            "{phase_plan}"
        );
        assert!(!phase_plan.contains("submit"), "{phase_plan}");

        let submit = reason(Refused::Submit);
        assert!(submit.contains(C1), "{submit}");
        assert!(submit.ends_with("then submit again"), "{submit}");
        assert!(!submit.contains("phase-plan"), "{submit}");
    }
}
