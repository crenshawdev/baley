#[allow(dead_code)]
#[path = "support/phase13.rs"]
mod phase13;
#[path = "support/support_records.rs"]
mod support_records;

use phase13::Client;
use serde_json::{Value, json};
use std::fs;
use support_records::{apply, query};

#[test]
fn debug_open_reads_recall_by_the_effective_backend() {
    let empty = json!({"backend":"none","results":[],"total":0,"incomplete":[]});
    for backend in [Some("builtin"), Some("none"), None] {
        let repo = support_records::recall_fixture(backend);
        let mut client = Client::open(repo.path());
        let request = json!({"request_id":"recall-debug-open","slug":"stale-token",
            "expected_version":0,"symptom":"cache stale token"});
        let opened = apply(&mut client, "debug-open", request.clone());
        let retained = &opened["record"]["recall"];
        assert_eq!(retained["backend"], backend.unwrap_or("builtin"), "{opened}");
        let recalled = client.call("cadence_query", json!({"operation":"recall","query":"cache stale token"}));
        assert_eq!(&recalled, retained);
        let filtered = client.call("cadence_query", json!({"operation":"recall","query":"cache stale token","phase":1,"limit":10}));
        let limited = client.call("cadence_query", json!({"operation":"recall","query":"cache stale token","phase":1,"limit":1}));
        if backend == Some("none") {
            for answer in [retained, &filtered, &limited] { assert_eq!(answer, &empty); }
        } else {
            let hits = retained["results"].as_array().unwrap();
            assert_eq!(retained["total"], 5);
            assert!(hits.iter().any(|hit| hit["source"] == "phases/1/SUMMARY.md" && hit["phase"] == 1));
            assert!(hits.iter().any(|hit| hit["source"] == "phases/2/SUMMARY.md" && hit["phase"] == 2));
            assert!(hits.iter().any(|hit| hit["provenance"]["kind"] == "record" && hit["phase"] == 1));
            assert!(hits.iter().any(|hit| hit["source"] == "phases/1.1/SUMMARY.md" && hit["phase"].is_null()));
            assert_eq!(filtered["total"], 2, "{filtered}");
            let hits = filtered["results"].as_array().unwrap();
            assert_eq!(hits.len(), 2);
            assert!(hits.iter().all(|hit| hit["phase"] == 1));
            assert!(hits.iter().any(|hit| hit["provenance"]["kind"] == "record"));
            assert!(hits.iter().any(|hit| hit["source"] == "phases/1/SUMMARY.md"));
            assert_eq!(limited["total"], 2);
            assert_eq!(limited["results"], json!([hits[0]]));
            let projection = opened["projection"].as_str().unwrap();
            assert!(projection.contains("phases/1/SUMMARY.md") && projection.contains("phase 1"));
        }
        client.finish();
        assert_eq!(phase13::reopened(repo.path()).snapshot.data["debug"]["records"]["stale-token"]["recall"], *retained);
        // Changed corpus and backend cannot rewrite the retained snapshot on restart or replay.
        fs::write(repo.path().join(".planning/phases/1/SUMMARY.md"), "## Deviations\n- replacement\n").unwrap();
        fs::write(repo.path().join(".planning/config.json"), r#"{"memory":{"backend":"none"}}"#).unwrap();
        let mut client = Client::open(repo.path());
        for operation in ["debug-status", "debug-continue"] {
            let answer = query(&mut client, operation, "stale-token");
            assert_eq!(answer["record"]["recall"], *retained);
            assert_eq!(answer["projection"], opened["projection"]);
        }
        assert_eq!(apply(&mut client, "debug-open", request), opened);
        client.finish();
    }
    #[cfg(unix)]
    for backend in ["builtin", "none"] {
        let repo = support_records::recall_fixture(Some(backend));
        let Some(_permissions) = support_records::UnreadableFile::new(repo.path().join(".planning/phases/1/SUMMARY.md")) else { continue; };
        let mut client = Client::open(repo.path());
        let opened = apply(&mut client, "debug-open", json!({"request_id":"recall-unreadable-open","slug":"unreadable",
            "expected_version":0,"symptom":"cache stale token"}));
        let recalled = client.call("cadence_query", json!({"operation":"recall","query":"cache stale token"}));
        assert_eq!(opened["record"]["recall"], recalled);
        if backend == "none" { assert_eq!(recalled, empty); }
        else {
            assert!(recalled["incomplete"].as_array().unwrap().iter().any(|v| v.as_str().unwrap().contains("phases/1/SUMMARY.md: source unavailable")), "{recalled}");
            assert!(recalled["results"].as_array().unwrap().iter().any(|hit| hit["source"] == "phases/2/SUMMARY.md" && hit["phase"] == 2));
        }
        client.finish();
    }
}

fn recorded(answer: &Value) {
    assert_eq!(answer["status"], "ok", "{answer}");
    let record = &answer["record"];
    assert_eq!(record["symptom"], "cache returns old value");
    assert_eq!(record["hypotheses"], json!([
        {"id":"h1","description":"stale process","rank_reason":"cheapest identity check","state":"refuted"},
        {"id":"h2","description":"wrong key","rank_reason":"next discriminating check","state":"untested"}
    ]));
    assert_eq!(record["observations"], json!([
        {"test":"print process version","result":"current version","rules_in":[],"rules_out":["h1"]}
    ]));
    assert_eq!(record["attempt_count"], 1);
    assert_eq!(record["attempts"], json!([{"description":"restart cache","result":"still stale"}]));
    assert_eq!(record["status"], "open");
    assert_eq!(record["resolution"], Value::Null);
}

#[test]
fn debug_session_resumes_from_the_record_alone() {
    let repo = support_records::fixture();
    let project = repo.path();
    let before = support_records::documents(project);
    let mut client = Client::open(project);
    let opened = apply(&mut client, "debug-open", json!({"request_id":"debug-record-open","slug":"cache-miss",
        "expected_version":0,"symptom":"cache returns old value"}));
    assert_eq!(opened["record"]["attempt_count"], 0);
    assert_eq!(opened["record"]["status"], "open");
    for (version, id, description, rank) in [(1,"h1","stale process","cheapest identity check"),
        (2,"h2","wrong key","next discriminating check")] {
        apply(&mut client, "debug-hypothesis", json!({"request_id":format!("debug-record-{id}"),"slug":"cache-miss",
            "expected_version":version,"hypothesis":{"id":id,"description":description,"rank_reason":rank,"state":"untested"}}));
    }
    apply(&mut client, "debug-observation", json!({"request_id":"debug-record-observation","slug":"cache-miss","expected_version":3,
        "observation":{"test":"print process version","result":"current version","rules_in":[],"rules_out":["h1"]}}));
    let attempt = json!({"request_id":"debug-record-attempt","slug":"cache-miss","expected_version":4,
        "attempt":{"description":"restart cache","result":"still stale"}});
    let failed = apply(&mut client, "debug-attempt", attempt.clone());
    recorded(&failed);
    assert_eq!(apply(&mut client, "debug-attempt", attempt.clone()), failed);
    let mut changed = attempt;
    changed["attempt"]["result"] = json!("different");
    let reused = client.call("cadence_apply", json!({"operation":"debug-attempt","request":changed}));
    assert_eq!(reused["code"], "request-reused");
    client.finish();
    let view = phase13::reopened(project);
    assert!(view.snapshot.data["debug"]["records"]["cache-miss"].is_object());
    let copy = support_records::copy_stopped(project);
    fs::write(copy.path().join(".planning/debug/cache-miss.md"), "# Debug\nSymptom: misleading\nAttempts: 999\n").unwrap();
    let mut copied = Client::open(copy.path());
    recorded(&query(&mut copied, "debug-continue", "cache-miss"));
    copied.finish();

    let mut client = Client::open(project);
    let continued = query(&mut client, "debug-continue", "cache-miss");
    recorded(&continued);
    recorded(&query(&mut client, "debug-status", "cache-miss"));
    let projection = fs::read_to_string(project.join(".planning/debug/cache-miss.md")).unwrap();
    assert_eq!(projection, continued["projection"].as_str().unwrap());
    for line in ["Symptom: cache returns old value", "Attempts: 1", "h1 [refuted]: stale process",
        "Rank reason: cheapest identity check", "h2 [untested]: wrong key", "Rank reason: next discriminating check",
        "Test: print process version", "Result: current version", "Rules out: h1"] { assert!(projection.contains(line), "missing {line}: {projection}"); }
    let listed = client.call("cadence_query", json!({"operation":"debug-list"}));
    assert_eq!(listed["records"].as_array().unwrap().iter().map(|r| r["slug"].as_str().unwrap()).collect::<Vec<_>>(), ["cache-miss"]);
    let missing = query(&mut client, "debug-status", "missing-slug");
    assert_eq!(missing["status"], "refused");
    assert!(missing.to_string().contains("missing-slug"));
    client.finish();
    for tool in ["Write", "Edit"] {
        assert_eq!(support_records::guard(project, tool, ".planning/debug/cache-miss.md")["hookSpecificOutput"]["permissionDecision"], "deny");
    }
    for (path, bytes) in before { assert_eq!(fs::read(project.join(path)).unwrap(), bytes); }

    let control = support_records::fixture();
    let staged = support_records::staged_note(control.path());
    let mut client = Client::open(control.path());
    apply(&mut client, "debug-open", json!({"request_id":"debug-control-open","slug":"resolved-control","expected_version":0,"symptom":"note missing"}));
    let resolved = apply(&mut client, "debug-resolve", json!({"request_id":"debug-control-resolve","slug":"resolved-control","expected_version":1,
        "resolution":"add plain note","reproduction":{"test":"read docs/note.txt","result":"plain note","passed":true}}));
    assert_eq!(resolved["record"]["status"], "resolved");
    assert_eq!(resolved["record"]["resolution"], json!({"description":"add plain note",
        "reproduction":{"test":"read docs/note.txt","result":"plain note","passed":true}}));
    assert_eq!(client.call("cadence_query", json!({"operation":"debug-list"}))["records"], json!([]));
    client.finish();
    assert_eq!(phase13::git_value(control.path(), &["write-tree"]), staged);
}
