#[path = "support/phase13.rs"]
mod phase13;
use phase13::*;
use serde_json::{Value, json};
use std::{fs, time::{Duration, Instant}};

fn read(project: &std::path::Path, attempt: &Value) -> Value {
    query(project, json!({"operation":"verification-read","phase":13,"attempt":attempt["id"]}))
}

#[test]
fn phase13_runner_retains_independent_receipts() {
    let fixture = Completed::new();
    let project = fixture.project();
    assert_eq!((fixture.pairs.len(), fixture.statements.len(), fixture.dispatches.len()), (2, 2, 2));
    assert_eq!(fixture.admission["status"], "ok");
    let attempt = query(project, json!({"operation":"verify-next","phase":13,"request_id":"runner-attempt"}))["attempt"].clone();
    assert_eq!(attempt["inputs"]["map"], fixture.map);
    let item = &fixture.pairs[0]["check"];
    let request = json!({"operation":"verification-run","request":{"request_id":"independent-a",
        "attempt":attempt["id"],"basis":attempt["inputs"]["basis"],"item":item}});
    let before = fs::read_to_string(project.join(".run/a-runs")).unwrap();
    assert_eq!(before, "run\nrun\nrun\n");
    let native_before = reopened(project).snapshot;
    let mut client = Client::open(project);
    let launch = client.call("cadence_apply", request.clone());
    assert_eq!(launch["status"], "ok", "{launch}");
    assert_eq!(launch["receipt"]["event"]["launch"]["material"]["command"], "python3 -B tests/a.py");
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        let report = client.call("cadence_query", json!({"operation":"verification-read","phase":13,"attempt":attempt["id"]}));
        assert_eq!(report["status"], "ok", "{report}");
        if let Some(record) = report["runs"].as_array().unwrap().iter().find(|r| r["event"]["kind"] == "result") { break record.clone(); }
        assert!(Instant::now() < deadline, "{report}");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(result["event"]["result"]["disposition"], json!({"kind":"exited","code":0}));
    assert_eq!(result["event"]["result"]["material_unchanged"], true);
    assert_eq!(result["event"]["source_after"], attempt["inputs"]["basis"]["source"]);
    client.finish();
    let after = tree(project);
    assert_eq!(apply(project, request.clone())["receipt"], launch["receipt"]);
    assert_eq!(read(project, &attempt)["runs"], json!([launch["receipt"], result]));
    assert_eq!(read(project, &attempt)["unknown_runs"], json!([]));
    assert_eq!(fs::read_to_string(project.join(".run/a-runs")).unwrap(), "run\nrun\nrun\nrun\n");
    let reopened = phase13::reopened(project).snapshot;
    assert_eq!(tree(project), after);
    for key in ["native_tasks", "native_plans", "native_admissions"] {
        assert_eq!(reopened.data[key], native_before.data[key], "verification never reopens execution");
    }
    let mut substituted = request.clone();
    substituted["request"]["request_id"] = json!("alternate");
    substituted["request"]["command"] = json!("printf substituted > .run/substitution");
    assert_eq!(apply(project, substituted)["status"], "refused");
    assert!(!project.join(".run/substitution").exists());
    let mut foreign = request.clone();
    foreign["request"]["request_id"] = json!("foreign");
    foreign["request"]["item"]["id"] = json!("artifact/shared");
    assert_eq!(apply(project, foreign)["rule"], "verification-item");
    fs::write(project.join("src/a.py"), "def answer():\n    return 8\n").unwrap();
    let mut stale = request.clone();
    stale["request"]["request_id"] = json!("stale");
    assert_eq!(apply(project, stale)["rule"], "verification-source");
    assert_eq!(apply(project, request.clone())["receipt"], launch["receipt"]);
    fs::write(project.join("src/a.py"), "def answer():\n    return 7\n").unwrap();
    assert_eq!(tree(project), after);
    // A real paused child is interrupted by killing its owning server.
    fs::write(project.join(".run/wait"), "wait").unwrap();
    let mut interrupted = request;
    interrupted["request"]["request_id"] = json!("interrupted");
    let mut client = Client::open(project);
    let pending = client.call("cadence_apply", interrupted.clone());
    assert_eq!(pending["status"], "ok", "{pending}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !project.join(".run/ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    client.child.kill().unwrap();
    client.child.wait().unwrap();
    drop(client);
    // The shell's child may outlive the shell; clean up the real fixture PID.
    let pid: i32 = fs::read_to_string(project.join(".run/ready")).unwrap().parse().unwrap();
    unsafe { libc::kill(pid, libc::SIGKILL); }
    fs::remove_file(project.join(".run/wait")).unwrap();
    let stopped = tree(project);
    assert_eq!(read(project, &attempt)["unknown_runs"], json!(["interrupted"]));
    assert_eq!(apply(project, interrupted)["receipt"], pending["receipt"]);
    assert_eq!(fs::read_to_string(project.join(".run/a-runs")).unwrap(), "run\nrun\nrun\nrun\nrun\n");
    assert_eq!(tree(project), stopped);
    let saved = phase13::reopened(project).snapshot;
    assert_eq!(saved.data["native_tasks"], native_before.data["native_tasks"]);
    assert_eq!(tree(project), stopped);
}
