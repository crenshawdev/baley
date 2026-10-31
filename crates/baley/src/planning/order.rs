//! A phase's planner order (design 0005 sections 4 and 5, D-11, D-12,
//! D-15): a planning or revision order prepared outside any transaction and
//! issued on the phase's stream, `phase/<n>`, with subject `phase/<n>`, in
//! its own transaction or inside a caller's.
//!
//! A phase has one live planner order: an issue declares that no planner
//! order of the phase is pending, so a second one recorded since the caller
//! looked makes the call `ledger-busy` with nothing recorded. A revision
//! starts a new chain at attempt 1, as a planning order does.

use std::num::NonZeroU32;

use baley_core::plan::PLANNER_ORDER_PENDING;
use baley_core::policy::{AcceptedNames, EffectivePolicy, Host, Role};
use baley_core::work_order::{PENDING, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, phase_subject};
use baley_store::{
    Absence, Command, IndexQuery, InstructionEvidence, KeyValue, Ledger, ObservedDocument,
    PageRequest, Payloads, ProjectId, StoreError, Transaction, Views,
};
use serde_json::{Map, Value, json};

use crate::envelope::Refusal;
use crate::instruction::planner::IDENTITY;
use crate::planning::inputs::{self, Revision};
use crate::work_order::issue::{self, Input, Issue};
use crate::work_order::route::{IssuedRoute, compose};

/// A planner order prepared outside any transaction: everything it is
/// issued with.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedOrder {
    /// The order's id, also its request.
    id: String,
    /// The phase it plans.
    phase: u32,
    /// The route it runs under.
    route: IssuedRoute,
    /// The evidence of the planner's text.
    instructions: Vec<InstructionEvidence>,
    /// The inputs read from the ledger, in order.
    inputs: Vec<(&'static str, Vec<u8>)>,
    /// The documents the inputs were read from, at the sequence that
    /// produced each.
    documents: Vec<ObservedDocument>,
}

impl PreparedOrder {
    /// The order's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The phase the order plans.
    pub fn phase(&self) -> u32 {
        self.phase
    }

    /// The documents its inputs were read from, which the recording's
    /// decision observes.
    pub fn documents(&self) -> &[ObservedDocument] {
        &self.documents
    }

    /// Issues the order inside `tx`, a transaction its caller runs,
    /// recorded at `recorded_at`. The caller declares [`planner_absences`]
    /// and observes [`Self::documents`] in its own decision. The answer is
    /// the receipt's members other than `status`.
    pub fn issue_within(
        &self,
        tx: &mut dyn Transaction,
        recorded_at: &str,
    ) -> Result<Map<String, Value>, StoreError> {
        self.with_issue(&[], &[], |issue| {
            issue::issue_within(tx, issue, recorded_at)
        })
    }

    /// Runs `then` over the order as `issue::record` takes it, with
    /// `absences` and `documents` as its caller's.
    fn with_issue<T>(
        &self,
        absences: &[Absence],
        documents: &[ObservedDocument],
        then: impl FnOnce(&Issue<'_>) -> T,
    ) -> T {
        let inputs: Vec<Input<'_>> = self
            .inputs
            .iter()
            .map(|(name, bytes)| Input::Bytes { name, bytes })
            .collect();
        let subject = phase_subject(self.phase);
        then(&Issue {
            id: &self.id,
            request: &self.id,
            parent: None,
            subject: &subject,
            route: &self.route,
            instructions: &self.instructions,
            inputs: &inputs,
            absences,
            streams: &[],
            documents,
            further: &Map::new(),
        })
    }
}

/// Prepares phase `phase`'s planner order as `id` for `command`'s project,
/// committing `stories` in that order: the route for `planner` at attempt 1
/// under `policy`, the host's effective policy, at the command's policy
/// version, with the catalog `names`; the evidence of `bal-phase-planner`;
/// and the inputs read now (D-12), a revision's draft and set among them
/// when `revision` names one (D-15). It has no parent and its id is also its
/// request, so a revision starts a new chain. Nothing is recorded. A fault
/// is the answer to give.
#[allow(clippy::too_many_arguments)]
pub fn prepare<S: Views + Payloads + ?Sized>(
    store: &S,
    command: &Command,
    phase: u32,
    stories: &[u32],
    revision: Option<Revision<'_>>,
    policy: &EffectivePolicy,
    host: Host,
    names: &AcceptedNames,
    id: &str,
) -> Result<PreparedOrder, Value> {
    let instructions = vec![crate::instruction::evidence(IDENTITY)?];
    let route = compose(
        policy,
        command.policy_version,
        host,
        Role::Planner,
        NonZeroU32::MIN,
        names,
    )
    .map_err(|refusal| Refusal::new(refusal.code(), refusal.to_string()).value())?;
    let read = inputs::read(store, &command.project, phase, stories, policy, revision)?;
    Ok(PreparedOrder {
        id: id.into(),
        phase,
        route,
        instructions,
        inputs: read.inputs,
        documents: read.documents,
    })
}

/// Issues `prepared` under `command`, the command its caller prepared, in
/// its own transaction. The decision declares no pending planner order of
/// the phase and observes every document the inputs were read from, so an
/// order issued, a truth revised or a plan approved since makes the call
/// `ledger-busy` with nothing recorded.
pub fn issue<S: Ledger + Payloads>(
    store: &S,
    command: &Command,
    prepared: &PreparedOrder,
) -> Value {
    let absences = planner_absences(prepared.phase);
    prepared.with_issue(&absences, &prepared.documents, |issue| {
        issue::answer(issue::record(store, command, Some(issue)), store)
    })
}

/// The fact a planner order of phase `phase` relies on being absent: a
/// pending planner order of the same phase on the `live` index (D-11).
pub fn planner_absences(phase: u32) -> Vec<Absence> {
    vec![Absence::Documents {
        view: WORK_ORDER_VIEW.into(),
        index: WORK_ORDER_LIVE_INDEX.into(),
        equals: live(phase),
    }]
}

/// The id of phase `phase`'s pending planner order in `project`, the newest
/// the `live` index gives for `planner`, the phase's subject and `pending`,
/// or none. A planning and a revision order share the subject, so either
/// one is found.
pub fn pending_planner<S: Views + ?Sized>(
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

/// The refusal for phase `phase`'s pending planner order `pending`, with
/// `not_issued` saying what was not issued.
pub fn planner_order_pending(phase: u32, pending: &str, not_issued: &str) -> Value {
    Refusal::new(
        PLANNER_ORDER_PENDING,
        format!(
            "phase {phase}'s planner work order {pending} is still pending, {not_issued}, since a phase has one live planner order. Read that order with `document`, or fail it with `work-order-fail` and retry it with `work-order-retry`"
        ),
    )
    .slot("operation")
    .details(json!({"work_order": pending}))
    .value()
}

/// The `live` index prefix of phase `phase`'s pending planner orders.
fn live(phase: u32) -> Vec<KeyValue> {
    let text = |value: &str| KeyValue::Text(value.into());
    vec![
        text(Role::Planner.name()),
        text(&phase_subject(phase)),
        text(PENDING),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    //! Stores are real SQLite stores in fresh temporary directories, with
    //! times supplied; expected values are written out from design 0005,
    //! D-11, D-12, D-15 and the plan's data store C, P0 and N0, never from
    //! running this code.

    use baley_core::plan::phase_stream;
    use baley_core::work_order::{WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION, failed_payload};
    use baley_store::{NewEvent, StreamName};
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::planning::fixtures::*;
    use crate::refine::order::tests::issued;
    use crate::scope::fixtures::*;

    /// O, phase 1's first planning order.
    pub(crate) const O: &str = "1f2e3d4c-5b6a-4978-8695-a4b3c2d1e0f1";
    /// O2, a second planner order of phase 1.
    pub(crate) const O2: &str = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";

    /// Phase 1's planning order for `stories` prepared as `id` under P0 and
    /// N0 for a command under `request_id` at `at`.
    pub(crate) fn prepared(
        store: &SqliteStore,
        stories: &[u32],
        id: &str,
        request_id: &str,
        at: &str,
    ) -> (Command, PreparedOrder) {
        let command = command("phase.plan", request_id, 7, at);
        let prepared = prepare(
            store,
            &command,
            1,
            stories,
            None,
            &p0(),
            Host::ClaudeCode,
            &n0(),
            id,
        )
        .expect("the order is prepared");
        (command, prepared)
    }

    /// Fails order `id` of phase 1 by hand under `request_id`.
    pub(crate) fn fail_by_hand(store: &SqliteStore, id: &str, request_id: &str) {
        append(
            store,
            request_id,
            T6,
            vec![NewEvent {
                stream: StreamName(phase_stream(1)),
                type_name: WORK_ORDER_FAILED.into(),
                type_version: WORK_ORDER_FAILED_VERSION,
                git: None,
                payload: failed_payload(id, "the planner stopped"),
                attachments: Vec::new(),
            }],
        );
    }

    #[test]
    fn a_planner_order_recorded_off_its_phase_or_beside_a_pending_one_is_caught() {
        let (_dir, store) = store_c();
        let (command_o, o) = prepared(&store, &[1], O, R1, T4);
        let (command_o2, o2) = prepared(&store, &[1], O2, R2, T4);
        let receipt = issue(&store, &command_o, &o);
        assert_eq!(receipt["status"], "ok", "{receipt}");

        let issued = issued(&store);
        assert_eq!(issued.len(), 1);
        assert_eq!(issued[0].stream, "phase/1");
        let payload = &issued[0].payload;
        assert_eq!(payload["subject"], "phase/1");
        assert_eq!(payload["role"], "planner");
        assert_eq!(payload["attempt"], 1);
        assert_eq!(payload["request"], O);
        assert_eq!(payload["parent"], Value::Null);
        assert_eq!(payload["agent"], "bal-planner-high");
        let identities: Vec<&str> = payload["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["identity"].as_str().unwrap())
            .collect();
        assert_eq!(identities, ["bal-phase-planner"]);
        assert_eq!(
            pending_planner(&store, &project(), 1).unwrap().as_deref(),
            Some(O)
        );

        let before = head(&store);
        let busy = issue(&store, &command_o2, &o2);
        assert_eq!(busy["status"], "failed", "{busy}");
        assert_eq!(busy["code"], "ledger-busy", "{busy}");
        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_planner_retry_issued_beside_a_pending_planner_order_is_caught() {
        let (_dir, store) = store_c();
        let (command_o, o) = prepared(&store, &[1], O, R1, T4);
        assert_eq!(issue(&store, &command_o, &o)["status"], "ok");
        fail_by_hand(&store, O, R2);
        let (command_o2, o2) = prepared(&store, &[1], O2, R3, T6);
        assert_eq!(issue(&store, &command_o2, &o2)["status"], "ok");

        let before = head(&store);
        let refusal = crate::mcp::work_order::retry_check(&store, &project(), O)
            .expect_err("the retry is refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "planner-order-pending", "{refusal}");
        assert_eq!(refusal["slot"], "operation", "{refusal}");
        assert_eq!(refusal["details"], json!({"work_order": O2}));
        assert_eq!(head(&store), before);
    }
}
