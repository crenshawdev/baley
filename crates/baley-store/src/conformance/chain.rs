//! EVD-R2, R3: immutable history, damage positions and external anchors.
use super::fixture::*;
use crate::*;
use serde_json::json;

fn sound<F: StoreFactory>(factory: &F) -> F::Store {
    let store = created(factory);
    fixture(&store);
    store
}
/// Verifies sound anchored work. Catches verifiers that always report damage.
pub fn an_untouched_chain_verifies_against_its_anchor<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let report = store.verify(&project(), Some(&anchor(&store))).unwrap();
    assert!(report.chain.is_intact());
    assert_eq!(report.chain.first_break, None);
    assert_eq!(report.chain.anchor, AnchorVerdict::Matches);
    assert_eq!(report.chain.head, store.head(&project()).unwrap());
    assert_eq!(report.chain.unanchored, None);
    assert!(report.payloads.is_empty());
}
/// Edits a payload without its hash. Catches missed or misplaced payload damage.
pub fn an_altered_payload_is_named_at_its_sequence<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::AlterPayload {
                seq: 3,
                payload: json!({"id":2,"state":"edited","rank":0,"owner":""}),
            },
        )
        .unwrap();
    assert!(matches!(
        store.verify(&project(), None).unwrap().chain.first_break,
        Some(Break {
            seq: 3,
            kind: BreakKind::Hash { .. }
        })
    ));
}
/// Inserts a valid row before an existing event. Catches a broken predecessor accepted after insertion.
pub fn an_inserted_event_is_named_at_the_event_after_it<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let mut event = history(&store)[2].clone();
    event.stream = "inserted".into();
    event.stream_version = 1;
    event.payload = json!({"id":8,"state":"open","rank":0,"owner":""});
    event.hash = event.compute_hash().unwrap();
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::Insert {
                at: 3,
                event: Box::new(event),
            },
        )
        .unwrap();
    assert!(matches!(
        store.verify(&project(), None).unwrap().chain.first_break,
        Some(Break {
            seq: 4,
            kind: BreakKind::PrevHash { .. }
        })
    ));
}
/// Deletes a middle row. Catches accepted sequence gaps.
pub fn a_deleted_event_is_named_at_its_sequence<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    factory
        .corrupt(&store, &project(), Corruption::Delete { seq: 3 })
        .unwrap();
    assert_eq!(
        store.verify(&project(), None).unwrap().chain.first_break,
        Some(Break {
            seq: 3,
            kind: BreakKind::Sequence { found: 4 }
        })
    );
}
/// Swaps two stored positions. Catches reads ordered by insertion or hash.
pub fn reordered_events_are_named_at_the_first_moved_sequence<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::Reorder {
                first: 3,
                second: 4,
            },
        )
        .unwrap();
    assert!(matches!(
        store.verify(&project(), None).unwrap().chain.first_break,
        Some(Break {
            seq: 3,
            kind: BreakKind::PrevHash { .. }
        })
    ));
}
/// Recomputes a locally consistent edited chain. Catches trusting local hashes over an anchor.
pub fn a_chain_recomputed_after_an_edit_is_a_rewrite_of_its_anchor<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let remote = anchor(&store);
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::RecomputeAfterEdit {
                seq: 3,
                payload: json!({"id":2,"state":"edited","rank":0,"owner":""}),
            },
        )
        .unwrap();
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Rewritten {
            seq: 5,
            anchored: remote.hash,
            found: store.head(&project()).unwrap().unwrap().hash
        }
    );
}
/// Appends work after an anchor. Catches incorrect unanchored bounds.
pub fn work_after_the_anchor_is_reported_as_the_unanchored_range<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let remote = anchor(&store);
    record(&store, "later", &[(3, "open")]).unwrap();
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.anchor, AnchorVerdict::Matches);
    assert_eq!(report.chain.first_break, None);
    assert_eq!(report.chain.unanchored, Some(6..=7));
}
/// Truncates behind an anchor. Catches a short chain reported as merely unanchored.
pub fn a_truncated_tail_is_a_truncation_of_its_anchor<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let remote = anchor(&store);
    factory
        .corrupt(&store, &project(), Corruption::Truncate { keep_through: 2 })
        .unwrap();
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Truncated {
            anchored: 5,
            head: 2
        }
    );
}
fn restored<F: StoreFactory>(factory: &F) -> (F::Store, Anchor) {
    let store = created(factory);
    record(&store, "r1", &[(1, "open")]).unwrap();
    let snapshot = factory.snapshot(&store).unwrap();
    record(&store, "r2", &[(2, "open"), (1, "done")]).unwrap();
    let remote = anchor(&store);
    (factory.restore(&snapshot, binary()).unwrap(), remote)
}
/// Opens a copy older than the anchor. Catches undetected database rollback.
pub fn a_restored_older_copy_is_a_truncation_of_its_anchor<F: StoreFactory>(factory: &F) {
    let (store, remote) = restored(factory);
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Truncated {
            anchored: 5,
            head: 2
        }
    );
}
/// Grows a restored copy beyond the anchor. Catches rollback accepted after regrowth.
pub fn a_regrown_older_copy_is_a_rewrite_of_its_anchor<F: StoreFactory>(factory: &F) {
    let (store, remote) = restored(factory);
    record(&store, "changed", &[(7, "open"), (8, "open"), (9, "open")]).unwrap();
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Rewritten {
            seq: 5,
            anchored: remote.hash,
            found: history(&store)[4].hash
        }
    );
}
/// Records the owner's acknowledgement through a transaction. Catches lost attribution, payload or acknowledgement.
pub fn an_owner_acknowledged_restore_is_reported_as_acknowledged<F: StoreFactory>(factory: &F) {
    let (store, remote) = restored(factory);
    let head = store.head(&project()).unwrap();
    let local = head.clone().unwrap();
    let payload = json!({"remote":"origin","tag":anchor_tag(&project(),5),"seq":5,"head":remote.hash.to_hex(),"restored_seq":2,"restored_head":local.hash.to_hex(),"checked_at":T0});
    store
        .transact(&command(ANCHOR_ACKNOWLEDGE_RESTORE, "ack"), &mut |tx| {
            tx.append(NewEvent {
                stream: StreamName("project".into()),
                type_name: ANCHOR_RESTORE_ACKNOWLEDGED.into(),
                type_version: 1,
                git: None,
                payload: payload.clone(),
                attachments: vec![],
            })?;
            Ok(done(json!("ok"), false))
        })
        .unwrap();
    let events = history(&store);
    assert_eq!(events[2].actor, Actor::Owner);
    assert_eq!(events[2].payload, payload);
    assert_eq!(events[2].type_version, 1);
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Acknowledged {
            anchored: 5,
            restored: head.clone(),
            acknowledged_seq: 3
        }
    );
    assert_eq!(
        report.chain.acknowledged_restores,
        vec![AcknowledgedRestore {
            seq: 3,
            anchor: remote,
            restored: head
        }]
    );
}
/// Corrupts validly encoded body bytes. Catches verification that never hashes the body.
pub fn a_body_corrupted_in_place_is_corrupt_while_the_chain_holds<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"abc");
    factory
        .corrupt(&store, &project(), Corruption::CorruptBody(body.hash))
        .unwrap();
    let report = store.verify(&project(), None).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(report.payloads, vec![PayloadFault::Corrupt(body.hash)]);
    assert_eq!(report.bodies_checked, 1);
}
/// Verifies a purged body. Catches tombstones treated as chain damage.
pub fn a_purged_body_is_a_tombstone_not_a_fault<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"abc");
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    let report = store.verify(&project(), None).unwrap();
    assert!(report.chain.is_intact());
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Purged {
            reason: "remove".into()
        })
    );
    assert!(report.payloads.is_empty());
    assert_eq!(report.tombstones_checked, 1);
    assert_eq!(report.bodies_checked, 0);
}
fn reduced<F: StoreFactory>(factory: &F) -> (F::Store, PayloadRef) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), &body_bytes());
    let excerpt = store
        .reduce(
            &command("payload.reduce", "reduce"),
            &PayloadReference {
                project: project(),
                seq: 1,
                hash: body.hash,
            },
        )
        .unwrap();
    (store, excerpt)
}
/// Verifies a reduced body's retained bytes. Catches excerpts skipped with originals.
pub fn a_reduced_bodys_excerpt_is_hashed<F: StoreFactory>(factory: &F) {
    let (store, _) = reduced(factory);
    let report = store.verify(&project(), None).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert!(report.payloads.is_empty());
    assert_eq!(report.tombstones_checked, 1);
    assert_eq!(report.bodies_checked, 1);
}
/// Corrupts an excerpt beside a valid tombstone. Catches excerpt corruption hidden by reduction.
pub fn a_corrupt_excerpt_faults_beside_its_tombstone<F: StoreFactory>(factory: &F) {
    let (store, excerpt) = reduced(factory);
    factory
        .corrupt(&store, &project(), Corruption::CorruptBody(excerpt.hash))
        .unwrap();
    let report = store.verify(&project(), None).unwrap();
    assert_eq!(report.chain.first_break, None);
    assert_eq!(report.payloads, vec![PayloadFault::Corrupt(excerpt.hash)]);
    assert_eq!(report.tombstones_checked, 1);
    assert_eq!(report.bodies_checked, 1);
}
fn local_anchor(store: &impl Ledger) -> Anchor {
    let anchor = anchor(store);
    let tag = anchor_tag(&project(), anchor.seq);
    store.transact(&command(ANCHOR_PUSH,"anchor"),&mut |tx| {
        tx.append(NewEvent {stream:StreamName("project".into()),type_name:ANCHOR_PUSHED.into(),type_version:1,git:None,payload:json!({"tag":tag,"seq":anchor.seq,"head":anchor.hash.to_hex(),"remote":"origin","observed_at":T0}),attachments:vec![]})?;
        tx.record_anchor(&anchor,&tag,"origin",T0)?;Ok(done(json!("ok"),false))
    }).unwrap();
    anchor
}
/// Supplies a remote ahead of the recorded row. Catches treating a local cache as witness.
pub fn a_local_anchor_row_behind_the_remote_is_reported_behind<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let local = local_anchor(&store);
    let remote = anchor(&store);
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(
        report.stored_anchor,
        Some(StoredAnchor {
            tag: anchor_tag(&project(), local.seq),
            anchor: local,
            pushed_at: T0.into()
        })
    );
    assert_eq!(
        report.stored_anchor_comparison,
        StoredAnchorComparison::LocalBehind
    );
}
/// Supplies another remote hash at the row's sequence. Catches comparison by sequence alone.
pub fn a_local_anchor_row_that_differs_from_the_remote_is_a_conflict<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let local = local_anchor(&store);
    let remote = Anchor {
        hash: Hash([9; 32]),
        ..local.clone()
    };
    let report = store.verify(&project(), Some(&remote)).unwrap();
    assert_eq!(report.stored_anchor.unwrap().anchor, local);
    assert_eq!(
        report.stored_anchor_comparison,
        StoredAnchorComparison::Conflict
    );
}
/// Purges an attachment and compares all recorded events. Catches event edits or deletions during purge.
pub fn a_purge_leaves_every_recorded_event_as_it_was<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"abc");
    let before = history(&store);
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    let after = history(&store);
    assert_eq!(&after[..before.len()], before);
    assert_eq!(after.len(), before.len() + 2);
    assert_eq!(after[before.len()].type_name, PAYLOAD_PURGED);
    assert_eq!(after[before.len() + 1].type_name, COMMAND_COMPLETED);
}
/// Swaps a stored caller for another valid one without its hash. Catches a caller kept outside the hashed envelope.
pub fn a_replaced_caller_is_named_at_its_sequence<F: StoreFactory>(factory: &F) {
    let store = sound(factory);
    store
        .transact(
            &by(command("fixture.add", "by-caller"), &server_caller()),
            &mut |tx| {
                tx.append(event(
                    2,
                    json!({"id": 3, "state": "open", "rank": 0, "owner": ""}),
                ))?;
                Ok(done(json!("ok"), false))
            },
        )
        .unwrap();
    assert!(store.verify(&project(), None).unwrap().chain.is_intact());
    factory
        .corrupt(
            &store,
            &project(),
            Corruption::ReplaceCaller {
                seq: 6,
                caller: hook_caller(),
            },
        )
        .unwrap();
    assert!(matches!(
        store.verify(&project(), None).unwrap().chain.first_break,
        Some(Break {
            seq: 6,
            kind: BreakKind::Hash { .. }
        })
    ));
}
