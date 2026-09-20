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
            for entry in next["result"]["attempt"]["view"]["entries"].as_array().unwrap() {
                let read = client.call("cadence_query", json!({"operation":"review-material","attempt":attempt,"entry":entry}));
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
