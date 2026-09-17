#[path = "support/phase31.rs"]
mod support;

use serde_json::{Value, json};
use support::ClosedRound;

#[test]
fn phase33_run_output_reads_as_bounded_text_slices() {
    let mut round = ClosedRound::admitted();
    round.close_tasks_with_output();
    let root = round.fixture.project().join(".planning");
    let before = cadence::context::persistence::read_snapshot(&root).unwrap().unwrap();
    let records = cadence::execution::history::records(&before.data, 31).unwrap();
    for run in round.runs.clone() {
        let result = records.iter().find_map(|record| match &record.request.event {
            cadence::execution::history::Event::Result(result) if result.run_id == run => Some(result),
            _ => None,
        }).unwrap();
        let identity = json!({"kind":"run-output","phase":31,"run":run});
        let index = round.client.call("cadence_query", json!({"operation":"document","identity":identity}));
        assert_eq!(index["status"], "ok", "{index}");
        let parts = index["parts"].as_array().unwrap();
        assert_eq!(parts[0]["part"], "launch");
        assert_eq!(parts[1]["part"], "result");
        let mut stdout = String::new();
        let mut stderr = String::new();
        for (i, part) in parts.iter().enumerate() {
            let slice = round.client.call("cadence_query", json!({"operation":"document","identity":identity,"part":part["part"]}));
            assert_eq!(slice["status"], "ok", "{slice}");
            assert_eq!(slice["next"], parts.get(i + 1).map_or(Value::Null, |p| p["part"].clone()));
            let body = slice["body"].as_str().unwrap();
            assert!(body.len() <= 24576);
            let name = part["part"].as_str().unwrap();
            if name.starts_with("stdout:") { stdout.push_str(body); }
            else if name.starts_with("stderr:") { stderr.push_str(body); }
            else {
                let metadata: Value = serde_json::from_str(body).unwrap();
                if name == "result" {
                    assert!(metadata["request"]["event"]["stdout"].get("text").is_none());
                    assert!(metadata["request"]["event"]["stderr"].get("bytes").is_none());
                    assert_eq!(metadata["request"]["event"]["stdout"]["digest"], result.stdout.digest);
                }
            }
        }
        assert_eq!(stdout, String::from_utf8_lossy(&result.stdout.bytes));
        assert_eq!(stderr, String::from_utf8_lossy(&result.stderr.bytes));
        if run == "fixture-one-b-verify" {
            assert_eq!(result.stdout.bytes.len(), 65536);
            assert_eq!(parts.iter().filter(|p| p["part"].as_str().unwrap().starts_with("stdout:")).count(), 3);
            assert_eq!(parts.iter().filter(|p| p["part"].as_str().unwrap().starts_with("stderr:")).count(), 8);
        }
        let history = round.client.call("cadence_query", json!({"operation":"execution-history","phase":31,"run":run}));
        assert_eq!(history["identity"], identity);
        assert!(history["result"]["request"]["event"]["stdout"].get("text").is_none());
        assert!(history["result"]["request"]["event"]["stderr"].get("bytes").is_none());
    }
    let after = cadence::context::persistence::read_snapshot(&root).unwrap().unwrap();
    assert_eq!(before.data, after.data);
}
