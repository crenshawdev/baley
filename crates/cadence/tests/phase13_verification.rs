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
