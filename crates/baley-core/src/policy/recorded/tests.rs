//! The recorded policy's decisions on supplied values. Expected payloads are
//! written out from design 0003 section 6 and the event's documented shape,
//! never from running this code.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use baley_store::{
    Actor, Change, DocKey, Event, EventSchema, FieldKind, FieldSpec, Hash, KeyValue, ProjectId,
    Projector, RequestId, ViewSpec,
};
use serde_json::{Value, json};

use super::*;
use crate::policy::schema::{Default as Builtin, Entry, Host, Kind, Rung, Schema, Scope};
use crate::policy::{EffectivePolicy, SettingsFile, effective_policy};
use crate::registry::Registry;

const GLOBAL: &str = "/c/config.toml";
const PROJECT: &str = "/r/baley.toml";
const CHECKOUT: &str = "/r";
const PROJECT_ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

fn entry(name: &str, kind: Kind, default: Builtin, scope: Scope) -> Entry {
    Entry {
        name: name.into(),
        kind,
        default,
        scope,
        owner: "test",
    }
}

/// One setting of each kind, and one only the project file may set.
fn schema() -> Schema {
    Schema::new(vec![
        entry("escalate", Kind::Bool, Builtin::Bool(false), Scope::Both),
        entry("effort", Kind::Rung, Builtin::Rung(Rung::High), Scope::Both),
        entry("model", Kind::ModelName, Builtin::Absent, Scope::Both),
        entry(
            "only_project",
            Kind::Bool,
            Builtin::Bool(true),
            Scope::Project,
        ),
    ])
}

fn file(path: impl Into<PathBuf>, text: &str, digest: &str) -> SettingsFile {
    SettingsFile {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
        digest: digest.into(),
    }
}

fn policy(
    host: Option<Host>,
    global: Option<&SettingsFile>,
    project: Option<&SettingsFile>,
) -> EffectivePolicy {
    effective_policy(&schema(), host, global, project).expect("valid files")
}

fn payload(policy: &EffectivePolicy, catalog_version: u64) -> serde_json::Value {
    let recorded = recorded_policy(Path::new(CHECKOUT), policy).expect("UTF-8 paths");
    effective_payload(&recorded, PROJECT_ID, catalog_version)
}

#[test]
fn a_payload_that_drops_or_misattributes_a_default_global_or_project_value_is_caught() {
    let global = file(GLOBAL, "escalate = true\n", "g1");
    let project = file(PROJECT, "only_project = false\n", "p1");
    let policy = policy(None, Some(&global), Some(&project));
    assert_eq!(
        payload(&policy, 7),
        json!({
            "project": PROJECT_ID,
            "checkout": "/r",
            "host": null,
            "values": {
                "effort": "high",
                "escalate": true,
                "model": null,
                "only_project": false,
            },
            "sources": {
                "effort": {"layer": "default"},
                "escalate": {
                    "layer": "global", "path": "/c/config.toml", "digest": "g1",
                    "line": 1, "column": 1,
                },
                "model": {"layer": "default"},
                "only_project": {
                    "layer": "project", "path": "/r/baley.toml", "digest": "p1",
                    "line": 1, "column": 1,
                },
            },
            "diagnostics": [],
            "catalog_version": 7,
        })
    );
}

#[test]
fn the_command_line_host_written_as_anything_but_null_is_caught() {
    let policy = policy(None, None, None);
    assert_eq!(payload(&policy, 0)["host"], json!(null));
}

#[test]
fn a_host_section_value_recorded_without_its_host_or_layer_is_caught() {
    let project = file(
        PROJECT,
        "[host.claude-code]\neffort = \"max\"\nmodel = \"m-1\"\n",
        "p2",
    );
    let policy = policy(Some(Host::ClaudeCode), None, Some(&project));
    assert_eq!(
        payload(&policy, 3),
        json!({
            "project": PROJECT_ID,
            "checkout": "/r",
            "host": "claude-code",
            "values": {
                "effort": "max",
                "escalate": false,
                "model": "m-1",
                "only_project": true,
            },
            "sources": {
                "effort": {
                    "layer": "project-host", "path": "/r/baley.toml", "digest": "p2",
                    "line": 2, "column": 1,
                },
                "escalate": {"layer": "default"},
                "model": {
                    "layer": "project-host", "path": "/r/baley.toml", "digest": "p2",
                    "line": 3, "column": 1,
                },
                "only_project": {"layer": "default"},
            },
            "diagnostics": [],
            "catalog_version": 3,
        })
    );
}

#[test]
fn a_diagnostic_missing_its_kind_scope_or_position_is_caught() {
    let global = file(GLOBAL, "only_project = false\nnope = 1\n", "g2");
    let project = file(PROJECT, "[host.cursor]\nescalate = true\n", "p3");
    let policy = policy(None, Some(&global), Some(&project));
    assert_eq!(
        payload(&policy, 0)["diagnostics"],
        json!([
            {
                "layer": "global", "path": "/c/config.toml", "name": "only_project",
                "line": 1, "column": 1, "kind": "wrong-scope", "scope": "project",
            },
            {
                "layer": "global", "path": "/c/config.toml", "name": "nope",
                "line": 2, "column": 1, "kind": "unknown-name",
            },
            {
                "layer": "project", "path": "/r/baley.toml", "name": "host.cursor",
                "line": 1, "column": 7, "kind": "unknown-host",
            },
        ])
    );
}

#[test]
fn a_whole_file_ref_for_a_file_that_supplies_no_value_leaking_into_the_payload_is_caught() {
    let global = file(GLOBAL, "# nothing set here\n", "g-comment");
    let policy = policy(None, Some(&global), None);
    assert_eq!(
        payload(&policy, 1),
        json!({
            "project": PROJECT_ID,
            "checkout": "/r",
            "host": null,
            "values": {
                "effort": "high",
                "escalate": false,
                "model": null,
                "only_project": true,
            },
            "sources": {
                "effort": {"layer": "default"},
                "escalate": {"layer": "default"},
                "model": {"layer": "default"},
                "only_project": {"layer": "default"},
            },
            "diagnostics": [],
            "catalog_version": 1,
        })
    );
}

#[test]
fn a_checkout_path_that_is_not_utf8_converted_lossily_instead_of_refused_is_caught() {
    let checkout = Path::new(OsStr::from_bytes(b"/w/caf\xe9"));
    let refusal = recorded_policy(checkout, &policy(None, None, None)).unwrap_err();
    assert_eq!(refusal.path, checkout);
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /w/caf\u{fffd} is not UTF-8, \
         and the recorded policy holds its paths as text"
    );
}

#[test]
fn a_global_file_path_that_is_not_utf8_converted_lossily_instead_of_refused_is_caught() {
    let path = Path::new(OsStr::from_bytes(b"/c/conf\xffig.toml"));
    let global = file(path, "escalate = true\n", "g3");
    let refusal =
        recorded_policy(Path::new(CHECKOUT), &policy(None, Some(&global), None)).unwrap_err();
    assert_eq!(refusal.path, path);
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/conf\u{fffd}ig.toml is not UTF-8, \
         and the recorded policy holds its paths as text"
    );
}

#[test]
fn policy_effective_registered_at_any_version_but_1_is_caught() {
    let mut registry = Registry::new();
    register_policy_events(&mut registry).unwrap();
    assert!(registry.reads("policy.effective", 1));
    assert!(!registry.reads("policy.effective", 2));
}

fn event(seq: u64, payload: Value) -> Event {
    Event {
        project_id: ProjectId(PROJECT_ID.into()),
        seq,
        stream: "project".into(),
        stream_version: seq,
        type_name: "policy.effective".into(),
        type_version: 1,
        actor: Actor::Baley,
        recorded_at: "2026-09-30T18:00:00Z".into(),
        request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
        git: None,
        policy_version: 0,
        payload,
        prev_hash: None,
        hash: Hash([0; 32]),
    }
}

/// A hand-written payload for `checkout` and `host`, with one value.
fn stored_payload(checkout: &str, host: Value, escalate: bool) -> Value {
    json!({
        "project": PROJECT_ID,
        "checkout": checkout,
        "host": host,
        "values": {"escalate": escalate},
        "sources": {"escalate": {"layer": "default"}},
        "diagnostics": [],
        "catalog_version": 2,
    })
}

fn put(changes: Vec<Change>) -> (DocKey, Value) {
    match <[Change; 1]>::try_from(changes) {
        Ok([Change::Put { key, body }]) => (key, body),
        other => panic!("expected one put, got {other:?}"),
    }
}

fn key(checkout: &str, host: &str) -> DocKey {
    DocKey(vec![
        KeyValue::Text(checkout.into()),
        KeyValue::Text(host.into()),
    ])
}

#[test]
fn a_policy_view_keyed_by_a_field_named_host_or_holding_an_index_is_caught() {
    assert_eq!(
        policy_spec(),
        ViewSpec {
            name: "policy".into(),
            version: 1,
            key: vec![
                FieldSpec {
                    name: "checkout".into(),
                    kind: FieldKind::Text,
                },
                FieldSpec {
                    name: "host_key".into(),
                    kind: FieldKind::Text,
                },
            ],
            indexes: Vec::new(),
            page_bound: 1,
        }
    );
}

#[test]
fn a_document_whose_version_is_not_the_events_sequence_is_caught() {
    let projector = PolicyProjector::new();
    let first = event(5, stored_payload("/r", Value::Null, true));
    let (at, body) = put(projector.apply(&first, &[]).unwrap());
    assert_eq!(at, key("/r", ""));
    assert_eq!(
        body,
        json!({
            "project": PROJECT_ID,
            "checkout": "/r",
            "host": null,
            "host_key": "",
            "values": {"escalate": true},
            "sources": {"escalate": {"layer": "default"}},
            "diagnostics": [],
            "catalog_version": 2,
            "version": 5,
        })
    );
}

#[test]
fn a_later_event_merged_into_the_stored_body_instead_of_replacing_it_is_caught() {
    let projector = PolicyProjector::new();
    let (at, first) = put(projector
        .apply(&event(5, stored_payload("/r", Value::Null, true)), &[])
        .unwrap());
    // The second policy was merged over a schema that no longer has
    // `escalate`, so a merge would keep the first body's value.
    let second = json!({
        "project": PROJECT_ID,
        "checkout": "/r",
        "host": null,
        "values": {"effort": "max"},
        "sources": {"effort": {"layer": "default"}},
        "diagnostics": [],
        "catalog_version": 3,
    });
    let (again, body) = put(projector
        .apply(&event(9, second), &[(at.clone(), first)])
        .unwrap());
    assert_eq!(again, at);
    assert_eq!(
        body,
        json!({
            "project": PROJECT_ID,
            "checkout": "/r",
            "host": null,
            "host_key": "",
            "values": {"effort": "max"},
            "sources": {"effort": {"layer": "default"}},
            "diagnostics": [],
            "catalog_version": 3,
            "version": 9,
        })
    );
}

#[test]
fn two_checkouts_of_one_project_sharing_a_document_is_caught() {
    let projector = PolicyProjector::new();
    let (one, _) = put(projector
        .apply(&event(5, stored_payload("/r", Value::Null, true)), &[])
        .unwrap());
    let (two, _) = put(projector
        .apply(
            &event(6, stored_payload("/w/other", Value::Null, true)),
            &[],
        )
        .unwrap());
    assert_eq!(one, key("/r", ""));
    assert_eq!(two, key("/w/other", ""));
}

#[test]
fn the_command_line_and_a_host_on_one_checkout_sharing_a_document_is_caught() {
    let projector = PolicyProjector::new();
    let (none, _) = put(projector
        .apply(&event(5, stored_payload("/r", Value::Null, true)), &[])
        .unwrap());
    let (claude, body) = put(projector
        .apply(
            &event(6, stored_payload("/r", json!("claude-code"), true)),
            &[],
        )
        .unwrap());
    assert_eq!(none, key("/r", ""));
    assert_eq!(claude, key("/r", "claude-code"));
    assert_eq!(body["host_key"], json!("claude-code"));
    assert_eq!(policy_key("/r", None), none);
    assert_eq!(policy_key("/r", Some(Host::ClaudeCode)), claude);
}

#[test]
fn an_empty_host_taking_the_command_lines_key_is_caught() {
    let refusal = PolicyProjector::new()
        .apply(&event(4, stored_payload("/r", json!(""), true)), &[])
        .unwrap_err();
    assert_eq!(
        refusal.0,
        "policy.effective at seq 4: host is neither null nor non-empty text"
    );
}

#[test]
fn a_payload_without_a_checkout_projected_under_some_key_is_caught() {
    let mut payload = stored_payload("/r", Value::Null, true);
    payload.as_object_mut().unwrap().remove("checkout");
    let refusal = PolicyProjector::new()
        .apply(&event(4, payload), &[])
        .unwrap_err();
    assert_eq!(
        refusal.0,
        "policy.effective at seq 4: checkout is missing, not text or empty"
    );
}

/// The `policy` document the projector writes for `payload` at `seq`.
fn stored_at(seq: u64, payload: Value) -> Value {
    put(PolicyProjector::new()
        .apply(&event(seq, payload), &[])
        .unwrap())
    .1
}

/// The payload for a global and a project text with their digests.
fn judged(global: (&str, &str), project: (&str, &str), catalog_version: u64) -> Value {
    let global = file(GLOBAL, global.0, global.1);
    let project = file(PROJECT, project.0, project.1);
    payload(
        &policy(None, Some(&global), Some(&project)),
        catalog_version,
    )
}

const FIRST_GLOBAL: (&str, &str) = ("escalate = true\nnope = 1\n", "g1");
const FIRST_PROJECT: (&str, &str) = ("only_project = false\n", "p1");

fn first_record() -> Value {
    stored_at(12, judged(FIRST_GLOBAL, FIRST_PROJECT, 4))
}

#[test]
fn an_unchanged_policy_recorded_again_or_given_the_wrong_version_in_force_is_caught() {
    let payload = judged(FIRST_GLOBAL, FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Unchanged { version: 12 }
    );
}

#[test]
fn a_changed_global_value_left_unrecorded_is_caught() {
    let payload = judged(("escalate = false\nnope = 1\n", "g1"), FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Record
    );
}

#[test]
fn a_changed_project_value_left_unrecorded_is_caught() {
    let payload = judged(FIRST_GLOBAL, ("only_project = true\n", "p1"), 4);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Record
    );
}

#[test]
fn a_changed_source_digest_left_unrecorded_is_caught() {
    let payload = judged((FIRST_GLOBAL.0, "g1-new"), FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Record
    );
}

#[test]
fn a_moved_diagnostic_left_unrecorded_is_caught() {
    let payload = judged(("escalate = true\n\nnope = 1\n", "g1"), FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Record
    );
}

#[test]
fn a_changed_catalog_version_left_unrecorded_is_caught() {
    let payload = judged(FIRST_GLOBAL, FIRST_PROJECT, 5);
    assert_eq!(
        judge_policy(&payload, Some(&first_record())),
        PolicyJudgement::Record
    );
}

#[test]
fn a_first_policy_left_unrecorded_is_caught() {
    let payload = judged(FIRST_GLOBAL, FIRST_PROJECT, 4);
    assert_eq!(judge_policy(&payload, None), PolicyJudgement::Record);
}

#[test]
fn a_stored_body_missing_a_compared_member_or_a_positive_version_taken_as_current_is_caught() {
    let payload = judged(FIRST_GLOBAL, FIRST_PROJECT, 4);
    let mut missing = first_record();
    missing.as_object_mut().unwrap().remove("diagnostics");
    assert_eq!(
        judge_policy(&payload, Some(&missing)),
        PolicyJudgement::Record
    );
    let mut unversioned = first_record();
    unversioned["version"] = json!(0);
    assert_eq!(
        judge_policy(&payload, Some(&unversioned)),
        PolicyJudgement::Record
    );
}

#[test]
fn a_comment_in_a_global_file_that_sets_nothing_recording_a_new_policy_is_caught() {
    let stored = stored_at(12, judged(("# a\n", "c1"), FIRST_PROJECT, 4));
    let payload = judged(("# a\n# b\n", "c2"), FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&stored)),
        PolicyJudgement::Unchanged { version: 12 }
    );
}

#[test]
fn a_comment_in_a_global_file_that_sets_a_value_left_unrecorded_is_caught() {
    let stored = stored_at(12, judged(("escalate = true\n", "g1"), FIRST_PROJECT, 4));
    let payload = judged(("escalate = true\n# b\n", "g2"), FIRST_PROJECT, 4);
    assert_eq!(
        judge_policy(&payload, Some(&stored)),
        PolicyJudgement::Record
    );
}

#[test]
fn a_purge_in_a_checkout_of_the_named_project_skipping_the_policy_step_is_caught() {
    assert_eq!(
        purge_policy(Some(PROJECT_ID), PROJECT_ID),
        PurgePolicy::RunStep
    );
}

#[test]
fn a_purge_from_another_projects_checkout_recording_that_projects_policy_is_caught() {
    assert_eq!(
        purge_policy(Some("0b9e8d7c-6a5f-4e3d-8c2b-1a0f9e8d7c6b"), PROJECT_ID),
        PurgePolicy::VersionZero
    );
}

#[test]
fn a_purge_outside_any_checkout_running_the_policy_step_is_caught() {
    assert_eq!(purge_policy(None, PROJECT_ID), PurgePolicy::VersionZero);
}
