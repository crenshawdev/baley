#[allow(dead_code)]
#[path = "support/phase13.rs"]
mod phase13;
#[path = "support/support_records.rs"]
mod support_records;

use cadence::rail::{receipts, risk};
use phase13::Client;
use serde_json::json;
use support_records::{apply, query};

#[test]
fn debug_blocking_fire_holds_resolve_until_a_receipt() {
    use support_records::{consequence, debug_readback, resolve, return_findings};
    let findings = json!([
        {"file":"src/auth/a.rs","line":1,"severity":"high","claim":"unchecked token","failure_scenario":"invalid token admitted"},
        {"file":"src/auth/b.rs","line":1,"severity":"high","claim":"missing permission","failure_scenario":"unauthorized caller admitted"}
    ]);
    for branch in ["gate-pass", "override", "rearm"] {
        let repo = support_records::risk_fixture();
        let project = repo.path();
        for path in ["src/auth/a.rs", "src/auth/b.rs"] {
            support_records::change(project, path, b"jwt.verify(token)\n", true);
        }
        let mut client = Client::open(project);
        apply(&mut client, "debug-open", json!({"request_id":"receipt-open","slug":"token-fix",
            "expected_version":0,"symptom":"invalid token admitted"}));
        let before = resolve(&mut client, "token-fix", "receipt-initial", true);
        assert_eq!(before["code"], "debug-review-pending", "{before}");
        client.finish();
        let view = phase13::reopened(project);
        receipts::confirmed_history(&view).unwrap();
        let parent = receipts::history(&view.snapshot.data).unwrap().1.remove(0);
        assert_eq!(parent.review_scope, ["src/auth/a.rs", "src/auth/b.rs"]);
        assert!(before.to_string().contains(&parent.id));
        let mut client = Client::open(project);
        let original = return_findings(&mut client, &parent.id, "parent", findings.clone());
        let original_id = original["identity"]["original"].as_str().unwrap();
        let after = resolve(&mut client, "token-fix", "receipt-after-return", true);
        assert_eq!(after["code"], "debug-review-pending", "{after}");
        for needle in [&parent.id, original_id, "unchecked token", "missing permission"] {
            assert!(after.to_string().contains(needle), "pending refusal omits {needle}: {after}");
        }
        let pending = debug_readback(&mut client, project, "token-fix",
            &[&parent.id, original_id, "unchecked token", "missing permission"]);
        assert_eq!(pending["record"]["status"], "open");
        client.finish();
        let mut client = Client::open(project);
        assert_eq!(debug_readback(&mut client, project, "token-fix", &[original_id]), pending);

        // A receipt naming a different fire never settles this one.
        let mut wrong = parent.clone();
        wrong.id = "unrelated-fire".into();
        let negative = consequence(&mut client, "wrong-fire", &wrong,
            json!({"kind":"gate-pass","evidence_id":"fixture-evidence"}));
        assert_eq!(negative["status"], "refused", "{negative}");
        assert_eq!(resolve(&mut client, "token-fix", "receipt-negative", true)["code"], "debug-review-pending");

        let mut accepted_ids = vec!["settled-receipt"];
        let terminal = match branch {
            "override" => {
                let blank = consequence(&mut client, "blank-override", &parent, json!({"kind":"override","reason":"  "}));
                assert_eq!(blank["status"], "refused", "{blank}");
                assert!(blank.to_string().contains("nonblank"), "{blank}");
                client.finish();
                assert!(receipts::history(&phase13::reopened(project).snapshot.data).unwrap().2.is_empty());
                client = Client::open(project);
                let accepted = consequence(&mut client, "settled-receipt", &parent,
                    json!({"kind":"override","reason":"Fixture owner accepts these two findings"}));
                assert_eq!(accepted["status"], "ok", "{accepted}");
                parent.clone()
            }
            "rearm" => {
                phase13::git(project, &["rm", "--cached", "--", "src/auth/b.rs"]);
                let scan = client.call("cadence_apply", json!({"operation":"risk-check","request_id":"narrowed-scan",
                    "scope":{"kind":"root-debug","occurrence":"token-fix"},
                    "source":{"kind":"staged","base":parent.binding.material.base_id()}}));
                assert_eq!(scan["status"], "ok", "{scan}");
                let status = client.call("cadence_query", json!({"operation":"risk-status",
                    "scope":{"kind":"root-debug","occurrence":"token-fix"},
                    "source":{"kind":"staged","base":parent.binding.material.base_id()}}));
                let recorded: risk::Recorded = serde_json::from_value(status["current_observation"].clone()).unwrap();
                let material = recorded.observation.resolution.material().unwrap();
                let admitted = client.call("cadence_apply", json!({"operation":"review-admit","request":{
                    "replay_key":"narrowed-admission","caller":"debug","trigger":"risk_surface","specialist":null,
                    "project":project,"cycle":"live","home":{"kind":"root-debug","id":"token-fix"},
                    "discriminator":"token-fix","phase":null,"plan":null,"anchor":null,"round":2,
                    "target":{"kind":"staged-tree","base":material.base_id(),"index":material.tip_id(),"head":null},
                    "risk_observation":"narrowed-scan"}}));
                assert_eq!(admitted["status"], "ok", "{admitted}");
                let child = receipts::Fire { id: admitted["result"]["fire"].as_str().unwrap().into(),
                    binding: receipts::Binding::new(parent.binding.boundary.clone(), &recorded).unwrap(),
                    review_scope: vec!["src/auth/a.rs".into()], rearm_of: Some(parent.id.clone()) };
                let rearm = json!({"kind":"rearm","next_fire":child});
                let accepted = consequence(&mut client, "parent-rearm", &parent, rearm.clone());
                assert_eq!(accepted["status"], "ok", "{accepted}");
                assert_eq!(resolve(&mut client, "token-fix", "receipt-child-unfired", true)["status"], "refused");
                let fired = client.call("cadence_apply", json!({"operation":"risk-fire","request_id":"narrowed-fire","fire":child}));
                assert_eq!(fired["status"], "ok", "{fired}");
                return_findings(&mut client, &child.id, "child", json!([findings[0]]));
                let blocked = resolve(&mut client, "token-fix", "receipt-child-pending", true);
                assert_eq!(blocked["code"], "debug-review-pending", "{blocked}");
                assert!(blocked.to_string().contains(&child.id), "{blocked}");
                debug_readback(&mut client, project, "token-fix", &[&parent.id, &child.id, "parent-rearm", "src/auth/a.rs"]);
                let duplicate = consequence(&mut client, "duplicate-parent", &parent, rearm);
                assert_eq!(duplicate["status"], "refused", "{duplicate}");
                assert!(duplicate.to_string().contains("multiple consequences"), "{duplicate}");
                // Reusing the current scan reaches the rail's one-round validator;
                // a rearmed child cannot originate another narrowed round.
                let grandchild = receipts::Fire { id:"third-fire".into(),
                    binding:child.binding.clone(),
                    review_scope:vec!["src/auth/a.rs".into()],rearm_of:Some(child.id.clone()) };
                let capped = consequence(&mut client, "child-rearm", &child, json!({"kind":"rearm","next_fire":grandchild}));
                assert_eq!(capped["status"], "refused", "{capped}");
                assert!(capped.to_string().contains("one re-arm"), "{capped}");
                accepted_ids.push("parent-rearm");
                child
            }
            _ => parent.clone(),
        };
        if branch != "override" {
            let settled = consequence(&mut client, "settled-receipt", &terminal,
                json!({"kind":"gate-pass","evidence_id":"fixture-evidence"}));
            assert_eq!(settled["status"], "ok", "{settled}");
        }
        let synchronized = debug_readback(&mut client, project, "token-fix", &accepted_ids);
        client.finish();
        let mut client = Client::open(project);
        assert_eq!(debug_readback(&mut client, project, "token-fix", &accepted_ids), synchronized);
        let verified = resolve(&mut client, "token-fix", "receipt-verification-fails", false);
        assert_eq!(verified["status"], "ok", "{verified}");
        assert_eq!(verified["record"]["status"], "open");
        assert_eq!(verified["record"]["attempt_count"], 1);
        let done = resolve(&mut client, "token-fix", "receipt-verification-passes", true);
        assert_eq!(done["status"], "ok", "{done}");
        assert_eq!(done["record"]["status"], "resolved");
        let final_status = debug_readback(&mut client, project, "token-fix", &accepted_ids);
        client.finish();
        let view = phase13::reopened(project);
        receipts::confirmed_history(&view).unwrap();
        assert!(!view.snapshot.data["review"].to_string().contains("\"verified\""));
        let mut client = Client::open(project);
        assert_eq!(debug_readback(&mut client, project, "token-fix", &accepted_ids), final_status);
        client.finish();
    }
}

#[test]
fn debug_resolve_risk_checks_the_index_without_a_phase() {
    for (case, path, bytes, staged, fires) in [
        ("clean", "", &b""[..], false, false),
        ("unstaged", "src/auth/login.rs", &b"jwt.verify(token)\n"[..], false, false),
        ("matched", "src/auth/login.rs", &b"jwt.verify(token)\n"[..], true, true),
        ("clear", "docs/note.txt", &b"plain note\n"[..], true, false),
        ("binary", "opaque.bin", &b"opaque\0bytes\n"[..], true, true),
    ] {
        let repo = support_records::risk_fixture();
        let project = repo.path();
        if !path.is_empty() { support_records::change(project, path, bytes, staged); }
        let base = phase13::git_value(project, &["rev-parse", "HEAD"]);
        let index = phase13::git_value(project, &["write-tree"]);
        if case == "binary" {
            assert!(phase13::git_value(project, &["diff", "--cached"]).contains("Binary files"));
        }
        let material = json!({"kind":"staged","base_id":base,"index_id":index});
        let mut client = Client::open(project);
        apply(&mut client, "debug-open", json!({"request_id":"risk-debug-open", "slug":"login-fix",
            "expected_version":0,"symptom":"login reproduction fails"}));
        let resolve = json!({"request_id":"risk-debug-resolve","slug":"login-fix","expected_version":1,
            "resolution":"repair login","reproduction":{"test":"repeat login","result":"reproduced check passes","passed":true}});
        let answer = client.call("cadence_apply", json!({"operation":"debug-resolve","request":resolve}));
        if !staged {
            assert_eq!(answer["status"], "refused", "{case}: {answer}");
            assert!(answer.to_string().contains("empty index"), "{case}: {answer}");
            assert_eq!(query(&mut client, "debug-status", "login-fix")["record"]["status"], "open");
            client.finish();
            assert!(receipts::history(&phase13::reopened(project).snapshot.data).unwrap().1.is_empty());
            continue;
        }
        let status = query(&mut client, "debug-status", "login-fix");
        let record = &status["record"];
        let review = &record["review"];
        assert_eq!(review["material"], material, "{case}: {status}");
        let occurrence = review["occurrence"].as_str().unwrap();
        let risk_status = client.call("cadence_query", json!({"operation":"risk-status",
            "scope":{"kind":"root-debug","occurrence":occurrence},"source":{"kind":"staged","base":base}}));
        assert_eq!(risk_status["status"], "ok", "{case}: {risk_status}");
        let checked = &risk_status["current_observation"];
        assert_eq!(checked["observation"]["scope"]["kind"], "root-debug");
        assert!(checked["observation"]["scope"]["phase"].is_null());
        assert_eq!(checked["observation"]["scope"]["occurrence"], occurrence);
        assert_eq!(checked["observation"]["source"], json!({"kind":"staged","base":base}));
        assert_eq!(checked["observation"]["resolution"], material);
        assert_eq!(checked["observation"]["outcome"], "checked");
        assert_eq!(checked["observation"]["scan"]["checked"], true);
        assert_eq!(checked["observation"]["scan"]["inconclusive"], case == "binary");
        if fires {
            assert_eq!(answer["status"], "refused", "{case}: {answer}");
            assert_eq!(record["status"], "open");
            let fire = review["fire"].as_str().unwrap();
            assert!(answer.to_string().contains(fire));
            let admission = client.call("cadence_query", json!({"operation":"review-admission","fire":fire}));
            assert_eq!(admission["result"]["caller"], "debug", "{admission}");
            assert_eq!(admission["result"]["home"]["kind"], "root-debug");
            assert_eq!(admission["result"]["discriminator"], occurrence);
            assert!(admission["result"]["phase"].is_null());
            assert!(admission["result"]["plan"].is_null());
            let next = client.call("cadence_query", json!({"operation":"review-next","fire":fire}));
            assert_eq!(next["result"]["state"], "dispatch", "{case}: {next}");
            assert!(next["result"]["dispatch"].is_object());
            let attempt = next["result"]["attempt"]["attempt"].as_str().unwrap();
            let inventory = client.call("cadence_query", json!({"operation":"review-inventory"}));
            let manifest_id = next["result"]["attempt"]["view"]["manifest"].as_str().unwrap();
            let entries = inventory["result"]["records"]["manifests"][manifest_id]["entries"].as_array().unwrap();
            for entry in entries.iter().filter(|entry| entry["availability"] == "available") {
                let read = client.call("cadence_query", json!({"operation":"review-material","attempt":attempt,"entry":entry["entry"]}));
                assert_eq!(read["status"], "ok", "{read}");
            }
            let retry = client.call("cadence_apply", json!({"operation":"debug-resolve","request":{
                "request_id":"risk-debug-retry","slug":"login-fix","expected_version":record["version"],
                "resolution":"repair login","reproduction":{"test":"repeat login","result":"reproduced check passes","passed":true}}}));
            assert_eq!(retry["status"], "refused", "{retry}");
            assert_eq!(query(&mut client, "debug-status", "login-fix")["record"]["review"], *review);
        } else {
            assert_eq!(answer["status"], "ok", "{case}: {answer}");
            assert_eq!(record["status"], "resolved");
            assert_eq!(checked["observation"]["scan"]["matches"], json!([]));
            assert!(review["fire"].is_null());
        }
        client.finish();
        let view = phase13::reopened(project);
        receipts::confirmed_history(&view).unwrap();
        let (observations, saved_fires, _) = receipts::history(&view.snapshot.data).unwrap();
        assert_eq!(observations.len(), 1, "{case}");
        assert_eq!(serde_json::to_value(&observations[0]).unwrap(), *checked);
        assert_eq!(risk::confirmed(&view, &observations[0].observation.scope,
            &observations[0].observation.request_id).unwrap(), Some(observations[0].clone()));
        assert_eq!(saved_fires.len(), usize::from(fires));
        let admissions = &view.snapshot.data["review"]["admissions"];
        assert_eq!(admissions.as_object().map_or(0, |a| a.len()), usize::from(fires));
        if fires {
            let fire = &saved_fires[0];
            assert_eq!(review["fire"], fire.id);
            assert_eq!(fire.review_scope, [path]);
            assert_eq!(serde_json::to_value(&fire.binding.material).unwrap(), material);
            assert!(fire.binding.matches(&observations[0]));
            let admitted = &admissions[&fire.id];
            let home = &view.snapshot.data["review"]["homes"][admitted["home"]["occurrence"].as_str().unwrap()];
            assert_eq!(home["path"], format!("reviews/{}", fire.id));
            let manifest = &view.snapshot.data["review"]["manifests"][admitted["artifact"].as_str().unwrap()];
            assert_eq!(manifest["target"], json!({"kind":"staged-tree","base":base,"index":index,"head":null}));
        }
        assert_eq!(phase13::git_value(project, &["write-tree"]), index);
        assert_eq!(support_records::guard(project, "Write", ".planning/debug/login-fix.md")
            ["hookSpecificOutput"]["permissionDecision"], "deny");
    }
}
