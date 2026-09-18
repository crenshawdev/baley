#[allow(dead_code)]
#[path = "support/phase13.rs"]
mod phase13;
#[path = "support/phase14.rs"]
mod phase14;

use cadence::store::model;
use phase13::{Client, apply, digest_of, reopened};
use phase14::{LEGACY_TICKED, TICKED, documents, legacy_fixture, natively_completed, progress_fixture};
use serde_json::{Value, json};
use std::{fs, path::Path, process::{Command, Stdio}};

fn progress(client: &mut Client) -> Value {
    let answer = client.call("cadence_query", json!({"operation":"progress"}));
    assert_eq!(answer["status"], "ok", "{answer}");
    assert!(answer.get("body").is_none());
    assert!(answer.as_object().unwrap().keys().all(|key| !key.contains("document")));
    assert_eq!(answer["text"].as_str().unwrap().lines().filter(|line| line.starts_with("Next:")).count(), 1);
    assert!(answer["text"].as_str().unwrap().len() <= 24_576);
    answer
}

#[test]
fn phase14_first_touch_declares_ticked_phases_it_cannot_derive() {
    let temp = legacy_fixture();
    let project = temp.path();
    let root = project.join(".planning");
    let mut before = documents(project);
    let import_digest = digest_of(&root.join("ROADMAP.md"));
    let mut client = Client::open(project);
    // First touch through progress: phases 2 and 3 are ticked and the legacy
    // table derives them short of complete, so the import declares them.
    let first = progress(&mut client);
    let name = project.file_name().unwrap().to_str().unwrap();
    let rows = "phase 1: First - complete - UAT 1 pass, 0 fail\n\
        phase 2: Second - complete (declared at import, unverified) - UAT 2 pass, 1 fail\n\
        phase 3: Third - complete (declared at import, unverified)\n\
        phase 4: Fourth - planned - plans 1\n";
    assert!(first["text"].as_str().unwrap().starts_with(&format!("# progress: {name}, phase 4 of 4: Fourth\n{rows}Issues: 0\n")), "{}", first["text"]);
    let next = client.call("cadence_query", json!({"operation":"execute-next","phase":4}));
    assert_ne!(next["code"], "state-conflict", "{next}");

    let declare = |client: &mut Client, phase: u32, id: &str| {
        client.call("cadence_apply", json!({"operation":"adoption-declare","phase":phase,"request_id":id}))
    };
    // A derivable phase and an unticked phase are refused in the typed shape.
    let d1 = declare(&mut client, 1, "d1");
    assert_eq!((d1["status"].as_str(), d1["rule"].as_str(), d1["id"].as_str()),
        (Some("refused"), Some("declaration-unneeded"), Some("complete")), "{d1}");
    let d4a = declare(&mut client, 4, "d4a");
    assert_eq!((d4a["status"].as_str(), d4a["rule"].as_str(), d4a["id"].as_str()),
        (Some("refused"), Some("declaration-unticked"), Some("ROADMAP.md:5")), "{d4a}");
    // The owner ticks phase 4 on disk after import and declares it explicitly.
    fs::write(root.join("ROADMAP.md"), LEGACY_TICKED).unwrap();
    before.insert(".planning/ROADMAP.md".into(), LEGACY_TICKED.as_bytes().to_vec());
    let d4 = declare(&mut client, 4, "d4");
    assert_eq!(d4["status"], "ok", "{d4}");
    assert_eq!(d4["replayed"], false);
    let record = d4["record"].clone();
    assert_eq!(record["provenance"], "declared-at-adoption");
    assert_eq!(record["phase"], 4);
    assert_eq!(record["roadmap"], json!({"line":5,"entry":3,"digest":model::digest(LEGACY_TICKED.as_bytes())}));
    assert_eq!(record["derived"], json!({"status":"planned","legacy_rule":"summary-and-uat"}));
    assert_eq!(record["human_results"], Value::Null);
    assert_eq!(record["claims"], json!([]));
    let mut replay = declare(&mut client, 4, "d4");
    assert_eq!(replay["replayed"], true, "{replay}");
    replay["replayed"] = json!(false);
    assert_eq!(replay, d4);
    let second = progress(&mut client);
    assert!(second["text"].as_str().unwrap().contains("\nphase 4: Fourth - complete (declared at adoption, unverified)\n"), "{}", second["text"]);
    let next = client.call("cadence_query", json!({"operation":"execute-next","phase":4}));
    assert_ne!(next["code"], "state-conflict", "{next}");
    // A caller-supplied record has no operation to arrive through.
    let forged = client.call("cadence_apply", json!({"operation":"adoption-record","phase":1,"request_id":"forged","record":record}));
    assert_eq!(forged["code"], "unknown-operation", "{forged}");
    client.finish();

    // A natively completed phase is never declared; the refusal names its completion.
    let (native, completion) = natively_completed();
    let dn = apply(native.project(), json!({"operation":"adoption-declare","phase":13,"request_id":"dn"}));
    assert_eq!((dn["status"].as_str(), dn["rule"].as_str(), dn["id"].as_str()),
        (Some("refused"), Some("declaration-native"), Some(completion.as_str())), "{dn}");

    // Reopened after restart: two records from the import, one from adoption, none for phase 1.
    let reopened = reopened(project);
    let records = reopened.snapshot.data["adoption"]["declared_completions"].as_array().unwrap().clone();
    assert_eq!(records.iter().map(|r| r["phase"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3, 4]);
    let shape = |record: &Value| {
        let mut shape = record.clone();
        for key in ["id", "root_binding", "import_generation", "source_generation"] {
            shape.as_object_mut().unwrap().remove(key);
        }
        shape
    };
    assert_eq!(shape(&records[0]), json!({
        "schema":"verification-declared-completion-1","phase":2,"provenance":"declared-at-import",
        "roadmap":{"line":3,"entry":1,"digest":import_digest},
        "derived":{"status":"executed","legacy_rule":"summary-and-uat"},
        "human_results":{"present":true,"pass":2,"fail":1,"skipped":0},"claims":[]}));
    assert_eq!(shape(&records[1]), json!({
        "schema":"verification-declared-completion-1","phase":3,"provenance":"declared-at-import",
        "roadmap":{"line":4,"entry":2,"digest":import_digest},
        "derived":{"status":"planned","legacy_rule":"summary-and-uat"},
        "human_results":null,"claims":[]}));
    assert_eq!(shape(&records[2]), json!({
        "schema":"verification-declared-completion-1","phase":4,"provenance":"declared-at-adoption",
        "roadmap":{"line":5,"entry":3,"digest":model::digest(LEGACY_TICKED.as_bytes())},
        "derived":{"status":"planned","legacy_rule":"summary-and-uat"},
        "human_results":null,"claims":[]}));
    assert_eq!(records[2]["id"].as_str().unwrap().len(), 64);
    assert_eq!(records[2]["root_binding"], records[0]["root_binding"]);
    // Every document byte is as it was, except the test's own tick of phase 4.
    assert_eq!(documents(project), before);
    assert!(String::from_utf8(before[Path::new(".planning/phases/2/UAT.md")].clone()).unwrap().contains("status: fail\n"));
}

#[test]
fn phase14_progress_reports_status_issues_and_one_next_action() {
    let fixture = progress_fixture();
    let project = fixture.project();
    let mut before = documents(project);
    let mut client = Client::open(project);
    let first = progress(&mut client);
    let header = format!("# progress: {}, phase 5 of 3: Legacy\n", project.file_name().unwrap().to_str().unwrap());
    let rows = "phase 5: Legacy - planned - plans 1\nphase 13: Plan publication - executed\nphase 28: Next phase - unplanned\n";
    assert_eq!(first["text"], format!("{header}{rows}Issues: 0\nRecord (phase 5): 0 routing decisions, 0 refusals, 0 gate fires\nCaptures: 0 active of 40\nNext: /cad-execute 5\n"));
    assert_eq!(first["phases"].as_array().unwrap().len(), 3);
    assert_eq!(first["issues"], json!([]));
    assert_eq!(first["dispatch"], Value::Null);
    assert_eq!(documents(project), before);

    fs::write(project.join(".planning/ROADMAP.md"), TICKED).unwrap();
    before.insert(".planning/ROADMAP.md".into(), TICKED.as_bytes().to_vec());
    let second = progress(&mut client);
    assert_eq!(second["phases"], first["phases"]);
    assert_eq!(second["issues"].as_array().unwrap().len(), 1);
    assert_eq!(second["text"], format!("{header}{rows}Issues: 1\n  ROADMAP.md:2 entry 0 declares phase 5 complete; derived planned\nRecord (phase 5): 0 routing decisions, 0 refusals, 0 gate fires\nCaptures: 0 active of 40\nNext: Resolve ROADMAP.md:2 entry 0: declare phase 5 with adoption-declare or untick it\n"));
    client.finish();
    let mut client = Client::open(project);
    let third = progress(&mut client);
    assert_eq!(serde_json::to_vec(&third).unwrap(), serde_json::to_vec(&second).unwrap());
    client.finish();
    assert_eq!(reopened(project).snapshot.data["derivation"]["intake"]["retired"], true);
    assert_eq!(documents(project), before);

    let rendered = Command::new(env!("CARGO_BIN_EXE_cadence"))
        .arg("progress-instructions").current_dir(project).stdin(Stdio::null()).output().unwrap();
    assert!(rendered.status.success(), "{}", String::from_utf8_lossy(&rendered.stderr));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(rendered.stdout, fs::read(root.join("skills/cad-progress/SKILL.md")).unwrap());
    for retired in ["cad-health", "cad-report"] {
        assert!(!root.join(format!("skills/{retired}/SKILL.md")).exists());
    }
}
