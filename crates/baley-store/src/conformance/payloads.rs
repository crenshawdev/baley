//! EVD-R11, R14: content addressing, streams and per-reference retention.
use super::fixture::*;
use crate::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;

pub(super) fn bytes(store: &impl Payloads, hash: &Hash) -> Vec<u8> {
    let PayloadBody::Present(mut reader) = store.open(hash).unwrap() else {
        panic!("present body")
    };
    let mut result = Vec::new();
    let mut chunk = [0; 8191];
    loop {
        let n = reader.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        result.extend_from_slice(&chunk[..n]);
    }
    result
}
/// Stores the FIPS abc vector. Catches hashing or measuring compressed bytes.
pub fn a_body_is_addressed_by_the_sha256_of_its_bytes<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach_class(
        &store,
        &command("fixture.add", "body"),
        b"abc",
        RetentionClass::Record,
    );
    assert_eq!(
        body,
        PayloadRef {
            hash: Hash::from_hex(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            )
            .unwrap(),
            bytes: 3,
            class: RetentionClass::Record
        }
    );
}
/// Reads a present body's status. Catches compressed length or a false tombstone.
pub fn a_present_body_reports_its_uncompressed_length<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), &body_bytes());
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Present { bytes: 196608 })
    );
}
/// Streams across several chunks. Catches lost or repeated stream positions.
pub fn a_body_over_several_chunks_streams_back_byte_for_byte<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let input = [noise(400_000), vec![b'x'; 400_000]].concat();
    let body = attach(&store, &command("fixture.add", "body"), &input);
    assert_eq!(bytes(&store, &body.hash), input);
}
/// Reduces a three-part body. Catches wrong excerpt edges, ranges or hash.
pub fn a_reduction_keeps_the_first_and_last_64_kib_under_their_own_hash<F: StoreFactory>(
    factory: &F,
) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), &body_bytes());
    let expected = [vec![b'a'; 65536], vec![b'c'; 65536]].concat();
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
    assert_eq!(
        excerpt,
        PayloadRef {
            hash: Hash(Sha256::digest(&expected).into()),
            bytes: 131072,
            class: RetentionClass::Record
        }
    );
    assert_eq!(bytes(&store, &excerpt.hash), expected);
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Reduced {
            excerpt: excerpt.clone(),
            kept: vec![0..65536, 131072..196608]
        })
    );
    let events = history(&store);
    let reduced = &events[2];
    assert_eq!(reduced.type_name, PAYLOAD_REDUCED);
    let payload = ReducedEvent::from_value(&reduced.payload).expect("reduction payload");
    assert_eq!(payload.original, body.hash);
    assert_eq!(payload.excerpt, excerpt);
    assert_eq!(payload.kept, [0..65536, 131072..196608]);
}
/// Releases a body in one of two projects. Catches removal of a still-required shared body.
pub fn a_shared_body_survives_one_projects_purge<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = attach(&store, &command("fixture.add", "body"), b"shared");
    let mut other = command("fixture.add", "other");
    other.project = other_project();
    assert_eq!(attach(&store, &other, b"shared"), body);
    let other_head = store.head(&other_project()).unwrap();
    let report = store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    assert_eq!(store.head(&other_project()).unwrap(), other_head);
    assert!(report.purged.is_empty());
    assert_eq!(report.shared, vec![body.hash]);
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Present { bytes: 6 })
    );
    assert_eq!(bytes(&store, &body.hash), b"shared");
}
pub(super) fn stored_answer(store: &impl Ledger) -> PayloadRef {
    let result = store
        .transact(&command("fixture.answer", "answer"), &mut |_| {
            Ok(done(json!("sensitive"), true))
        })
        .unwrap();
    let Recorded::New {
        outcome: Outcome {
            answer: Answer::Stored(reference),
            ..
        },
        ..
    } = result
    else {
        panic!("stored answer")
    };
    reference
}
/// Purges a stored answer while retaining its document. Catches request rows rewritten with the body.
pub fn a_purged_answer_leaves_its_request_document_unchanged<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let body = stored_answer(&store);
    let before = store
        .get(&project(), "request", &request("fixture.answer", "answer"))
        .unwrap();
    let documents = read_back(&store);
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    assert_eq!(
        store.status(&body.hash),
        Ok(PayloadStatus::Purged {
            reason: "remove".into()
        })
    );
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.answer", "answer"))
            .unwrap(),
        before
    );
    assert_eq!(read_back(&store), documents);
}
/// Retries a purged answer. Catches repeated decisions or references returned as available bytes.
pub fn a_retry_after_a_purge_gets_the_tombstone<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let body = stored_answer(&store);
    store
        .purge(&command("payload.purge", "purge"), &[body.hash], "remove")
        .unwrap();
    let before = history(&store);
    let head = store.head(&project()).unwrap();
    let reference = body.clone();
    assert_eq!(
        store.transact(&command("fixture.answer", "answer"), &mut |_| panic!(
            "retry ran"
        )),
        Ok(Recorded::Replayed {
            outcome: Outcome {
                kind: OutcomeKind::Done,
                answer: Answer::Tombstone {
                    reference: body,
                    status: PayloadStatus::Purged {
                        reason: "remove".into()
                    }
                }
            }
        })
    );
    assert_eq!(history(&store), before);
    assert_eq!(store.head(&project()).unwrap(), head);
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.answer", "answer"))
            .unwrap()
            .unwrap()
            .body["answer"]["stored"],
        reference.to_value()
    );
}
/// Reduces one body and purges another, each by a caller. Catches store-owned retention events recorded without the caller.
pub fn a_reduction_and_a_purge_record_their_callers<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let reduced = attach(&store, &command("fixture.add", "first"), &body_bytes());
    let purged = attach(&store, &command("fixture.add", "second"), b"other body");
    let (server, hook) = (server_caller(), hook_caller());
    let before = history(&store).len();
    store
        .reduce(
            &by(command("payload.reduce", "reduce"), &server),
            &PayloadReference {
                project: project(),
                seq: 1,
                hash: reduced.hash,
            },
        )
        .unwrap();
    let after_reduce = history(&store);
    store
        .purge(
            &by(command("payload.purge", "purge"), &hook),
            &[purged.hash],
            "remove",
        )
        .unwrap();
    let events = history(&store);
    let seen = |range: &[Event]| -> Vec<(String, Option<Caller>)> {
        range
            .iter()
            .map(|e| (e.type_name.clone(), e.caller.clone()))
            .collect()
    };
    assert_eq!(
        seen(&after_reduce[before..]),
        [
            (PAYLOAD_REDUCED.to_string(), Some(server.clone())),
            (COMMAND_COMPLETED.to_string(), Some(server)),
        ]
    );
    assert_eq!(
        seen(&events[after_reduce.len()..]),
        [
            (PAYLOAD_PURGED.to_string(), Some(hook.clone())),
            (COMMAND_COMPLETED.to_string(), Some(hook)),
        ]
    );
}
