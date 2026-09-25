use super::*;
use cadence::store::{
    MutationContext, Policy, Result,
    transaction::Transaction,
    writer::{Operation, Store},
};
use serde_json::json;

#[test]
fn a_store_crossing_refuses_session_input() {
    let error = store_input_error(cadence::acquisition::Error::Crossing(cadence::acquisition::Crossing {
        file: ".planning/state.json".into(), size: 1_073_741_825, bound: 1_073_741_824,
    }));
    assert!(matches!(error, cadence::store::Error::Invalid(reason)
        if reason == ".planning/state.json: size 1073741825 exceeds acquisition bound 1073741824"));
}

struct Allow;
impl Policy for Allow {
    fn validate(&mut self, _: &MutationContext<'_>) -> Result<()> {
        Ok(())
    }
}
/// A session's current view: import metadata and unrelated data.
fn session_view() -> View {
    let manifest: ImportManifest = serde_json::from_value(json!({"format":1,"complete":true,
        "source_generation":"fixture","sources":[],"active":{"repo":"/fixture/project/.planning/config.v4.json","global":null},
        "created":[],"warnings":[]}))
    .unwrap();
    View {
        items: vec![],
        decisions: vec![],
        snapshot: cadence::store::model::Snapshot::new(
            7,
            b"",
            b"",
            json!({"import":manifest,"unrelated":{"keep":true}}),
        )
        .unwrap(),
    }
}

#[test]
fn a_derivation_may_replace_everything_but_import_and_layers() {
    let current = session_view();
    let mut data = current.snapshot.data.clone();
    data["unrelated"] = json!({"keep":[1,2]});
    data["current"] = json!({"legacy":"unchanged"});
    data["derivation"] = json!({"memo":"fixture"});
    assert_eq!(check_derivation(&current, &current, &data), Ok(()));
}

#[test]
fn a_derivation_may_not_change_import_or_layers() {
    let current = session_view();
    for field in ["import", LAYERS] {
        let mut data = current.snapshot.data.clone();
        data[field] = Value::Null;
        assert!(check_derivation(&current, &current, &data).is_err(), "{field}");
    }
}

#[test]
fn a_derivation_from_a_stale_view_is_refused() {
    let current = session_view();
    let mut stale = current.clone();
    stale.snapshot = cadence::store::model::Snapshot::new(6, b"", b"", current.snapshot.data.clone()).unwrap();
    assert_eq!(
        check_derivation(&current, &stale, &current.snapshot.data),
        Err(Error::Conflict(cadence::store::writer::STALE_SNAPSHOT.into()))
    );
}

fn first_run_answers() -> Vec<write::Update> {
    serde_json::from_value(json!([
        {"key":"roles.cad-planner.model","value":null},
        {"key":"roles.cad-planner.effort","value":"high"},
        {"key":"roles.cad-assumptions-analyzer.model","value":null},
        {"key":"roles.cad-assumptions-analyzer.effort","value":"high"},
        {"key":"roles.cad-verifier.model","value":null},
        {"key":"roles.cad-verifier.effort","value":"high"},
        {"key":"roles.cad-reviewer.model","value":null},
        {"key":"roles.cad-reviewer.effort","value":"medium"},
        {"key":"roles.cad-executor.model","value":null},
        {"key":"roles.cad-executor.effort","value":"high"},
        {"key":"roles.cad-plan-checker.model","value":null},
        {"key":"roles.cad-plan-checker.effort","value":"low"},
        {"key":"review.triggers.risk_surface.waive_routing_floor","value":[]}
    ]))
    .unwrap()
}

#[tokio::test]
async fn first_global_batch_returns_thirteen_leaves_from_missing_parent_registration() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("project/.planning");
    let active = Paths {
        repo: root.join("config.v4.json"),
        global: Some(fixture.path().join("global/nested/config.v4.json")),
    };
    let storage = write::register(&root, &active).unwrap();
    let writer = write::ConfigWriter {
        root,
        active: active.clone(),
        store: Store::open(storage, Allow).await.unwrap(),
        config: Arc::new(Mutex::new(Reload::new(active, FileIo))),
    };
    let result = writer
        .batch(Layer::Global, &first_run_answers())
        .await
        .unwrap();
    assert_eq!(
        result.changed_keys,
        [
            "review.triggers.risk_surface.waive_routing_floor",
            "roles.cad-assumptions-analyzer.effort",
            "roles.cad-assumptions-analyzer.model",
            "roles.cad-executor.effort",
            "roles.cad-executor.model",
            "roles.cad-plan-checker.effort",
            "roles.cad-plan-checker.model",
            "roles.cad-planner.effort",
            "roles.cad-planner.model",
            "roles.cad-reviewer.effort",
            "roles.cad-reviewer.model",
            "roles.cad-verifier.effort",
            "roles.cad-verifier.model",
        ]
    );
}

#[derive(Clone)]
struct SuppliedConfig(BTreeMap<PathBuf, Vec<u8>>);
impl ConfigIo for SuppliedConfig {
    fn identity(&mut self, path: &Path) -> Result<std::path::PathBuf> {
        Ok(path.into())
    }
    fn read(&mut self, path: &Path) -> Result<Input> {
        Ok(Input {
            identity: path.into(),
            bytes: self.0.get(path).cloned(),
            stamp: None,
        })
    }
}

#[test]
fn prepare_import_uses_active_roles_over_the_legacy_global() {
    let root = Path::new("/fixture/project/.planning");
    let legacy = Paths {
        repo: root.join("config.json"),
        global: Some("/fixture/global/config.json".into()),
    };
    let active = Paths {
        repo: root.join("config.v4.json"),
        global: Some("/fixture/global/config.v4.json".into()),
    };
    let original = br#"{"roles":{"cad-executor":{"model":"opus"}},"stakes":{"old":3}}"#;
    let mut io = SuppliedConfig(
        [
            (legacy.global.clone().unwrap(), original.to_vec()),
            (
                active.global.clone().unwrap(),
                br#"{"roles":{"cad-executor":{"model":"sonnet"}}}"#.to_vec(),
            ),
        ]
        .into(),
    );
    let result = prepare_import(root, &legacy, &active, &mut io, false).unwrap();
    assert_eq!(
        result.generation.effective.raw_global,
        Some(json!({"roles":{"cad-executor":{"model":"sonnet"}}}))
    );
}

#[test]
fn register_missing_global_parent_creates_infrastructure_without_config_pins() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("project/.planning");
    let active = Paths {
        repo: root.join("config.v4.json"),
        global: Some(fixture.path().join("global/nested/config.v4.json")),
    };
    let mut result = write::register(&root, &active).unwrap();
    assert_eq!(
        (
            result.read("repo-config").unwrap().bytes,
            result.read("global-config").unwrap().bytes
        ),
        (None, None)
    );
}

#[test]
fn register_refuses_symlink_ancestors() {
    let fixture = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(fixture.path(), fixture.path().join("alias")).unwrap();
    let active = Paths {
        repo: fixture.path().join("config.v4.json"),
        global: Some(fixture.path().join("alias/nested/config.v4.json")),
    };
    assert_eq!(
        write::register(fixture.path(), &active).err(),
        Some(Error::Conflict("unsafe directory identity".into()))
    );
}

#[test]
fn prepare_import_refuses_unusable_active_global_without_legacy_normalization() {
    let root = Path::new("/fixture/project/.planning");
    let legacy = Paths {
        repo: root.join("config.json"),
        global: Some("/fixture/global/config.json".into()),
    };
    let active = Paths {
        repo: root.join("config.v4.json"),
        global: Some("/fixture/global/config.v4.json".into()),
    };
    let mut io = SuppliedConfig(
        [(
            active.global.clone().unwrap(),
            br#"{"roles":{"cad-executor":{"effort":"invalid"}}}"#.to_vec(),
        )]
        .into(),
    );
    assert_eq!(
        prepare_import(root, &legacy, &active, &mut io, false).err(),
        Some(Error::Policy(
            "config unavailable: unusable roles.cad-executor.effort".into()
        ))
    );
}

#[test]
fn snapshot_replacement_preserves_the_import_manifest_and_unrelated_namespaces() {
    let previous = json!({"import":{"complete":true},"evidence":{"keep":1},
        "derivation":{"keep":2},"execution":{"keep":3},"rail_receipts":{"keep":4}});
    assert_eq!(
        replace_current(&previous, json!({"new":"payload"})),
        Ok(json!({
            "import":{"complete":true},"evidence":{"keep":1},
            "derivation":{"keep":2},"execution":{"keep":3},"rail_receipts":{"keep":4},
            "current":{"new":"payload"}
        }))
    );
}

#[test]
fn snapshot_replacement_keeps_a_wrapped_import_manifest_at_its_original_location() {
    assert_eq!(
        replace_current(
            &json!({"import":{"complete":true},
        "current":{"import":{"old":null},"unrelated":[1,2]}}),
            json!({"answer":13})
        ),
        Ok(
            json!({"import":{"complete":true},"current":{"import":{"old":null},
            "unrelated":[1,2],"current":{"answer":13}}})
        )
    );
}

#[test]
fn session_rewrite_returns_the_preserved_import_manifest() {
    let current = session_view();
    let Operation::CompareRewriteSnapshot { expected_generation, data, .. } =
        conditional(&current, Operation::RewriteSnapshot(json!({"answer":13}))).unwrap()
    else {
        panic!("a snapshot rewrite was not pinned to the current snapshot")
    };
    assert_eq!(
        (expected_generation, data["import"].clone(), data["current"].clone(), data["unrelated"].clone()),
        (7, current.snapshot.data["import"].clone(), json!({"answer":13}), json!({"keep":true}))
    );
}

#[test]
fn session_transaction_snapshot_returns_the_preserved_import_manifest() {
    let current = session_view();
    let transaction = Transaction {
        id: "snapshot-input".into(),
        items: vec![],
        decisions: vec![],
        snapshot: Some(json!({"answer":13})),
        external: vec![],
    };
    let Operation::CompareTransact { expected_generation, transaction, .. } =
        conditional(&current, Operation::Transact(transaction)).unwrap()
    else {
        panic!("a transaction carrying a snapshot was not pinned to the current snapshot")
    };
    let data = transaction.snapshot.unwrap();
    assert_eq!(
        (expected_generation, data["import"].clone(), data["current"].clone()),
        (7, current.snapshot.data["import"].clone(), json!({"answer":13}))
    );
}

#[test]
fn session_conditional_rewrite_returns_exact_stale_generation_refusal() {
    let current = session_view();
    assert_eq!(
        cadence::store::writer::precondition(&current.snapshot, 6, "stale-generation"),
        Err(Error::Conflict("conditional snapshot precondition changed".into()))
    );
}
