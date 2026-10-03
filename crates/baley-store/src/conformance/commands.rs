//! EVD-R5, R6, R7: atomic commands, request identity and stale inputs.
use super::fixture::*;
use crate::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Ignores a failing append after storing a body. Catches swallowed errors and partial commits.
pub fn a_projector_failure_records_nothing<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    fixture(&store);
    let failing = factory.reopen(&store, failing()).unwrap();
    let before = history(&store);
    let head = store.head(&project()).unwrap();
    let documents = read_back(&store);
    let cmd = command("fixture.add", "failed");
    let mut runs = 0;
    for _ in 0..2 {
        let result = failing.transact(&cmd, &mut |tx| {
            runs += 1;
            let body = tx.put_payload(b"output", RetentionClass::Output)?;
            let mut e = event(
                2,
                json!({"id":TAIL,"state":"open","rank":0,"owner":"","output":body.to_value()}),
            );
            e.attachments.push(body);
            let _ignored = tx.append(e);
            Ok(done(json!("ok"), false))
        });
        assert!(matches!(result,Err(StoreError::Projector { view,seq:6,.. }) if view == "tally"));
        assert_eq!(history(&store), before);
        assert_eq!(store.head(&project()).unwrap(), head);
        assert_eq!(read_back(&store), documents);
        for view in ["item", "tally"] {
            assert_eq!(store.get(&project(), view, &id(TAIL)).unwrap(), None);
        }
        assert_eq!(
            store
                .get(&project(), "request", &request("fixture.add", "failed"))
                .unwrap(),
            None
        );
        let hash = Hash(Sha256::digest(b"output").into());
        assert_eq!(
            store.status(&hash),
            Err(StoreError::Refused(Refusal::UnknownPayload(hash)))
        );
    }
    assert_eq!(runs, 2);
}
/// Repeats an answered request. Catches decisions or records repeated on retry.
pub fn a_replayed_request_returns_its_outcome_and_records_nothing<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let first = record(&store, "r1", &[(1, "open")]).unwrap();
    let Recorded::New { outcome, head } = first else {
        panic!("new request")
    };
    assert_eq!(
        outcome,
        Outcome {
            kind: OutcomeKind::Done,
            answer: Answer::Inline(json!("ok"))
        }
    );
    let before = history(&store);
    let documents = store
        .get(&project(), "request", &request("fixture.add", "r1"))
        .unwrap();
    assert_eq!(
        store.transact(&command("fixture.add", "r1"), &mut |_| panic!("retry ran")),
        Ok(Recorded::Replayed { outcome })
    );
    assert_eq!(history(&store), before);
    assert_eq!(store.head(&project()).unwrap(), Some(head));
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.add", "r1"))
            .unwrap(),
        documents
    );
}
/// Reuses an id with another digest. Catches lookup by identity alone.
pub fn a_reused_request_id_with_another_digest_is_refused<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    record(&store, "r1", &[(1, "open")]).unwrap();
    let before = history(&store);
    let mut cmd = command("fixture.add", "r1");
    cmd.digest = Hash([2; 32]);
    assert_eq!(
        store.transact(&cmd, &mut |_| panic!("mismatched retry ran")),
        Err(StoreError::Refused(Refusal::RequestDigestMismatch {
            request_id: cmd.request_id.clone()
        }))
    );
    assert_eq!(history(&store), before);
}
/// Uses one id for different kinds. Catches answers keyed without command kind.
pub fn one_request_id_under_two_command_kinds_is_two_requests<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    for (kind, answer) in [("fixture.add", "added"), ("fixture.remove", "removed")] {
        let cmd = command(kind, "same");
        let outcome = Outcome {
            kind: OutcomeKind::Done,
            answer: Answer::Inline(json!(answer)),
        };
        assert!(
            matches!(store.transact(&cmd,&mut |_| Ok(done(json!(answer),false))).unwrap(),Recorded::New {outcome:found,..} if found == outcome)
        );
        assert_eq!(
            store.transact(&cmd, &mut |_| panic!("retry ran")),
            Ok(Recorded::Replayed { outcome })
        );
    }
    assert_eq!(history(&store).len(), 2);
}
/// Uses one kind and id in two projects with different answers. Catches cross-project request collisions.
pub fn one_request_id_in_two_projects_is_two_requests<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    for (project, answer) in [(project(), "p"), (other_project(), "q")] {
        let mut cmd = command("fixture.add", "same");
        cmd.project = project;
        assert!(
            matches!(store.transact(&cmd,&mut |_| Ok(done(json!(answer),false))).unwrap(),Recorded::New {outcome,..} if outcome.answer == Answer::Inline(json!(answer)))
        );
    }
    for (project, answer) in [(project(), "p"), (other_project(), "q")] {
        let mut cmd = command("fixture.add", "same");
        cmd.project = project;
        assert_eq!(
            store.transact(&cmd, &mut |_| panic!("retry ran")),
            Ok(Recorded::Replayed {
                outcome: Outcome {
                    kind: OutcomeKind::Done,
                    answer: Answer::Inline(json!(answer))
                }
            })
        );
        assert_eq!(store.head(&cmd.project).unwrap().unwrap().seq, 1);
    }
}
fn stale(store: &(impl Ledger + Views), observed: Observed, error: StaleInput) {
    let before = history(store);
    let head = store.head(&project()).unwrap();
    let result = store.transact(&command("fixture.check", "check"), &mut |tx| {
        tx.append(event(2, json!({"id":9,"state":"open","rank":0,"owner":""})))?;
        Ok(Decision {
            observed: observed.clone(),
            ..done(json!("ok"), false)
        })
    });
    assert_eq!(result, Err(StoreError::Stale(error)));
    assert_eq!(history(store), before);
    assert_eq!(store.head(&project()).unwrap(), head);
    assert_eq!(store.get(&project(), "item", &id(9)).unwrap(), None);
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.check", "check"))
            .unwrap(),
        None
    );
}
/// Changes a document after it was observed. Catches missing sequence comparison.
pub fn a_document_moved_since_it_was_seen_is_stale<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    record(&store, "r1", &[(1, "open")]).unwrap();
    record(&store, "r2", &[(1, "done")]).unwrap();
    stale(
        &store,
        Observed {
            documents: vec![ObservedDocument {
                view: "item".into(),
                key: id(1),
                produced_seq: Some(1),
            }],
            ..Observed::default()
        },
        StaleInput::Document {
            view: "item".into(),
            key: id(1),
            seen: Some(1),
            now: Some(3),
        },
    );
}
/// Records an event previously absent. Catches unchecked event absences.
pub fn an_event_that_appeared_since_its_absence_was_seen_is_stale<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    record(&store, "r1", &[(1, "open")]).unwrap();
    let absence = Absence::Event(EventMatch {
        type_name: "fixture.item".into(),
        stream: None,
        fields: BTreeMap::from([("id".into(), json!(1))]),
    });
    stale(
        &store,
        Observed {
            absences: vec![absence.clone()],
            ..Observed::default()
        },
        StaleInput::Absence(absence),
    );
}
/// Creates a document matching an absent prefix. Catches unchecked document absences.
pub fn a_document_that_appeared_since_its_absence_was_seen_is_stale<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    record(&store, "r1", &[(1, "open")]).unwrap();
    let absence = Absence::Documents {
        view: "item".into(),
        index: "by_state_rank".into(),
        equals: vec![KeyValue::Text("open".into())],
    };
    stale(
        &store,
        Observed {
            absences: vec![absence.clone()],
            ..Observed::default()
        },
        StaleInput::Absence(absence),
    );
}
/// Supplies a changed HEAD. Catches git facts recorded without comparison.
pub fn a_git_head_moved_since_it_was_seen_is_stale<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    stale(
        &store,
        Observed {
            git: vec![GitObservation {
                checkout: "/work".into(),
                head_seen: "aaa".into(),
                head_now: "bbb".into(),
                index_seen: "idx".into(),
                index_now: "idx".into(),
            }],
            ..Observed::default()
        },
        StaleInput::Git {
            checkout: "/work".into(),
            fact: GitFact::Head,
            seen: "aaa".into(),
            now: "bbb".into(),
        },
    );
}
/// Records one command by a caller and one with none. Catches a completion recorded without its command's context, and a caller stamped on the domain events only.
pub fn a_command_stamps_its_caller_on_every_event_it_appends<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let server = server_caller();
    store
        .transact(
            &by(command("fixture.add", "by-caller"), &server),
            &mut |tx| {
                for id in [1, 2] {
                    tx.append(event(
                        2,
                        json!({"id": id, "state": "open", "rank": 0, "owner": ""}),
                    ))?;
                }
                Ok(done(json!("ok"), false))
            },
        )
        .unwrap();
    record(&store, "no-caller", &[(3, "open")]).unwrap();
    let events = history(&store);
    let types: Vec<&str> = events.iter().map(|e| e.type_name.as_str()).collect();
    assert_eq!(
        types,
        [
            "fixture.item",
            "fixture.item",
            COMMAND_COMPLETED,
            "fixture.item",
            COMMAND_COMPLETED
        ]
    );
    for event in &events[..3] {
        assert_eq!(event.caller, Some(server.clone()), "seq {}", event.seq);
    }
    for event in &events[3..] {
        assert_eq!(event.caller, None, "seq {}", event.seq);
    }
}
