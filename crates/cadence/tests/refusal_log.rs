#[allow(dead_code)]
#[path = "support/serve.rs"]
mod serve;
#[allow(dead_code)]
#[path = "support/refusal_fixtures.rs"]
mod refusal_fixtures;

use serde_json::json;

#[test]
fn native_refusals_reach_the_log() {
    let fixture = refusal_fixtures::Active::new();
    let project = fixture.project();
    let before = serve::reopened(project);
    let projections = serve::tree(project);
    let mut client = serve::Client::open(project);
    let mut answers = vec![];
    for request in fixture.requests() {
        let answer = client.call("cadence_apply", request);
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_ne!(answer["code"], "invalid-arguments", "must reach the semantic gate: {answer}");
        answers.push(answer);
    }
    eprintln!("semantic refusals: {answers:?}");
    client.finish();
    let after = serve::reopened(project);
    assert_eq!(after.snapshot.data, before.snapshot.data);
    assert_eq!(after.decisions.len(), before.decisions.len() + 5);
    assert_eq!(&after.decisions[..before.decisions.len()], &before.decisions);
    for (record, answer) in after.decisions[before.decisions.len()..].iter().zip(&answers) {
        let cadence::store::model::Decision::BoundaryV1(saved) = &record.decision else { panic!("not a boundary"); };
        refusal_fixtures::assert_answer(&saved.boundary, answer);
    }
    for (path, bytes) in &projections {
        if ![".planning/state.json", ".planning/decisions.jsonl"].iter().any(|p| path == std::path::Path::new(p)) {
            assert_eq!(serve::tree(project).get(path), Some(bytes), "{}", path.display());
        }
    }
    let mut client = serve::Client::open(project);
    let why = client.call("cadence_query", json!({"operation":"why","phase":28,"part":"refusals"}));
    assert_eq!(why["status"], "ok", "{why}");
    let rows = why["refusals"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    for (row, answer) in rows.iter().zip(&answers) {
        for field in ["code", "rule", "slot", "reason", "id"] { assert_eq!(row[field], answer[field], "{field}"); }
    }
    let long = json!({"operation":"execution-authorize","phase":28,"request_id":"refusal-long",
        "owner":"Fixture Owner","at":"2026-09-21T12:00:00Z","response":"Proceed","disposition":"é".repeat(900)});
    let answer = client.call("cadence_apply", long.clone());
    assert_eq!(answer["status"], "refused");
    assert!(answer["reason"].as_str().unwrap().len() > 1024, "{answer}");
    assert_eq!(client.call("cadence_apply", long), answer);
    client.finish();
    let after_long = serve::reopened(project);
    assert_eq!(after_long.snapshot.data, before.snapshot.data);
    assert_eq!(after_long.decisions.len(), before.decisions.len() + 6);
    let cadence::store::model::Decision::BoundaryV1(saved) = &after_long.decisions.last().unwrap().decision else { panic!("not a boundary"); };
    let cadence::execution::boundary::Receipt::Compact { envelope: cadence::execution::boundary::Envelope::Refused { reason, .. } } = &saved.boundary.receipt else { panic!("not refused"); };
    assert!(reason.len() <= 1024);
    assert!(reason.ends_with("[cut] cap=1024 bytes"));
    let mut client = serve::Client::open(project);
    let why = client.call("cadence_query", json!({"operation":"why","phase":28,"part":"refusals"}));
    assert_eq!(why["refusals"].as_array().unwrap().len(), 6);
    client.finish();
}
