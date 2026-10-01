//! The recorded policy's decisions on supplied values. Expected payloads are
//! written out from design 0003 section 6 and the event's documented shape,
//! never from running this code.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use baley_store::EventSchema;
use serde_json::json;

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
