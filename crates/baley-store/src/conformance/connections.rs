//! EVD-R8: independent connections read and extend one chain.
use super::fixture::*;
use crate::*;
use serde_json::json;

/// Reads during another connection's decision. Catches readers waiting for writers.
pub fn a_read_on_another_connection_runs_during_a_write<F: StoreFactory>(factory: &F) {
    let a = created(factory);
    fixture(&a);
    let b = factory.reopen(&a, binary()).expect("second connection");
    let before = a.head(&project()).unwrap();
    a.transact(&command("fixture.add", "next"), &mut |tx| {
        tx.append(event(2, json!({"id":3,"state":"open","rank":0,"owner":""})))?;
        assert_eq!(b.head(&project()).unwrap(), before);
        Ok(done(json!("ok"), false))
    })
    .unwrap();
    let after = b.head(&project()).unwrap().unwrap();
    assert_eq!(after.seq, 7);
    assert_eq!(a.head(&project()).unwrap(), Some(after));
}

/// Alternates writes through independent connections. Catches a cached head that forks the chain.
pub fn writes_through_two_connections_extend_one_chain<F: StoreFactory>(factory: &F) {
    let a = created(factory);
    let b = factory.reopen(&a, binary()).unwrap();
    for (store, request, seq) in [(&a, "a", 2), (&b, "b", 4), (&a, "c", 6), (&b, "d", 8)] {
        let result = record(store, request, &[(1, "open")]).unwrap();
        assert!(
            matches!(result, Recorded::New { head: Head { seq: found, .. }, .. } if found == seq)
        );
    }
    assert_eq!(
        history(&a).iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(a.head(&project()).unwrap(), b.head(&project()).unwrap());
    assert_eq!(a.verify(&project(), None).unwrap().chain.first_break, None);
}
