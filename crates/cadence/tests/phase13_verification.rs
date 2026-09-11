#[path = "support/phase13.rs"]
mod phase13;
use phase13::*;
use serde_json::{Value, json};

fn inspected_patch(project: &std::path::Path, id: &str) -> (Value, Value) {
    let dispatch = query(project, json!({"operation":"verify-next","phase":13,"request_id":id}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let attempt = dispatch["attempt"].clone();
    let mut items = Vec::new();
    let mut client = Client::open(project);
    for item in attempt["inputs"]["map"]["items"].as_array().unwrap() {
        let mut runs = Vec::new();
        if item["kind"] == "check" {
            let run = format!("{id}-{}", item["id"].as_str().unwrap());
            let launched = client.call("cadence_apply", json!({"operation":"verification-run","request":{
                "request_id":run,"attempt":attempt["id"],"basis":attempt["inputs"]["basis"],
                "item":{"id":item["id"],"item_revision":item["item_revision"]}}}));
            assert_eq!(launched["status"], "ok", "{launched}");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                let read = client.call("cadence_query", json!({"operation":"verification-read","phase":13,"attempt":attempt["id"]}));
                if let Some(result) = read["runs"].as_array().unwrap().iter()
                    .find(|r| r["event"]["kind"] == "result" && r["event"]["run_id"] == run) {
                    assert_eq!(result["event"]["result"]["disposition"], json!({"kind":"exited","code":0}));
                    assert_eq!(result["event"]["result"]["material_unchanged"], true);
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "{read}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            runs.push(run);
        }
        items.push(json!({"id":item["id"],"item_revision":item["item_revision"],
            "verdict":"accepted","observed":"Inspected the specified fixture evidence.","runs":runs}));
    }
    client.finish();
    let patch = json!({"request_id":format!("{id}-patch"),"attempt":attempt["id"],
        "basis":attempt["inputs"]["basis"],"items":items});
    (attempt, patch)
}

// Refusal history may append; everything except that separate history remains
// byte-identical after the real server has exited and the store is reopened.
fn acceptance_bytes(project: &std::path::Path) -> Vec<u8> {
    let reopened = reopened(project);
    let mut data = reopened.snapshot.data;
    let effective = data["verification"]["patches"].clone();
    data.as_object_mut().unwrap().remove("verification");
    let mut files = tree(project);
    files.remove(&std::path::PathBuf::from(".planning/state.json"));
    files.remove(&std::path::PathBuf::from(".planning/decisions.jsonl"));
    serde_json::to_vec(&(data, effective, files)).unwrap()
}

#[test]
fn phase13_mismatched_verdict_patch_is_refused() {
    let fixture = Completed::new();
    let project = fixture.project();
    let (attempt, valid) = inspected_patch(project, "patch-attempt");
    let baseline = acceptance_bytes(project);
    let mut variants = Vec::new();
    let mut unknown = valid.clone();
    unknown["items"][0]["id"] = json!("unknown/item");
    variants.push((unknown, "verification-item", "items[0].id", "unknown/item"));
    let mut missing = valid.clone();
    missing["items"].as_array_mut().unwrap().remove(0);
    variants.push((missing, "verification-item", "items", "artifact/shared"));
    let mut duplicate = valid.clone();
    duplicate["items"].as_array_mut().unwrap().push(valid["items"][0].clone());
    variants.push((duplicate, "verification-item", "items[4].id", "artifact/shared"));
    let mut revision = valid.clone();
    revision["items"][0]["item_revision"] = json!("wrong-revision");
    variants.push((revision, "verification-item", "items[0].item_revision", "artifact/shared"));
    for (pointer, slot, replacement) in [
        ("/basis/map_digest", "basis.map_digest", json!("stale-map")),
        ("/basis/truths/0/version", "basis.truths", json!(2)),
        ("/basis/publications/0/content_revision", "basis.publications", json!("wrong-content")),
        ("/basis/occurrence", "basis.occurrence", json!("foreign-occurrence")),
        ("/basis/root_binding", "basis.root_binding", json!("foreign-root")),
        ("/basis/project", "basis.project", json!("/foreign-project")),
        ("/basis/context_digest", "basis.context_digest", json!("wrong-context")),
        ("/basis/admission_digests", "basis.admission_digests", json!([])),
        ("/basis/execution_digest", "basis.execution_digest", json!("wrong-execution")),
        ("/basis/source/head", "basis.source", json!("foreign-head")),
        ("/basis/source/tree", "basis.source", json!("foreign-tree")),
        ("/basis/source/index_digest", "basis.source", json!("foreign-index")),
        ("/basis/source/material_digest", "basis.source", json!("foreign-material")),
    ] {
        let mut patch = valid.clone();
        *patch.pointer_mut(pointer).unwrap() = replacement;
        variants.push((patch, "verification-basis", slot, ""));
    }
    let mut no_run = valid.clone();
    no_run["items"][1]["runs"] = json!([]);
    variants.push((no_run, "verification-run", "items[1].runs", "check/A"));
    let mut executor_run = valid.clone();
    executor_run["items"][1]["runs"] = json!(["green-1"]);
    variants.push((executor_run, "verification-run", "items[1].runs", "check/A"));
    for (index, (mut patch, rule, slot, item)) in variants.into_iter().enumerate() {
        patch["request_id"] = json!(format!("invalid-{index}"));
        let response = apply(project, json!({"operation":"verification-submit","patch":patch}));
        assert_eq!(response["status"], "refused", "mismatched patch accepted: {response}");
        assert_eq!(response["rule"], rule, "refusal must identify the mismatched item/input: {response}");
        assert_eq!(response["slot"], slot, "{response}");
        if !item.is_empty() { assert_eq!(response["id"], item, "{response}"); }
        if rule == "verification-basis" || slot.ends_with("item_revision") {
            assert!(response["details"].get("requested").is_some(), "{response}");
            assert!(response["details"].get("current").is_some(), "{response}");
        }
        assert_eq!(acceptance_bytes(project), baseline);
        let claims = reopened(project).snapshot.data["verification"]["claims"].clone();
        let retained = claims.as_array().unwrap().iter().find(|c| c["patch"]["request_id"] == patch["request_id"]).unwrap();
        assert_eq!(retained["patch"], patch);
        assert_eq!(retained["answer"]["rule"], rule);
        let before_replay = tree(project);
        assert_eq!(apply(project, json!({"operation":"verification-submit","patch":patch})), response);
        assert_eq!(tree(project), before_replay);
    }
    for (field, value) in [("phase_pass", json!(true)), ("truth_status", json!("met")),
        ("write_file", json!({"path":"findings.md","content":"passed"}))] {
        let mut patch = valid.clone();
        patch[field] = value;
        let before = tree(project);
        let response = apply(project, json!({"operation":"verification-submit","patch":patch}));
        assert_eq!(response["status"], "refused");
        assert_eq!(response["rule"], "verification-shape");
        assert!(response["reason"].as_str().unwrap().contains(field), "{response}");
        assert_eq!(acceptance_bytes(project), baseline);
        assert_eq!(tree(project), before, "transport-invalid input is not a domain claim");
    }
    let accepted = apply(project, json!({"operation":"verification-submit","patch":valid}));
    assert_eq!(accepted["status"], "ok", "valid complete control: {accepted}");
    assert_eq!(accepted["receipt"]["patch"], valid);
    assert_eq!(accepted["receipt"]["application"], "accepted");
    assert_eq!(reopened(project).snapshot.data["verification"]["patches"], json!([valid]));
    let accepted_bytes = acceptance_bytes(project);
    let before = tree(project);
    assert_eq!(apply(project, json!({"operation":"verification-submit","patch":valid})), accepted);
    assert_eq!(tree(project), before);
    let mut conflict = valid.clone();
    conflict["items"][0]["observed"] = json!("Changed claim under the same request.");
    let response = apply(project, json!({"operation":"verification-submit","patch":conflict}));
    assert_eq!(response["rule"], "verification-request-reuse");
    assert_eq!(response["slot"], "request_id");
    assert_eq!(acceptance_bytes(project), accepted_bytes);
    let mut second = valid.clone();
    second["request_id"] = json!("second-completion");
    assert_eq!(apply(project, json!({"operation":"verification-submit","patch":second}))["rule"], "verification-attempt-complete");
    assert_eq!(acceptance_bytes(project), accepted_bytes);
    // Actual source staleness, not a forged HEAD in the input.
    std::fs::write(project.join("src/a.py"), "def answer():\n    return 7 # repaired implementation\n").unwrap();
    git_value(project, &["add", "src/a.py"]);
    git_value(project, &["commit", "-m", "Fixture repair"]);
    let mut stale = valid.clone();
    stale["request_id"] = json!("source-stale");
    let before = acceptance_bytes(project);
    let response = apply(project, json!({"operation":"verification-submit","patch":stale}));
    assert_eq!(response["rule"], "verification-basis", "{response}");
    assert_eq!(response["slot"], "basis.source");
    assert_eq!(response["details"]["requested"], attempt["inputs"]["basis"]["source"]);
    assert_eq!(response["details"]["current"]["head"], git_value(project, &["rev-parse", "HEAD"]));
    assert_eq!(acceptance_bytes(project), before);
    let (_, current) = inspected_patch(project, "current-source");
    // Actual approved union extension; an admitted plan is never replaced.
    let gap = proposal(project, "new-map", &[(None, attached(vec![artifact("artifact/gap", &["truth/A"])]))]);
    publish(project, &gap);
    let map = query(project, json!({"operation":"evidence-read","phase":13}));
    let before = acceptance_bytes(project);
    let response = apply(project, json!({"operation":"verification-submit","patch":current}));
    assert_eq!(response["rule"], "verification-basis", "{response}");
    assert_eq!(response["slot"], "basis.map_digest");
    assert_eq!(response["details"]["requested"], current["basis"]["map_digest"]);
    assert_eq!(response["details"]["current"], map["input_digest"]);
    assert_eq!(acceptance_bytes(project), before);
    assert_eq!(apply(project, json!({"operation":"verification-submit","patch":valid})), accepted);
    assert_eq!(acceptance_bytes(project), before, "historical replay does not reinstall acceptance");
    for name in ["findings.md", ".planning/findings.md", ".planning/phases/13/UAT.md"] {
        assert!(!project.join(name).exists());
    }
}

#[test]
fn phase13_dispatch_carries_current_verification_inputs() {
    let fixture = Completed::new();
    let project = fixture.project();
    assert_eq!(fixture.dispatches.len(), 2);
    let execution = history(project);
    assert!(execution["tasks"].as_array().unwrap().iter().all(|t| t["state"]["completed"] == true));
    let request = json!({"operation":"verify-next","phase":13,"request_id":"verify-first"});
    let dispatch = query(project, request.clone());
    assert_eq!(dispatch["status"], "ok", "verifier dispatch must be retained: {dispatch}");
    let attempt = &dispatch["attempt"];
    let prompt = attempt["prompt"].as_str().expect("retained verifier prompt");
    let operational: Value = serde_json::from_str(prompt.split("<operational-input>\n").nth(1).unwrap()
        .split("\n</operational-input>").next().unwrap()).unwrap();
    assert_eq!(operational["map"], fixture.map);
    assert_eq!(operational["map"]["truths"], json!([
        {"id":"truth/A","version":1,"text":"When a parcel arrives, the recipient gets the parcel.","kind":"property"},
        {"id":"truth/B","version":1,"text":"When a second parcel arrives, the recipient gets the parcel.","kind":"property"}
    ]));
    let ids: Vec<_> = operational["map"]["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["artifact/shared", "check/A", "check/B", "link/parcel"]);
    let associations: Vec<_> = operational["map"]["associations"].as_array().unwrap().iter()
        .map(|a| (a["origin"]["plan"].as_u64().unwrap(), a["origin"]["item_id"].as_str().unwrap(), a["truth_id"].as_str().unwrap(), a["reason"].as_str().unwrap())).collect();
    assert_eq!(associations, [
        (1,"artifact/shared","truth/A","This causes the promised delivery."),
        (1,"artifact/shared","truth/B","This causes the promised delivery."),
        (1,"check/A","truth/A","This causes the promised delivery."),
        (1,"link/parcel","truth/A","This causes the promised delivery."),
        (2,"artifact/shared","truth/A","This causes the promised delivery."),
        (2,"artifact/shared","truth/B","This causes the promised delivery."),
        (2,"check/B","truth/B","This causes the promised delivery.")]);
    assert_eq!(operational["basis"]["map_digest"], fixture.map["input_digest"]);
    assert_eq!(operational["basis"]["publications"], fixture.admission["receipt"]["request"]["contract"]["plans"]);
    assert_eq!(operational["admissions"], json!([fixture.admission["receipt"]]));
    assert_eq!(operational["execution"]["events"], execution["events"]);
    assert_eq!(operational["execution"]["plan_events"], execution["plan_events"]);
    let pairs: Vec<_> = operational["execution"]["events"].as_array().unwrap().iter()
        .filter(|e| e["request"]["event"]["kind"] == "close")
        .flat_map(|e| e["request"]["event"]["submission"]["checks"].as_array().unwrap().clone()).collect();
    assert_eq!(pairs, fixture.pairs);
    let statements: Vec<_> = operational["execution"]["events"].as_array().unwrap().iter()
        .filter(|e| e["request"]["event"]["kind"] == "owner-statement")
        .map(|e| { let mut v = e["request"]["event"].clone(); v.as_object_mut().unwrap().remove("kind"); v }).collect();
    assert_eq!(statements, fixture.statements);
    let commands: Vec<_> = operational["checks"].as_array().unwrap().iter().map(|c| c["spec"]["command"].as_str().unwrap()).collect();
    assert_eq!(commands, ["python3 -B tests/a.py", "python3 -B tests/b.py"]);
    for check in operational["checks"].as_array().unwrap() {
        let saved = fixture.map["items"].as_array().unwrap().iter().find(|i| i["id"] == check["id"]).unwrap();
        assert_eq!(check["spec"], saved["spec"]);
    }
    assert_eq!(operational["basis"]["source"]["head"], git_value(project, &["rev-parse","HEAD"]));
    assert_eq!(operational["basis"]["source"]["tree"], git_value(project, &["rev-parse","HEAD^{tree}"]));
    for key in ["index_digest", "material_digest"] { assert!(!operational["basis"]["source"][key].as_str().unwrap().is_empty()); }
    assert!(prompt.contains("**Verifier.** For each evidence item: open it, run it, or trace it. Return a\nverdict per item - accepted, rejected or not seen - with what you observed.\nA summary is not evidence. An item whose check could not have failed is\nrejected, not accepted. You do not set a truth's status; the binary derives\nit from your verdicts."));
    assert!(prompt.contains("Strict item patch schema"));
    assert!(!prompt.contains("configured-alternative"));
    assert!(!prompt.contains("All imaginary checks passed"));
    assert!(!prompt.contains("findings_file"));
    let before = tree(project);
    let stored = reopened(project).snapshot;
    assert_eq!(tree(project), before);
    assert_eq!(query(project, request.clone())["attempt"], *attempt);
    assert_eq!(query(project, json!({"operation":"verification-read","phase":13,"attempt":attempt["id"]}))["attempt"], *attempt);
    assert_eq!(tree(project), before);
    assert_eq!(reopened(project).snapshot, stored);
    // A historical replay is stable even when current source changes.
    std::fs::write(project.join("src/a.py"), "def answer():\n    return 8\n").unwrap();
    assert_eq!(query(project, request.clone())["attempt"], *attempt);
    let dirty = query(project, json!({"operation":"verify-next","phase":13,"request_id":"dirty"}));
    assert_eq!(dirty["status"], "refused", "{dirty}");
    assert_eq!(dirty["rule"], "verification-source");
    let changed = query(project, json!({"operation":"verify-next","phase":28,"request_id":"verify-first"}));
    assert_eq!(changed["status"], "refused", "{changed}");
    assert_eq!(changed["rule"], "verification-request-reuse");
    std::fs::write(project.join("src/a.py"), "def answer():\n    return 7\n").unwrap();
    let plan = project.join(".planning/phases/13/PLAN-2.md");
    let original = std::fs::read(&plan).unwrap();
    std::fs::write(&plan, b"drifted native publication\n").unwrap();
    let refused = query(project, json!({"operation":"verify-next","phase":13,"request_id":"drift"}));
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["rule"], "installed-plan");
    std::fs::write(&plan, original).unwrap();
    assert_eq!(query(project, request)["attempt"], *attempt);
    assert_eq!(tree(project), before);
    let missing = phase13::fixture();
    let before = tree(missing.path());
    let refused = query(missing.path(), json!({"operation":"verify-next","phase":13,"request_id":"missing"}));
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["rule"], "native-approved-truths");
    // An absent authority must not be replaced with a fabricated attempt.
    assert!(!serde_json::to_string(&tree(missing.path())).unwrap().contains("verification-attempt-1"));
    assert_eq!(tree(missing.path()), before);
}
