#[path = "support/phase13.rs"]
mod phase13;
use phase13::*;
use serde_json::{Value, json};

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
