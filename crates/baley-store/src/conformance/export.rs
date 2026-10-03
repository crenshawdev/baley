//! EVD-R15: standalone exports and purge reports naming unreachable copies.
use super::fixture::*;
use crate::*;
use serde_json::json;

/// Opens an exported project independently. Catches incomplete exports and another project's payload leaking.
pub fn an_export_verifies_alone_and_holds_no_other_project<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let own = attach(
        &store,
        &command("fixture.add", "own"),
        b"exported project body",
    );
    let mut cmd = command("fixture.add", "other");
    cmd.project = other_project();
    let other = attach(&store, &cmd, b"other project only");
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    let exported = factory.open_export(&target, binary()).unwrap();
    assert_eq!(
        exported.projects().unwrap(),
        vec![(project(), "fixture".into())]
    );
    assert_eq!(history(&exported), history(&store));
    let report = exported.verify(&project(), None).unwrap();
    assert!(report.chain.is_intact());
    assert!(report.payloads.is_empty());
    assert_eq!(report.bodies_checked, 1);
    assert_eq!(
        super::payloads::bytes(&exported, &own.hash),
        b"exported project body"
    );
    assert_eq!(
        exported.status(&other.hash),
        Err(StoreError::Refused(Refusal::UnknownPayload(other.hash)))
    );
}
/// Exports a released body still shared elsewhere. Catches source availability leaking into the export.
pub fn an_export_tombstones_a_body_its_project_released<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"shared");
    let mut cmd = command("fixture.add", "other");
    cmd.project = other_project();
    attach(&store, &cmd, b"shared");
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    let exported = factory.open_export(&target, binary()).unwrap();
    assert!(matches!(
        exported.status(&body.hash),
        Ok(PayloadStatus::Purged { .. })
    ));
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Present { bytes: 6 })
    );
    let report = exported.verify(&project(), None).unwrap();
    assert!(report.chain.is_intact());
    assert!(report.payloads.is_empty());
    assert_eq!(report.tombstones_checked, 1);
}
/// Compares the export report with both stores. Catches a report naming an unverified head.
pub fn an_export_reports_its_verified_head<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let target = factory.export_target();
    let report = store.export(&project(), &target, T0).unwrap();
    let exported = factory.open_export(&target, binary()).unwrap();
    assert_eq!(report.target, target);
    assert_eq!(report.head, store.head(&project()).unwrap());
    assert_eq!(report.head, exported.head(&project()).unwrap());
}
/// Purges after an export of its project. Catches omitted unreachable copies.
pub fn a_purge_lists_an_earlier_export_of_its_project<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"abc");
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    assert_eq!(
        store
            .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
            .unwrap()
            .unreachable,
        vec![target]
    );
}
/// Exports only after releasing a reference. Catches reports listing copies that never received the body.
pub fn a_purge_skips_an_export_made_after_the_release<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"shared");
    let mut cmd = command("fixture.add", "other");
    cmd.project = other_project();
    attach(&store, &cmd, b"shared");
    store
        .purge(&command("payload.purge", "release"), &[body.hash], "remove")
        .unwrap();
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    let mut purge = command("payload.purge", "purge");
    purge.project = other_project();
    assert!(
        store
            .purge(&purge, &[body.hash], "remove")
            .unwrap()
            .unreachable
            .is_empty()
    );
}
/// Purges a body copied through another project. Catches export lookup limited to the purging project.
pub fn a_shared_purge_lists_another_projects_export<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"shared");
    let mut cmd = command("fixture.add", "other");
    cmd.project = other_project();
    attach(&store, &cmd, b"shared");
    let target = factory.export_target();
    store.export(&other_project(), &target, T0).unwrap();
    let report = store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    assert_eq!(report.shared, vec![body.hash]);
    assert_eq!(report.unreachable, vec![target]);
}
/// Retries a purge after exporting its body. Catches replay reports dropping unreachable targets.
pub fn a_replayed_purge_lists_the_same_exports<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"abc");
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    let cmd = command("payload.purge", "purge");
    let first = store.purge(&cmd, &[body.hash], "remove").unwrap();
    let before = history(&store);
    let replay = store.purge(&cmd, &[body.hash], "remove").unwrap();
    assert_eq!(first.unreachable, vec![target]);
    assert_eq!(replay, first);
    assert_eq!(history(&store), before);
}
/// Exports a chain holding server, hook and no callers. Catches an export that drops or nulls callers, which would otherwise show only as an unverified export.
pub fn an_export_keeps_every_events_caller<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    for (request, caller) in [
        ("by-server", Some(server_caller())),
        ("by-hook", Some(hook_caller())),
        ("by-none", None),
    ] {
        let mut cmd = command("fixture.add", request);
        cmd.caller = caller;
        store
            .transact(&cmd, &mut |tx| {
                tx.append(event(
                    2,
                    json!({"id": 1, "state": request, "rank": 0, "owner": ""}),
                ))?;
                Ok(done(json!("ok"), false))
            })
            .unwrap();
    }
    let source: Vec<_> = history(&store).into_iter().map(|e| e.caller).collect();
    assert!(source.contains(&Some(server_caller())));
    assert!(source.contains(&Some(hook_caller())));
    assert!(source.contains(&None));
    let target = factory.export_target();
    store.export(&project(), &target, T0).unwrap();
    let exported = factory.open_export(&target, binary()).unwrap();
    let kept: Vec<_> = history(&exported).into_iter().map(|e| e.caller).collect();
    assert_eq!(kept, source);
    assert!(exported.verify(&project(), None).unwrap().chain.is_intact());
}
