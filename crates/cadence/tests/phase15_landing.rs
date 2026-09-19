#[path = "support/phase13.rs"]
#[allow(dead_code)]
mod phase13;
#[path = "support/phase14.rs"]
#[allow(dead_code)]
mod phase14;
#[path = "support/phase15.rs"]
mod phase15;
use phase13::{Client, apply, query, git, git_value, reopened};
use phase15::{Fixture, close, deferred, risk};
use serde_json::json;
use std::{fs, path::Path};

#[path = "support/phase15_prune.rs"]
mod phase15_prune;

#[test]
fn phase15_prune_retries_to_one_result_from_every_write_point() {
    phase15_prune::exercise();
}

#[test]
fn phase15_close_refuses_unsettled_records_and_land_refuses_unruled_deferred() {
    let fixture = Fixture::new(&[15, 16]);
    let project = fixture.project();
    let (scan, _, clear) = risk(project, 15, true);
    let member = deferred(project, 16);
    let before = phase14::documents(project);
    let expected = json!([
        {"kind":"risk","phase":15,"identity":scan["confirmation"]["decision_id"]},
        {"kind":"deferred","phase":16,"identity":member}
    ]);
    let request = close("close-both", &[15, 16]);
    let refused = apply(project, request.clone());
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["code"], "milestone-unsettled", "{refused}");
    assert_eq!(refused["unsettled"], expected);
    assert_eq!(phase14::documents(project), before);
    assert_eq!(apply(project, request), refused, "restart replays the exact refusal");
    let report = query(project, json!({"operation":"milestone-read","occurrence":"phase15-test","selection":{"phases":[15,16],"label":"Fixture milestone"}}));
    assert_eq!(report["unsettled"], expected);
    assert_eq!(report["audits"].as_array().unwrap().iter().map(|a| a["phase"].as_u64().unwrap()).collect::<Vec<_>>(), [15,16]);
    let saved = reopened(project).snapshot;
    assert!(saved.data["rail_observations"].as_object().unwrap().values().any(|r| r["confirmation"] == scan["confirmation"]));
    assert!(saved.data["rail_observations"].as_object().unwrap().values().any(|r| r["confirmation"] == clear["confirmation"]));
    let queue = query(project, json!({"operation":"review-deferred"}));
    assert!(queue["result"]["members"].as_array().unwrap().iter().any(|m| m["member"] == member));

    let risk_only = Fixture::new(&[15]);
    let project = risk_only.project();
    let (scan, fire, _) = risk(project, 15, false);
    let refused = apply(project, close("risk-before", &[15]));
    assert_eq!(refused["unsettled"], json!([{"kind":"risk","phase":15,"identity":scan["confirmation"]["decision_id"]}]));
    let settled = apply(project, json!({"operation":"risk-consequence","request_id":"settle-risk","receipt":{
        "id":"risk-settled","fire":fire,"consequence":{"kind":"gate-pass","evidence_id":"fixture-contracted-review"}}}));
    assert_eq!(settled["status"], "ok", "{settled}");
    let before = phase14::documents(project);
    let ready = apply(project, close("risk-after", &[15]));
    assert_eq!(ready["status"], "ok", "{ready}");
    assert_eq!(ready["close"]["state"], "ready");
    assert_eq!(ready["close"]["generation"], 1);
    assert_eq!(ready["close"]["selection"], json!({"phases":[15],"label":"Fixture milestone"}));
    assert_eq!(phase14::documents(project), before, "plan 1 records readiness only");
    assert_eq!(apply(project, close("risk-after", &[15])), ready);
    let mut changed = close("risk-after", &[15]);
    changed["request"]["selection"]["label"] = json!("Changed");
    assert_eq!(apply(project, changed)["code"], "request-reused");

    let landing = Fixture::new(&[16]);
    let project = landing.project();
    let member = deferred(project, 16);
    let refused = apply(project, close("deferred-only", &[16]));
    let expected = json!([{"kind":"deferred","phase":16,"identity":member}]);
    assert_eq!(refused["code"], "milestone-unsettled");
    assert_eq!(refused["unsettled"], expected);
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare"]);
    git(project, &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    git(project, &["push", "origin", "HEAD:refs/heads/main"]);
    let head = git_value(project, &["rev-parse", "HEAD"]);
    let start = apply(project, json!({"operation":"land-start","request":{"request_id":"start-landing","occurrence":"landing-fixture",
        "expected_generation":0,"source":{"branch":"fixture/landing","head":head},"base":{"branch":"main","head":head},
        "remote":{"name":"origin","url":remote.path().to_str().unwrap()}}}));
    assert_eq!(start["status"], "ok", "{start}");
    let local_refs = git_value(project, &["show-ref"]);
    let remote_refs = git_value(remote.path(), &["show-ref"]);
    let before = phase14::documents(project);
    let trace = project.join(".git-trace");
    let mut client = Client::open_with_env(project, Path::new(env!("CARGO_BIN_EXE_cadence")), &[("GIT_TRACE", trace.as_os_str())]);
    let publish = json!({"operation":"land-publish","request":{"request_id":"publish-refused","landing":start["landing"]["id"],"expected_generation":1}});
    let refused = client.call("cadence_apply", publish.clone());
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["code"], "landing-unsettled");
    assert_eq!(refused["unsettled"], expected);
    client.finish();
    assert!(!trace.exists() || fs::read(&trace).unwrap().is_empty(), "refused publish must invoke zero Git subprocesses");
    assert_eq!(git_value(project, &["show-ref"]), local_refs);
    assert_eq!(git_value(remote.path(), &["show-ref"]), remote_refs);
    assert_eq!(phase14::documents(project), before);
    assert_eq!(apply(project, publish), refused);
    let read = query(project, json!({"operation":"land-read","landing":start["landing"]["id"]}));
    assert_eq!(read["landing"], start["landing"]);
    assert_eq!(read["refusals"][0]["answer"], refused);
    let queue = query(project, json!({"operation":"review-deferred"}));
    assert!(queue["result"]["members"].as_array().unwrap().iter().any(|m| m["member"] == member));
}
