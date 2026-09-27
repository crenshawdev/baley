//! EVD-R10: replay from events, generation isolation and view verification.
use super::fixture::*;
use crate::*;
use serde_json::json;

/// Projects the fixture chain. Catches folds that ignore existing documents.
pub fn live_projection_equals_the_hand_written_documents<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    assert_eq!(read_back(&store), expected());
}
/// Rebuilds beside a damaged live document. Catches replay copying live rows.
pub fn a_rebuild_ignores_a_corrupted_live_document<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::AlterDocument {
                view: "tally".into(),
                key: id(1),
                body: json!({"id":1,"seen":99}),
            },
        )
        .unwrap();
    store.rebuild(&project()).unwrap();
    assert_eq!(read_back(&store), expected());
}
/// Commits between replay and the final turn. Catches missing tail catch-up.
pub fn a_command_during_a_rebuild_reaches_the_new_generation<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let report = factory
        .rebuild_between(&store, &project(), &mut || {
            record(&store, "tail", &[(3, "open")]).unwrap();
        })
        .unwrap();
    assert_eq!(report.events, 7);
    assert_eq!(
        doc(&store, "item", &id(3)),
        Some((json!({"id":3,"state":"open","rank":0,"owner":""}), 6))
    );
    assert_eq!(
        doc(&store, "tally", &id(3)),
        Some((json!({"id":3,"seen":1}), 6))
    );
    assert_eq!(read_back(&store), expected());
}
/// Fails projection on a newly committed tail. Catches a flip before the tail succeeds.
pub fn a_projector_failing_on_the_tail_leaves_the_old_generation_live<F: StoreFactory>(
    factory: &F,
) {
    let store = created(factory);
    fixture(&store);
    let failing = factory.reopen(&store, failing()).unwrap();
    let result = factory.rebuild_between(&failing, &project(), &mut || {
        record(&store, "tail", &[(TAIL, "open")]).unwrap();
    });
    assert!(matches!(result,Err(StoreError::Projector {view,seq:6,..}) if view == "tally"));
    assert_eq!(read_back(&store), expected());
    assert_eq!(
        doc(&store, "item", &id(TAIL)),
        Some((json!({"id":TAIL,"state":"open","rank":0,"owner":""}), 6))
    );
    assert_eq!(
        doc(&store, "tally", &id(TAIL)),
        Some((json!({"id":TAIL,"seen":1}), 6))
    );
    assert!(
        store
            .get(&project(), "request", &request("fixture.add", "tail"))
            .unwrap()
            .is_some()
    );
}
/// Abandons after one event of five. Catches writes into the live generation during replay.
pub fn an_abandoned_rebuild_changes_nothing_live<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    assert_eq!(factory.crash_rebuild(&store, &project(), 1).unwrap(), 1);
    assert_eq!(read_back(&store), expected());
}
/// Rebuilds after an abandoned batch. Catches orphan generations preventing replay.
pub fn a_rebuild_after_an_abandoned_one_matches_the_hand_written_documents<F: StoreFactory>(
    factory: &F,
) {
    let store = created(factory);
    fixture(&store);
    assert_eq!(factory.crash_rebuild(&store, &project(), 1).unwrap(), 1);
    store.rebuild(&project()).unwrap();
    assert_eq!(read_back(&store), expected());
    assert_eq!(
        store.verify_views(&project()).unwrap(),
        ViewsReport {
            checked_seq: 5,
            differing: vec![]
        }
    );
}
fn protect<F: StoreFactory>(factory: &F, verify: bool) {
    let store = created(factory);
    fixture(&store);
    let generation = store.rebuild(&project()).unwrap().generation;
    factory
        .corrupt(&store, &project(), Corruption::MarkLiveGenerationBuilding)
        .unwrap();
    for _ in 0..2 {
        let result = if verify {
            store.verify_views(&project()).map(|_| ())
        } else {
            store.rebuild(&project()).map(|_| ())
        };
        assert_eq!(
            result,
            Err(StoreError::LiveGenerationProtected {
                project: project(),
                generation
            })
        );
        assert_eq!(read_back(&store), expected());
    }
}
/// Rebuilds with a marker naming live rows. Catches cleanup deleting the live generation.
pub fn a_building_marker_on_the_live_generation_stops_a_rebuild<F: StoreFactory>(factory: &F) {
    protect(factory, false);
}
/// Verifies with a marker naming live rows. Catches live rows classified as abandoned work.
pub fn a_building_marker_on_the_live_generation_stops_view_verification<F: StoreFactory>(
    factory: &F,
) {
    protect(factory, true);
}
/// Replays as a binary that cannot read the events. Catches skipped unreadable events.
pub fn an_unreadable_event_stops_a_rebuild_with_nothing_live_changed<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let blind = factory.reopen(&store, blind()).unwrap();
    assert!(
        matches!(blind.rebuild(&project()),Err(StoreError::Refused(Refusal::ProjectReadOnly {project:p,..})) if p==project())
    );
    assert_eq!(read_back(&store), expected());
}
/// Projects and replays a v1 event while retaining its stored form. Catches missing or persisted upcasts.
pub fn an_old_event_replays_upcast_and_stays_as_stored<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    store
        .transact(&command("fixture.add", "old"), &mut |tx| {
            tx.append(event(1, json!({"id":1,"status":"open"})))?;
            Ok(done(json!("ok"), false))
        })
        .unwrap();
    let before = history(&store);
    assert_eq!(before[0].type_version, 1);
    assert_eq!(before[0].payload, json!({"id":1,"status":"open"}));
    let expected = Some((json!({"id":1,"state":"open","rank":0,"owner":""}), 1));
    assert_eq!(doc(&store, "item", &id(1)), expected);
    store.rebuild(&project()).unwrap();
    assert_eq!(doc(&store, "item", &id(1)), expected);
    assert_eq!(history(&store), before);
}
/// Verifies a damaged document against replay at one head. Catches comparisons against live input.
pub fn view_verification_names_a_corrupted_live_document<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::AlterDocument {
                view: "tally".into(),
                key: id(1),
                body: json!({"id":1,"seen":99}),
            },
        )
        .unwrap();
    assert_eq!(
        store.verify_views(&project()).unwrap(),
        ViewsReport {
            checked_seq: 5,
            differing: vec![("tally".into(), id(1))]
        }
    );
}
/// Fails scratch replay then verifies with a working projector. Catches unfinished markers left by ordinary errors.
pub fn a_projector_error_in_view_verification_leaves_no_unfinished_generation<F: StoreFactory>(
    factory: &F,
) {
    let store = created(factory);
    record(&store, "tail", &[(TAIL, "open")]).unwrap();
    let failing = factory.reopen(&store, failing()).unwrap();
    assert!(
        matches!(failing.verify_views(&project()),Err(StoreError::Projector {view,seq:1,..}) if view=="tally")
    );
    assert_eq!(
        store.verify_views(&project()).unwrap(),
        ViewsReport {
            checked_seq: 2,
            differing: vec![]
        }
    );
}
/// Verifies beside abandoned work twice. Catches verification consuming an unfinished generation.
pub fn view_verification_refuses_while_a_generation_is_unfinished<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory.crash_rebuild(&store, &project(), 1).unwrap();
    let first = store.verify_views(&project());
    assert!(matches!(&first,Err(StoreError::UnfinishedGeneration {project:p,..}) if *p==project()));
    assert_eq!(store.verify_views(&project()), first);
}
/// Reuses a cursor after a generation flip. Catches cursor translation across generations.
pub fn a_cursor_issued_before_a_flip_is_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(2);
    q.page.after = store.find(&project(), "item", &q).unwrap().next;
    assert!(q.page.after.is_some());
    store.rebuild(&project()).unwrap();
    assert_eq!(
        store.find(&project(), "item", &q),
        Err(StoreError::Refused(Refusal::InvalidCursor))
    );
}

/// Rebuilds after reducing an attachment. Catches replay that needs the original body.
pub fn a_rebuild_after_a_reduction_matches_the_live_views<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let body = attach(&store, &command("fixture.add", "body"), &body_bytes());
    store
        .reduce(
            &command("payload.reduce", "reduce"),
            &PayloadReference {
                project: project(),
                seq: 6,
                hash: body.hash,
            },
        )
        .unwrap();
    let before = read_back(&store);
    let request_before = store
        .get(&project(), "request", &request("payload.reduce", "reduce"))
        .unwrap();
    store.rebuild(&project()).unwrap();
    assert_eq!(read_back(&store), before);
    assert_eq!(
        store
            .get(&project(), "request", &request("payload.reduce", "reduce"))
            .unwrap(),
        request_before
    );
    assert!(store.verify_views(&project()).unwrap().differing.is_empty());
}
/// Rebuilds after purging a stored answer. Catches replay dropping the answer reference.
pub fn a_rebuild_after_a_purge_matches_the_live_views<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let body = super::payloads::stored_answer(&store);
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    let before = read_back(&store);
    let request_before = Some(Document {
        key: request("fixture.answer", "answer"),
        produced_seq: 6,
        projector_version: 2,
        body: json!({"kind":"fixture.answer","request_id":"answer","digest":"01".repeat(32),
            "state":"completed","scope":[],"outcome":"done","answer":{"stored":{
                "payload":body.hash.to_hex(),"bytes":11,"class":"record"}}}),
    });
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.answer", "answer"))
            .unwrap(),
        request_before
    );
    store.rebuild(&project()).unwrap();
    assert_eq!(read_back(&store), before);
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.answer", "answer"))
            .unwrap(),
        request_before
    );
    assert!(store.verify_views(&project()).unwrap().differing.is_empty());
}
