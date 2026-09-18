#[allow(dead_code)]
#[path = "support/phase13.rs"]
mod phase13;
#[path = "support/phase14.rs"]
mod phase14;

use phase13::{Client, reopened};
use phase14::{TICKED, documents, progress_fixture};
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
