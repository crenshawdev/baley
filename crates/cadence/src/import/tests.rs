use super::*;
use cadence::store::{
    MutationContext, Policy, Result,
    filesystem::Filesystem,
    model::{Disposition, Evidence},
    transaction::Transaction,
    writer::{Operation, Store},
};
use serde_json::json;

fn source(path: &str, text: &str) -> Source {
    Source {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
    }
}
struct Allow;
impl Policy for Allow {
    fn validate(&mut self, _: &MutationContext<'_>) -> Result<()> {
        Ok(())
    }
}
fn frozen(path: &str) -> Vec<u8> {
    let out = std::process::Command::new("git")
        .current_dir(repository_root())
        .args(["show", &format!("v3.7.12:{path}")])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

#[test]
fn capture_import_preserves_independent_identities_and_all_source_bytes() {
    let capture = source(
        "CAPTURE.md",
        "# Capture\n\n## Todos\n- [ ] (phase 03.1) same\n  continuation with unknown meaning\n  ````rust\n- [ ] not an item\n  ```\n## Notes\n  ````\n- [ ] same\n- [x] complete\n## Seeds\n- seed\n## Notes\n- 2026-01-01 note\n## Unknown\n- evidence only\n",
    );
    let result = items::translate(Some(&capture), None, None).unwrap();
    assert_eq!(result.records.len(), 5);
    assert_eq!(
        result
            .records
            .iter()
            .map(|r| r.kind.as_str())
            .collect::<Vec<_>>(),
        ["todo", "todo", "todo", "seed", "note"]
    );
    assert_eq!(result.records[0].text, "same");
    assert_eq!(result.records[1].text, "same");
    assert_ne!(result.records[0].id, result.records[1].id);
    assert!(result.records[2].completed);
    assert_eq!(result.evidence[0].source, capture);
    assert_eq!(result.evidence[0].label, "non_effective_original_source");
    let Evidence::Text(provenance) = &result.records[0].origin.original else {
        panic!("missing provenance")
    };
    let provenance: serde_json::Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(provenance[0]["phase_spelling"], json!("03.1"));
    assert!(
        result
            .records
            .iter()
            .all(|r| !r.text.contains("continuation")
                && !r.text.contains("not an item")
                && !r.text.contains("evidence only"))
    );
    assert_eq!(
        result,
        items::translate(Some(&capture), None, None).unwrap()
    );
    assert!(
        items::translate(None, None, None)
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn cross_ledger_decline_wins_preserves_uncertainty_and_replay_does_not_append() {
    let filed = source(
        "FILED.md",
        "- 2026-01-01 github org/repo abc: shared finding\n- 2026-01-02 github org/repo def unconfirmed: uncertain finding\n",
    );
    let declined = source(
        "DECLINED.md",
        "## Fingerprints\n- 2026-01-03 github org/repo abc: rejected shared finding\n## Decisions\n### Human decline\nDeclined because the tradeoff is wrong.\n## Nested reasoning\nKeep all of this.\n```md\n### not a separate decision\n```\n",
    );
    let translated = items::translate(None, Some(&filed), Some(&declined)).unwrap();
    assert_eq!(translated.records.len(), 4);
    assert!(translated.records[1].filing_uncertain); // frozen planning-files.mjs:1366-1367
    assert!(!translated.records[0].filing_uncertain);
    assert_eq!(translated.records[0].id, translated.records[2].id);
    assert!(matches!(
        translated.records[2].disposition,
        Disposition::Declined { .. }
    ));
    assert!(
        translated
            .warnings
            .iter()
            .any(|w| w.contains("FILED/DECLINED conflict"))
    );
    let Disposition::Declined { reason } = &translated.records[3].disposition else {
        panic!("prose not declined")
    };
    assert!(reason.contains("Nested reasoning") && reason.contains("tradeoff"));
    assert_eq!(
        translated
            .evidence
            .iter()
            .map(|e| e.source.clone())
            .collect::<Vec<_>>(),
        [filed, declined]
    );
    let transaction = Transaction {
        id: "same-source-generation".into(),
        items: translated.records.clone(),
        decisions: vec![],
        snapshot: Some(json!({"source_evidence":translated.evidence})),
        external: vec![],
    };
    let dir = tempfile::tempdir().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = Store::open(Filesystem::new(dir.path()).unwrap(), Allow)
            .await
            .unwrap();
        let first = store
            .request(Operation::Transact(transaction.clone()))
            .await
            .unwrap();
        assert_eq!(
            first
                .recall_items()
                .iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>(),
            ["uncertain finding"]
        );
        assert_eq!(
            store
                .request(Operation::Transact(transaction))
                .await
                .unwrap(),
            first
        );
        assert_eq!(first.items.len(), 4);
    });
}

#[test]
fn actual_frozen_declines_include_authored_decisions_and_exclude_them_from_recall() {
    let declined = Source {
        path: "DECLINED.md".into(),
        bytes: frozen(".planning/DECLINED.md"),
    };
    let filed = Source {
        path: "FILED.md".into(),
        bytes: frozen(".planning/FILED.md"),
    };
    let result = items::translate(None, Some(&filed), Some(&declined)).unwrap();
    assert_eq!(
        result
            .records
            .iter()
            .filter(|r| r.kind == "decline_decision")
            .count(),
        8
    );
    assert!(
        result
            .records
            .iter()
            .any(|r| r.text.starts_with("/cad-stakes:"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("FILED/DECLINED conflict"))
    );
    assert_eq!(result.evidence[1].source.bytes, declined.bytes);
    let invalid = Source {
        path: "CAPTURE.md".into(),
        bytes: vec![0xff, 0x00],
    };
    let result = items::translate(Some(&invalid), None, None).unwrap();
    assert!(result.records.is_empty());
    assert_eq!(result.evidence[0].source, invalid);
}

#[test]
fn frozen_cursor_survives_restart_without_deriving_phase_status() {
    let state = Source {
        path: "STATE.md".into(),
        bytes: frozen(".planning/STATE.md"),
    };
    let translated = decisions::translate(Some(&state), None, None).unwrap();
    assert_eq!(translated.cursor["phase"], json!(1.0));
    assert_eq!(translated.cursor["total"], json!(0));
    assert_eq!(translated.cursor["name"], json!("no active cycle"));
    assert_eq!(translated.cursor["status"], json!("ready to plan"));
    assert_eq!(translated.cursor["next"], json!("/cad-phase add"));
    assert_eq!(
        translated.cursor["original_fields"]["phase"],
        json!("1 of 0 (no active cycle)")
    );
    assert_eq!(translated.evidence[0].source, state);
    let dir = tempfile::tempdir().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = Store::open(Filesystem::new(dir.path()).unwrap(), Allow)
            .await
            .unwrap();
        let first = store
            .request(Operation::RewriteSnapshot(
                json!({"cursor":translated.cursor,"source_evidence":translated.evidence}),
            ))
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open(Filesystem::new(dir.path()).unwrap(), Allow)
            .await
            .unwrap();
        assert_eq!(reopened.request(Operation::Read).await.unwrap(), first);
    });
}

#[test]
fn mixed_legacy_logs_admit_only_decisions_and_keep_requested_observed_effort_distinct() {
    use cadence::store::model::Decision;
    let mut raw = String::new();
    let routing = json!({"family":"routing","event":"resolve","phase":"03.1","agent":"cad-executor-high","role":"cad-executor","effort":"high","model_source":"repo","agent_id":"a","observed_effort":" \t "});
    for row in [
        routing.clone(),
        json!({"family":"routing","event":"resolve","phase":3,"agent":"cad-executor-high","agent_id":"b","effort":"high","observed_effort":"host-unfamiliar"}),
        json!({"family":"outcome","event":"risk_check","phase":3,"checked":false,"inconclusive":true,"reason":"unresolved-range"}),
        json!({"family":"outcome","event":"census_undeclared","phase":3,"censuses":["one"]}),
        json!({"family":"read","event":"recall","tokens":100}),
        json!({"family":"lifecycle","event":"dispatch","tokens":200}),
        json!({"family":"lifecycle","event":"record_rotated"}),
        json!({"family":"routing","event":"resolve","phase":null,"agent":"cad-executor-high"}),
        json!({"family":"outcome","event":"unknown"}),
    ] {
        raw.push_str(&format!("{row}\n"));
    }
    raw.push_str("broken row\n{\"family\":\"outcome\"");
    let current = source("trace.jsonl", &raw);
    let rotated = source("trace.1.jsonl", &format!("{routing}\n"));
    let result = decisions::translate(None, Some(&current), Some(&rotated)).unwrap();
    assert_eq!(result.records.len(), 5); // equal payload in another source is not proof of a carry
    assert_ne!(result.records[0].id, result.records[4].id);
    let Decision::Routing {
        requested_effort,
        observed_effort,
        receipt,
        ..
    } = &result.records[0].decision
    else {
        panic!("routing missing")
    };
    assert_eq!(*requested_effort, Evidence::Text("high".into()));
    assert_eq!(*observed_effort, Evidence::Missing);
    assert_eq!(*receipt, Evidence::Missing);
    assert!(
        !serde_json::to_string(&result.records[0])
            .unwrap()
            .contains("observed_effort")
    );
    let Decision::Routing {
        observed_effort, ..
    } = &result.records[1].decision
    else {
        panic!("routing missing")
    };
    assert_eq!(*observed_effort, Evidence::Text("host-unfamiliar".into()));
    assert!(matches!(result.records[2].decision, Decision::Gate { .. }));
    assert!(matches!(
        result.records[3].decision,
        Decision::Refusal { .. }
    ));
    assert!(
        result
            .warnings
            .iter()
            .any(|s| s.contains("malformed/incomplete"))
    );
    assert_eq!(result.evidence[0].source, current);
    assert_eq!(
        result.records,
        decisions::translate(None, Some(&current), Some(&rotated))
            .unwrap()
            .records
    );
    let transaction = Transaction {
        id: "log-source-generation".into(),
        items: vec![],
        decisions: result.records,
        snapshot: None,
        external: vec![],
    };
    let dir = tempfile::tempdir().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = Store::open(Filesystem::new(dir.path()).unwrap(), Allow)
            .await
            .unwrap();
        let first = store
            .request(Operation::Transact(transaction.clone()))
            .await
            .unwrap();
        assert_eq!(
            store
                .request(Operation::Transact(transaction))
                .await
                .unwrap(),
            first
        );
    });
}

#[test]
fn proven_rotation_copy_coalesces_events_and_retains_both_origins() {
    let anchor = json!({"family":"lifecycle","event":"phase_start","phase":3,"corr":"a"});
    let gate = json!({"family":"outcome","event":"risk_check","phase":3,"corr":"a","checked":true});
    let rotated = source("trace.1.jsonl", &format!("{anchor}\n{gate}\n{gate}\n"));
    let marker = json!({"family":"lifecycle","event":"record_rotated","file":"trace.1.jsonl","carried_bytes":rotated.bytes.len(),"corr":"a"});
    let current = source(
        "trace.jsonl",
        &format!("{anchor}\n{gate}\n{gate}\n{marker}\n"),
    );
    let result = decisions::translate(None, Some(&current), Some(&rotated)).unwrap();
    assert_eq!(result.records.len(), 2); // two real identical events, each with two source positions
    assert_ne!(result.records[0].id, result.records[1].id);
    for record in result.records {
        let Evidence::Text(original) = record.origin.original else {
            panic!("missing origins")
        };
        let origins: Vec<serde_json::Value> = serde_json::from_str(&original).unwrap();
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0]["path"], json!("trace.jsonl"));
        assert_eq!(origins[1]["path"], json!("trace.1.jsonl"));
    }
}

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn external_temp() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = repository_root();
    assert!(
        !dir.path().starts_with(repo),
        "import fixtures require TMPDIR outside repository"
    );
    dir
}

#[test]
fn derivation_snapshot_preserves_full_data_and_restart_manifest() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("STATE.md"),
        "Phase: 3 of 4\nStatus: paused\nNext: Exact next text\n",
    )
    .unwrap();
    let expected = rt.block_on(async {
        let factory = SessionFactory::new(None, Arc::new(|_, _| Ok(())));
        let session = factory.first_touch(root.path()).await.unwrap();
        let before = session.derivation_view().await.unwrap();
        let mut data = before.snapshot.data.clone();
        data["unrelated"] = json!({"keep":[1,2]});
        data["current"] = json!({"legacy":"unchanged"});
        data["derivation"] = json!({"memo":"fixture"});
        let written = session
            .commit_derivation(&before, data.clone())
            .await
            .unwrap();
        assert_eq!(written.snapshot.data, data);
        for field in ["import", "source_evidence", "archive", "cursor"] {
            assert_eq!(written.snapshot.data[field], before.snapshot.data[field]);
            let mut bad = data.clone();
            bad[field] = Value::Null;
            assert!(
                session.commit_derivation(&written, bad).await.is_err(),
                "{field}"
            );
        }
        assert_eq!(written.snapshot.operations, before.snapshot.operations);
        let mut next = data.clone();
        next["derivation"] = json!({"memo":"winner"});
        let winner = session.commit_derivation(&written, next).await.unwrap();
        assert!(session.commit_derivation(&written, data).await.is_err());
        assert_eq!(session.derivation_view().await.unwrap(), winner);
        winner.snapshot.data
    });
    rt.block_on(async {
        let factory = SessionFactory::new(None, Arc::new(|_, _| Ok(())));
        let session = factory.first_touch(root.path()).await.unwrap();
        let reopened = session.derivation_view().await.unwrap();
        assert_eq!(reopened.snapshot.data, expected);
        assert_eq!(
            serde_json::to_value(session.import_manifest()).unwrap(),
            expected["import"]
        );
    });
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
    let fixture = external_temp();
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
    fn read(&mut self, path: &Path) -> Result<Input> {
        Ok(Input {
            identity: path.into(),
            bytes: self.0.get(path).cloned(),
            stamp: None,
        })
    }
}

#[test]
fn prepare_import_uses_active_roles_and_retains_conflicting_legacy_evidence() {
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
    let result = prepare_import(root, &legacy, &active, &mut io, false, &Value::Null).unwrap();
    assert_eq!(
        (
            result.generation.effective.raw_global,
            result.transaction.snapshot.unwrap()["source_evidence"].clone()
        ),
        (
            Some(json!({"roles":{"cad-executor":{"model":"sonnet"}}})),
            json!([{
                "source":{"path":"/fixture/global/config.json","bytes":original.as_slice()},
                "generation":"4811eab5b9e5fb01dd97de0e9e9d7c06d57b6a84e0dc63bdce9a4fa8638e0884","label":"non_effective_original_source","layer":"global"
            }])
        )
    );
}

#[test]
fn register_missing_global_parent_creates_infrastructure_without_config_pins() {
    let fixture = external_temp();
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
    let fixture = external_temp();
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
        prepare_import(root, &legacy, &active, &mut io, false, &Value::Null).err(),
        Some(Error::Policy(
            "config unavailable: unusable roles.cad-executor.effort".into()
        ))
    );
}

#[test]
fn snapshot_replacement_preserves_provenance_and_unrelated_namespaces() {
    let previous = json!({"import":{"complete":true},"source_evidence":[{"original":[null,1]}],
        "archive":{"path":"ARCHIVE.md"},"cursor":{"phase":8},"evidence":{"keep":1},
        "derivation":{"keep":2},"execution":{"keep":3},"rail_receipts":{"keep":4}});
    assert_eq!(
        replace_current(&previous, json!({"new":"payload"})),
        Ok(json!({
            "import":{"complete":true},"source_evidence":[{"original":[null,1]}],
            "archive":{"path":"ARCHIVE.md"},"cursor":{"phase":8},"evidence":{"keep":1},
            "derivation":{"keep":2},"execution":{"keep":3},"rail_receipts":{"keep":4},
            "current":{"new":"payload"}
        }))
    );
}

#[test]
fn snapshot_replacement_keeps_wrapped_historical_evidence_at_its_original_location() {
    assert_eq!(
        replace_current(
            &json!({"import":{"complete":true},
        "current":{"source_evidence":[{"old":null}],"unrelated":[1,2]}}),
            json!({"answer":13})
        ),
        Ok(
            json!({"import":{"complete":true},"current":{"source_evidence":[{"old":null}],
            "unrelated":[1,2],"current":{"answer":13}}})
        )
    );
}

struct SnapshotMemory(BTreeMap<String, cadence::store::Observed>);
impl Storage for SnapshotMemory {
    type Prepared = (String, Vec<u8>);
    fn read(&mut self, target: &str) -> Result<cadence::store::Observed> {
        Ok(self
            .0
            .get(target)
            .cloned()
            .unwrap_or(cadence::store::Observed {
                bytes: None,
                identity: "missing".into(),
                directory_identity: "fixture".into(),
            }))
    }
    fn prepare(&mut self, target: &str, bytes: &[u8]) -> Result<Self::Prepared> {
        Ok((target.into(), bytes.into()))
    }
    fn install(&mut self, prepared: &Self::Prepared) -> Result<()> {
        self.0.insert(
            prepared.0.clone(),
            cadence::store::Observed {
                bytes: Some(prepared.1.clone()),
                identity: "installed".into(),
                directory_identity: "fixture".into(),
            },
        );
        Ok(())
    }
    fn discard(&mut self, _: Self::Prepared) -> Result<()> {
        Ok(())
    }
    fn confirm(&mut self, target: &str, _: &[u8]) -> Result<cadence::store::Observed> {
        self.read(target)
    }
    fn resync(&mut self, target: &str, _: &[u8]) -> Result<cadence::store::Observed> {
        self.read(target)
    }
    fn remove(&mut self, target: &str) -> Result<()> {
        self.0.remove(target);
        Ok(())
    }
}

async fn snapshot_session() -> Session<SuppliedConfig> {
    let active = Paths {
        repo: "/fixture/project/.planning/config.v4.json".into(),
        global: None,
    };
    let manifest: ImportManifest = serde_json::from_value(json!({"format":1,"complete":true,
        "source_generation":"fixture","sources":[],"active":active,"created":[],"warnings":[]}))
    .unwrap();
    let snapshot = cadence::store::model::Snapshot::new(
        7,
        b"",
        b"",
        json!({
            "import":manifest,"source_evidence":[{"preserved":[1,null]}],"cursor":{"phase":8},
            "archive":{"available":true},"unrelated":{"keep":true}
        }),
    )
    .unwrap();
    let memory = SnapshotMemory(
        [
            (ITEMS, Vec::new()),
            (DECISIONS, Vec::new()),
            (STATE, snapshot.render().unwrap()),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            (
                name.into(),
                cadence::store::Observed {
                    bytes: Some(bytes),
                    identity: "fixture".into(),
                    directory_identity: "fixture".into(),
                },
            )
        })
        .collect(),
    );
    Session {
        root: "/fixture/project/.planning".into(),
        drafts: Default::default(),
        store: Store::open(memory, Allow).await.unwrap(),
        config: Arc::new(Mutex::new(Reload::new(
            active.clone(),
            SuppliedConfig(BTreeMap::new()),
        ))),
        manifest,
        active,
    }
}

#[tokio::test]
async fn session_rewrite_returns_preserved_source_evidence() {
    let session = snapshot_session().await;
    assert_eq!(
        session
            .request(Operation::RewriteSnapshot(json!({"answer":13})))
            .await
            .map(|view| (
                view.snapshot.generation,
                view.snapshot.data["source_evidence"].clone(),
                view.snapshot.data["current"].clone(),
                view.snapshot.data["unrelated"].clone()
            )),
        Ok((
            8,
            json!([{"preserved":[1,null]}]),
            json!({"answer":13}),
            json!({"keep":true})
        ))
    );
}

#[tokio::test]
async fn session_transaction_snapshot_returns_preserved_source_evidence() {
    let session = snapshot_session().await;
    assert_eq!(
        session
            .request(Operation::Transact(Transaction {
                id: "snapshot-input".into(),
                items: vec![],
                decisions: vec![],
                snapshot: Some(json!({"answer":13})),
                external: vec![],
            }))
            .await
            .map(|view| (
                view.snapshot.generation,
                view.snapshot.data["source_evidence"].clone(),
                view.snapshot.data["current"].clone()
            )),
        Ok((8, json!([{"preserved":[1,null]}]), json!({"answer":13})))
    );
}

#[tokio::test]
async fn session_conditional_rewrite_returns_exact_stale_generation_refusal() {
    let session = snapshot_session().await;
    assert_eq!(
        session
            .request(Operation::CompareRewriteSnapshot {
                expected_generation: 6,
                expected_integrity: "stale-generation".into(),
                data: json!({"answer":13}),
            })
            .await,
        Err(Error::Conflict(
            "conditional snapshot precondition changed".into()
        ))
    );
}
