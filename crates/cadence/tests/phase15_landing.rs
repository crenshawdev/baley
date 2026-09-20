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
fn phase15_interrupted_landing_reconciles_against_the_remote() {
    use phase15::{Publishing, effect_exit};
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
    for index in 0..3 {
        let auth = Publishing::authorize(&mut client, &landing, &format!("resume-absent-grant-{index}"), inputs[index].clone());
        let mut missing = request("land-resume", &format!("resume-no-grant-{index}"), &landing, &auth, &inputs[index]);
        missing["request"]["authorization"] = Value::Null;
        let calls = fixture.invocations();
        assert_eq!(client.call("cadence_apply", missing)["code"], "landing-authorization-required");
        assert_eq!(fixture.invocations(), calls, "permission precedes remote reads");
        let answer = client.call("cadence_apply", request("land-resume", &format!("resume-absent-effect-{index}"), &landing, &auth, &inputs[index]));
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
    use phase15::{Publishing, imported_auto_close};
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
