//! The writer's decisions over values: whether a guarded write still holds
//! its precondition, what view a set of store files makes, and whether files
//! another writer left can replace the view this writer holds.

use super::model::{
    DECISIONS, Disposition, Evidence, ITEMS, ItemRecord, Origin, STATE, Snapshot, VERSION,
    render_lines,
};
use super::writer::{STALE_SNAPSHOT, View, later_generation, precondition, view};
use super::{Error, Observed};
use serde_json::json;
use std::collections::BTreeMap;

fn item(id: &str) -> ItemRecord {
    ItemRecord {
        version: VERSION,
        id: id.into(),
        revision: 1,
        origin: Origin { source: "capture".into(), original: Evidence::Missing },
        text: id.into(),
        kind: "todo".into(),
        phase: None,
        disposition: Disposition::Captured,
        completed: false,
        filing_uncertain: false,
    }
}

fn file(bytes: &[u8], directory: &str) -> Observed {
    Observed { bytes: Some(bytes.to_vec()), identity: "file".into(), directory_identity: directory.into() }
}

fn absent() -> Observed {
    Observed { bytes: None, identity: "missing".into(), directory_identity: "store".into() }
}

/// A store's three files at `generation` holding `items`, in `directory`.
fn store_in(generation: u64, items: &[ItemRecord], directory: &str) -> BTreeMap<String, Observed> {
    let items = render_lines(items).unwrap();
    let state = Snapshot::new(generation, &items, b"", json!({"generation": generation}))
        .unwrap()
        .render()
        .unwrap();
    BTreeMap::from([
        (ITEMS.into(), file(&items, directory)),
        (DECISIONS.into(), file(b"", directory)),
        (STATE.into(), file(&state, directory)),
    ])
}

fn store(generation: u64, items: &[ItemRecord]) -> BTreeMap<String, Observed> {
    store_in(generation, items, "store")
}

fn snapshot(data: serde_json::Value) -> Snapshot {
    Snapshot::new(4, b"", b"", data).unwrap()
}

#[test]
fn the_current_generation_and_integrity_meet_the_precondition() {
    let current = snapshot(json!({"answer": 1}));
    assert_eq!(precondition(&current, 4, &current.integrity.clone()), Ok(()));
}

#[test]
fn a_generation_moved_on_by_an_intervening_write_is_stale() {
    let current = snapshot(json!({"answer": 1}));
    assert_eq!(
        precondition(&current, 3, &current.integrity.clone()),
        Err(Error::Conflict(STALE_SNAPSHOT.into()))
    );
}

#[test]
fn an_integrity_changed_by_a_snapshot_rewrite_at_the_same_generation_is_stale() {
    let current = snapshot(json!({"answer": 1}));
    let expected = snapshot(json!({"answer": 2})).integrity;
    assert_ne!(expected, current.integrity);
    assert_eq!(precondition(&current, 4, &expected), Err(Error::Conflict(STALE_SNAPSHOT.into())));
}

#[test]
fn three_absent_files_are_a_new_store_at_generation_0() {
    let files = BTreeMap::from([(ITEMS.into(), absent()), (DECISIONS.into(), absent()), (STATE.into(), absent())]);
    let new = view(&files).unwrap();
    assert_eq!((new.snapshot.generation, new.items.len(), new.decisions.len()), (0, 0, 0));
}

#[test]
fn a_state_file_without_its_items_or_decisions_is_refused() {
    for missing in [ITEMS, DECISIONS] {
        let mut files = store(1, &[]);
        files.insert(missing.into(), absent());
        assert_eq!(
            view(&files).map(|_| ()),
            Err(Error::Conflict("owned generation lost a store file".into())),
            "{missing}"
        );
    }
}

#[test]
fn records_without_a_state_file_are_refused() {
    let mut files = store(1, &[item("a")]);
    files.insert(STATE.into(), absent());
    assert_eq!(view(&files).map(|_| ()), Err(Error::Conflict("owned records lack a snapshot".into())));
}

#[test]
fn a_state_file_that_does_not_match_its_items_is_refused() {
    let mut files = store(1, &[]);
    files.insert(ITEMS.into(), file(&render_lines(&[item("a")]).unwrap(), "store"));
    assert_eq!(
        view(&files).map(|_| ()),
        Err(Error::Conflict("snapshot integrity or version mismatch".into()))
    );
}

#[test]
fn the_view_holds_the_items_in_file_order_and_the_snapshot_generation() {
    let read = view(&store(2, &[item("a"), item("b")])).unwrap();
    assert_eq!(read.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(read.snapshot.data, json!({"generation": 2}));
}

fn read(files: &BTreeMap<String, Observed>) -> View {
    view(files).unwrap()
}

#[test]
fn a_later_generation_that_only_appends_replaces_the_view() {
    let before = store(1, &[item("a")]);
    let after = store(2, &[item("a"), item("b")]);
    assert_eq!(later_generation(&read(&before), &before, &read(&after), &after), Ok(()));
}

#[test]
fn files_that_do_not_move_the_generation_on_are_an_external_change() {
    let before = store(2, &[item("a")]);
    for generation in [1, 2] {
        let after = store(generation, &[item("a"), item("b")]);
        assert_eq!(
            later_generation(&read(&before), &before, &read(&after), &after),
            Err(Error::Conflict("externally changed store generation".into())),
            "{generation}"
        );
    }
}

#[test]
fn a_later_generation_that_drops_or_replaces_an_item_is_an_external_change() {
    let before = store(1, &[item("a"), item("b")]);
    for items in [vec![item("a")], vec![item("b"), item("a")]] {
        let after = store(2, &items);
        assert_eq!(
            later_generation(&read(&before), &before, &read(&after), &after),
            Err(Error::Conflict("externally changed store generation".into()))
        );
    }
}

#[test]
fn a_later_generation_in_a_replaced_store_directory_is_an_external_change() {
    let before = store(1, &[item("a")]);
    let after = store_in(2, &[item("a"), item("b")], "replaced");
    assert_eq!(
        later_generation(&read(&before), &before, &read(&after), &after),
        Err(Error::Conflict("externally changed store generation".into()))
    );
}

mod boundaries {
    use super::super::model::{Decision, DecisionRecord, digest};
    use super::super::writer::{BoundaryAdmission, Prior, boundary_admission, prior};
    use super::*;
    use cadence::execution::model::{BoundaryDecision, BoundaryTool};

    fn request(phase: u32, name: &str) -> BoundaryDecision {
        BoundaryDecision {
            phase,
            tool: BoundaryTool::CadenceApply,
            operation: "execution-refusal".into(),
            request_digest: digest(name.as_bytes()),
            outcome: "refused".into(),
            subject_id: None,
            prompt_digest: None,
            response_digest: digest(format!("answer to {name}").as_bytes()),
        }
    }

    /// A boundary decision already in the log, written by hand.
    fn logged(phase: u32, index: usize, terminal: bool) -> DecisionRecord {
        let id = digest(format!("logged {phase} {index}").as_bytes());
        DecisionRecord {
            version: VERSION,
            id: id.clone(),
            revision: 1,
            origin: Origin { source: "execution-boundary".into(), original: Evidence::Missing },
            decision: Decision::Boundary {
                phase,
                tool: "cadence-apply".into(),
                operation: "execution-refusal".into(),
                request_digest: id.clone(),
                outcome: if terminal { "log-bound" } else { "refused" }.into(),
                subject_id: None,
                store_generation: index as u64 + 1,
                prompt_digest: None,
                response_digest: id,
                terminal,
            },
            at: Some(1),
        }
    }

    fn log(phase: u32, count: usize) -> Vec<DecisionRecord> {
        (0..count).map(|index| logged(phase, index, false)).collect()
    }

    /// The boundary fields of an admitted record: phase, outcome, generation,
    /// whether it is terminal, and its time.
    fn fields(record: &DecisionRecord) -> (u32, &str, u64, bool, Option<u64>) {
        let Decision::Boundary { phase, outcome, store_generation, terminal, .. } = &record.decision else {
            panic!("not a boundary decision: {record:?}")
        };
        (*phase, outcome.as_str(), *store_generation, *terminal, record.at)
    }

    #[test]
    fn a_phase_under_its_limit_admits_the_decision_to_append_at_the_given_generation_and_time() {
        let BoundaryAdmission::Proceed(record) =
            boundary_admission(&log(3, 255), &request(3, "next"), 256, Some(7)).unwrap()
        else {
            panic!("not admitted")
        };
        assert_eq!(fields(&record), (3, "refused", 256, false, Some(7)));
    }

    #[test]
    fn the_decision_after_a_phase_uses_256_is_its_terminal_log_bound_decision() {
        let BoundaryAdmission::Terminal(record) =
            boundary_admission(&log(3, 256), &request(3, "next"), 257, Some(7)).unwrap()
        else {
            panic!("not terminal")
        };
        assert_eq!(fields(&record), (3, "log-bound", 257, true, Some(7)));
    }

    #[test]
    fn decisions_of_other_phases_do_not_count_toward_the_limit() {
        let mut decisions = log(3, 255);
        decisions.extend(log(4, 10));
        assert!(matches!(
            boundary_admission(&decisions, &request(3, "next"), 266, Some(7)).unwrap(),
            BoundaryAdmission::Proceed(_)
        ));
    }

    #[test]
    fn a_decision_already_in_the_log_replays_even_once_the_phase_is_at_its_limit() {
        let BoundaryAdmission::Proceed(first) =
            boundary_admission(&[], &request(3, "first"), 1, Some(7)).unwrap()
        else {
            panic!("not admitted")
        };
        for mut decisions in [vec![], log(3, 256)] {
            decisions.push(first.clone());
            assert_eq!(
                boundary_admission(&decisions, &request(3, "first"), 300, Some(8)).unwrap(),
                BoundaryAdmission::Replay
            );
        }
    }

    #[test]
    fn a_boundary_decision_for_phase_0_is_refused() {
        assert_eq!(
            boundary_admission(&[], &request(0, "zero"), 1, Some(7)),
            Err(Error::Invalid("boundary phase must be positive".into()))
        );
    }

    fn view_with(decisions: Vec<DecisionRecord>, operations: &[(&str, &str)]) -> View {
        let mut snapshot = Snapshot::new(1, b"", b"", serde_json::Value::Null).unwrap();
        snapshot.operations = operations.iter().map(|(id, fingerprint)| (id.to_string(), fingerprint.to_string())).collect();
        View { items: vec![], decisions, snapshot }
    }

    #[test]
    fn an_operation_identity_not_in_the_view_is_new() {
        let view = view_with(log(3, 2), &[("op-1", "content-1")]);
        assert_eq!(prior(&view, "op-2", "content-2", 3), Ok(Prior::New));
    }

    #[test]
    fn a_recorded_operation_identity_replays_its_own_content_and_is_reused_by_other_content() {
        let view = view_with(vec![], &[("op-1", "content-1")]);
        assert_eq!(prior(&view, "op-1", "content-1", 3), Ok(Prior::Replay));
        assert_eq!(prior(&view, "op-1", "content-2", 3), Ok(Prior::Reused));
    }

    #[test]
    fn once_a_phase_holds_its_terminal_decision_every_request_for_it_replays() {
        let view = view_with(vec![logged(3, 256, true)], &[("op-1", "content-1")]);
        assert_eq!(prior(&view, "op-new", "anything", 3), Ok(Prior::Replay));
        assert_eq!(prior(&view, "op-1", "content-2", 3), Ok(Prior::Replay));
        assert_eq!(prior(&view, "op-new", "anything", 4), Ok(Prior::New));
    }

    #[test]
    fn a_store_holding_a_boundary_decision_in_the_legacy_format_is_refused_for_execution() {
        use super::super::writer::require_current_execution;
        assert_eq!(
            require_current_execution(&view_with(vec![logged(3, 0, false)], &[])),
            Err(cadence::execution::boundary::Failure::LegacyExecution)
        );
    }

    #[test]
    fn a_blank_operation_identity_is_refused() {
        assert_eq!(
            prior(&view_with(vec![], &[]), " \t", "content", 3),
            Err(Error::Invalid("empty operation identity".into()))
        );
    }
}

mod scoped {
    use super::super::model::{BoundaryRecordV1, Decision, DecisionRecord, digest};
    use super::super::writer::{BoundaryChange, ScopedAdmission, scoped_admission};
    use super::*;
    use cadence::envelope::Envelope;
    use cadence::execution::boundary::{BoundaryScope, BoundaryV1, PreparedAnswer};
    use cadence::execution::model::BoundaryTool;

    const PHASE: BoundaryScope = BoundaryScope::Execution { phase: 3 };

    fn boundary(scope: BoundaryScope, operation: &str, name: &str) -> BoundaryV1 {
        let answer = PreparedAnswer::new(Envelope::Refused { code: "refused".into(), reason: name.into() }).unwrap();
        BoundaryV1::new(scope, BoundaryTool::CadenceApply, operation.into(), digest(name.as_bytes()), None, &answer)
    }

    /// A scoped decision already in the log, written by hand.
    fn logged(scope: BoundaryScope, operation: &str, index: usize) -> DecisionRecord {
        DecisionRecord {
            version: VERSION,
            id: digest(format!("logged {scope:?} {operation} {index}").as_bytes()),
            revision: 1,
            origin: Origin { source: "execution-boundary-v1".into(), original: Evidence::Missing },
            decision: Decision::BoundaryV1(BoundaryRecordV1 {
                boundary: boundary(scope, operation, &format!("logged {index}")),
                store_generation: index as u64 + 1,
                terminal: false,
            }),
            at: Some(1),
        }
    }

    fn log(scope: BoundaryScope, operation: &str, count: usize) -> Vec<DecisionRecord> {
        (0..count).map(|index| logged(scope.clone(), operation, index)).collect()
    }

    fn admit(decisions: &[DecisionRecord], decision: &BoundaryV1) -> ScopedAdmission {
        scoped_admission(decisions, "new-decision", decision, &BoundaryChange::Observe, 300, Some(9)).unwrap()
    }

    fn view_of(decisions: Vec<DecisionRecord>, data: serde_json::Value) -> View {
        View { items: vec![], decisions, snapshot: Snapshot::new(1, b"", b"", data).unwrap() }
    }

    /// The terminal log-bound decision of `scope`, written by hand.
    fn terminal(scope: BoundaryScope) -> DecisionRecord {
        let mut record = logged(scope, "log-bound", 256);
        let Decision::BoundaryV1(value) = &mut record.decision else { unreachable!() };
        value.terminal = true;
        record
    }

    #[test]
    fn a_scope_holding_its_terminal_decision_is_answered_by_it_and_no_other_scope_is() {
        use super::super::writer::terminal_v1;
        let mut decisions = log(PHASE, "executor", 256);
        decisions.push(terminal(PHASE));
        let id = decisions[256].id.clone();
        let view = view_of(decisions, serde_json::Value::Null);
        assert_eq!(terminal_v1(&view, &PHASE).map(|found| found.id), Some(id.as_str()));
        assert!(terminal_v1(&view, &BoundaryScope::Execution { phase: 4 }).is_none());
        assert!(terminal_v1(&view_of(log(PHASE, "executor", 256), serde_json::Value::Null), &PHASE).is_none());
    }

    #[test]
    fn an_execution_occurrence_with_no_scoped_decision_is_in_the_legacy_format() {
        use super::super::writer::require_current_execution;
        use cadence::execution::boundary::Failure;
        let data = json!({"execution": {"occurrences": {"3": {"active": null}}}});
        assert_eq!(require_current_execution(&view_of(vec![], data.clone())), Err(Failure::LegacyExecution));
        assert_eq!(require_current_execution(&view_of(log(BoundaryScope::Execution { phase: 4 }, "executor", 1), data.clone())), Err(Failure::LegacyExecution));
        assert_eq!(require_current_execution(&view_of(log(PHASE, "executor", 1), data)), Ok(()));
    }

    #[test]
    fn a_scope_under_its_limit_proceeds() {
        assert_eq!(admit(&log(PHASE, "executor", 255), &boundary(PHASE, "executor", "next")), ScopedAdmission::Proceed);
    }

    #[test]
    fn the_write_after_a_scope_uses_256_is_its_terminal_log_bound_decision() {
        let ScopedAdmission::Terminal(record) = admit(&log(PHASE, "executor", 256), &boundary(PHASE, "executor", "next"))
        else {
            panic!("not terminal")
        };
        let Decision::BoundaryV1(value) = &record.decision else { panic!("not scoped: {record:?}") };
        assert_eq!(
            (&value.boundary.scope, value.boundary.operation.as_str(), value.store_generation, value.terminal, record.at),
            (&PHASE, "log-bound", 300, true, Some(9))
        );
    }

    #[test]
    fn a_scope_counts_only_its_own_ordinary_decisions() {
        let mut decisions = log(PHASE, "executor", 255);
        decisions.extend(log(BoundaryScope::RootRefusal, "executor", 10));
        decisions.extend(log(BoundaryScope::Execution { phase: 4 }, "executor", 10));
        decisions.extend(log(PHASE, "native-refusal", 10));
        assert_eq!(admit(&decisions, &boundary(PHASE, "executor", "next")), ScopedAdmission::Proceed);
    }

    #[test]
    fn a_native_refusal_is_never_limited() {
        assert_eq!(
            admit(&log(PHASE, "executor", 256), &boundary(PHASE, "native-refusal", "next")),
            ScopedAdmission::Proceed
        );
    }

    #[test]
    fn a_logged_decision_under_another_operation_replays_an_observation_and_refuses_a_change() {
        let decisions = log(PHASE, "executor", 2);
        let decision = boundary(PHASE, "executor", "logged 1");
        let id = decisions[1].id.clone();
        assert_eq!(
            scoped_admission(&decisions, &id, &decision, &BoundaryChange::Observe, 3, Some(9)),
            Ok(ScopedAdmission::Replay)
        );
        let change = BoundaryChange::FinalizeRisk { phase: 3, requirements: vec![] };
        assert_eq!(
            scoped_admission(&decisions, &id, &decision, &change, 3, Some(9)),
            Err(Error::Conflict("boundary decision already admitted under another operation".into()))
        );
    }
}

mod writes {
    use super::super::model::{Decision, DecisionRecord};
    use super::super::transaction::{ExternalChange, Transaction};
    use super::super::writer::{sealed, transact};
    use super::*;

    fn routing(id: &str, observed_effort: Evidence) -> DecisionRecord {
        DecisionRecord {
            version: VERSION,
            id: id.into(),
            revision: 1,
            origin: Origin { source: "worker".into(), original: Evidence::Missing },
            decision: Decision::Routing {
                choice: "worker-a".into(),
                config_provenance: BTreeMap::new(),
                requested_effort: Evidence::Text("high".into()),
                observed_effort,
                receipt: Evidence::Missing,
            },
            at: Some(1),
        }
    }

    /// A view holding item `a` and decision `first`, with data `{"kept": true}`.
    fn current() -> View {
        View {
            items: vec![item("a")],
            decisions: vec![routing("first", Evidence::Missing)],
            snapshot: Snapshot::new(1, b"", b"", json!({"kept": true})).unwrap(),
        }
    }

    fn transaction(snapshot: Option<serde_json::Value>) -> Transaction {
        Transaction {
            id: "tx-1".into(),
            items: vec![item("b")],
            decisions: vec![routing("second", Evidence::Text(" \t".into()))],
            snapshot,
            external: vec![ExternalChange {
                target: "global-config".into(),
                expected: file(b"old\n", "config"),
                bytes: b"new\n".to_vec(),
            }],
        }
    }

    #[test]
    fn a_transaction_appends_its_items_and_decisions_after_the_existing_ones() {
        let mut next = current();
        transact(&mut next, &mut BTreeMap::new(), transaction(None), "fp".into()).unwrap();
        assert_eq!(next.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(next.decisions.iter().map(|record| record.id.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    }

    #[test]
    fn a_transaction_records_its_identity_with_its_fingerprint() {
        let mut operations = BTreeMap::from([("tx-0".to_string(), "older".to_string())]);
        transact(&mut current(), &mut operations, transaction(None), "fp".into()).unwrap();
        assert_eq!(
            operations,
            BTreeMap::from([("tx-0".to_string(), "older".to_string()), ("tx-1".to_string(), "fp".to_string())])
        );
    }

    #[test]
    fn a_transactions_decisions_are_normalized_as_they_are_appended() {
        let mut next = current();
        transact(&mut next, &mut BTreeMap::new(), transaction(None), "fp".into()).unwrap();
        assert_eq!(next.decisions[1], routing("second", Evidence::Missing));
    }

    #[test]
    fn snapshot_data_in_a_transaction_replaces_the_data_and_its_absence_keeps_it() {
        let mut replaced = current();
        transact(&mut replaced, &mut BTreeMap::new(), transaction(Some(json!({"new": 1}))), "fp".into()).unwrap();
        assert_eq!(replaced.snapshot.data, json!({"new": 1}));
        let mut kept = current();
        transact(&mut kept, &mut BTreeMap::new(), transaction(None), "fp".into()).unwrap();
        assert_eq!(kept.snapshot.data, json!({"kept": true}));
    }

    #[test]
    fn a_transaction_answers_the_external_participants_it_brings() {
        let external = transact(&mut current(), &mut BTreeMap::new(), transaction(None), "fp".into()).unwrap();
        assert_eq!(external.iter().map(|change| change.target.as_str()).collect::<Vec<_>>(), ["global-config"]);
    }

    #[test]
    fn a_transaction_whose_item_skips_a_revision_is_refused() {
        let mut skipped = transaction(None);
        skipped.items[0].id = "a".into();
        skipped.items[0].revision = 3;
        assert_eq!(
            transact(&mut current(), &mut BTreeMap::new(), skipped, "fp".into()).map(|_| ()),
            Err(Error::Invalid("unsupported version, empty identity, or inconsistent revision".into()))
        );
    }

    fn observed() -> BTreeMap<String, Observed> {
        [ITEMS, DECISIONS, STATE].into_iter().map(|name| (name.to_string(), file(name.as_bytes(), "store"))).collect()
    }

    fn own() -> Vec<super::super::transaction::Participant> {
        vec![ExternalChange { target: "phase-summary:3".into(), expected: absent(), bytes: b"# Summary\n".to_vec() }]
    }

    #[test]
    fn a_sealed_write_is_at_the_given_generation_with_its_operations_and_data() {
        let operations = BTreeMap::from([("tx-1".to_string(), "fp".to_string())]);
        let (next, _) = sealed(current(), operations.clone(), own(), 5, &observed()).unwrap();
        assert_eq!(
            (next.snapshot.generation, &next.snapshot.operations, &next.snapshot.data),
            (5, &operations, &json!({"kept": true}))
        );
    }

    #[test]
    fn the_store_files_follow_the_writes_own_participants_with_the_state_last_each_expected_as_observed() {
        let (_, participants) = sealed(current(), BTreeMap::new(), own(), 5, &observed()).unwrap();
        assert_eq!(
            participants.iter().map(|p| p.target.as_str()).collect::<Vec<_>>(),
            ["phase-summary:3", ITEMS, DECISIONS, STATE]
        );
        for participant in &participants[1..] {
            assert_eq!(participant.expected, observed()[&participant.target], "{}", participant.target);
        }
    }

    #[test]
    fn the_sealed_files_read_back_as_the_sealed_view() {
        let (next, participants) = sealed(current(), BTreeMap::new(), vec![], 5, &observed()).unwrap();
        let files = participants
            .iter()
            .map(|p| (p.target.clone(), file(&p.bytes, "store")))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(view(&files).unwrap(), next);
    }
}

mod checked {
    use super::super::writer::CheckedPolicy;
    use super::super::{MutationContext, Policy, Result};
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A store policy that records each call and allows it.
    struct Recording(Arc<Mutex<usize>>);
    impl Policy for Recording {
        fn validate(&mut self, _: &MutationContext<'_>) -> Result<()> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn checked(check: Result<()>) -> (CheckedPolicy<Recording>, Arc<Mutex<usize>>) {
        let calls = Arc::new(Mutex::new(0));
        let check = Box::new(move || check.clone());
        (CheckedPolicy { policy: Recording(calls.clone()), check: Some(check) }, calls)
    }

    fn validate(policy: &mut CheckedPolicy<Recording>) -> Result<()> {
        let snapshot = Snapshot::new(1, b"", b"", serde_json::Value::Null).unwrap();
        policy.validate(&MutationContext { operation: "store", snapshot: &snapshot })
    }

    #[test]
    fn a_failing_input_check_refuses_before_the_store_policy_runs() {
        let changed = Error::Conflict("interview config inputs changed".into());
        let (mut policy, calls) = checked(Err(changed.clone()));
        assert_eq!(validate(&mut policy), Err(changed));
        assert_eq!(*calls.lock().unwrap(), 0);
    }

    #[test]
    fn a_passing_input_check_hands_every_validation_to_the_store_policy() {
        let (mut policy, calls) = checked(Ok(()));
        validate(&mut policy).unwrap();
        validate(&mut policy).unwrap();
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[test]
    fn a_failing_input_check_refuses_a_routing_admission_too() {
        let changed = Error::Conflict("interview config inputs changed".into());
        let (mut policy, calls) = checked(Err(changed.clone()));
        let snapshot = Snapshot::new(1, b"", b"", serde_json::Value::Null).unwrap();
        let inputs = cadence::execution::model::ConfigInputs {
            repo: cadence::execution::model::ConfigInput { identity: "/project/config.v4.json".into(), content: None, stamp: None },
            global: None,
            global_alias: false,
        };
        assert_eq!(
            policy.validate_routing_admission(&MutationContext { operation: "store", snapshot: &snapshot }, &inputs),
            Err(changed)
        );
        assert_eq!(*calls.lock().unwrap(), 0);
    }
}
