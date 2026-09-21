#[allow(dead_code)]
#[path = "support/serve.rs"]
mod serve;
#[path = "support/support_records.rs"]
mod support_records;

use serve::Client;
use serde_json::{Value, json};
use std::fs;
use support_records::apply;

fn criteria() -> Value {
    json!([
        {"id":"isolation","given":"two tenants share the same object id",
         "when":"each tenant reads its cached object","then":"each receives only its own value",
         "failure":"either tenant receives the other tenant's value"},
        {"id":"invalidation","given":"a tenant has a cached object",
         "when":"that tenant updates the object","then":"its next read returns the new value",
         "failure":"the next read returns the stale value"},
        {"id":"latency","given":"one thousand warm tenant-local reads",
         "when":"the reads run with tenant-local keys","then":"p95 latency stays below five milliseconds",
         "failure":"p95 latency is five milliseconds or greater"}
    ])
}

fn refused(client: &mut Client, operation: &str, request: Value) -> Value {
    let answer = client.call("cadence_apply", json!({"operation":operation,"request":request}));
    assert_eq!(answer["status"], "refused", "{answer}");
    answer
}

#[test]
fn spike_criteria_precede_material_and_the_verdict_is_bounded() {
    let repo = support_records::fixture();
    let project = repo.path();
    let historical = support_records::install_spike_history(project);
    let external = tempfile::tempdir().unwrap();
    let experiment = external.path().join("tenant-cache-experiment");
    assert!(!experiment.exists());
    let mut client = Client::open(project);
    let open = json!({"request_id":"spike-open-tenant-cache","slug":"tenant-cache","expected_version":0,
        "question":"Can tenant-local keys prevent cross-tenant cache hits?",
        "decision":"Choose the cache key scheme","criteria":criteria()});
    let opened = apply(&mut client, "spike-open", open.clone());
    let record = &opened["record"];
    assert_eq!(record["question"], "Can tenant-local keys prevent cross-tenant cache hits?");
    assert_eq!(record["decision"], "Choose the cache key scheme");
    assert_eq!(record["criteria"], criteria());
    assert_eq!(record["observations"], json!([]));
    assert_eq!(record["verdict"], Value::Null);
    let projection_path = project.join(".planning/spikes/tenant-cache/SPIKE.md");
    let projection = fs::read_to_string(&projection_path).unwrap();
    assert_eq!(opened["projection"], projection);
    for text in ["Can tenant-local keys prevent cross-tenant cache hits?", "Choose the cache key scheme"] {
        assert!(projection.contains(text), "{projection}");
    }
    let mut previous = 0;
    for criterion in criteria().as_array().unwrap() {
        let position = projection.find(criterion["id"].as_str().unwrap()).unwrap();
        assert!(position > previous);
        previous = position;
        for field in ["given", "when", "then", "failure"] {
            assert!(projection.contains(criterion[field].as_str().unwrap()), "{projection}");
        }
    }
    assert!(!projection.contains("## Results"));
    assert_eq!(fs::read_dir(projection_path.parent().unwrap()).unwrap().count(), 1);
    assert!(!experiment.exists());
    for (tool, path) in [("Write", ".planning/spikes/tenant-cache/SPIKE.md"),
        ("Edit", ".planning/spikes/tenant-cache/SPIKE.md"),
        ("Write", ".planning/spikes/tenant-cache/experiment.rs"),
        ("Write", ".planning/spikes/tenant-cache/deep/experiment.rs")] {
        assert_eq!(support_records::guard(project, tool, path)["hookSpecificOutput"]["permissionDecision"], "deny");
    }
    assert_eq!(apply(&mut client, "spike-open", open.clone()), opened);
    fs::create_dir(&experiment).unwrap();
    fs::write(experiment.join("experiment.rs"), "fn main() { println!(\"throwaway tenant keys\"); }\n").unwrap();

    refused(&mut client, "spike-verdict", json!({"request_id":"spike-missing-results","slug":"tenant-cache",
        "expected_version":1,"verdict":"validated","criteria":["isolation","invalidation","latency"]}));
    refused(&mut client, "spike-observation", json!({"request_id":"spike-foreign-observation","slug":"tenant-cache",
        "expected_version":1,"observation":{"criterion":"foreign","result":"not a criterion"}}));
    let results = [
        ("isolation", "Tenant alpha received alpha; tenant beta received beta across 1000 shared object ids."),
        ("invalidation", "Updating alpha replaced alpha's value; beta's cached value stayed unchanged."),
        ("latency", "Warm reads measured p95 2.4 milliseconds over 1000 requests.")
    ];
    for (index, (criterion, result)) in results.iter().enumerate() {
        apply(&mut client, "spike-observation", json!({"request_id":format!("spike-observe-{criterion}"),
            "slug":"tenant-cache","expected_version":index + 1,"observation":{"criterion":criterion,"result":result}}));
    }
    let verdict = json!({"request_id":"spike-verdict-validated","slug":"tenant-cache","expected_version":4,
        "verdict":"validated","criteria":["isolation","invalidation","latency"]});
    let mut unsupported = verdict.clone();
    unsupported["request_id"] = json!("spike-verdict-confirmed");
    unsupported["verdict"] = json!("confirmed");
    let rejection = refused(&mut client, "spike-verdict", unsupported);
    for word in ["validated", "invalidated", "inconclusive"] {
        assert!(rejection["reason"].as_str().unwrap().contains(word), "{rejection}");
    }
    for (id, identities) in [("missing", json!(["isolation", "invalidation"])),
        ("foreign", json!(["isolation", "invalidation", "foreign"]))] {
        let mut bad = verdict.clone();
        bad["request_id"] = json!(format!("spike-verdict-{id}"));
        bad["criteria"] = identities;
        refused(&mut client, "spike-verdict", bad);
    }
    let validated = apply(&mut client, "spike-verdict", verdict.clone());
    assert_eq!(validated["record"]["verdict"], "validated");
    assert_eq!(validated["record"]["observations"], json!([
        {"criterion":"isolation","result":results[0].1},
        {"criterion":"invalidation","result":results[1].1},
        {"criterion":"latency","result":results[2].1}
    ]));
    let rendered = fs::read_to_string(&projection_path).unwrap();
    assert_eq!(validated["projection"], rendered);
    for (_, result) in results { assert!(rendered.contains(result)); }
    let mut changed = open.clone();
    changed["request_id"] = json!("spike-reopen-changed");
    changed["expected_version"] = json!(5);
    changed["criteria"][0]["then"] = json!("accept cross-tenant hits");
    refused(&mut client, "spike-open", changed);
    assert_eq!(fs::read_to_string(&projection_path).unwrap(), rendered);
    let mut reused = verdict.clone();
    reused["verdict"] = json!("invalidated");
    assert_eq!(refused(&mut client, "spike-verdict", reused)["code"], "request-reused");
    let close = json!({"request_id":"spike-close-external","slug":"tenant-cache","expected_version":5,
        "throwaway_location":experiment});
    for (id, path) in [("record", project.join(".planning/spikes/tenant-cache/scratch")),
        ("source", project.join("src/experiment.rs"))] {
        let mut bad = close.clone();
        bad["request_id"] = json!(format!("spike-close-{id}"));
        bad["throwaway_location"] = json!(path);
        refused(&mut client, "spike-close", bad);
    }
    let closed = apply(&mut client, "spike-close", close.clone());
    assert_eq!(closed["record"]["status"], "closed");
    assert_eq!(closed["record"]["throwaway_location"], json!(experiment));
    assert_eq!(closed["record"]["criteria"], criteria());
    assert_eq!(closed["record"]["observations"], validated["record"]["observations"]);
    assert!(closed["projection"].as_str().unwrap().contains(experiment.to_str().unwrap()));
    assert!(!project.join("src/experiment.rs").exists());
    assert_eq!(fs::read_dir(projection_path.parent().unwrap()).unwrap().count(), 1);
    client.finish();
    let saved = serve::reopened(project);
    assert_eq!(saved.snapshot.data["spike"]["records"]["tenant-cache"], closed["record"]);
    let history_now = support_records::spike_history(&project.join(".planning/spikes"));
    assert_eq!(history_now.iter().filter(|(p, _)| !p.starts_with("tenant-cache")).map(|(p,b)| (p.clone(),b.clone())).collect::<std::collections::BTreeMap<_,_>>(), historical);
    let mut client = Client::open(project);
    assert_eq!(apply(&mut client, "spike-close", close), closed);
    assert_eq!(apply(&mut client, "spike-verdict", verdict), validated);
    assert_eq!(apply(&mut client, "spike-open", open), opened);
    client.finish();
    let after = serve::reopened(project);
    assert_eq!(after.snapshot.generation, saved.snapshot.generation);
    assert_eq!(after.snapshot.data["spike"]["records"]["tenant-cache"], closed["record"]);
    assert_eq!(fs::read_to_string(&projection_path).unwrap(), closed["projection"].as_str().unwrap());
    assert_eq!(support_records::spike_history(&project.join(".planning/spikes")), history_now);

    for word in ["invalidated", "inconclusive"] {
        let case = support_records::fixture();
        let mut client = Client::open(case.path());
        apply(&mut client, "spike-open", json!({"request_id":format!("{word}-open"),"slug":"tenant-cache","expected_version":0,
            "question":"Can tenant-local keys prevent cross-tenant cache hits?","decision":"Choose the cache key scheme","criteria":criteria()}));
        for (index, id) in ["isolation", "invalidation", "latency"].iter().enumerate() {
            apply(&mut client, "spike-observation", json!({"request_id":format!("{word}-{id}"),"slug":"tenant-cache",
                "expected_version":index + 1,"observation":{"criterion":id,"result":"Caller reports this criterion did not establish success."}}));
        }
        let answer = apply(&mut client, "spike-verdict", json!({"request_id":format!("{word}-verdict"),"slug":"tenant-cache",
            "expected_version":4,"verdict":word,"criteria":["isolation","invalidation","latency"]}));
        assert_eq!(answer["record"]["verdict"], word);
        client.finish();
    }
}
