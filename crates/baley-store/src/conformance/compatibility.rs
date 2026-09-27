//! EVD-R19: epoch fences and forward-only view compatibility.
use super::fixture::*;
use crate::*;
use serde_json::json;

fn read_only<T: std::fmt::Debug>(result: Result<T, StoreError>) {
    assert!(
        matches!(result,Err(StoreError::Refused(Refusal::ProjectReadOnly {project:p,..})) if p == project())
    );
}
fn fenced_command(store: &impl Ledger) {
    let before = history(store);
    read_only(store.transact(&command("fixture.add", "fenced"), &mut |_| {
        panic!("fenced decision ran")
    }));
    assert_eq!(history(store), before);
}
/// Opens a newer epoch for reading. Catches open failure or writes to an unknown epoch.
pub fn a_store_stamped_with_a_newer_epoch_opens_read_only<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let before = history(&store);
    let epoch = factory.stamp_newer_epoch(&store).unwrap();
    let newer = factory.reopen(&store, binary()).unwrap();
    assert_eq!(history(&newer), before);
    assert_eq!(
        newer.head(&project()).unwrap(),
        store.head(&project()).unwrap()
    );
    assert_eq!(
        newer.transact(&command("fixture.add", "fenced"), &mut |_| panic!(
            "epoch fence skipped"
        )),
        Err(StoreError::ReadOnly {
            needed_epoch: epoch
        })
    );
    assert_eq!(history(&newer), before);
}
/// Raises the epoch beside an open connection. Catches a fence checked only at open.
pub fn a_newer_epoch_fences_an_open_connection_at_its_next_write<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let before = history(&store);
    let epoch = factory.stamp_newer_epoch(&store).unwrap();
    assert_eq!(
        store.transact(&command("fixture.add", "fenced"), &mut |_| panic!(
            "epoch fence skipped"
        )),
        Err(StoreError::ReadOnly {
            needed_epoch: epoch
        })
    );
    assert_eq!(history(&store), before);
}
/// Reads after a newer projector flips. Catches absence returned from an obsolete view table.
pub fn a_newer_live_view_refuses_an_older_binarys_reads<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let newer = factory.reopen(&store, newer_views()).unwrap();
    newer.rebuild(&project()).unwrap();
    read_only(store.get(&project(), "item", &id(1)));
    read_only(store.find(&project(), "item", &query(2)));
}
/// Writes after a newer projector flips. Catches stale projector writes.
pub fn a_newer_live_view_refuses_an_older_binarys_commands<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory
        .reopen(&store, newer_views())
        .unwrap()
        .rebuild(&project())
        .unwrap();
    fenced_command(&store);
}
/// Reads after an added view flips. Catches fences that compare only individual versions.
pub fn a_newer_view_set_refuses_an_older_binarys_reads<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory
        .reopen(&store, newer_set())
        .unwrap()
        .rebuild(&project())
        .unwrap();
    read_only(store.get(&project(), "item", &id(1)));
    read_only(store.find(&project(), "item", &query(2)));
}
/// Writes after an added view flips. Catches commands omitting a newer view.
pub fn a_newer_view_set_refuses_an_older_binarys_commands<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    factory
        .reopen(&store, newer_set())
        .unwrap()
        .rebuild(&project())
        .unwrap();
    fenced_command(&store);
}
/// Attempts backward rebuild and verification. Catches replacement of newer documents.
pub fn an_older_binary_never_rebuilds_a_newer_view<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let newer = factory.reopen(&store, newer_views()).unwrap();
    newer.rebuild(&project()).unwrap();
    let before = read_back(&newer);
    read_only(store.rebuild(&project()));
    read_only(store.verify_views(&project()));
    assert_eq!(read_back(&newer), before);
}
/// First reads an older generation as a newer binary. Catches a missing forward rebuild.
pub fn an_older_view_version_rebuilds_forward_before_use<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let newer = factory.reopen(&store, newer_views()).unwrap();
    assert_eq!(
        newer.get(&project(), "item", &id(1)).unwrap(),
        Some(Document {
            key: id(1),
            produced_seq: 4,
            projector_version: 3,
            body: json!({"id":1,"state":"done","rank":0,"owner":"","new_field":true})
        })
    );
}
/// Removes a view under a higher set version. Catches permanent fencing of retired views.
pub fn a_removed_view_rebuilds_forward_before_use<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let removed = factory.reopen(&store, removed_view()).unwrap();
    assert_eq!(
        doc(&removed, "item", &id(1)),
        Some((json!({"id":1,"state":"done","rank":0,"owner":""}), 4))
    );
    assert!(matches!(
        record(&removed, "next", &[(3, "open")]).unwrap(),
        Recorded::New { .. }
    ));
    read_only(store.get(&project(), "item", &id(1)));
}
/// Changes a set without raising its version. Catches catalog identity trusted by version alone.
pub fn a_view_set_changed_without_a_new_version_is_refused_at_open<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    assert!(
        matches!(factory.reopen(&store,changed_set()),Err(StoreError::Refused(Refusal::MalformedKey {view,..})) if view == "quiet")
    );
}
