#[path = "support/phase13.rs"]
mod phase13;
use phase13::*;
use serde_json::{Value, json};

fn inspected_patch(project: &std::path::Path, id: &str) -> (Value, Value) {
    inspected_with(project, id, &[])
}

// A fresh dispatch, one independent run per saved check, and one complete
// handwritten patch; `verdicts` overrides (item, verdict, observed) rows.
fn inspected_with(project: &std::path::Path, id: &str, verdicts: &[(&str, &str, &str)]) -> (Value, Value) {
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
        let (verdict, observed) = verdicts.iter().find(|(name, _, _)| item["id"] == *name)
            .map_or(("accepted", "Inspected the specified fixture evidence."), |(_, verdict, observed)| (verdict, observed));
        items.push(json!({"id":item["id"],"item_revision":item["item_revision"],
            "verdict":verdict,"observed":observed,"runs":runs}));
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

fn report(project: &std::path::Path) -> Value {
    query(project, json!({"operation":"verification-read","phase":13}))
}

fn submitted(project: &std::path::Path, id: &str, verdicts: &[(&str, &str, &str)]) -> (Value, Value) {
    let (attempt, patch) = inspected_with(project, id, verdicts);
    let answer = apply(project, json!({"operation":"verification-submit","patch":patch}));
    assert_eq!(answer["status"], "ok", "complete patch for {id}: {answer}");
    (attempt, patch)
}

// Handwritten truth row. Items are (id, kind, verdict, observed) in id order;
// a check's independent run id follows the inspection id it was launched under.
fn row(map: &Value, run: &str, truth: &str, text: &str, status: &str, reason: &str, items: &[(&str, &str, &str, &str)]) -> Value {
    let items: Vec<_> = items.iter().map(|(id, kind, verdict, observed)| {
        let saved = map["items"].as_array().unwrap().iter().find(|i| i["id"] == *id).unwrap();
        json!({"id":id,"kind":kind,"item_revision":saved["item_revision"],"verdict":verdict,"observed":observed,
            "runs":if *kind == "check" { json!([format!("{run}-{id}")]) } else { json!([]) },
            "reasons":["This causes the promised delivery."]})
    }).collect();
    json!({"id":truth,"version":1,"text":text,"status":status,"reason":reason,"items":items})
}

fn pending(truth: &str, text: &str) -> Value {
    json!({"id":truth,"version":1,"text":text,"status":"pending","reason":"no complete applicable verification","items":[]})
}

const A: &str = "When a parcel arrives, the recipient gets the parcel.";
const B: &str = "When a second parcel arrives, the recipient gets the parcel.";
const SEEN: &str = "Inspected the specified fixture evidence.";
const MET: &str = "every item accepted and none is an observation";
const CONCERNS: &str = "every item accepted and at least one is an observation";
const LEGACY: &str = "historical classification only; never native evidence";

#[test]
fn phase13_report_derives_truth_status_from_every_item() {
    let fixture = Completed::new();
    let project = fixture.project();
    let map = &fixture.map;
    let context = reopened(project).snapshot.data["context"].clone();
    // Execution is complete and SUMMARY claims success; nothing is verified yet.
    let before = tree(project);
    let read = report(project);
    assert_eq!(read["status"], "ok", "{read}");
    assert_eq!(read["schema"], "verification-report-1");
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["current"]["reason"], "no verification attempt");
    assert_eq!(read["current"]["attempt"], Value::Null);
    assert_eq!(read["current"]["verified_at"], Value::Null);
    assert_eq!(read["current"]["observed"]["source"]["head"], git_value(project, &["rev-parse", "HEAD"]));
    assert_eq!(read["truths"], json!([pending("truth/A", A), pending("truth/B", B)]));
    assert_eq!(read["history"], json!([]));
    assert_eq!(read["legacy"], json!({"summary_document":true,"uat_document":false,"authority":LEGACY}));
    let text = read["report"].as_str().unwrap();
    assert!(text.contains("Current: none - no verification attempt"), "{text}");
    assert!(text.contains("| truth/A | pending | - |"), "{text}");
    assert!(text.contains("Legacy: SUMMARY present, UAT absent - historical classification only, never native evidence."), "{text}");
    assert_eq!(tree(project), before, "readback writes nothing");
    // An open attempt with finished runs is still not a verification.
    let (first, first_patch) = inspected_patch(project, "all-accepted");
    let read = report(project);
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["current"]["reason"], "no complete verification on the current basis");
    assert_eq!(read["truths"], json!([pending("truth/A", A), pending("truth/B", B)]));
    assert_eq!(read["history"][0]["attempt"], first["id"]);
    assert_eq!(read["history"][0]["applicability"], "open");
    assert_eq!(read["history"][0]["reason"], "attempt has no complete patch");
    assert_eq!(read["history"][0]["application"], Value::Null);
    let answer = apply(project, json!({"operation":"verification-submit","patch":first_patch}));
    assert_eq!(answer["status"], "ok", "{answer}");
    let read = report(project);
    assert_eq!(read["current"], json!({"applicable":true,"attempt":first["id"],"patch":"all-accepted-patch",
        "reason":"complete patch on the current basis","verified_at":first["inputs"]["basis"],
        "observed":first["inputs"]["basis"],"unavailable":null}));
    assert_eq!(read["current"]["verified_at"]["map_digest"], map["input_digest"]);
    assert_eq!(read["current"]["verified_at"]["source"]["head"], git_value(project, &["rev-parse", "HEAD"]));
    assert_eq!(read["current"]["verified_at"]["source"]["tree"], git_value(project, &["rev-parse", "HEAD^{tree}"]));
    assert_eq!(read["current"]["verified_at"]["publications"], fixture.admission["receipt"]["request"]["contract"]["plans"]);
    let met_a = row(map, "all-accepted", "truth/A", A, "met", MET, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]);
    let met_b = row(map, "all-accepted", "truth/B", B, "met", MET, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "accepted", SEEN)]);
    assert_eq!(read["truths"], json!([met_a, met_b]));
    assert_eq!(read["history"][0]["applicability"], "current");
    assert_eq!(read["history"][0]["application"], "accepted");
    assert_eq!(read["history"][0]["patch"], "all-accepted-patch");
    assert_eq!(read["history"][0]["truths"], read["truths"]);
    let text = read["report"].as_str().unwrap();
    assert!(text.contains(&format!("Current: attempt {}, patch all-accepted-patch", first["id"].as_str().unwrap())), "{text}");
    assert!(text.contains("| truth/A | met | artifact/shared accepted; check/A accepted; link/parcel accepted |"), "{text}");
    assert!(text.contains("| truth/B | met | artifact/shared accepted; check/B accepted |"), "{text}");
    // Restart: the store is reopened and a fresh server derives the same rows.
    let stored = reopened(project).snapshot;
    assert_eq!(report(project), read);
    assert_eq!(reopened(project).snapshot, stored);
    assert_eq!(query(project, json!({"operation":"verification-read","phase":13,"attempt":first["id"]}))["truths"], read["truths"]);
    // A rejected shared artifact reaches every association it names, and the
    // accepted checks cannot cover it.
    let (second, _) = submitted(project, "rejected-artifact", &[("artifact/shared", "rejected", "The destination is an empty placeholder directory.")]);
    let read = report(project);
    assert_eq!(read["current"]["attempt"], second["id"]);
    let unmet_a = row(map, "rejected-artifact", "truth/A", A, "unmet", "rejected or not seen: artifact/shared", &[
        ("artifact/shared", "artifact", "rejected", "The destination is an empty placeholder directory."),
        ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]);
    let unmet_b = row(map, "rejected-artifact", "truth/B", B, "unmet", "rejected or not seen: artifact/shared", &[
        ("artifact/shared", "artifact", "rejected", "The destination is an empty placeholder directory."),
        ("check/B", "check", "accepted", SEEN)]);
    assert_eq!(read["truths"], json!([unmet_a, unmet_b]));
    assert_eq!(read["history"][0]["applicability"], "historical");
    assert_eq!(read["history"][0]["reason"], format!("superseded by attempt {}", second["id"].as_str().unwrap()));
    assert_eq!(read["history"][0]["truths"], json!([met_a, met_b]), "historical judgments keep their rows");
    assert_eq!(read["history"][1]["applicability"], "current");
    let text = read["report"].as_str().unwrap();
    assert!(text.contains("| truth/A | unmet | artifact/shared rejected; check/A accepted; link/parcel accepted |"), "{text}");
    assert!(text.contains("| truth/B | unmet | artifact/shared rejected; check/B accepted |"), "{text}");
    assert!(text.contains(&format!("- attempt {}: historical - superseded by attempt {}; truth/A met; truth/B met",
        first["id"].as_str().unwrap(), second["id"].as_str().unwrap())), "{text}");
    // A rejected link named by one truth leaves the other truth met.
    let (third, _) = submitted(project, "rejected-link", &[("link/parcel", "rejected", "The sender never hands the recipient a parcel.")]);
    let read = report(project);
    assert_eq!(read["current"]["attempt"], third["id"]);
    assert_eq!(read["truths"], json!([
        row(map, "rejected-link", "truth/A", A, "unmet", "rejected or not seen: link/parcel", &[
            ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN),
            ("link/parcel", "link", "rejected", "The sender never hands the recipient a parcel.")]),
        row(map, "rejected-link", "truth/B", B, "met", MET, &[
            ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "accepted", SEEN)])]));
    // Explicit not_seen is complete and negative.
    let (fourth, _) = submitted(project, "not-seen", &[("check/B", "not_seen", "tests/b.py could not be opened during inspection.")]);
    let read = report(project);
    assert_eq!(read["current"]["attempt"], fourth["id"]);
    let not_seen_a = row(map, "not-seen", "truth/A", A, "met", MET, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]);
    let not_seen_b = row(map, "not-seen", "truth/B", B, "unmet", "rejected or not seen: check/B", &[
        ("artifact/shared", "artifact", "accepted", SEEN),
        ("check/B", "check", "not_seen", "tests/b.py could not be opened during inspection.")]);
    assert_eq!(read["truths"], json!([not_seen_a, not_seen_b]));
    assert_eq!(read["history"].as_array().unwrap().len(), 4);
    assert_eq!(read["history"][2]["reason"], format!("superseded by attempt {}", fourth["id"].as_str().unwrap()));
    // No truth status was written into the approved context, and execution
    // completion never became acceptance.
    assert_eq!(reopened(project).snapshot.data["context"], context);
    assert_eq!(reopened(project).snapshot.data["verification"]["patches"].as_array().unwrap().len(), 4);
    // A real repair commit makes every judgment historical until reverified.
    let old_head = git_value(project, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("src/b.py"), "def answer():\n    return 7 # repaired implementation\n").unwrap();
    git_value(project, &["add", "src/b.py"]);
    git_value(project, &["commit", "-m", "Fixture repair"]);
    let new_head = git_value(project, &["rev-parse", "HEAD"]);
    let read = report(project);
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["current"]["attempt"], Value::Null);
    assert_eq!(read["current"]["reason"], "no complete verification on the current basis");
    assert_eq!(read["current"]["observed"]["source"]["head"], new_head);
    assert_eq!(read["truths"], json!([pending("truth/A", A), pending("truth/B", B)]));
    for (index, attempt) in [&first, &second, &third, &fourth].into_iter().enumerate() {
        assert_eq!(read["history"][index]["attempt"], attempt["id"]);
        assert_eq!(read["history"][index]["applicability"], "historical");
        assert_eq!(read["history"][index]["basis"], attempt["inputs"]["basis"]);
        assert_eq!(read["history"][index]["basis"]["source"]["head"], old_head);
        assert_eq!(read["history"][index]["differs"], json!(["basis.source"]));
    }
    assert_eq!(read["history"][3]["reason"], "basis differs from current: basis.source");
    assert_eq!(read["history"][3]["truths"], json!([not_seen_a, not_seen_b]));
    let text = read["report"].as_str().unwrap();
    assert!(text.contains("Current: none - no complete verification on the current basis"), "{text}");
    // Uncommitted source is ambiguous: nothing is current and the refusal is located.
    std::fs::write(project.join("src/a.py"), "def answer():\n    return 9\n").unwrap();
    let read = report(project);
    assert_eq!(read["status"], "ok", "{read}");
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["current"]["reason"], "current verification inputs unavailable");
    assert_eq!(read["current"]["unavailable"]["rule"], "verification-source");
    assert_eq!(read["current"]["observed"], Value::Null);
    assert_eq!(read["truths"], json!([pending("truth/A", A), pending("truth/B", B)]));
    assert_eq!(read["history"][3]["reason"], "current verification inputs unavailable");
    std::fs::write(project.join("src/a.py"), "def answer():\n    return 7\n").unwrap();
    // A fresh applicable attempt restores the current rows; the rejected
    // original attempt stays in history with its own identity.
    let (fifth, _) = submitted(project, "fresh", &[]);
    let read = report(project);
    assert_eq!(read["current"]["attempt"], fifth["id"]);
    assert_eq!(read["current"]["verified_at"]["source"]["head"], new_head);
    assert_eq!(read["truths"], json!([
        row(map, "fresh", "truth/A", A, "met", MET, &[
            ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]),
        row(map, "fresh", "truth/B", B, "met", MET, &[
            ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "accepted", SEEN)])]));
    assert_eq!(read["history"].as_array().unwrap().len(), 5);
    assert_eq!(read["history"][3]["applicability"], "historical");
    assert_eq!(read["history"][3]["reason"], "basis differs from current: basis.source");
    assert_eq!(read["history"][3]["truths"], json!([not_seen_a, not_seen_b]));
    assert_eq!(read["history"][4]["applicability"], "current");
    let stored = reopened(project).snapshot;
    assert_eq!(report(project), read);
    assert_eq!(reopened(project).snapshot, stored);
    for name in ["findings.md", ".planning/findings.md", ".planning/phases/13/UAT.md"] {
        assert!(!project.join(name).exists());
    }
    // A separate generic fixture phase carries a supplementary observation:
    // all accepted caps at concerns, and a negative observation verdict is unmet.
    let generic = Completed::with_observation();
    let host = generic.project();
    let map = &generic.map;
    let observed = |observed: &str| row(map, "seen", "truth/B", B, "concerns", CONCERNS, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "accepted", SEEN),
        ("observation/host", "observation", "accepted", observed)]);
    submitted(host, "seen", &[("observation/host", "accepted", "Seen by Fixture Owner on 2026-09-11 on the real host.")]);
    let read = report(host);
    assert_eq!(read["truths"], json!([
        row(map, "seen", "truth/A", A, "met", MET, &[
            ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]),
        observed("Seen by Fixture Owner on 2026-09-11 on the real host.")]));
    let text = read["report"].as_str().unwrap();
    assert!(text.contains("| truth/B | concerns | artifact/shared accepted; check/B accepted; observation/host accepted |"), "{text}");
    submitted(host, "unseen", &[("observation/host", "not_seen", "No host episode was available to the inspector.")]);
    let read = report(host);
    assert_eq!(read["truths"][0]["status"], "met");
    assert_eq!(read["truths"][1], row(map, "unseen", "truth/B", B, "unmet", "rejected or not seen: observation/host", &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "accepted", SEEN),
        ("observation/host", "observation", "not_seen", "No host episode was available to the inspector.")]));
    submitted(host, "refuted", &[("observation/host", "rejected", "The host showed no delivery.")]);
    let read = report(host);
    assert_eq!(read["truths"][0]["status"], "met");
    assert_eq!(read["truths"][1]["status"], "unmet");
    assert_eq!(read["truths"][1]["reason"], "rejected or not seen: observation/host");
    assert_eq!(read["history"][0]["truths"][1]["status"], "concerns", "the capped judgment stays in history");
    let stored = reopened(host).snapshot;
    assert_eq!(report(host), read);
    assert_eq!(reopened(host).snapshot, stored);
}

fn waive(id: &str, submission: &Value) -> Value {
    json!({"operation":"truth-waive","request_id":id,"submission":submission,
        "approval":{"approved":true,"owner":"Fixture Owner","at":"2026-09-11T15:00:00Z","submission":submission}})
}

fn waiver(basis: &Value, truth: &str, version: u32, reason: &str) -> Value {
    json!({"truth":{"id":truth,"version":version},"basis":basis,"reason":reason,
        "owner":"Fixture Owner","at":"2026-09-11T15:00:00Z","supersedes":null,"revoked":false})
}

#[test]
fn phase13_owner_waiver_is_distinct_from_met() {
    let fixture = Completed::new();
    let project = fixture.project();
    let map = &fixture.map;
    let context = reopened(project).snapshot.data["context"].clone();
    // One met and one unmet truth, with the rejection retained.
    let rejected = "tests/b.py asserts nothing about the second parcel.";
    let (reviewed, _) = submitted(project, "reviewed", &[("check/B", "rejected", rejected)]);
    let basis = reviewed["inputs"]["basis"].clone();
    let met_a = row(map, "reviewed", "truth/A", A, "met", MET, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]);
    let unmet_b = row(map, "reviewed", "truth/B", B, "unmet", "rejected or not seen: check/B", &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "rejected", rejected)]);
    let read = report(project);
    assert_eq!(read["truths"], json!([met_a, unmet_b]));
    assert_eq!(read["counts"], json!({"met":1,"concerns":0,"unmet":1,"pending":0,"waived":0}));
    assert_eq!(read["waivers"], json!([]));
    let valid = waiver(&basis, "truth/B", 1, "The second parcel ships in phase 14.");
    let before = tree(project);
    let stored = reopened(project).snapshot;
    // Only exact owner approval waives. Each variant is refused with the
    // offending input located and nothing durable changes.
    let mut variants: Vec<(Value, &str, &str)> = Vec::new();
    let mut unapproved = waive("unapproved", &valid);
    unapproved["approval"]["approved"] = json!(false);
    variants.push((unapproved, "verification-approval", "approval.approved"));
    let mut mismatched = waive("mismatched", &valid);
    mismatched["approval"]["submission"]["reason"] = json!("A different reason than the owner saw.");
    variants.push((mismatched, "verification-approval", "approval.submission"));
    let mut anonymous = waive("anonymous", &valid);
    anonymous["approval"]["owner"] = json!("   ");
    variants.push((anonymous, "verification-approval", "approval.owner"));
    let mut undated = waive("undated", &valid);
    undated["approval"]["at"] = json!("");
    variants.push((undated, "verification-approval", "approval.at"));
    for (field, slot) in [("reason", "submission.reason"), ("owner", "submission.owner"), ("at", "submission.at")] {
        let mut blank = valid.clone();
        blank[field] = json!("  ");
        variants.push((waive(&format!("blank-{field}"), &blank), "verification-waiver", slot));
    }
    variants.push((waive("stale-version", &waiver(&basis, "truth/B", 2, "Version two never existed.")), "verification-truth", "submission.truth"));
    variants.push((waive("unknown-truth", &waiver(&basis, "truth/C", 1, "No such promise.")), "verification-truth", "submission.truth"));
    variants.push((waive("met-truth", &waiver(&basis, "truth/A", 1, "Already met; nothing to waive.")), "verification-truth", "submission.truth"));
    let mut stale = valid.clone();
    stale["basis"]["source"]["head"] = json!("0000000000000000000000000000000000000000");
    variants.push((waive("stale-source", &stale), "verification-basis", "basis.source"));
    let mut foreign = valid.clone();
    foreign["basis"]["map_digest"] = json!("stale-map");
    variants.push((waive("stale-map", &foreign), "verification-basis", "basis.map_digest"));
    let mut orphan = valid.clone();
    orphan["supersedes"] = json!("no-such-waiver");
    variants.push((waive("orphan-supersession", &orphan), "verification-waiver", "submission.supersedes"));
    let mut revoke_nothing = valid.clone();
    revoke_nothing["revoked"] = json!(true);
    variants.push((waive("revoke-nothing", &revoke_nothing), "verification-waiver", "submission.supersedes"));
    for (request, rule, slot) in variants {
        let response = apply(project, request.clone());
        assert_eq!(response["status"], "refused", "{request}\n{response}");
        assert_eq!(response["rule"], rule, "{response}");
        assert_eq!(response["slot"], slot, "{response}");
        assert_eq!(tree(project), before, "an invalid waiver changes nothing");
        assert_eq!(report(project)["truths"], json!([met_a, unmet_b]));
    }
    // A verifier cannot author a waiver from its patch arm, and an absent
    // approval is a transport shape refusal rather than a prepared payload.
    let mut prepared = waive("prepared", &valid);
    prepared.as_object_mut().unwrap().remove("approval");
    assert_eq!(apply(project, prepared)["rule"], "verification-shape");
    let (_, mut patch) = inspected_patch(project, "verifier-authored");
    patch["waivers"] = json!([valid]);
    assert_eq!(apply(project, json!({"operation":"verification-submit","patch":patch}))["rule"], "verification-shape");
    assert_eq!(reopened(project).snapshot.data["verification"].get("waivers"), None);
    // The exact owner waiver is effective and shown beside the met truth.
    let accepted = apply(project, waive("waive-b", &valid));
    assert_eq!(accepted["status"], "ok", "{accepted}");
    let record = accepted["receipt"]["record"].clone();
    assert_eq!(record["schema"], "verification-waiver-1");
    assert_eq!(record["kind"], "waive");
    assert_eq!(record["request_id"], "waive-b");
    assert_eq!(record["submission"], valid);
    assert_eq!(record["approval"]["owner"], "Fixture Owner");
    assert_eq!(record["reviewed"]["attempt"], reviewed["id"]);
    assert_eq!(record["reviewed"]["patch"], "reviewed-patch");
    assert_eq!(record["reviewed"]["status"], "unmet");
    let waived_b = {
        let mut row = unmet_b.clone();
        row["status"] = json!("waived");
        row["derived"] = json!("unmet");
        row["waiver"] = json!({"id":record["id"],"request_id":"waive-b","owner":"Fixture Owner",
            "at":"2026-09-11T15:00:00Z","reason":"The second parcel ships in phase 14."});
        row
    };
    let read = report(project);
    assert_eq!(read["truths"], json!([met_a, waived_b]));
    assert_eq!(read["counts"], json!({"met":1,"concerns":0,"unmet":0,"pending":0,"waived":1}));
    assert_eq!(read["waivers"].as_array().unwrap().len(), 1);
    assert_eq!(read["waivers"][0]["id"], record["id"]);
    assert_eq!(read["waivers"][0]["effective"], true);
    assert_eq!(read["waivers"][0]["truth"], json!({"id":"truth/B","version":1}));
    assert_eq!(read["advice"], Value::Null);
    assert_eq!(read["history"][0]["truths"], json!([met_a, unmet_b]), "the derived judgment is kept as derived");
    let text = read["report"].as_str().unwrap();
    assert!(text.contains("| truth/A | met | artifact/shared accepted; check/A accepted; link/parcel accepted |"), "{text}");
    assert!(text.contains(&format!("| truth/B | waived (derived unmet) | artifact/shared accepted; check/B rejected |")), "{text}");
    assert!(text.contains("Waived: truth/B by Fixture Owner at 2026-09-11T15:00:00Z - The second parcel ships in phase 14."), "{text}");
    assert!(text.contains("Counts: met 1, concerns 0, unmet 0, pending 0, waived 1"), "{text}");
    // Restart and replay: one immutable record, the same answer, no new bytes.
    let saved = reopened(project).snapshot;
    assert_eq!(saved.data["verification"]["waivers"], json!([record]));
    assert_eq!(saved.data["context"], context);
    assert_eq!(saved.data["verification"]["patches"], stored.data["verification"]["patches"]);
    assert_eq!(report(project), read);
    let after = tree(project);
    assert_eq!(apply(project, waive("waive-b", &valid)), accepted);
    assert_eq!(tree(project), after);
    let mut changed = valid.clone();
    changed["reason"] = json!("A different reason under the same request.");
    let reused = apply(project, waive("waive-b", &changed));
    assert_eq!(reused["rule"], "verification-waiver-reuse", "{reused}");
    let duplicate = apply(project, waive("waive-b-again", &valid));
    assert_eq!(duplicate["rule"], "verification-waiver", "{duplicate}");
    assert_eq!(duplicate["slot"], "submission.supersedes");
    assert_eq!(reopened(project).snapshot.data["verification"]["waivers"], json!([record]));
    assert_eq!(tree(project), after);
    // A later verifier patch neither erases the waiver nor is covered by it:
    // the new judgment is derived from its own verdicts and the retained
    // waiver needs explicit owner reaffirmation against the new evidence.
    let (again, _) = submitted(project, "again", &[("check/B", "rejected", rejected)]);
    let unmet_b_again = row(map, "again", "truth/B", B, "unmet", "rejected or not seen: check/B", &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/B", "check", "rejected", rejected)]);
    let met_a_again = row(map, "again", "truth/A", A, "met", MET, &[
        ("artifact/shared", "artifact", "accepted", SEEN), ("check/A", "check", "accepted", SEEN), ("link/parcel", "link", "accepted", SEEN)]);
    let read = report(project);
    assert_eq!(read["current"]["attempt"], again["id"]);
    assert_eq!(read["truths"], json!([met_a_again, unmet_b_again]));
    assert_eq!(read["counts"]["waived"], 0);
    assert_eq!(read["waivers"][0]["id"], record["id"]);
    assert_eq!(read["waivers"][0]["effective"], false);
    assert_eq!(read["waivers"][0]["reason"], format!("reviewed attempt {} is not the current attempt {}", reviewed["id"].as_str().unwrap(), again["id"].as_str().unwrap()));
    assert_eq!(reopened(project).snapshot.data["verification"]["waivers"], json!([record]));
    // Reaffirmation is a separate owner event naming the retained waiver.
    let mut reaffirm = waiver(&again["inputs"]["basis"], "truth/B", 1, "Still shipping in phase 14.");
    reaffirm["supersedes"] = record["id"].clone();
    let reaffirmed = apply(project, waive("reaffirm-b", &reaffirm));
    assert_eq!(reaffirmed["status"], "ok", "{reaffirmed}");
    let second = reaffirmed["receipt"]["record"].clone();
    assert_eq!(second["kind"], "reaffirm");
    let read = report(project);
    assert_eq!(read["truths"][1]["status"], "waived");
    assert_eq!(read["truths"][1]["waiver"]["id"], second["id"]);
    assert_eq!(read["truths"][1]["waiver"]["reason"], "Still shipping in phase 14.");
    assert_eq!(read["counts"], json!({"met":1,"concerns":0,"unmet":0,"pending":0,"waived":1}));
    assert_eq!(read["waivers"][0]["effective"], false);
    assert_eq!(read["waivers"][0]["superseded_by"], second["id"]);
    assert_eq!(read["waivers"][1]["effective"], true);
    // Revocation is another owner event; the derived unmet judgment returns.
    let mut revoke = reaffirm.clone();
    revoke["supersedes"] = second["id"].clone();
    revoke["revoked"] = json!(true);
    revoke["reason"] = json!("Phase 14 will not take the second parcel after all.");
    let revoked = apply(project, waive("revoke-b", &revoke));
    assert_eq!(revoked["status"], "ok", "{revoked}");
    assert_eq!(revoked["receipt"]["record"]["kind"], "revoke");
    let read = report(project);
    assert_eq!(read["truths"], json!([met_a_again, unmet_b_again]));
    assert_eq!(read["counts"], json!({"met":1,"concerns":0,"unmet":1,"pending":0,"waived":0}));
    assert_eq!(read["waivers"][1]["effective"], false);
    assert_eq!(read["waivers"][1]["revoked_by"], revoked["receipt"]["record"]["id"]);
    assert_eq!(read["waivers"].as_array().unwrap().len(), 3);
    assert_eq!(reopened(project).snapshot.data["verification"]["waivers"].as_array().unwrap().len(), 3);
    // Several waivers cue "revisit the plan"; waived never counts as met.
    let (both, _) = submitted(project, "both", &[("check/A", "rejected", "tests/a.py asserts nothing."), ("check/B", "rejected", rejected)]);
    let read = report(project);
    assert_eq!(read["counts"], json!({"met":0,"concerns":0,"unmet":2,"pending":0,"waived":0}));
    for (id, truth) in [("waive-a-both", "truth/A"), ("waive-b-both", "truth/B")] {
        let answer = apply(project, waive(id, &waiver(&both["inputs"]["basis"], truth, 1, "Deferred to phase 14.")));
        assert_eq!(answer["status"], "ok", "{answer}");
    }
    let read = report(project);
    assert_eq!(read["counts"], json!({"met":0,"concerns":0,"unmet":0,"pending":0,"waived":2}));
    assert_eq!(read["truths"][0]["status"], "waived");
    assert_eq!(read["truths"][0]["derived"], "unmet");
    assert_eq!(read["truths"][1]["status"], "waived");
    assert_eq!(read["advice"], "revisit the plan: 2 truths are waived");
    assert!(read["report"].as_str().unwrap().contains("Revisit the plan: 2 truths are waived."));
    assert_eq!(reopened(project).snapshot.data["context"], context);
    let final_tree = tree(project);
    let final_snapshot = reopened(project).snapshot;
    assert_eq!(report(project), read);
    assert_eq!(tree(project), final_tree);
    assert_eq!(reopened(project).snapshot, final_snapshot);
    for name in ["findings.md", ".planning/findings.md", ".planning/phases/13/UAT.md"] {
        assert!(!project.join(name).exists());
    }
}

const HISTORICAL_UAT: &str = "---\nstatus: testing\nphase: 13\n---\n\n## Items\n\n### 1. Delivery\nexpected: the parcel arrives\nstatus: fail\nfirst_pass: fail\nreported: \"the parcel never came\"\n\n### 2. Receipt\nexpected: the recipient signs\nstatus: pass\n";
const REQUIREMENTS_T5: &str = "# Requirements\n\n## Active\n\n- **T1**: the first parcel is delivered\n- **T2**: the second parcel is delivered\n\n## Traceability\n\n| Requirement | Phase | Status |\n|-------------|-------|--------|\n| T2 | Phase 13 | Pending |\n";
const ROADMAP_OPEN: &str = "## Phases\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";
const ROADMAP_DONE: &str = "## Phases\n- [x] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";

fn completion(root: &std::path::Path, id: &str, attempt: &Value, basis: &Value) -> Value {
    let requirements = root.join("REQUIREMENTS.md");
    json!({"operation":"verification-complete","request_id":id,"attempt":attempt,"basis":basis,
        "projections":{"roadmap":digest_of(&root.join("ROADMAP.md")),
            "requirements":requirements.exists().then(|| digest_of(&requirements))}})
}

#[test]
fn phase13_incomplete_verification_cannot_complete_phase() {
    let mut fixture = Completed::published(false, |project| {
        std::fs::write(project.join(".planning/REQUIREMENTS.md"), REQUIREMENTS_T5).unwrap();
        std::fs::write(project.join(".planning/phases/13/UAT.md"), HISTORICAL_UAT).unwrap();
    });
    let project = fixture.project().to_path_buf();
    let project = project.as_path();
    let root = project.join(".planning");
    let (roadmap, requirements, uat) = (root.join("ROADMAP.md"), root.join("REQUIREMENTS.md"), root.join("phases/13/UAT.md"));
    // Publication seeded the one missing active row, Pending, after the
    // existing row; it raised nothing and touched no other byte.
    let seeded = format!("{REQUIREMENTS_T5}| T1 | Phase 13 | Pending |\n");
    assert_eq!(std::fs::read_to_string(&requirements).unwrap(), seeded);
    assert_eq!(std::fs::read_to_string(&roadmap).unwrap(), ROADMAP_OPEN);
    let context = reopened(project).snapshot.data["context"].clone();
    let placeholder = json!({"project":"","root_binding":"","phase":13,"occurrence":"","context_digest":"","truths":[],
        "publications":[],"map_digest":"","admission_digests":[],"execution_digest":"",
        "source":{"head":"","tree":"","index_digest":"","material_digest":""}});
    let projections_unchanged = |uat_expected: &str| {
        assert_eq!(std::fs::read_to_string(&roadmap).unwrap(), ROADMAP_OPEN);
        assert_eq!(std::fs::read_to_string(&requirements).unwrap(), seeded);
        assert_eq!(std::fs::read_to_string(&uat).unwrap(), uat_expected);
        let saved = reopened(project).snapshot;
        assert_eq!(saved.data["verification"].get("completions"), None, "no completion authority");
        assert_eq!(saved.data["context"], context, "approved context unchanged");
    };
    let refused = |request: Value, rule: &str, slot: &str, uat_expected: &str| -> Value {
        let answer = apply(project, request.clone());
        assert_eq!(answer["status"], "refused", "{request}\n{answer}");
        assert_eq!(answer["rule"], rule, "{answer}");
        assert_eq!(answer["slot"], slot, "{answer}");
        projections_unchanged(uat_expected);
        answer
    };
    // Publication alone: the located refusal names the unadmitted execution.
    let answer = refused(completion(&root, "after-publication", &json!(""), &placeholder), "admission-required", "admissions", HISTORICAL_UAT);
    assert_eq!(answer["phase"], 13);
    // Execution complete, SUMMARY present, no verdicts: still not acceptance.
    fixture.execute();
    let next = query(project, json!({"operation":"execute-next","phase":13}));
    assert_eq!((next["status"].as_str(), next["outcome"].as_str()), (Some("ok"), Some("complete")), "execution is complete: {next}");
    let read = report(project);
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["completion"], json!({"status":"incomplete","applicable":false,"reason":"no completion recorded","record":null}));
    let basis = read["current"]["observed"].clone();
    assert!(basis.is_object(), "{read}");
    let answer = refused(completion(&root, "after-execution", &json!(""), &basis), "verification-incomplete", "verification", HISTORICAL_UAT);
    assert!(answer["reason"].as_str().unwrap().contains("no complete verification"), "{answer}");
    // An open attempt with finished runs is not a verification either.
    let (open, _) = inspect(project, "open", &[]);
    refused(completion(&root, "open-attempt", &open["id"], &basis), "verification-incomplete", "verification", HISTORICAL_UAT);
    // One unmet truth: the refusal names it, its negative item and the
    // imported human failure that is also unfinished.
    let (partial, _) = verify(project, "partial", &[("check/B", "rejected", "tests/b.py proves nothing about the second parcel.")]);
    let answer = refused(completion(&root, "partial", &partial["id"], &basis), "verification-incomplete", "truths", HISTORICAL_UAT);
    assert_eq!(answer["id"], "truth/B");
    assert_eq!(answer["details"]["requested"]["unfinished"], json!([
        {"kind":"truth","id":"truth/B","version":1,"status":"unmet","reason":"rejected or not seen: check/B","items":[{"id":"check/B","verdict":"rejected"}]},
        {"kind":"human","id":"1","status":"fail","source":"imported","first_pass":"fail"}]));
    assert_eq!(answer["details"]["current"]["counts"], json!({"met":1,"concerns":0,"unmet":1,"pending":0,"waived":0}));
    // A real repair commit makes that verification historical.
    std::fs::write(project.join("src/b.py"), "def answer():\n    return 7 # repaired implementation\n").unwrap();
    git_value(project, &["add", "src/b.py"]);
    git_value(project, &["commit", "-m", "Fixture repair"]);
    let answer = refused(completion(&root, "stale", &partial["id"], &basis), "verification-basis", "basis.source", HISTORICAL_UAT);
    assert_eq!(answer["details"]["current"]["head"], git_value(project, &["rev-parse", "HEAD"]));
    let read = report(project);
    assert_eq!(read["current"]["applicable"], false);
    assert_eq!(read["history"][1]["applicability"], "historical");
    let basis = read["current"]["observed"].clone();
    refused(completion(&root, "stale-attempt", &partial["id"], &basis), "verification-incomplete", "verification", HISTORICAL_UAT);
    // Every item accepted on fresh independent runs; the human failure remains.
    let (accepted, _) = verify(project, "accepted", &[]);
    let read = report(project);
    assert_eq!(read["counts"], json!({"met":2,"concerns":0,"unmet":0,"pending":0,"waived":0}));
    let answer = refused(completion(&root, "human-conflict", &accepted["id"], &basis), "verification-incomplete", "humans", HISTORICAL_UAT);
    assert_eq!(answer["id"], "1");
    assert_eq!(answer["details"]["requested"]["unfinished"], json!([{"kind":"human","id":"1","status":"fail","source":"imported","first_pass":"fail"}]));
    let stored = reopened(project).snapshot;
    assert_eq!(apply(project, completion(&root, "human-conflict", &accepted["id"], &basis)), answer, "repeating asks nothing new");
    assert_eq!(reopened(project).snapshot, stored);
    // The verifier's patch arm cannot resolve human work, and a wrong attempt cannot either.
    let overwrite = json!({"request_id":"resolve-by-patch","attempt":accepted["id"],"basis":basis,"items":[],"humans":[{"id":"1","outcome":"passed"}]});
    assert_eq!(apply(project, json!({"operation":"verification-submit","patch":overwrite}))["rule"], "verification-shape");
    refused(completion(&root, "wrong-attempt", &partial["id"], &basis), "verification-attempt", "attempt", HISTORICAL_UAT);
    assert_eq!(reopened(project).snapshot.data["verification"].get("humans"), None);
    // Only the authorized human path resolves it; first pass stays fail.
    let occurrence = query(project, json!({"operation":"plan-read","phase_address":"13"}))["occurrence"].clone();
    let submission = json!({"phase":13,"occurrence":occurrence,"id":"1","reply":"The parcel arrived on the second attempt.",
        "outcome":"passed","owner":"Fixture Owner","at":"2026-09-11T17:00:00Z","supersedes":null});
    let resolved = apply(project, json!({"operation":"verification-human-result","request_id":"resolve-1","submission":submission,
        "approval":{"approved":true,"owner":"Fixture Owner","at":"2026-09-11T17:00:00Z","submission":submission}}));
    assert_eq!(resolved["status"], "ok", "{resolved}");
    assert_eq!(resolved["receipt"]["record"]["first_pass"], "fail");
    let rendered = std::fs::read_to_string(&uat).unwrap();
    assert!(rendered.starts_with(HISTORICAL_UAT), "{rendered}");
    assert!(rendered.contains("\n### 1. 1\nname: Delivery\nstatus: pass\nfirst_pass: fail\nreported: \"The parcel arrived on the second attempt.\"\n"), "{rendered}");
    let read = report(project);
    assert_eq!(read["humans"][0]["resolved"], true);
    assert_eq!(read["humans"][0]["first_pass"], "fail");
    assert_eq!(read["humans"][0]["history"][0]["reply"], "The parcel arrived on the second attempt.");
    // Interleaved reads change nothing; a stale caller preimage still refuses.
    assert_eq!(query(project, json!({"operation":"plan-read","phase_address":"13"}))["status"], "ok");
    assert_eq!(query(project, json!({"operation":"execute-next","phase":13}))["outcome"], "complete");
    let mut stale = completion(&root, "stale-preimage", &accepted["id"], &basis);
    stale["projections"]["requirements"] = json!(cadence::store::model::digest(REQUIREMENTS_T5.as_bytes()));
    refused(stale, "verification-projection", "projections.requirements", &rendered);
    // The fully applicable completion: authority and both projections, once.
    let request = completion(&root, "complete-13", &accepted["id"], &basis);
    let done = apply(project, request.clone());
    assert_eq!(done["status"], "ok", "{done}");
    let record = done["receipt"]["record"].clone();
    assert_eq!(record["label"], "complete");
    assert_eq!(record["attempt"], accepted["id"]);
    assert_eq!(record["basis"], basis);
    assert_eq!(record["truths"], json!([{"id":"truth/A","version":1,"status":"met","derived":"met","waiver":null},
        {"id":"truth/B","version":1,"status":"met","derived":"met","waiver":null}]));
    assert_eq!(record["humans"], json!([{"id":"1","status":"pass","first_pass":"fail","source":"native"},
        {"id":"2","status":"pass","first_pass":"pass","source":"imported"}]));
    assert_eq!(record["projections"]["requirements"]["rows"], json!(["T1"]));
    assert_eq!(std::fs::read_to_string(&roadmap).unwrap(), ROADMAP_DONE);
    assert_eq!(std::fs::read_to_string(&requirements).unwrap(), seeded.replace("| T1 | Phase 13 | Pending |", "| T1 | Phase 13 | Complete |"));
    assert_eq!(std::fs::read_to_string(&uat).unwrap(), rendered, "completion leaves UAT.md alone");
    let saved = reopened(project).snapshot;
    assert_eq!(saved.data["context"], context);
    assert_eq!(saved.data["verification"]["completions"], json!([record]));
    let read = report(project);
    assert_eq!(read["completion"]["status"], "complete");
    assert_eq!(read["completion"]["applicable"], true);
    assert_eq!(read["current"]["attempt"], accepted["id"], "untracked projections are not source");
    // Once only: exact replay answers the same, a second completion refuses,
    // and the lifecycle now agrees with the checked box.
    let after = tree(project);
    let replay = apply(project, request);
    assert_eq!(replay["receipt"]["record"], record);
    assert_eq!(replay["receipt"]["replayed"], true);
    assert_eq!(tree(project), after);
    let again = apply(project, completion(&root, "complete-13-again", &accepted["id"], &basis));
    assert_eq!(again["rule"], "verification-complete", "{again}");
    assert_eq!(tree(project), after);
    let next = query(project, json!({"operation":"execute-next","phase":13}));
    assert_eq!((next["status"].as_str(), next["outcome"].as_str()), (Some("ok"), Some("complete")), "native completion is the lifecycle authority: {next}");
    assert_eq!(tree(project), after);
    assert_eq!(report(project), read);
    assert_eq!(reopened(project).snapshot, saved);
    // A later publication changes the native inputs: the completion is no
    // longer applicable, the report says which input, and the lifecycle
    // names the exact disagreement with the checked box.
    let gap = proposal(project, "gap", &[(None, attached(vec![artifact("artifact/gap", &["truth/A"])]))]);
    publish(project, &gap);
    let read = report(project);
    assert_eq!(read["completion"]["status"], "incomplete");
    assert_eq!(read["completion"]["applicable"], false);
    assert_eq!(read["completion"]["reason"], "native inputs changed since completion: publications");
    assert_eq!(read["completion"]["record"], record, "history is preserved");
    assert_eq!(read["current"]["applicable"], false);
    let next = query(project, json!({"operation":"execute-next","phase":13}));
    assert_eq!(next["status"], "refused", "{next}");
    assert_eq!(next["code"], "state-conflict", "{next}");
    assert!(next["reason"].as_str().unwrap().contains("ROADMAP.md:2"), "{next}");
    assert_eq!(std::fs::read_to_string(&roadmap).unwrap(), ROADMAP_DONE, "a query repairs nothing");
    assert_eq!(reopened(project).snapshot.data["verification"]["completions"], json!([record]));
    assert_eq!(reopened(project).snapshot.data["context"], context);
    for name in ["findings.md", ".planning/findings.md"] {
        assert!(!project.join(name).exists());
    }
}
