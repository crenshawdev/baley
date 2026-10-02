//! Checkout admission's decisions on supplied values. Expected payloads are
//! written out from the event's documented shape, never from running this
//! code.

use baley_store::{
    Actor, Change, DocKey, Event, EventSchema, FieldKind, FieldSpec, Hash, IndexField, IndexSpec,
    KeyValue, Order, ProjectId, Projector, RequestId, ViewSpec,
};
use serde_json::{Value, json};

use super::*;
use crate::registry::Registry;

fn checkout(root_commit: Option<&str>, remote_url: Option<&str>) -> Checkout {
    Checkout {
        path: "/r".into(),
        root_commit: root_commit.map(str::to_owned),
        remote_url: remote_url.map(str::to_owned),
    }
}

#[test]
fn a_seen_payload_dropping_or_renaming_a_member_is_caught() {
    assert_eq!(
        seen_payload(&checkout(
            Some("abc123"),
            Some("https://github.com/o/r.git")
        )),
        json!({
            "path": "/r",
            "root_commit": "abc123",
            "remote_url": "https://github.com/o/r.git",
        })
    );
}

#[test]
fn a_seen_payload_omitting_or_blanking_an_absent_field_is_caught() {
    let payload = seen_payload(&checkout(None, None));
    assert_eq!(
        payload,
        json!({"path": "/r", "root_commit": null, "remote_url": null})
    );
    assert_eq!(payload.as_object().map(|members| members.len()), Some(3));
}

#[test]
fn checkout_seen_registered_at_any_version_but_1_is_caught() {
    let mut registry = Registry::new();
    register_checkout_events(&mut registry).unwrap();
    assert!(registry.reads("checkout.seen", 1));
    assert!(!registry.reads("checkout.seen", 2));
}

const PROJECT_ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

fn event(seq: u64, payload: Value) -> Event {
    Event {
        project_id: ProjectId(PROJECT_ID.into()),
        seq,
        stream: "project".into(),
        stream_version: seq,
        type_name: "checkout.seen".into(),
        type_version: 1,
        actor: Actor::Baley,
        recorded_at: "2026-10-01T09:00:00Z".into(),
        request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
        git: None,
        policy_version: 0,
        payload,
        prev_hash: None,
        hash: Hash([0; 32]),
    }
}

fn seen(path: &str, root_commit: Option<&str>, remote_url: Option<&str>) -> Value {
    seen_payload(&Checkout {
        path: path.into(),
        root_commit: root_commit.map(str::to_owned),
        remote_url: remote_url.map(str::to_owned),
    })
}

fn put(changes: Vec<Change>) -> (DocKey, Value) {
    match <[Change; 1]>::try_from(changes) {
        Ok([Change::Put { key, body }]) => (key, body),
        other => panic!("expected one put, got {other:?}"),
    }
}

fn key(path: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(path.into())])
}

#[test]
fn a_checkout_view_keyed_or_indexed_by_remote_url_is_caught() {
    assert_eq!(
        checkout_spec(),
        ViewSpec {
            name: "checkout".into(),
            version: 1,
            key: vec![FieldSpec {
                name: "path".into(),
                kind: FieldKind::Text,
            }],
            indexes: vec![IndexSpec {
                name: "by_path".into(),
                fields: vec![IndexField {
                    name: "path".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                }],
            }],
            page_bound: 100,
        }
    );
}

#[test]
fn a_later_checkout_seen_merged_into_the_stored_row_instead_of_replacing_it_is_caught() {
    let projector = CheckoutProjector::new();
    let first = seen("/r", Some("c1"), Some("https://h/o/r.git"));
    let second = seen("/r", None, None);
    let (first_key, first_body) = put(projector.apply(&event(2, first.clone()), &[]).unwrap());
    let (second_key, second_body) = put(projector.apply(&event(5, second.clone()), &[]).unwrap());
    assert_eq!(first_key, key("/r"));
    assert_eq!(second_key, key("/r"));
    assert_eq!(first_body, first);
    assert_eq!(second_body, second);
}

#[test]
fn two_checkouts_of_one_project_sharing_a_row_is_caught() {
    let projector = CheckoutProjector::new();
    let (one, _) = put(projector
        .apply(&event(2, seen("/a", None, None)), &[])
        .unwrap());
    let (other, _) = put(projector
        .apply(&event(3, seen("/b", None, None)), &[])
        .unwrap());
    assert_eq!(one, key("/a"));
    assert_eq!(other, key("/b"));
}

#[test]
fn a_malformed_checkout_seen_accepted_into_the_view_is_caught() {
    let projector = CheckoutProjector::new();
    let cases = [
        json!("a string"),
        json!({"root_commit": null, "remote_url": null}),
        json!({"path": "", "root_commit": null, "remote_url": null}),
        json!({"path": 7, "root_commit": null, "remote_url": null}),
        json!({"path": "/r", "remote_url": null}),
        json!({"path": "/r", "root_commit": "", "remote_url": null}),
        json!({"path": "/r", "root_commit": 7, "remote_url": null}),
        json!({"path": "/r", "root_commit": null}),
        json!({"path": "/r", "root_commit": null, "remote_url": ""}),
        json!({"path": "/r", "root_commit": null, "remote_url": 7}),
    ];
    for payload in cases {
        let refused = projector
            .apply(&event(9, payload.clone()), &[])
            .expect_err(&format!("{payload} must be refused"));
        assert!(
            refused.0.starts_with("checkout.seen at seq 9: "),
            "{}",
            refused.0
        );
    }
}

#[test]
fn a_checkout_projector_handling_or_removing_anything_beyond_checkout_seen_is_caught() {
    let projector = CheckoutProjector::new();
    assert_eq!(projector.handles(), ["checkout.seen"]);
    assert!(projector.keys(&event(2, seen("/r", None, None))).is_empty());
}

const GITHUB: &str = "https://github.com/o/r.git";

fn here(root_commit: Option<&str>, remote_url: Option<&str>) -> Checkout {
    Checkout {
        path: "/b".into(),
        root_commit: root_commit.map(str::to_owned),
        remote_url: remote_url.map(str::to_owned),
    }
}

fn conflict(verdict: CheckoutVerdict) -> ProjectIdConflict {
    match verdict {
        CheckoutVerdict::Conflict(conflict) => conflict,
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn a_fork_with_another_remote_admitted_or_its_refusal_missing_a_path_or_the_way_out_is_caught() {
    let rows = [seen(
        "/a",
        Some("c1"),
        Some("https://github.com/o/other.git"),
    )];
    let refusal = conflict(judge_checkout(&here(Some("c1"), Some(GITHUB)), &rows));
    assert_eq!(refusal.code(), "project-id-conflict");
    let text = refusal.to_string();
    assert!(text.starts_with("project-id-conflict: "), "{text}");
    for part in [
        "/a",
        "/b",
        GITHUB,
        "https://github.com/o/other.git",
        "baley init --new-id",
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }
}

#[test]
fn two_different_https_origins_admitted_as_one_project_is_caught() {
    let rows = [seen("/a", None, Some("https://git.example.com/o/r.git"))];
    let verdict = judge_checkout(&here(None, Some("https://github.com/o/r.git")), &rows);
    assert!(
        matches!(verdict, CheckoutVerdict::Conflict(_)),
        "{verdict:?}"
    );
}

#[test]
fn an_https_clone_and_an_ssh_clone_of_one_repository_treated_as_one_remote_is_caught() {
    let rows = [seen("/a", None, Some("git@github.com:o/r.git"))];
    let verdict = judge_checkout(&here(None, Some(GITHUB)), &rows);
    assert!(
        matches!(verdict, CheckoutVerdict::Conflict(_)),
        "{verdict:?}"
    );
}

#[test]
fn urls_that_differ_only_in_case_or_scheme_treated_as_one_remote_is_caught() {
    let upper = [seen("/a", None, Some("HTTPS://GITHUB.COM/O/R.GIT"))];
    let verdict = judge_checkout(&here(None, Some(GITHUB)), &upper);
    assert!(
        matches!(verdict, CheckoutVerdict::Conflict(_)),
        "{verdict:?}"
    );
    let bare = [seen("/a", None, Some("github.com/o/r.git"))];
    let verdict = judge_checkout(&here(None, Some(GITHUB)), &bare);
    assert!(
        matches!(verdict, CheckoutVerdict::Conflict(_)),
        "{verdict:?}"
    );
}

#[test]
fn a_second_clone_of_the_same_url_refused_as_a_fork_is_caught() {
    let rows = [seen("/a", Some("c1"), Some(GITHUB))];
    assert_eq!(
        judge_checkout(&here(Some("c1"), Some(GITHUB)), &rows),
        CheckoutVerdict::Record
    );
}

#[test]
fn a_checkout_with_no_remote_refused_as_a_fork_is_caught() {
    let rows = [seen("/a", Some("c1"), Some(GITHUB))];
    assert_eq!(
        judge_checkout(&here(Some("c1"), None), &rows),
        CheckoutVerdict::Record
    );
}

#[test]
fn a_row_with_no_remote_conflicting_with_a_checkouts_url_is_caught() {
    let rows = [seen("/a", Some("c1"), None)];
    assert_eq!(
        judge_checkout(&here(Some("c1"), Some(GITHUB)), &rows),
        CheckoutVerdict::Record
    );
}

#[test]
fn a_remote_url_change_at_the_same_path_refused_as_a_fork_with_itself_is_caught() {
    let rows = [seen("/b", Some("c1"), Some("https://github.com/o/old.git"))];
    assert_eq!(
        judge_checkout(&here(Some("c1"), Some(GITHUB)), &rows),
        CheckoutVerdict::Record
    );
}

#[test]
fn a_different_root_commit_at_another_path_with_the_same_url_refused_is_caught() {
    let rows = [seen("/a", Some("c1"), Some(GITHUB))];
    assert_eq!(
        judge_checkout(&here(Some("c2"), Some(GITHUB)), &rows),
        CheckoutVerdict::Record
    );
}

#[test]
fn an_own_row_equal_in_both_fields_recorded_again_or_a_changed_one_left_alone_is_caught() {
    let same = [seen("/b", Some("c1"), Some(GITHUB))];
    assert_eq!(
        judge_checkout(&here(Some("c1"), Some(GITHUB)), &same),
        CheckoutVerdict::Unchanged
    );
    assert_eq!(
        judge_checkout(&here(Some("c2"), Some(GITHUB)), &same),
        CheckoutVerdict::Record
    );
    let bare = [seen("/b", None, None)];
    assert_eq!(
        judge_checkout(&here(None, None), &bare),
        CheckoutVerdict::Unchanged
    );
    assert_eq!(
        judge_checkout(&here(Some("c1"), None), &bare),
        CheckoutVerdict::Record
    );
}

#[test]
fn an_own_row_not_as_the_projector_writes_it_taken_as_current_is_caught() {
    let missing_url = [json!({"path": "/b", "root_commit": "c1"})];
    assert_eq!(
        judge_checkout(&here(Some("c1"), None), &missing_url),
        CheckoutVerdict::Record
    );
}

#[test]
fn an_unchanged_own_row_judged_before_another_checkouts_conflict_is_caught() {
    let rows = [
        seen("/b", Some("c1"), Some(GITHUB)),
        seen("/a", Some("c1"), Some("https://github.com/o/other.git")),
    ];
    let verdict = judge_checkout(&here(Some("c1"), Some(GITHUB)), &rows);
    assert!(
        matches!(verdict, CheckoutVerdict::Conflict(_)),
        "{verdict:?}"
    );
}

#[test]
fn the_conflict_naming_a_path_that_depends_on_page_order_is_caught() {
    let early = seen("/a", None, Some("https://one.example/r.git"));
    let late = seen("/c", None, Some("https://two.example/r.git"));
    let checkout = here(None, Some(GITHUB));
    let forward = conflict(judge_checkout(&checkout, &[early.clone(), late.clone()]));
    let backward = conflict(judge_checkout(&checkout, &[late, early]));
    assert_eq!(forward.other_path, "/a");
    assert_eq!(forward, backward);
}

fn operations(plan: CheckoutPlan) -> Vec<CheckoutOperation> {
    match plan.action {
        CheckoutAction::Proceed(operations) => operations,
        other => panic!("expected operations, got {other:?}"),
    }
}

#[test]
fn a_new_checkout_requesting_its_record_after_the_step_or_the_step_after_the_command_is_caught() {
    let checkout = here(Some("c1"), Some(GITHUB));
    assert_eq!(
        operations(plan_checkout_admission(&checkout, &[], None)),
        vec![
            CheckoutOperation::RecordSeen(json!({
                "path": "/b",
                "root_commit": "c1",
                "remote_url": GITHUB,
            })),
            CheckoutOperation::PolicyStep,
            CheckoutOperation::Command,
        ]
    );
}

#[test]
fn a_changed_root_commit_left_unrecorded_is_caught() {
    let rows = [seen("/b", Some("c1"), Some(GITHUB))];
    let operations = operations(plan_checkout_admission(
        &here(Some("c2"), Some(GITHUB)),
        &rows,
        None,
    ));
    assert!(matches!(
        operations.first(),
        Some(CheckoutOperation::RecordSeen(_))
    ));
}

#[test]
fn an_unchanged_checkout_recorded_again_is_caught() {
    let rows = [seen("/b", Some("c1"), Some(GITHUB))];
    assert_eq!(
        operations(plan_checkout_admission(
            &here(Some("c1"), Some(GITHUB)),
            &rows,
            None
        )),
        vec![CheckoutOperation::PolicyStep, CheckoutOperation::Command]
    );
}

#[test]
fn a_fork_that_still_requests_a_record_the_step_or_the_command_is_caught() {
    let rows = [seen("/a", None, Some("https://github.com/o/other.git"))];
    let plan = plan_checkout_admission(&here(None, Some(GITHUB)), &rows, None);
    match plan.action {
        CheckoutAction::Refuse(refusal) => assert_eq!(refusal.other_path, "/a"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_stored_policy_version_dropped_or_invented_is_caught() {
    let checkout = here(None, None);
    let versions = [
        (Some(json!({"version": 7})), 7),
        (None, 0),
        (Some(json!({"values": {}})), 0),
        (Some(json!({"version": 0})), 0),
        (Some(json!({"version": "7"})), 0),
    ];
    for (stored, expected) in versions {
        assert_eq!(
            plan_checkout_admission(&checkout, &[], stored.as_ref()).policy_version,
            expected,
            "{stored:?}"
        );
    }
}

#[test]
fn a_refusal_that_drops_the_stored_policy_version_is_caught() {
    let rows = [seen("/a", None, Some("https://github.com/o/other.git"))];
    let stored = json!({"version": 7});
    let plan = plan_checkout_admission(&here(None, Some(GITHUB)), &rows, Some(&stored));
    assert!(matches!(plan.action, CheckoutAction::Refuse(_)));
    assert_eq!(plan.policy_version, 7);
}
