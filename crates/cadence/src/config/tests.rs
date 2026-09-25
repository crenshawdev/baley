use super::merge::{deep_merge, get, merge};
use super::*;
use serde_json::json;

#[test]
fn deep_merge_inherits_absent_keys_and_replaces_with_null_arrays_and_scalars() {
    let base = json!({"a":{"x":1,"y":2},"b":[1,2],"c":true});
    assert_eq!(
        deep_merge(&base, &json!({"a":{"x":null},"b":[],"c":false})),
        json!({"a":{"x":null,"y":2},"b":[],"c":false})
    );
    assert_eq!(deep_merge(&base, &json!({"a":null}))["a"], Value::Null);
}

#[test]
fn merge_keeps_both_raw_layers_records_each_winning_layer_and_fills_defaults() {
    let global = json!({"roles":{"cad-executor":{"model":"one","effort":"high"},"cad-planner":{"effort":"max"}},
        "git":{"protected_branches":["main","stable"]}});
    let repo = json!({"roles":{"cad-executor":{"model":null}},"git":{"protected_branches":[]}});
    let merged = merge(Some(global.clone()), Some(repo.clone()), false);
    assert_eq!(merged.raw_global, Some(global));
    assert_eq!(merged.raw_repo, Some(repo));
    assert_eq!(
        get(&merged.values, "roles.cad-executor.model"),
        Some(&Value::Null)
    );
    assert_eq!(
        get(&merged.values, "roles.cad-executor.effort"),
        Some(&json!("high"))
    );
    assert_eq!(
        get(&merged.values, "roles.cad-planner.effort"),
        Some(&json!("max"))
    );
    assert_eq!(
        get(&merged.values, "git.protected_branches"),
        Some(&json!([]))
    );
    assert_eq!(merged.sources["roles.cad-executor.model"], Layer::Repo);
    assert_eq!(merged.sources["roles.cad-planner.effort"], Layer::Global);
    assert!(get(&merged.repo, "workflow.verifier").is_none());
    assert_eq!(get(&merged.values, "workflow.verifier"), Some(&json!(true)));
}

#[test]
fn global_only_settings_cannot_be_suppressed_by_repo_null_or_ancestors() {
    let global = json!({"workflow":{"test_command":"trusted","lint_command":"lint"},"review":{"key_file":"keys"}});
    for ancestor in [Value::Null, json!(false), json!([]), json!("blocked")] {
        let result = merge(
            Some(global.clone()),
            Some(json!({"workflow":ancestor,"review":ancestor})),
            false,
        );
        for key in GLOBAL_ONLY {
            assert_eq!(get(&result.values, key), get(&global, key));
        }
        assert_eq!(result.diagnostics.scope.is_empty(), ancestor.is_null());
    }
    let repo = json!({"workflow":{"test_command":null,"lint_command":"unsafe"},"review":{"key_file":null}});
    let result = merge(Some(global.clone()), Some(repo.clone()), false);
    for key in GLOBAL_ONLY {
        assert_eq!(get(&result.values, key), get(&global, key));
    }
    assert_eq!(result.diagnostics.scope.len(), 1);
    let explicit = merge(None, Some(repo), true);
    assert_eq!(
        get(&explicit.values, "workflow.lint_command"),
        Some(&json!("unsafe"))
    );
    assert_eq!(explicit.sources["workflow.lint_command"], Layer::Repo);
    assert!(explicit.global_intent);
}

#[test]
fn invalid_scope_and_migration_diagnostics_are_independent() {
    let result = merge(
        Some(json!(false)),
        Some(
            json!({"workflow":{"test_command":"bad"},"git":{"auto_close":false},"unrecognized":{"x":1}}),
        ),
        false,
    );
    assert_eq!(result.diagnostics.invalid_layer.len(), 1);
    assert_eq!(result.diagnostics.scope.len(), 1);
    assert_eq!(result.diagnostics.migration.len(), 2);
    assert!(get(&result.values, "unrecognized").is_none());
    assert_eq!(result.raw_repo.unwrap()["unrecognized"], json!({"x":1}));
    let slug = merge(
        Some(json!({"git":{"forge_repo":"owner/repo"}})),
        None,
        false,
    );
    assert_eq!(slug.sources["git.forge_repo"], Layer::Global);
    assert_eq!(slug.diagnostics.scope[0].key, "git.forge_repo");
}

#[test]
fn capture_threshold_reports_active_identities_without_refusing_append() {
    use cadence::store::model::{Disposition, Evidence, ItemRecord, Origin, VERSION};
    let mut records = Vec::new();
    for id in 0..5 {
        records.push(ItemRecord {
            version: VERSION,
            id: id.to_string(),
            revision: 1,
            origin: Origin {
                source: "capture".into(),
                original: Evidence::Missing,
            },
            text: "same text\nwith continuation".into(),
            kind: "todo".into(),
            phase: None,
            completed: false,
            disposition: Disposition::Captured,
            filing_uncertain: false,
        });
    }
    let mut revision = records[0].clone();
    revision.revision = 2;
    records.push(revision);
    records[1].completed = true;
    records[2].disposition = Disposition::Filed {
        pointer: "GH-1".into(),
    };
    records[3].disposition = Disposition::Declined {
        reason: "no".into(),
    };
    let report = capture_report(&records, 1);
    assert_eq!(
        report,
        CaptureReport {
            active: 2,
            bound: 1,
            exceeded: true,
            unit: "items"
        }
    );
}

fn config_paths(dir: &std::path::Path) -> reload::Paths {
    reload::Paths {
        global: Some(dir.join("global.json")),
        repo: dir.join("repo.json"),
    }
}
fn write_json(path: &std::path::Path, value: Value) {
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn reload_reads_a_byte_change_at_equal_size_and_modification_time() {
    use std::fs::{File, FileTimes};
    let dir = tempfile::tempdir().unwrap();
    let paths = config_paths(dir.path());
    std::fs::write(&paths.repo, b"{\"git\":{\"on_protected\":\"allow\"}}").unwrap();
    let mut reader = reload::Reload::new(paths.clone(), reload::FileIo);
    let old = reader.refresh().unwrap();
    let time = std::fs::metadata(&paths.repo).unwrap().modified().unwrap();
    std::fs::write(&paths.repo, b"{\"git\":{\"on_protected\":\"refuse\"}}").unwrap();
    // Equal-size change with the exact same mtime, not a coarse sleep-based test.
    std::fs::write(&paths.repo, b"{\"git\":{\"on_protected\":\"ask\"}}  ").unwrap();
    File::open(&paths.repo)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time))
        .unwrap();
    let changed = reader.refresh().unwrap();
    assert_eq!(
        old.repo.bytes.as_ref().unwrap().len(),
        changed.repo.bytes.as_ref().unwrap().len()
    );
    assert!(changed.number > old.number);
    assert_eq!(
        get(&changed.effective.values, "git.on_protected"),
        Some(&json!("ask"))
    );
}

#[test]
fn reload_reads_a_rename_with_identical_bytes_as_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let paths = config_paths(dir.path());
    std::fs::write(&paths.repo, b"{\"git\":{\"on_protected\":\"ask\"}}").unwrap();
    let mut reader = reload::Reload::new(paths.clone(), reload::FileIo);
    let before = reader.refresh().unwrap();
    let replacement = dir.path().join("checkout");
    std::fs::write(&replacement, before.repo.bytes.as_ref().unwrap()).unwrap();
    std::fs::rename(replacement, &paths.repo).unwrap();
    let renamed = reader.refresh().unwrap();
    assert!(renamed.number > before.number);
    assert_ne!(renamed.repo.stamp, before.repo.stamp);
}

#[test]
fn reload_reads_a_removed_layer_as_absent_in_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let paths = config_paths(dir.path());
    std::fs::write(&paths.repo, b"{\"git\":{\"on_protected\":\"ask\"}}").unwrap();
    let mut reader = reload::Reload::new(paths.clone(), reload::FileIo);
    let present = reader.refresh().unwrap();
    std::fs::remove_file(&paths.repo).unwrap();
    let absent = reader.refresh().unwrap();
    assert!(absent.repo.bytes.is_none());
    assert!(absent.number > present.number);
}

/// The file seam, counting its reads.
struct CountedIo {
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl reload::ConfigIo for CountedIo {
    fn identity(&mut self, path: &std::path::Path) -> cadence::store::Result<std::path::PathBuf> {
        reload::ConfigIo::identity(&mut reload::FileIo, path)
    }
    fn read(&mut self, path: &std::path::Path) -> cadence::store::Result<reload::Input> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        reload::ConfigIo::read(&mut reload::FileIo, path)
    }
}

/// A repo config and a global path linked to it, read through a counted seam.
fn aliased(
    dir: &std::path::Path,
) -> (reload::Reload<CountedIo>, std::sync::Arc<std::sync::atomic::AtomicUsize>, reload::Paths) {
    use std::os::unix::fs::symlink;
    let paths = config_paths(dir);
    write_json(
        &paths.repo,
        json!({"workflow":{"verifier":false,"test_command":"repo-command"}}),
    );
    symlink(&paths.repo, paths.global.as_ref().unwrap()).unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reader = reload::Reload::new(paths.clone(), CountedIo { reads: count.clone() });
    (reader, count, paths)
}

#[test]
fn a_global_path_aliasing_the_repo_file_is_read_once_as_the_repo_layer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut reader, count, _) = aliased(dir.path());
    let shared = reader.refresh().unwrap();
    assert!(shared.global.is_none());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(shared.effective.sources["workflow.verifier"], Layer::Repo);
    assert_eq!(
        get(&shared.effective.values, "workflow.test_command"),
        Some(&json!("repo-command"))
    );
}

#[test]
fn a_retargeted_global_link_is_resolved_again_and_separates_the_layers() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let (mut reader, count, paths) = aliased(dir.path());
    let shared = reader.refresh().unwrap();
    let other = dir.path().join("other.json");
    write_json(&other, json!({"workflow":{"test_command":"trusted"}}));
    std::fs::remove_file(paths.global.as_ref().unwrap()).unwrap();
    symlink(&other, paths.global.as_ref().unwrap()).unwrap();
    let separated = reader.refresh().unwrap();
    assert!(separated.global.is_some());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(
        get(&separated.effective.values, "workflow.test_command"),
        Some(&json!("trusted"))
    );
    assert!(separated.number > shared.number);
}

#[test]
fn a_missing_file_identity_is_its_name_under_the_canonical_parent() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        reload::identity(&dir.path().join("missing.json")).unwrap(),
        dir.path().join("missing.json")
    );
}

#[test]
fn malformed_and_nonobject_layers_are_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let paths = config_paths(dir.path());
    let mut reader = reload::Reload::new(paths.clone(), reload::FileIo);
    for bytes in [
        "{",
        "null",
        "false",
        "[]",
        "{\"git\":{\"on_protected\":\"nonsense\"}}",
        "{\"git\":null}",
    ] {
        std::fs::write(&paths.repo, bytes).unwrap();
        assert!(reader.refresh().is_err(), "{bytes}");
    }
    write_json(&paths.repo, json!({"workflow":{"verifier":true}}));
    assert!(reader.refresh().is_ok());
}

/// A layer the reader cannot read makes the config unavailable, and the same
/// reader recovers once the layer reads again.
#[test]
fn a_layer_that_cannot_be_read_is_unavailable_until_it_reads_again() {
    struct Denied {
        denied: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl reload::ConfigIo for Denied {
        fn identity(&mut self, path: &std::path::Path) -> cadence::store::Result<std::path::PathBuf> {
            Ok(path.into())
        }
        fn read(&mut self, path: &std::path::Path) -> cadence::store::Result<reload::Input> {
            if self.denied.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(cadence::store::Error::Io(format!("config {} is unreadable", path.display())));
            }
            Ok(reload::Input { identity: path.into(), bytes: Some(br#"{"workflow":{"verifier":true}}"#.to_vec()), stamp: None })
        }
    }
    let denied = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut reader = reload::Reload::new(
        config_paths(std::path::Path::new("/project")),
        Denied { denied: denied.clone() },
    );
    assert!(reader.refresh().is_err());
    denied.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(reader.refresh().is_ok());
}

/// Root can open a file whose mode grants no read to anyone, so the mode, not
/// the open, decides that a layer is unreadable.
#[test]
fn a_mode_without_any_read_bit_is_unreadable() {
    for mode in [0o000, 0o200, 0o222, 0o111, 0o100_000] {
        assert!(reload::unreadable_mode(mode), "{mode:o}");
    }
    for mode in [0o400, 0o040, 0o004, 0o644, 0o100_600] {
        assert!(!reload::unreadable_mode(mode), "{mode:o}");
    }
}

#[test]
fn explicit_config_writes_validate_frozen_types_and_forge_grammars() {
    for (key, good, bad) in [
        (
            "git.forge_repo",
            json!("org/group/repo"),
            json!("org/../repo"),
        ),
        (
            "git.forge_host",
            json!("Example.com:65535"),
            json!("example.com:01"),
        ),
        ("planning.max_capture_bullets", json!(1), json!(0)),
        ("review.request_timeout_ms", json!(600000), json!(600001)),
        ("git.protected_branches", json!(["main"]), json!([1])),
        (
            "review.triggers.risk_surface.surfaces",
            json!([]),
            Value::Null,
        ),
    ] {
        assert!(
            write::validate_update(Layer::Repo, key, &good).is_ok(),
            "{key}"
        );
        assert!(
            write::validate_update(Layer::Repo, key, &bad).is_err(),
            "{key}"
        );
    }
    for bad in ["x", "-org/repo", "org/re po", "org/", "org/.."] {
        assert!(write::validate_update(Layer::Repo, "git.forge_repo", &json!(bad)).is_err());
    }
    for bad in [
        "-host",
        "host-",
        "host:0",
        "host:65536",
        "host:",
        "host:abc",
        "a..b",
    ] {
        assert!(write::validate_update(Layer::Repo, "git.forge_host", &json!(bad)).is_err());
    }
    assert!(
        write::validate_update(
            Layer::Repo,
            "git.forge_repo",
            &json!(format!("a/{}", "x".repeat(199)))
        )
        .is_err()
    );
    assert!(
        write::validate_update(Layer::Repo, "git.forge_host", &json!("a".repeat(254))).is_err()
    );
}

#[test]
fn guard_hard_fail_is_a_strict_bool_defaulting_to_false() {
    let spec = &schema()["git.guard_hard_fail"];
    assert_eq!(spec["default"], false);
    for value in [json!(false), json!(true)] {
        assert!(reload::valid_type(spec, &value, false));
    }
    for value in [
        Value::Null,
        json!(0),
        json!(1),
        json!("true"),
        json!([]),
        json!({}),
    ] {
        assert!(!reload::valid_type(spec, &value, false));
        assert!(
            reload::validate_effective(&merge(
                None,
                Some(json!({"git":{"guard_hard_fail":value}})),
                false
            ))
            .is_err()
        );
    }
    assert_eq!(
        get(&merge(None, None, false).values, "git.guard_hard_fail"),
        Some(&json!(false))
    );
}

#[test]
fn batch_preparation_preserves_unknown_values_and_distinguishes_absent_null() {
    use write::{Update, prepare_batch};
    let updates = [
        Update {
            key: "roles.cad-executor.model".into(),
            value: Value::Null,
        },
        Update {
            key: "roles.cad-executor.effort".into(),
            value: json!("xhigh"),
        },
    ];
    assert_eq!(
        prepare_batch(Layer::Repo, &json!({"unknown":{"saved":7}}), &updates).unwrap(),
        (
            json!({"unknown":{"saved":7},"roles":{"cad-executor":{"model":null,"effort":"xhigh"}}}),
            vec![
                "roles.cad-executor.effort".to_string(),
                "roles.cad-executor.model".to_string()
            ]
        )
    );
}

#[test]
fn batch_preparation_returns_literal_refusals_for_invalid_tail_and_duplicates() {
    use write::{Update, prepare_batch};
    for (key, value, reason) in [
        (
            "roles.cad-executor.effort",
            json!("impossible"),
            "invalid value for roles.cad-executor.effort",
        ),
        ("stakes", json!("high"), "unknown config key stakes"),
        (
            "workflow.test_command",
            json!("test"),
            "wrong config layer for workflow.test_command",
        ),
        (
            "roles.cad-executor.model",
            Value::Null,
            "duplicate config key roles.cad-executor.model",
        ),
    ] {
        let updates = [
            Update {
                key: "roles.cad-executor.model".into(),
                value: json!("sonnet"),
            },
            Update {
                key: key.into(),
                value,
            },
        ];
        assert_eq!(
            prepare_batch(Layer::Repo, &json!({}), &updates),
            Err(cadence::store::Error::Invalid(reason.into()))
        );
    }
}

#[test]
fn batch_preparation_empty_and_identical_stored_values_return_no_changes() {
    use write::{Update, prepare_batch};
    assert_eq!(
        prepare_batch(Layer::Repo, &json!({"a":1}), &[]).unwrap(),
        (json!({"a":1}), vec![])
    );
    assert_eq!(
        prepare_batch(
            Layer::Repo,
            &json!({"roles":{"cad-executor":{"model":null}}}),
            &[Update {
                key: "roles.cad-executor.model".into(),
                value: Value::Null
            }]
        )
        .unwrap(),
        (json!({"roles":{"cad-executor":{"model":null}}}), vec![])
    );
}
