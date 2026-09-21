#[path = "support/serve.rs"]
#[allow(dead_code)]
mod serve;
#[path = "support/query_fixtures.rs"]
#[allow(dead_code)]
mod query_fixtures;
#[path = "support/landing_fixtures.rs"]
mod landing_fixtures;
use serve::{Client, apply, query, git, git_value, reopened};
use landing_fixtures::{Fixture, close, deferred, risk};
use serde_json::json;
use std::{fs, path::Path};

#[path = "support/prune_fixture.rs"]
mod prune_fixture;
#[path = "support/production_source.rs"]
mod production_source;

#[test]
fn fault_drivers_are_gated_to_debug_builds() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let is_fault_driver = |line: &str| line.contains("process::exit(")
        || line.contains("\"CADENCE_PRUNE_STOP\"")
        || line.contains("\"CADENCE_LANDING_EXIT_AFTER_EFFECT\"");
    let sites = production_source::production_sites(&root, &is_fault_driver);
    assert!(!sites.is_empty(), "production fault drivers must be found");
    let offenders = production_source::production_sites(&root, &|line| {
        is_fault_driver(line) && !line.contains("cfg!(debug_assertions)")
    });
    assert!(offenders.is_empty(), "fault drivers without a debug-build gate:\n{}", offenders.join("\n"));
}

#[test]
fn phase15_interrupted_landing_reconciles_against_the_remote() {
    use landing_fixtures::{Publishing, effect_exit};
    use serde_json::Value;
    let forge = json!({"provider":"github","repo":"fixture/repo","host":"github.com"});
    let inputs = [json!({"step":"push"}),
        json!({"step":"open","forge":forge,"title":"Resume title","body":"Resume body"}),
        json!({"step":"merge","forge":forge,"pr":7})];
    let operations = ["land-publish", "land-open", "land-merge"];
    let steps = ["push", "open", "merge"];
    let request = |operation: &str, id: &str, landing: &Value, auth: &Value, input: &Value| {
        json!({"operation":operation,"request":{"request_id":id,"landing":landing["id"],
            "expected_generation":landing["generation"],"authorization":auth["id"],"inputs":input}})
    };
    let saved = |fixture: &Publishing, landing: &Value| {
        reopened(fixture.project.path()).snapshot.data["landings"]["records"][landing["id"].as_str().unwrap()].clone()
    };
    // An owner-authorized push can already exist without a local invocation or receipt.
    let fixture = Publishing::new(false);
    fixture.durable_forge();
    let mut client = fixture.client();
    let landing = fixture.start(&mut client, "resume-external");
    let auth = Publishing::authorize(&mut client, &landing, "resume-external-push", inputs[0].clone());
    client.finish();
    git(fixture.project.path(), &["push", "origin", "HEAD:refs/heads/fixture/execution"]);
    let trace = fixture.project.path().join(".run/resume.trace");
    let mut client = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
    let answer = client.call("cadence_apply", request("land-resume", "resume-external-effect", &landing, &auth, &inputs[0]));
    assert_eq!(answer["status"], "ok", "land-resume must reconcile the actual remote: {answer}");
    assert_eq!(answer["receipt"]["proof"]["object"], fixture.head);
    assert_eq!(answer["receipt"]["provenance"]["kind"], "reconciled");
    client.finish();
    assert!(!fixture.trace().iter().any(|event| event["event"] == "cmd_name" && event["name"] == "push"));

    for interrupted in 0..3 {
        let fixture = Publishing::new(false);
        fixture.durable_forge();
        let mut client = fixture.client();
        let mut landing = fixture.start(&mut client, "resume-interrupted");
        for index in 0..interrupted {
            let auth = Publishing::authorize(&mut client, &landing, &format!("resume-prerequisite-{index}"), inputs[index].clone());
            let answer = client.call("cadence_apply", request(operations[index], &format!("resume-prerequisite-effect-{index}"), &landing, &auth, &inputs[index]));
            assert_eq!(answer["status"], "ok", "{answer}");
            landing = answer["landing"].clone();
        }
        let auth = Publishing::authorize(&mut client, &landing, "resume-interrupted-grant", inputs[interrupted].clone());
        client.finish();
        let child = fixture.client_with_env(&[("CADENCE_LANDING_EXIT_AFTER_EFFECT", steps[interrupted].as_ref())]);
        effect_exit(child, request(operations[interrupted], "resume-interrupted-effect", &landing, &auth, &inputs[interrupted]));
        let pending = saved(&fixture, &landing);
        assert!(pending["steps"][interrupted]["receipt"].is_null(), "{pending}");
        assert_eq!(pending["steps"][interrupted]["intent"]["authorization"], auth);
        let mutations = fixture.mutations();
        assert_eq!(mutations.len(), interrupted);
        let good_pr = if interrupted > 0 { fixture.pr_state() } else { Value::Null };
        // A stale tracking ref must never supply the reconciliation proof.
        git(fixture.project.path(), &["update-ref", "refs/remotes/origin/fixture/execution", &fixture.base]);
        let trace = fixture.project.path().join(".run/resume.trace");
        let cases: &[&str] = if interrupted == 0 { &["unavailable", "moved"] }
            else { &["unavailable", "head", "base", "repo", "identity", "closed", "unknown", "ambiguous"] };
        for case in cases {
            let unavailable = fixture.remote.path().with_extension("unavailable");
            if interrupted == 0 {
                if *case == "unavailable" { fs::rename(fixture.remote.path(), &unavailable).unwrap(); }
                else { git(fixture.remote.path(), &["update-ref", "refs/heads/fixture/execution", &fixture.base]); }
            } else if *case == "unavailable" {
                fs::write(fixture.project.path().join(".run/read-fails"), "fail").unwrap();
            } else {
                let mut bad = good_pr.clone();
                match *case {
                    "head" => bad[0]["head"]["sha"] = json!(fixture.base),
                    "base" => bad[0]["base"]["ref"] = json!("different"),
                    "repo" => bad[0]["head"]["repo"]["full_name"] = json!("other/repo"),
                    "identity" => bad[0]["number"] = json!(8),
                    "closed" => { bad[0]["state"] = json!("closed"); bad[0]["merged"] = json!(false); bad[0]["merged_at"] = Value::Null; },
                    "unknown" => bad[0]["state"] = json!("unknown"),
                    "ambiguous" => { let duplicate = bad[0].clone(); bad.as_array_mut().unwrap().push(duplicate); },
                    _ => unreachable!(),
                }
                // Before create has a receipt, any single positive identity is valid.
                if interrupted == 1 && *case == "identity" { bad[0]["number"] = json!(0); }
                fixture.set_pr_state(&bad);
            }
            let mut child = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
            let refused = child.call("cadence_apply", request("land-resume", &format!("resume-refused-{case}"), &landing, &auth, &inputs[interrupted]));
            assert_eq!(refused["status"], "refused", "{case}: {refused}");
            assert_eq!(refused["code"], "landing-reconciliation-discrepancy", "{case}: {refused}");
            assert_eq!(refused["details"]["landing"], landing["id"]);
            assert_eq!(refused["details"]["step"], steps[interrupted]);
            assert!(refused["reason"].as_str().is_some_and(|s| !s.is_empty()));
            child.finish();
            assert!(saved(&fixture, &landing)["steps"][interrupted]["receipt"].is_null());
            assert_eq!(fixture.mutations(), mutations);
            if interrupted == 0 {
                if *case == "unavailable" { fs::rename(&unavailable, fixture.remote.path()).unwrap(); }
                else { git(fixture.remote.path(), &["update-ref", "refs/heads/fixture/execution", &fixture.head]); }
            } else {
                if *case == "unavailable" { fs::remove_file(fixture.project.path().join(".run/read-fails")).unwrap(); }
                fixture.set_pr_state(&good_pr);
            }
        }
        let remote_refs = git_value(fixture.remote.path(), &["show-ref"]);
        let reflog = git_value(fixture.remote.path(), &["reflog", "show", "--format=%H", "refs/heads/fixture/execution"]);
        assert!(!reflog.is_empty(), "the bare reflog oracle must be enabled");
        let before_reads = fixture.invocations().len();
        let resume = request("land-resume", "resume-reconcile", &landing, &auth, &inputs[interrupted]);
        let mut child = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
        let answer = child.call("cadence_apply", resume.clone());
        assert_eq!(answer["status"], "ok", "{answer}");
        let receipt = &answer["receipt"];
        assert_eq!(receipt["step"], steps[interrupted]);
        assert_eq!(receipt["authorization"], auth["id"]);
        assert_eq!(receipt["provenance"]["kind"], "reconciled");
        assert_eq!(receipt["provenance"]["request_id"], "resume-reconcile");
        assert_eq!(answer["next_step"], ["open", "merge", "confirm-merge"][interrupted]);
        if interrupted == 0 {
            assert_eq!(receipt["proof"]["reference"], "refs/heads/fixture/execution");
            assert_eq!(receipt["proof"]["object"], fixture.head);
            assert_eq!(receipt["proof"]["remote"], landing["remote"]);
        } else {
            assert_eq!(receipt["proof"]["forge"], forge);
            assert_eq!(receipt["proof"]["number"], 7);
            assert_eq!(receipt["proof"]["state"], if interrupted == 1 { "OPEN" } else { "MERGED" });
            assert_eq!(receipt["proof"]["source"], landing["source"]);
            assert_eq!(receipt["proof"]["base"], landing["base"]);
            assert!(fixture.invocations()[before_reads..].iter().any(|args| args.as_array().unwrap().contains(&json!("GET"))));
        }
        let read = child.call("cadence_query", json!({"operation":"land-read","landing":landing["id"]}));
        assert_eq!(read["landing"], answer["landing"]);
        assert!(read["landing"]["merge_confirmation"].is_null());
        child.finish();
        let persisted = saved(&fixture, &landing);
        assert_eq!(persisted["steps"][interrupted]["receipt"], *receipt);
        assert_eq!(persisted["generation"], landing["generation"].as_u64().unwrap() + 1);
        let mut child = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
        assert_eq!(child.call("cadence_apply", resume.clone()), answer);
        let again = child.call("cadence_apply", request("land-resume", "resume-again", &persisted, &auth, &inputs[interrupted]));
        assert_eq!(again["receipt"], *receipt, "{again}");
        let mut changed = resume;
        changed["request"]["authorization"] = json!("different");
        assert_eq!(child.call("cadence_apply", changed)["code"], "request-reused");
        child.finish();
        assert_eq!(saved(&fixture, &landing), persisted, "resume must not append a second step receipt or confirm merge");
        assert_eq!(fixture.mutations(), mutations);
        assert_eq!(git_value(fixture.remote.path(), &["show-ref"]), remote_refs);
        assert_eq!(git_value(fixture.remote.path(), &["reflog", "show", "--format=%H", "refs/heads/fixture/execution"]), reflog);
        let events = fixture.trace();
        assert!(events.iter().any(|e| e["event"] == "cmd_name" && e["name"] == "ls-remote"));
        assert!(!events.iter().any(|e| e["event"] == "cmd_name" && e["name"] == "push"), "even a no-op push is forbidden on resume: {events:?}");
    }

    // Definitive absence permits only the retained, exactly authorized step.
    let fixture = Publishing::new(false);
    fixture.durable_forge();
    let mut client = fixture.client();
    let mut landing = fixture.start(&mut client, "resume-absent");
    for (index, input) in inputs.iter().enumerate() {
        let auth = Publishing::authorize(&mut client, &landing, &format!("resume-absent-grant-{index}"), input.clone());
        let mut missing = request("land-resume", &format!("resume-no-grant-{index}"), &landing, &auth, input);
        missing["request"]["authorization"] = Value::Null;
        let calls = fixture.invocations();
        assert_eq!(client.call("cadence_apply", missing)["code"], "landing-authorization-required");
        assert_eq!(fixture.invocations(), calls, "permission precedes remote reads");
        let answer = client.call("cadence_apply", request("land-resume", &format!("resume-absent-effect-{index}"), &landing, &auth, input));
        assert_eq!(answer["status"], "ok", "{answer}");
        landing = answer["landing"].clone();
        assert_eq!(landing["steps"].as_array().unwrap().iter().filter(|s| !s["receipt"].is_null()).count(), index + 1);
    }
    assert_eq!(fixture.mutations().len(), 2);
    assert!(landing["merge_confirmation"].is_null());
    client.finish();
}

#[test]
fn phase15_publish_steps_refuse_without_a_landing_authorization() {
    use landing_fixtures::{Publishing, imported_auto_close};
    let forge = json!({"provider":"github","repo":"fixture/repo","host":"github.com"});
    let inputs = [json!({"step":"push"}), json!({"step":"open","forge":forge,"title":"Confirmed title","body":"Confirmed body\nSecond line"}),
        json!({"step":"merge","forge":forge,"pr":7}), json!({"step":"tag-push","tag":"v1.0.0","head":"0000000000000000000000000000000000000000"})];
    let operations = ["land-publish", "land-open", "land-merge", "land-tag-push"];
    let steps = ["push", "open", "merge", "tag-push"];
    for auto_close in [false, true] {
        let fixture = Publishing::new(auto_close);
        let project = fixture.project.path();
        let mut client = fixture.client();
        let landing = fixture.start(&mut client, "clear");
        let other = fixture.start(&mut client, "other");
        let local_refs = git_value(project, &["show-ref"]);
        let remote_refs = git_value(fixture.remote.path(), &["show-ref"]);
        if auto_close { assert!(imported_auto_close(&reopened(project).snapshot.data)); }
        for (index, operation) in operations.iter().enumerate() {
            let other_auth = Publishing::authorize(&mut client, &other, &format!("other-{index}"), inputs[index].clone());
            let wrong_step = Publishing::authorize(&mut client, &landing, &format!("wrong-{index}"), inputs[(index + 1) % inputs.len()].clone());
            for (case, authorization) in [("absent", json!(null)), ("other", other_auth["id"].clone()), ("step", wrong_step["id"].clone())] {
                let refused = client.call("cadence_apply", json!({"operation":operation,"request":{
                    "request_id":format!("{case}-{index}"),"landing":landing["id"],"expected_generation":1,
                    "authorization":authorization,"inputs":inputs[index]}}));
                assert_eq!(refused["status"], "refused", "{refused}");
                assert_eq!(refused["code"], "landing-authorization-required", "{refused}");
                assert_eq!(refused["details"]["landing"], landing["id"]);
                assert_eq!(refused["details"]["step"], steps[index]);
            }
        }
        assert_eq!(git_value(project, &["show-ref"]), local_refs);
        assert_eq!(git_value(fixture.remote.path(), &["show-ref"]), remote_refs);
        assert!(fixture.invocations().is_empty());
        let auth = Publishing::authorize(&mut client, &landing, "push-authorization", inputs[0].clone());
        let request = json!({"operation":"land-publish","request":{"request_id":"push-authorized","landing":landing["id"],
            "expected_generation":1,"authorization":auth["id"],"inputs":inputs[0]}});
        let pushed = client.call("cadence_apply", request.clone());
        assert_eq!(pushed["status"], "ok", "{pushed}");
        assert_eq!(pushed["receipt"]["landing"], landing["id"]);
        assert_eq!(pushed["receipt"]["authorization"], auth["id"]);
        assert_eq!(pushed["receipt"]["step"], "push");
        assert_eq!(git_value(fixture.remote.path(), &["rev-parse", "refs/heads/fixture/execution"]), fixture.head);
        client.finish();
        let mut client = fixture.client();
        assert_eq!(client.call("cadence_apply", request.clone()), pushed);
        let mut changed = request;
        changed["request"]["authorization"] = json!("changed");
        assert_eq!(client.call("cadence_apply", changed)["code"], "request-reused");
        let read = client.call("cadence_query", json!({"operation":"land-read","landing":landing["id"]}));
        assert!(read["landing"]["authorizations"].as_array().unwrap().contains(&auth));
        let receipts: Vec<_> = read["landing"]["steps"].as_array().unwrap().iter().filter(|s| !s["receipt"].is_null()).collect();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0]["receipt"], pushed["receipt"]);
        assert_eq!(read["git"]["branch"], "fixture/execution");
        assert_eq!(read["git"]["ahead"], 0);
        assert_eq!(read["git"]["dirty"], false);
        assert_eq!(read["git"]["remote"]["source_head"], fixture.head);
        assert_eq!(read["tracker"]["issues"], json!([]));
        let current = read["landing"].clone();
        let open_auth = Publishing::authorize(&mut client, &current, "open-authorized", inputs[1].clone());
        let opened = client.call("cadence_apply", json!({"operation":"land-open","request":{"request_id":"open-effect",
            "landing":current["id"],"expected_generation":current["generation"],"authorization":open_auth["id"],"inputs":inputs[1]}}));
        assert_eq!(opened["status"], "ok", "{opened}");
        assert_eq!(opened["receipt"]["result"]["number"], 7);
        let merge_auth = Publishing::authorize(&mut client, &opened["landing"], "merge-authorized", inputs[2].clone());
        let merged = client.call("cadence_apply", json!({"operation":"land-merge","request":{"request_id":"merge-effect",
            "landing":current["id"],"expected_generation":opened["landing"]["generation"],"authorization":merge_auth["id"],"inputs":inputs[2]}}));
        assert_eq!(merged["status"], "ok", "{merged}");
        assert_eq!(git_value(project, &["branch", "--show-current"]), "fixture/execution");
        let calls = fixture.invocations();
        assert_eq!(calls.iter().filter(|args| args.as_array().unwrap().contains(&json!("POST"))).count(), 1);
        assert_eq!(calls.iter().filter(|args| args.as_array().unwrap().contains(&json!("PUT"))).count(), 1);
        assert!(calls.iter().any(|args| args.as_array().unwrap().contains(&json!("title=Confirmed title"))));
        assert!(calls.iter().any(|args| args.as_array().unwrap().contains(&json!("body=Confirmed body\nSecond line"))));
        // A grant recorded before HEAD changes does not cover that new commit.
        let stale_auth = Publishing::authorize(&mut client, &other, "stale-head", inputs[0].clone());
        git(project, &["commit", "--allow-empty", "-m", "Fixture changed head"]);
        let refused = client.call("cadence_apply", json!({"operation":"land-publish","request":{"request_id":"changed-head",
            "landing":other["id"],"expected_generation":1,"authorization":stale_auth["id"],"inputs":inputs[0]}}));
        assert_eq!(refused["code"], "landing-source-changed", "{refused}");
        assert_eq!(git_value(fixture.remote.path(), &["rev-parse", "refs/heads/fixture/execution"]), fixture.head);
        client.finish();
    }
    let native = Fixture::new(&[16]);
    let member = deferred(native.project(), 16);
    let fixture = Publishing::attach(native.temp);
    let mut client = fixture.client();
    let landing = fixture.start(&mut client, "unruled");
    let local_refs = git_value(fixture.project.path(), &["show-ref"]);
    let remote_refs = git_value(fixture.remote.path(), &["show-ref"]);
    for (index, operation) in operations.iter().enumerate() {
        let auth = Publishing::authorize(&mut client, &landing, &format!("unruled-{index}"), inputs[index].clone());
        let refused = client.call("cadence_apply", json!({"operation":operation,"request":{
            "request_id":format!("unruled-effect-{index}"),"landing":landing["id"],"expected_generation":1,"authorization":auth["id"],"inputs":inputs[index]}}));
        assert_eq!(refused["code"], "landing-unsettled", "{refused}");
        assert_eq!(refused["details"]["landing"], landing["id"]);
        assert_eq!(refused["details"]["step"], steps[index]);
        assert_eq!(refused["unsettled"], json!([{"kind":"deferred","phase":16,"identity":member}]));
    }
    assert_eq!(git_value(fixture.project.path(), &["show-ref"]), local_refs);
    assert_eq!(git_value(fixture.remote.path(), &["show-ref"]), remote_refs);
    assert!(fixture.invocations().is_empty());
    client.finish();
}

#[test]
fn phase15_prune_retries_to_one_result_from_every_write_point() {
    prune_fixture::exercise();
}

#[test]
fn phase15_close_refuses_unsettled_records_and_land_refuses_unruled_deferred() {
    let fixture = Fixture::new(&[15, 16]);
    let project = fixture.project();
    let (scan, _, clear) = risk(project, 15, true);
    let member = deferred(project, 16);
    let before = query_fixtures::documents(project);
    let expected = json!([
        {"kind":"risk","phase":15,"identity":scan["confirmation"]["decision_id"]},
        {"kind":"deferred","phase":16,"identity":member}
    ]);
    let request = close("close-both", &[15, 16]);
    let refused = apply(project, request.clone());
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["code"], "milestone-unsettled", "{refused}");
    assert_eq!(refused["unsettled"], expected);
    assert_eq!(query_fixtures::documents(project), before);
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
    let before = query_fixtures::documents(project);
    let ready = apply(project, close("risk-after", &[15]));
    assert_eq!(ready["status"], "ok", "{ready}");
    assert_eq!(ready["close"]["state"], "ready");
    assert_eq!(ready["close"]["generation"], 1);
    assert_eq!(ready["close"]["selection"], json!({"phases":[15],"label":"Fixture milestone"}));
    assert_eq!(query_fixtures::documents(project), before, "plan 1 records readiness only");
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
    let before = query_fixtures::documents(project);
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
    assert_eq!(query_fixtures::documents(project), before);
    assert_eq!(apply(project, publish), refused);
    let read = query(project, json!({"operation":"land-read","landing":start["landing"]["id"]}));
    assert_eq!(read["landing"], start["landing"]);
    assert_eq!(read["refusals"][0]["answer"], refused);
    let queue = query(project, json!({"operation":"review-deferred"}));
    assert!(queue["result"]["members"].as_array().unwrap().iter().any(|m| m["member"] == member));
}
#[test]
fn phase15_confirmed_merge_orders_cleanup_and_reap_checks_containment() {
    use landing_fixtures::{Publishing, effect_exit};
    use serde_json::Value;
    let operations = ["land-checkout", "land-pull", "land-tag", "land-reap"];
    let steps = ["checkout", "pull", "tag", "reap"];
    let request = |operation: &str, id: &str, landing: &Value| json!({"operation":operation,"request":{
        "request_id":id,"landing":landing["id"],"expected_generation":landing["generation"]}});
    let confirmation = |landing: &Value, commit: &str, tag: bool, reap: bool| json!({"operation":"land-confirm-merge","request":{
        "request_id":"cleanup-owner-confirmation","landing":landing["id"],"expected_generation":landing["generation"],
        "source":landing["source"],"base":landing["base"],"remote":landing["remote"],
        "merged":{"forge":{"provider":"github","repo":"fixture/repo","host":"github.com"},"pr":7,"commit":commit},
        "tag":if tag { json!({"name":"v15.5.0","message":"Release 15.5.0"}) } else { Value::Null },
        "reap":reap,"owner":"Fixture Owner","at":"2026-09-19T15:00:00Z"}});
    // Each local effect is also interrupted after success, before its receipt.
    for interrupted in [None, Some(0), Some(1), Some(2), Some(3)] {
        let fixture = Publishing::new(false);
        let (mut landing, merged) = fixture.cleanup_ready(true);
        let project = fixture.project.path();
        let trace = project.join(".run/resume.trace");
        let mut client = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
        let refs = git_value(project, &["show-ref"]);
        let documents = query_fixtures::documents(project);
        let records = reopened(project).snapshot.data;
        for operation in operations {
            let refused = client.call("cadence_apply", request(operation, &format!("cleanup-before-{operation}"), &landing));
            assert_eq!(refused["code"], "landing-merge-confirmation-required", "{refused}");
            assert!(refused["reason"].as_str().unwrap().contains("confirmation"));
        }
        assert!(fixture.cleanup_commands().is_empty());
        assert_eq!(git_value(project, &["show-ref"]), refs);
        assert_eq!(git_value(project, &["branch", "--show-current"]), "fixture/execution");
        let confirm = confirmation(&landing, &merged, true, true);
        let confirmed = client.call("cadence_apply", confirm.clone());
        assert_eq!(confirmed["status"], "ok", "{confirmed}");
        let owner_record = confirmed["confirmation"].clone();
        assert_eq!(owner_record["owner"], "Fixture Owner");
        assert_eq!(owner_record["merged"]["commit"], merged);
        assert_eq!(owner_record["source"], landing["source"]);
        assert_eq!(owner_record["base"], landing["base"]);
        assert_eq!(owner_record["expected_generation"], landing["generation"]);
        landing = confirmed["landing"].clone();
        assert_eq!(client.call("cadence_apply", confirm.clone()), confirmed);
        let mut changed = confirm;
        changed["request"]["reap"] = json!(false);
        assert_eq!(client.call("cadence_apply", changed)["code"], "request-reused");
        let refused = client.call("cadence_apply", request("land-tag", "cleanup-out-of-order", &landing));
        assert_eq!(refused["code"], "landing-predecessor-required", "{refused}");
        assert_eq!(refused["details"]["predecessor"], "checkout");
        assert!(fixture.cleanup_commands().is_empty());
        let mut receipts: Vec<Value> = Vec::new();
        for index in 0..4 {
            let call = request(operations[index], &format!("cleanup-local-{index}"), &landing);
            if interrupted == Some(index) {
                client.finish();
                let child = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str()),
                    ("CADENCE_LANDING_EXIT_AFTER_EFFECT", steps[index].as_ref())]);
                effect_exit(child, call.clone());
                let saved = reopened(project).snapshot.data;
                assert!(saved["landings"]["records"][landing["id"].as_str().unwrap()]["steps"][[3, 4, 5, 7][index]]["receipt"].is_null());
                client = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
            }
            let answer = client.call("cadence_apply", call.clone());
            assert_eq!(answer["status"], "ok", "{answer}");
            assert_eq!(answer["receipt"]["confirmation"], owner_record["id"]);
            assert_eq!(answer["receipt"]["step"], steps[index]);
            assert_eq!(answer["receipt"]["state"], "done");
            assert!(answer["receipt"]["intended"].is_object());
            assert!(answer["receipt"]["actual"].is_object());
            if index > 0 { assert_eq!(answer["receipt"]["predecessors"][index - 1], receipts[index - 1]["id"]); }
            receipts.push(answer["receipt"].clone());
            landing = answer["landing"].clone();
            assert_eq!(client.call("cadence_apply", call), answer);
        }
        client.finish();
        let commands = fixture.cleanup_commands();
        assert_eq!(commands.iter().map(|args| args[1].as_str()).collect::<Vec<_>>(), ["checkout", "pull", "tag", "branch"]);
        assert!(commands[0].contains(&"main".to_owned()));
        assert!(commands[1].contains(&"--ff-only".to_owned()));
        assert!(commands[1].contains(&fixture.remote.path().to_str().unwrap().to_owned()));
        assert!(commands[1].contains(&"main".to_owned()));
        assert!(commands[3].contains(&"-d".to_owned()));
        assert!(!commands[3].contains(&"-D".to_owned()));
        assert_eq!(git_value(project, &["branch", "--show-current"]), "main");
        assert_eq!(git_value(project, &["rev-parse", "main"]), merged);
        assert_eq!(git_value(project, &["cat-file", "-t", "refs/tags/v15.5.0"]), "tag");
        assert_eq!(git_value(project, &["rev-parse", "v15.5.0^{}"]), merged);
        assert_eq!(git_value(project, &["for-each-ref", "--format=%(objectname)", "refs/heads/fixture/execution"]), "");
        let reflog = git_value(project, &["reflog", "show", "--format=%H %gs", "HEAD"]);
        assert!(reflog.contains("checkout: moving from fixture/execution to main"));
        assert!(reflog.lines().next().unwrap().starts_with(&merged));
        let mut client = fixture.client();
        let read = client.call("cadence_query", json!({"operation":"land-read","landing":landing["id"]}));
        assert_eq!(read["landing"], landing);
        assert_eq!(read["landing"]["merge_confirmation"], owner_record);
        for receipt in &receipts { assert!(read["done"].as_array().unwrap().contains(receipt)); }
        client.finish();
        assert_eq!(query_fixtures::documents(project), documents);
        let after = reopened(project).snapshot.data;
        for key in ["rail_observations", "rail_receipts", "deferred"] { assert_eq!(after[key], records[key]); }
        assert_eq!(fixture.mutations().len(), 2, "cleanup must not mutate a tracker");
    }
    for contained in [false, true] {
        let fixture = Publishing::new(false);
        let (landing, merged) = fixture.cleanup_ready(contained);
        let project = fixture.project.path();
        let trace = project.join(".run/resume.trace");
        let mut client = fixture.client_with_env(&[("GIT_TRACE2_EVENT", trace.as_os_str())]);
        let confirmed = client.call("cadence_apply", confirmation(&landing, &merged, false, !contained));
        assert_eq!(confirmed["status"], "ok", "{confirmed}");
        let mut landing = confirmed["landing"].clone();
        for operation in &operations[..3] {
            let answer = client.call("cadence_apply", request(operation, &format!("cleanup-skip-{operation}"), &landing));
            assert_eq!(answer["status"], "ok", "{answer}");
            if *operation == "land-tag" { assert_eq!(answer["receipt"]["state"], "skipped"); }
            landing = answer["landing"].clone();
        }
        let answer = client.call("cadence_apply", request("land-reap", "cleanup-final-reap", &landing));
        if contained { assert_eq!(answer["receipt"]["state"], "skipped", "{answer}"); }
        else {
            assert_eq!(answer["code"], "landing-reap-uncontained", "{answer}");
            assert_eq!(answer["details"]["source"], json!({"branch":"fixture/execution","head":fixture.head}));
            assert_eq!(answer["details"]["base"], json!({"branch":"main","head":merged}));
            assert!(answer["reason"].as_str().unwrap().contains("fixture/execution"));
            assert!(answer["reason"].as_str().unwrap().contains("main"));
        }
        client.finish();
        assert_eq!(fixture.cleanup_commands().iter().map(|args| args[1].as_str()).collect::<Vec<_>>(), ["checkout", "pull"]);
        assert_eq!(git_value(project, &["rev-parse", "fixture/execution"]), fixture.head);
    }
}

#[test]
fn reap_refusal_codes_name_the_gate_that_fired() {
    use cadence::{landing::{cleanup, model::{Landing, Remote, Revision, Start, Step}}, store::Error};
    use landing_fixtures::Publishing;
    use serde_json::Value;

    let fixture = Publishing::new(false);
    let (landing, merged) = fixture.cleanup_ready(true);
    let root = fixture.project.path();
    let mut client = fixture.client();
    let confirmed = client.call("cadence_apply", json!({"operation":"land-confirm-merge","request":{
        "request_id":"reap-code-confirmation","landing":landing["id"],"expected_generation":landing["generation"],
        "source":landing["source"],"base":landing["base"],"remote":landing["remote"],
        "merged":{"forge":{"provider":"github","repo":"fixture/repo","host":"github.com"},"pr":7,"commit":merged},
        "tag":null,"reap":true,"owner":"Fixture Owner","at":"2026-09-19T15:00:00Z"}}));
    assert_eq!(confirmed["status"], "ok", "{confirmed}");
    let mut landing = confirmed["landing"].clone();
    let request = |operation: &str, landing: &Value| json!({"operation":operation,"request":{
        "request_id":format!("reap-code-{operation}"),"landing":landing["id"],"expected_generation":landing["generation"]}});
    for operation in ["land-checkout", "land-pull", "land-tag"] {
        let answer = client.call("cadence_apply", request(operation, &landing));
        assert_eq!(answer["status"], "ok", "{answer}");
        landing = answer["landing"].clone();
    }
    git(root, &["checkout", "fixture/execution"]);
    let answer = client.call("cadence_apply", request("land-reap", &landing));
    git(root, &["checkout", "main"]);
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["code"], "landing-reap-checked-out", "{answer}");
    assert_eq!(answer["details"]["step"], "reap");
    assert!(answer["reason"].as_str().unwrap().contains("fixture/execution"));
    client.finish();

    let landing = Landing::new(root.to_str().unwrap().into(), &Start {
        request_id:"reap-code-direct".into(), occurrence:"reap-code-fixture".into(), expected_generation:1,
        source:Revision { branch:"fixture/execution".into(), head:fixture.head.clone() },
        base:Revision { branch:"main".into(), head:merged.clone() },
        remote:Remote { name:"origin".into(), url:fixture.remote.path().to_str().unwrap().into() },
    });
    let clean = cleanup::State {
        branch:"main".into(), head:merged.clone(), source:Some(fixture.head.clone()), base:merged.clone(),
        index:String::new(), worktree:String::new(), tag:None, tag_target:None, tag_message:None,
    };
    let checked_out = cleanup::State { branch:landing.source.branch.clone(), ..clean.clone() };
    let moved = cleanup::State { source:Some(merged), ..clean.clone() };
    for (state, config, code) in [
        (clean, json!({"git":{"protected_branches":["fixture/execution"]}}), "landing-reap-protected"),
        (checked_out, json!({}), "landing-reap-checked-out"),
        (moved, json!({}), "landing-reap-moved"),
    ] {
        let failure = cleanup::reap_gate(root, &landing, &state, &config).unwrap_err();
        assert_eq!(failure.code, Some(code));
        let refused = cleanup::refused(root, &landing, &Step::Reap, &failure);
        assert_eq!(refused["status"], "refused", "{refused}");
        assert_eq!(refused["code"], code, "{refused}");
    }
    let error = Error::Invalid("local state changed before invocation".into());
    assert_eq!(cleanup::failure(root, &landing, &Step::Reap, &error)["code"], "landing-cleanup-discrepancy");
    let failure = cleanup::Failure::from(error);
    assert_eq!(failure.code, None);
    assert_eq!(cleanup::refused(root, &landing, &Step::Reap, &failure)["code"], "landing-cleanup-discrepancy");
}

#[test]
fn phase15_undo_reverts_exact_hashes_and_marks_the_record() {
    use landing_fixtures::UndoFixture;
    use serde_json::Value;
    let request = |id: &str, manifest: &Value, mode: &str| json!({"operation":"undo-phase","request":{
        "request_id":id,"phase":13,"manifest":manifest["id"],"mode":mode}});
    let completed_hashes = |answer: &Value| answer["undo"]["completed"].as_array().unwrap().iter()
        .map(|step| step["hash"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    for mode in ["committed", "no-commit", "conflict", "legacy"] {
        let fixture = UndoFixture::new(mode != "legacy");
        let project = fixture.project();
        let [first, second, third, docs]: [String; 4] = fixture.hashes.clone().try_into().unwrap();
        let expected = vec![docs, third, second, first];
        if mode == "conflict" {
            fs::write(project.join("src/third.txt"), "later conflicting work\n").unwrap();
            git(project, &["add", "src/third.txt"]);
            git(project, &["commit", "-m", "feat(99): retain later conflicting work"]);
        }
        let before_head = git_value(project, &["rev-parse", "HEAD"]);
        let before_docs = query_fixtures::documents(project);
        let before_store = reopened(project).snapshot.data;
        let mut client = fixture.client();
        if mode != "legacy" {
            let progress = client.call("cadence_query", json!({"operation":"progress"}));
            assert_eq!(progress["status"], "ok", "{progress}");
            assert_eq!(progress["phases"][0]["status"], "complete", "{progress}");
        }
        let read = client.call("cadence_query", json!({"operation":"undo-read","phase":13}));
        assert_eq!(read["status"], "ok", "undo-read must expose the recorded exact manifest: {read}");
        let manifest = &read["manifest"];
        assert_eq!(manifest["hashes"], json!(fixture.hashes), "{read}");
        assert_eq!(manifest["source"], if mode == "legacy" { "SUMMARY" } else { "execution" });
        assert!(!manifest["hashes"].as_array().unwrap().contains(&json!(fixture.decoy)));
        let wire = request(&format!("undo-{mode}"), manifest, if mode == "no-commit" { mode } else { "committed" });
        // A different immutable manifest must refuse before any Git mutation.
        let mut wrong = wire.clone();
        wrong["request"]["request_id"] = json!(format!("undo-{mode}-wrong"));
        wrong["request"]["manifest"] = json!("not-the-recorded-manifest");
        let refusal = client.call("cadence_apply", wrong);
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(git_value(project, &["rev-parse", "HEAD"]), before_head);
        assert!(fixture.reverts().is_empty());
        let answer = client.call("cadence_apply", wire.clone());
        assert_eq!(answer["status"], if mode == "conflict" { "refused" } else { "ok" }, "{answer}");
        if mode == "conflict" {
            assert_eq!(completed_hashes(&answer), vec![expected[0].clone()]);
            assert_eq!(answer["undo"]["conflict"]["hash"], expected[1]);
            assert_eq!(answer["undo"]["conflict"]["paths"], json!(["src/third.txt"]));
            assert!(git_value(project, &["ls-files", "-u"]).contains("src/third.txt"));
            assert_eq!(fixture.reverts(), expected[..2]);
            assert_eq!(git_value(project, &["rev-list", "--count", &format!("{before_head}..HEAD")]), "1");
        } else {
            assert_eq!(completed_hashes(&answer), expected);
            assert_eq!(fixture.reverts(), expected);
            for path in ["src/p13.txt", "src/second.txt", "src/third.txt", "docs.txt"] {
                assert_eq!(fs::read_to_string(project.join(path)).unwrap(), "pending\n");
            }
        }
        assert_eq!(fs::read_to_string(project.join("decoy.txt")).unwrap(), "keep this unrelated commit\n");
        client.finish();
        let after = reopened(project).snapshot.data;
        for namespace in [cadence::execution::history::NAMESPACE, cadence::execution::history::PLAN_NAMESPACE,
            cadence::execution::admission::NAMESPACE, cadence::verification::persistence::NAMESPACE] {
            assert_eq!(after[namespace], before_store[namespace], "retained {namespace}");
        }
        assert_eq!(after["verification"], before_store["verification"], "verification evidence is retained");
        if mode == "no-commit" || mode == "conflict" {
            assert_eq!(after["execution"]["occurrences"]["13"], before_store["execution"]["occurrences"]["13"]);
            assert_eq!(after["cursor"], before_store["cursor"]);
            assert_eq!(query_fixtures::documents(project), before_docs);
            if mode == "no-commit" {
                assert_eq!(git_value(project, &["rev-parse", "HEAD"]), before_head);
                assert_eq!(git_value(project, &["diff", "--cached", "--name-only"]),
                    "docs.txt\nsrc/p13.txt\nsrc/second.txt\nsrc/third.txt");
            }
        } else {
            let subjects = git_value(project, &["log", "--reverse", "--format=%s", &format!("{before_head}..HEAD")]);
            let subjects: Vec<_> = subjects.lines().collect();
            assert_eq!(subjects.len(), 4, "document repair belongs to the last revert, with no fifth commit");
            for (subject, original) in subjects.iter().zip(&expected) { assert!(subject.contains(original), "{subjects:?}"); }
            if mode != "legacy" {
                let marker = &after["execution"]["occurrences"]["13"]["undone"];
                assert_eq!(marker["undo"], answer["undo"]["id"]);
                assert_eq!(marker["manifest"], manifest["id"]);
                assert_eq!(marker["occurrence"], manifest["occurrence"]);
            } else {
                assert!(after["execution"]["occurrences"]["13"].is_null(), "legacy undo invents no native execution");
            }
            let roadmap = fs::read_to_string(project.join(".planning/ROADMAP.md")).unwrap();
            assert!(roadmap.contains("- [ ] **Phase 13:"), "{roadmap}");
            let changed = git_value(project, &["show", "--format=", "--name-only", "HEAD"]);
            assert!(changed.contains(".planning/ROADMAP.md") || mode == "legacy", "{changed}");
            assert!(changed.contains(".planning/STATE.md"), "{changed}");
            assert_eq!(git_value(project, &["status", "--porcelain"]), "");
        }
        let trace_before_retry = fixture.reverts();
        let mut restarted = fixture.client();
        let reread = restarted.call("cadence_query", json!({"operation":"undo-read","phase":13}));
        assert_eq!(reread["manifest"], *manifest);
        let retry = restarted.call("cadence_apply", wire);
        assert_eq!(retry, answer, "restart replays the exact persisted completed set");
        assert_eq!(fixture.reverts(), trace_before_retry, "retry runs no successful hash twice");
        if mode == "committed" || mode == "legacy" {
            let progress = restarted.call("cadence_query", json!({"operation":"progress"}));
            assert_eq!(progress["status"], "ok", "{progress}");
            assert_eq!(progress["phases"][0]["status"], "planned", "{progress}");
            assert_eq!(progress["issues"], json!([]));
            let state = fs::read_to_string(project.join(".planning/STATE.md")).unwrap();
            assert!(state.contains("13") && state.to_lowercase().contains("planned"), "{state}");
            assert!(state.contains(progress["next"]["instruction"].as_str().unwrap()), "{state}; {progress}");
            let selected = restarted.call("cadence_query", json!({"operation":"execute-next","phase":13}));
            assert_ne!(selected["code"], "phase-not-current", "{selected}");
        }
        restarted.finish();
    }
}

#[test]
fn phase15_version_drift_is_reported_before_bump_or_tag() {
    use serde_json::Value;
    let propose = |landing: &Value, version: &str, id: &str| json!({"operation":"milestone-release","request":{
        "request_id":id,"landing":landing["id"],"expected_generation":landing["generation"],
        "version":version,"tag":format!("v{version}"),"manifest":{"path":"release.json","format":"json"}}});
    let confirm = |report: &Value, id: &str| json!({"operation":"milestone-release-confirm","request":{
        "request_id":id,"release":report["id"],"digest":report["digest"],
        "owner":"Fixture Owner","at":"2026-09-20T12:00:00Z"}});
    let snapshot = |project: &std::path::Path| (
        std::fs::read(project.join("release.json")).unwrap(),
        git_value(project, &["rev-parse", "HEAD"]),
        std::fs::read(project.join(".git/index")).unwrap(),
        git_value(project, &["show-ref", "--tags"]),
    );
    let mut fixture = landing_fixtures::release_fixture();
    let project = fixture.project.path().to_owned();
    let mut client = fixture.client();
    let landing = fixture.start(&mut client, "release-main");
    let before = snapshot(&project);
    let refused = client.call("cadence_apply", propose(&landing, "1.2.0", "release-collision"));
    assert_eq!(refused["code"], "release-collision", "{refused}");
    assert_eq!(refused["details"]["collision"]["tag"], "v1.2.0");
    assert_eq!(snapshot(&project), before);
    let request = propose(&landing, "1.3.0", "release-proposal");
    let answer = client.call("cadence_apply", request.clone());
    assert_eq!(answer["status"], "ok", "{answer}");
    let report = answer["release"].clone();
    assert_eq!(report["state"], "awaiting-confirmation");
    assert_eq!(report["manifest_version"], "1.1.0");
    assert_eq!(report["newest"]["tag"], "v1.2.0");
    assert_eq!(report["newest"]["version"], "1.2.0");
    assert_eq!(report["newest"]["commit"], fixture.head);
    assert_eq!(report["drift"], true);
    assert_eq!(report["head"], fixture.head);
    assert_eq!(report["request"]["manifest"], json!({"path":"release.json","format":"json"}));
    assert_eq!(report["manifest_bytes"], json!(before.0));
    assert_eq!(report["tags"].as_array().unwrap().len(), 4);
    assert_eq!(snapshot(&project), before);
    client.finish();
    client = fixture.client();
    assert_eq!(client.call("cadence_apply", request), answer);
    let mut wrong = confirm(&report, "release-wrong-digest");
    wrong["request"]["digest"] = json!("wrong");
    assert_eq!(client.call("cadence_apply", wrong)["code"], "release-confirmation");
    assert_eq!(snapshot(&project), before);
    let confirmation = confirm(&report, "release-confirm");
    let bumped = client.call("cadence_apply", confirmation.clone());
    assert_eq!(bumped["status"], "ok", "{bumped}");
    let commit = bumped["commit"].as_str().unwrap();
    assert_eq!(git_value(&project, &["rev-parse", "HEAD"]), commit);
    assert_eq!(git_value(&project, &["rev-list", "--count", &format!("{}..HEAD", before.1)]), "1");
    assert_eq!(git_value(&project, &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]), "release.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(project.join("release.json")).unwrap()).unwrap();
    assert_eq!(manifest, json!({"version":"1.3.0","keep":true}));
    assert_eq!(std::fs::read(project.join("sibling.json")).unwrap(), b"{\"version\":\"9.0.0\"}\n");
    assert_eq!(git_value(&project, &["tag", "--list", "v1.3.0"]), "");
    assert_eq!(git_value(fixture.remote.path(), &["tag", "--list", "v1.3.0"]), "");
    client.finish();
    client = fixture.client();
    assert_eq!(client.call("cadence_apply", confirmation), bumped);
    assert_eq!(git_value(&project, &["rev-parse", "HEAD"]), commit);
    let bound = client.call("cadence_query", json!({"operation":"land-read","landing":landing["id"]}))["landing"].clone();
    assert_eq!(bound["source"]["head"], commit);
    assert_eq!(bound["release"]["version"], "1.3.0");
    assert_eq!(bound["release"]["tag"], "v1.3.0");
    let early = client.call("cadence_apply", json!({"operation":"land-tag","request":{
        "request_id":"release-early-tag","landing":bound["id"],"expected_generation":bound["generation"]}}));
    assert_eq!(early["code"], "landing-merge-confirmation-required", "{early}");
    client.finish();
    // Drive the separately authorized publish/merge path on this same release.
    fixture.head = commit.to_owned();
    fixture.durable_forge();
    git(&project, &["branch", "main", &fixture.base]);
    client = fixture.client();
    let forge = json!({"provider":"github","repo":"fixture/repo","host":"github.com"});
    let mut bound = bound;
    for (index, (operation, inputs)) in [
        ("land-publish", json!({"step":"push"})),
        ("land-open", json!({"step":"open","forge":forge,"title":"Release","body":"Release 1.3.0"})),
        ("land-merge", json!({"step":"merge","forge":forge,"pr":7})),
    ].into_iter().enumerate() {
        let auth = landing_fixtures::Publishing::authorize(&mut client, &bound, &format!("release-publish-{index}"), inputs.clone());
        let result = client.call("cadence_apply", json!({"operation":operation,"request":{
            "request_id":format!("release-effect-{index}"),"landing":bound["id"],"expected_generation":bound["generation"],
            "authorization":auth["id"],"inputs":inputs}}));
        assert_eq!(result["status"], "ok", "{result}");
        bound = result["landing"].clone();
    }
    git(&project, &["push", "origin", "HEAD:main"]);
    let confirmed = client.call("cadence_apply", json!({"operation":"land-confirm-merge","request":{
        "request_id":"release-merge-confirm","landing":bound["id"],"expected_generation":bound["generation"],
        "source":bound["source"],"base":bound["base"],"remote":bound["remote"],
        "merged":{"forge":forge,"pr":7,"commit":commit},"tag":{"name":"v1.3.0","message":"Release 1.3.0"},
        "reap":false,"owner":"Fixture Owner","at":"2026-09-20T13:00:00Z"}}));
    assert_eq!(confirmed["status"], "ok", "{confirmed}");
    bound = confirmed["landing"].clone();
    for operation in ["land-checkout", "land-pull"] {
        let result = client.call("cadence_apply", json!({"operation":operation,"request":{
            "request_id":format!("release-{operation}"),"landing":bound["id"],"expected_generation":bound["generation"]}}));
        assert_eq!(result["status"], "ok", "{result}");
        bound = result["landing"].clone();
    }
    git(&project, &["tag", "1.3.0"]);
    let refused = client.call("cadence_apply", json!({"operation":"land-tag","request":{
        "request_id":"release-alias-tag","landing":bound["id"],"expected_generation":bound["generation"]}}));
    assert_eq!(refused["status"], "refused", "{refused}");
    assert!(refused["reason"].as_str().unwrap().contains("1.3.0"));
    assert_eq!(git_value(&project, &["tag", "--list", "v1.3.0"]), "");
    git(&project, &["tag", "-d", "1.3.0"]);
    let tagged = client.call("cadence_apply", json!({"operation":"land-tag","request":{
        "request_id":"release-final-tag","landing":bound["id"],"expected_generation":bound["generation"]}}));
    assert_eq!(tagged["status"], "ok", "{tagged}");
    assert_eq!(git_value(&project, &["rev-parse", "v1.3.0^{commit}"]), commit);
    assert_eq!(git_value(fixture.remote.path(), &["tag", "--list", "v1.3.0"]), "");
    client.finish();
    // Every observed basis component is independently stale; no changed basis bumps.
    for change in ["manifest", "tags", "head"] {
        let fixture = landing_fixtures::release_fixture();
        let project = fixture.project.path();
        let mut client = fixture.client();
        let landing = fixture.start(&mut client, "release-stale");
        let report = client.call("cadence_apply", propose(&landing, "1.3.0", "release-stale-proposal"))["release"].clone();
        match change {
            "manifest" => std::fs::write(project.join("release.json"), "{\"version\":\"1.1.1\"}\n").unwrap(),
            "tags" => git(project, &["tag", "another-label"]),
            _ => git(project, &["commit", "--allow-empty", "-m", "Fixture head advance"]),
        }
        let before = snapshot(project);
        let refused = client.call("cadence_apply", confirm(&report, "release-stale-confirm"));
        assert_eq!(refused["code"], "release-basis-changed", "{change}: {refused}");
        assert_eq!(snapshot(project), before);
        assert_eq!(git_value(project, &["tag", "--list", "v1.3.0"]), "");
        client.finish();
    }
    let fixture = landing_fixtures::release_fixture();
    let project = fixture.project.path();
    let mut client = fixture.client();
    let landing = fixture.start(&mut client, "release-inputs");
    git(project, &["tag", "1.3.0+build.7"]);
    let refused = client.call("cadence_apply", propose(&landing, "1.3.0", "release-alias-collision"));
    assert_eq!(refused["code"], "release-collision", "{refused}");
    assert_eq!(refused["details"]["collision"]["tag"], "1.3.0+build.7");
    git(project, &["tag", "v1.10.0"]);
    git(project, &["tag", "v1.9.0"]);
    let report = client.call("cadence_apply", propose(&landing, "2.0.0", "release-semver"));
    assert_eq!(report["release"]["newest"]["tag"], "v1.10.0", "{report}");
    let mut unsupported = propose(&landing, "2.0.0", "release-unsupported");
    unsupported["request"]["manifest"]["format"] = json!("toml");
    assert_eq!(client.call("cadence_apply", unsupported)["code"], "release-input");
    std::fs::remove_file(project.join("release.json")).unwrap();
    let refused = client.call("cadence_apply", propose(&landing, "2.0.0", "release-unreadable"));
    assert_eq!(refused["code"], "release-input", "{refused}");
    assert!(refused["reason"].as_str().unwrap().contains("release.json"));
    client.finish();
}
