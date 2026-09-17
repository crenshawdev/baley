#[path = "support/phase31.rs"]
#[allow(dead_code)]
mod support;

use sha2::{Digest, Sha256};
use std::fs;
use support::ClosedRound;

#[test]
fn phase33_last_close_installs_binary_rendered_summary() {
    let mut round = ClosedRound::admitted();
    round.close_tasks();
    let path = round.fixture.project().join(".planning/phases/31/SUMMARY.md");
    assert!(path.exists(), "last close must install SUMMARY.md");
    let bytes = fs::read(&path).unwrap();
    let summary = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(round.closes[1]["summary"], serde_json::json!({"revision":format!("{:x}", Sha256::digest(&bytes))}));
    for (task, commit) in [("fixture-one-a", &round.commits[1]), ("fixture-one-b", &round.commits[2])] {
        assert_eq!(summary.lines().filter(|line| *line == format!("| 1 | {task} | completed | {commit} | passed |")).count(), 1);
    }
    for line in summary.lines().filter(|line| !line.trim().is_empty()) {
        assert!(round.client.request_lines.iter().all(|request| !request.contains(line)), "summary prose crossed the request boundary: {line}");
    }
    // A second observation sees the exact installed, untracked projection as clean.
    let source = round.client.call("cadence_query", serde_json::json!({"operation":"verification-read","phase":31}));
    assert_ne!(source["code"], "evidence-source-dirty", "{source}");
}
