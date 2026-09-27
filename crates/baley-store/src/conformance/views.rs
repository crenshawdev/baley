//! EVD-R9: declared keys, indexes, order, paging and bounds.
use super::fixture::*;
use crate::*;

fn keys(page: &Page<Document>) -> Vec<DocKey> {
    page.items.iter().map(|d| d.key.clone()).collect()
}

/// Pages through ties and a descending field. Catches skipped or repeated positions.
pub fn pages_follow_the_declared_order_without_gaps_or_repeats<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(2);
    for (expected, more) in [
        (vec![id(2), id(3)], true),
        (vec![id(1), id(4)], true),
        (vec![id(5), id(6)], false),
    ] {
        let page = store.find(&project(), "item", &q).unwrap();
        assert_eq!(keys(&page), expected);
        assert_eq!(page.next.is_some(), more);
        q.page.after = page.next;
    }
}
/// Reads a full final page. Catches cursors issued merely because a page is full.
pub fn a_last_full_page_has_no_next_cursor<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(2);
    q.index = "by_owner".into();
    q.equals = vec![KeyValue::Text("a".into())];
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(2), id(3)]);
    q.page.after = Some(page.next.expect("second page"));
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(4), id(5)]);
    assert_eq!(page.next, None);
}
/// Requests beyond the view bound. Catches an uncapped page.
pub fn no_page_exceeds_the_views_bound<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(100);
    for (expected, more) in [
        (vec![id(2), id(3), id(1)], true),
        (vec![id(4), id(5), id(6)], false),
    ] {
        let page = store.find(&project(), "item", &q).unwrap();
        assert_eq!(keys(&page), expected);
        assert_eq!(page.next.is_some(), more);
        q.page.after = page.next;
    }
}
/// Selects one index prefix and pages its descending remainder. Catches misplaced equality fields.
pub fn equality_on_an_index_prefix_keeps_the_rest_of_the_order<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(2);
    q.equals = vec![KeyValue::Text("open".into())];
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(1), id(4)]);
    q.page.after = page.next;
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(5), id(6)]);
    assert_eq!(page.next, None);
    q.page.after = None;
    q.equals.push(KeyValue::Integer(2));
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(5), id(6)]);
    assert_eq!(page.next, None);
    q.index = "by_owner".into();
    q.equals = vec![KeyValue::Text("a".into())];
    q.page.limit = 100;
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(2), id(3), id(4)]);
    q.page.after = page.next;
    let page = store.find(&project(), "item", &q).unwrap();
    assert_eq!(keys(&page), vec![id(5)]);
    assert_eq!(page.next, None);
}
/// Names an undeclared index. Catches fallback scanning.
pub fn a_find_on_an_undeclared_index_is_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let mut q = query(2);
    q.index = "missing".into();
    assert_eq!(
        store.find(&project(), "item", &q),
        Err(StoreError::Refused(Refusal::UndeclaredIndex {
            view: "item".into(),
            index: "missing".into()
        }))
    );
}
/// Reads an unknown view by key and index. Catches undeclared storage access.
pub fn a_read_of_an_unknown_view_is_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let error = Err(StoreError::Refused(Refusal::UnknownView("missing".into())));
    assert_eq!(store.get(&project(), "missing", &id(1)), error);
    assert_eq!(
        store.find(&project(), "missing", &query(2)),
        Err(StoreError::Refused(Refusal::UnknownView("missing".into())))
    );
}
/// Uses wrong key arity and type. Catches loosely matched keys.
pub fn a_key_that_does_not_fit_the_view_is_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    for key in [
        DocKey(vec![]),
        DocKey(vec![KeyValue::Text("1".into())]),
        DocKey(vec![KeyValue::Integer(1), KeyValue::Integer(2)]),
    ] {
        assert!(
            matches!(store.get(&project(), "item", &key), Err(StoreError::Refused(Refusal::MalformedKey { view, .. })) if view == "item")
        );
    }
}
/// Supplies wrong index value arity and type. Catches invalid equality bindings.
pub fn equality_values_that_do_not_fit_the_index_are_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    for equals in [
        vec![KeyValue::Integer(1)],
        vec![KeyValue::Text("open".into()), KeyValue::Text("2".into())],
        vec![
            KeyValue::Text("open".into()),
            KeyValue::Integer(2),
            KeyValue::Integer(3),
        ],
    ] {
        let mut q = query(2);
        q.equals = equals;
        assert!(
            matches!(store.find(&project(), "item", &q), Err(StoreError::Refused(Refusal::MalformedKey { view, .. })) if view == "item")
        );
    }
}
/// Reuses a cursor with other query bindings. Catches cursors treated as bare positions.
pub fn a_cursor_from_another_query_is_refused<F: StoreFactory>(factory: &F) {
    let store = stocked(factory);
    let cursor = store
        .find(&project(), "item", &query(2))
        .unwrap()
        .next
        .unwrap();
    for (project, index, equals, cursor) in [
        (project(), "by_owner", vec![], cursor.clone()),
        (
            project(),
            "by_state_rank",
            vec![KeyValue::Text("done".into())],
            store
                .find(
                    &project(),
                    "item",
                    &IndexQuery {
                        equals: vec![KeyValue::Text("open".into())],
                        ..query(2)
                    },
                )
                .unwrap()
                .next
                .unwrap(),
        ),
        (other_project(), "by_state_rank", vec![], cursor),
        (project(), "by_state_rank", vec![], {
            let mut damaged = store
                .find(&project(), "item", &query(2))
                .unwrap()
                .next
                .unwrap();
            damaged.0.pop();
            damaged
        }),
    ] {
        let q = IndexQuery {
            index: index.into(),
            equals,
            page: PageRequest {
                limit: 2,
                after: Some(cursor),
            },
        };
        assert_eq!(
            store.find(&project, "item", &q),
            Err(StoreError::Refused(Refusal::InvalidCursor))
        );
    }
}
