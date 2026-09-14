//! Phase 36 acceptance checks cross the real stdio, publication and retirement boundary.
#[path = "support/phase13.rs"]
#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{fs, path::Path, process::{Command, Stdio}};
use support::Client;

const PHASE: u32 = 36;
const OLD_COMMAND: &str = "python3 -B tests/old_control.py";
const NEW_COMMAND: &str = "python3 -B tests/new_control.py";
const OWNER: &str = "Fixture Owner";
const AT: &str = "2026-09-14T16:00:00Z";
const REASON: &str = "The admitted implementation cannot satisfy the approved contract.";

fn call(project: &Path, tool: &str, request: Value) -> Value {
    let mut client = Client::open(project);
    let answer = client.call(tool, request);
    client.finish();
    answer
}

fn git(project: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=Cadence-Phase36", "-c", "user.email=phase36@example.invalid"])
        .args(args)
        .current_dir(project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GNUPGHOME", project.join(".fixture-gnupg"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
}

fn fixture() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    fs::create_dir_all(project.join(".planning/phases/36")).unwrap();
    fs::create_dir(project.join(".fixture-gnupg")).unwrap();
    fs::set_permissions(project.join(".fixture-gnupg"), fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new("gpg")
        .env("GNUPGHOME", project.join(".fixture-gnupg"))
        .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", "", "--quick-generate-key",
            "Cadence-Phase36 <phase36@example.invalid>", "ed25519", "sign", "0"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    fs::write(project.join(".planning/ROADMAP.md"),
        "## Phases\n- [ ] **Phase 36: Released checks**\n- [ ] **Phase 37: Next phase**\n").unwrap();
    fs::write(project.join(".planning/config.json"), serde_json::to_vec(&json!({
        "review":{"triggers":{"risk_surface":{"surfaces":cadence::rail::risk::CATEGORIES}}}
    })).unwrap()).unwrap();
    fs::create_dir(project.join("src")).unwrap();
    fs::write(project.join(".gitignore"), ".planning/\n.fixture-gnupg/\n__pycache__/\n").unwrap();
    fs::write(project.join("src/control.py"), "def answer():\n    return 6\n").unwrap();
    git(project, &["init", "--initial-branch=fixture/released-checks"]);
    git(project, &["config", "user.signingkey", "phase36@example.invalid"]);
    git(project, &["add", ".gitignore", "src/control.py"]);
    git(project, &["commit", "-S", "-m", "Fixture phase 36 subject"]);
    temp
}

fn association() -> Value {
    json!([{"truth_id":"T1","truth_version":1,
        "reason":"This proves the owner-visible release of blocked check ownership."}])
}

fn old_check() -> Value {
    json!({"kind":"check","id":"check/released","reason":"The old check identifies the blocked definition.",
        "spec":{"command":OLD_COMMAND,"expected":{"kind":"property","value":"the old answer is six"},
            "test":{"file":"tests/old_control.py","function":"OldCheck.test_answer"},
            "setup":"The first admitted plan owns this check.",
            "call":"Run the old public-boundary check.",
            "boundary":"Real binary over MCP stdio and durable native execution history.","fakes":[]},
        "associations":association()})
}

fn changed_check() -> Value {
    json!({"kind":"check","id":"check/released","reason":"The new check identifies the repaired definition.",
        "spec":{"command":NEW_COMMAND,"expected":{"kind":"literal","value":"the repaired answer is seven"},
            "test":{"file":"tests/new_control.py","function":"NewCheck.test_repaired_answer"},
            "setup":"The first admitted plan has blocked with its owner task unclosed.",
            "call":"Run the changed public-boundary check.",
            "boundary":"Real binary over MCP stdio and durable native execution history.","fakes":[]},
        "associations":association()})
}

fn artifact() -> Value {
    json!({"kind":"artifact","id":"artifact/remains","reason":"The blocked plan's non-check work remains current.",
        "spec":{"locators":["src/control.py"],"substance":"The blocked publication remains visible for later repair."},
        "associations":association()})
}

fn map(items: Vec<Value>) -> Value { json!({"mode":"attached","items":items}) }

fn body(map: &Value) -> String {
    format!("# Fixture plan\n\n## Evidence map\n\n```json\n{}\n```\n\n", support::section_json(map, 0))
}

fn history(project: &Path) -> Value {
    let answer = call(project, "cadence_query", json!({"operation":"execution-history","phase":PHASE}));
    assert_eq!(answer["status"], "ok", "{answer}");
    answer
}

fn task(project: &Path) -> Value {
    history(project)["tasks"].as_array().unwrap().iter()
        .find(|entry| entry["task"]["plan"] == 1 && entry["task"]["task"] == "blocked-owner")
        .unwrap().clone()
}

fn publish_and_block(project: &Path) -> (Value, Value) {
    let truth = json!({"id":"T1","trigger":"an admitted plan blocks with its check-owning task unclosed",
        "observer":"the owner","verb":"sees","outcome":"a changed definition accepted by the later plan preview",
        "kind":"property","observable":true,"fixed_oracle":true});
    let context = call(project, "cadence_apply", support::approve(json!({
        "operation":"context-submit","submission":{"phase":PHASE,"title":"Released checks",
        "scope":"Blocked ownership only.","durable_decisions":[],"decisions":[],"assumptions":[],"truths":[truth]}
    })));
    assert_eq!(context["persisted"], true, "{context}");

    let initial_map = map(vec![old_check(), artifact()]);
    let mut client = Client::open(project);
    let allocation = client.read("36", Some(1));
    assert_eq!(allocation["status"], "ok", "{allocation}");
    let target = allocation["targets"][0].clone();
    let submission = json!({"phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"publish-blocked-plan","inventory_basis":allocation["inventory"]["basis"],"plans":[{
            "target":target,"content":{"phase":PHASE,"plan":target["plan"],"requirements":["T1"],
            "files":["src/control.py"],"directories":[],"execution":{"schema":1,"suite":OLD_COMMAND,
                "tasks":[{"id":"blocked-owner","verify":[OLD_COMMAND]}]},
            "body":body(&initial_map),"evidence_map":initial_map}}]});
    let preview = client.call("cadence_query", json!({"operation":"plan-read","phase_address":"36",
        "submission":submission}));
    assert_eq!(preview["status"], "ok", "{preview}");
    let published = client.call("cadence_apply", support::approve(json!({
        "operation":"plan-submit","submission":preview["submission"]
    })));
    assert_eq!(published["persisted"], true, "{published}");
    let old_revision = published["results"][0]["map_revision"].clone();
    client.finish();

    let mut client = Client::open(project);
    let plans = client.read("36", None);
    let evidence = client.call("cadence_query", json!({"operation":"evidence-read","phase":PHASE}));
    client.finish();
    let publication = &plans["native"]["publications"]["1"];
    let check = evidence["items"].as_array().unwrap().iter()
        .find(|item| item["id"] == "check/released").unwrap();
    let contract = json!({"phase":PHASE,"occurrence":plans["occurrence"],"plans":[{
        "plan":1,"publication_request":publication["publication_request"],
        "content_revision":publication["revision"],"map_revision":publication["map_revision"]}],
        "allocation":[{"plan":1,"task":"blocked-owner","checks":[{
            "id":"check/released","item_revision":check["item_revision"]}]}]});
    let admitted = call(project, "cadence_apply", json!({"operation":"execution-admit","request":{
        "request_id":"admit-blocked-plan","expected_set_version":0,"contract":contract}}));
    assert_eq!(admitted["status"], "ok", "{admitted}");
    let authorized = call(project, "cadence_apply", json!({"operation":"execution-authorize","phase":PHASE,
        "request_id":"authorize-blocked-plan","owner":OWNER,"at":AT,"response":"Run the approved blocked plan."}));
    assert_eq!(authorized["status"], "ok", "{authorized}");
    let dispatch = call(project, "cadence_query", json!({"operation":"execute-next","phase":PHASE}));
    assert_eq!(dispatch["outcome"], "dispatch", "{dispatch}");
    let current = task(project);
    let started = call(project, "cadence_apply", json!({"operation":"execution-task-start","request":{
        "request_id":"start-blocked-owner","task":current["task"],"attempt":"attempt-blocked-owner",
        "expected_version":current["state"]["version"],"predecessor":null,
        "checks":contract["allocation"][0]["checks"]}}));
    assert_eq!(started["status"], "ok", "{started}");
    let current = task(project);
    let retired = call(project, "cadence_apply", json!({"operation":"execution-task-retire","request":{
        "request_id":"retire-blocked-owner","task":current["task"],"attempt":"attempt-blocked-owner",
        "expected_version":current["state"]["version"],"owner":OWNER,"at":AT,"reason":REASON}}));
    assert_eq!(retired["status"], "ok", "{retired}");
    assert_eq!(retired["outcome"]["disposition"], "blocked", "{retired}");
    (old_revision, contract)
}

#[test]
fn phase36_blocked_check_can_be_republished_with_changed_spec() {
    let fixture = fixture();
    let project = fixture.path();
    let (old_map_revision, _) = publish_and_block(project);

    let changed = changed_check();
    let later_map = map(vec![changed.clone()]);
    let mut client = Client::open(project);
    let allocation = client.read("36", Some(1));
    assert_eq!(allocation["status"], "ok", "{allocation}");
    assert_eq!(allocation["map_history"][0]["publication"]["revision"], old_map_revision);
    let target = allocation["targets"][0].clone();
    let submission = json!({"phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"preview-changed-check","inventory_basis":allocation["inventory"]["basis"],"plans":[{
            "target":target,"content":{"phase":PHASE,"plan":target["plan"],"requirements":["T1"],
            "files":["src/control.py"],"directories":[],"execution":{"schema":1,"suite":NEW_COMMAND,
                "tasks":[{"id":"repair-owner","verify":[NEW_COMMAND]}]},
            "body":body(&later_map),"evidence_map":later_map}}]});
    let preview = client.call("cadence_query", json!({"operation":"plan-read","phase_address":"36",
        "submission":submission}));
    client.finish();

    assert_eq!(preview["status"], "ok", "{preview}");
    assert_eq!(preview["persisted"], false);
    assert_eq!(preview["submission"]["plans"][0]["content"]["evidence_map"]["items"], json!([changed]));
    assert_eq!(preview["coverage"]["uncovered"], json!([]));
    assert_eq!(preview["coverage"]["without_check"], json!([]));
}

#[test]
fn phase36_evidence_read_retains_released_check_as_superseded() {
    let fixture = fixture();
    let project = fixture.path();
    let (old_map_revision, _) = publish_and_block(project);

    let evidence = call(project, "cadence_query", json!({"operation":"evidence-read","phase":PHASE}));

    assert_eq!(evidence["status"], "ok", "{evidence}");
    assert_eq!(evidence["schema"], "acceptance-map-view-1");
    assert_eq!(evidence["coherence"], "consistent");
    assert_eq!(evidence["contributions"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["items"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["items"][0]["kind"], "artifact");
    assert_eq!(evidence["items"][0]["id"], "artifact/remains");
    assert_eq!(evidence["items"][0]["reason"],
        "The blocked plan's non-check work remains current.");
    assert_eq!(evidence["items"][0]["spec"], artifact()["spec"]);

    let history = &evidence["history"][0];
    assert_eq!(evidence["history"].as_array().unwrap().len(), 1);
    assert_eq!(history["status"], "superseded");
    assert_eq!(history["superseded_by"], Value::Null);
    assert_eq!(history["publication"]["revision"], old_map_revision);
    assert_eq!(history["publication"]["items"], json!([old_check(), artifact()]));
    let item_revisions = &history["publication"]["item_revisions"];
    let artifact_revision = &item_revisions["artifact/remains"];

    assert_eq!(evidence["items"][0]["item_revision"], *artifact_revision);
    assert_eq!(evidence["aliases"], json!([{
        "origin":{"plan":1,"map_revision":old_map_revision,
            "item_id":"artifact/remains","item_revision":artifact_revision},
        "id":"artifact/remains","item_revision":artifact_revision
    }]));
    assert_eq!(evidence["associations"], json!([{
        "truth_id":"T1","truth_version":1,
        "reason":"This proves the owner-visible release of blocked check ownership.",
        "origin":{"plan":1,"map_revision":old_map_revision,
            "item_id":"artifact/remains","item_revision":artifact_revision,"association_index":0}
    }]));
    assert_eq!(evidence["coverage"]["uncovered"], json!([]));
    assert_eq!(evidence["coverage"]["without_check"], json!(["T1"]));
    assert_eq!(evidence["coverage"]["checks"],
        json!([{"truth_id":"T1","truth_version":1,"item_ids":[]}]))
}

#[test]
fn phase36_extension_reassigns_released_check() {
    let fixture = fixture();
    let project = fixture.path();
    let (_, old_contract) = publish_and_block(project);
    let old_assignment = old_contract["allocation"][0].clone();

    let later_map = map(vec![old_check()]);
    let mut client = Client::open(project);
    let allocation = client.read("36", Some(1));
    assert_eq!(allocation["status"], "ok", "{allocation}");
    let target = allocation["targets"][0].clone();
    let submission = json!({"phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"publish-later-owner","inventory_basis":allocation["inventory"]["basis"],
        "plans":[{"target":target,"content":{"phase":PHASE,"plan":target["plan"],
            "requirements":["T1"],"files":["src/control.py"],"directories":[],
            "execution":{"schema":1,"suite":OLD_COMMAND,
                "tasks":[{"id":"later-owner","verify":[OLD_COMMAND]}]},
            "body":body(&later_map),"evidence_map":later_map}}]});
    let preview = client.call("cadence_query", json!({"operation":"plan-read","phase_address":"36",
        "submission":submission}));
    assert_eq!(preview["status"], "ok", "{preview}");
    let published = client.call("cadence_apply", support::approve(json!({
        "operation":"plan-submit","submission":preview["submission"]
    })));
    assert_eq!(published["persisted"], true, "{published}");
    client.finish();

    let mut client = Client::open(project);
    let plans = client.read("36", None);
    let evidence = client.call("cadence_query", json!({"operation":"evidence-read","phase":PHASE}));
    client.finish();
    let check = evidence["items"].as_array().unwrap().iter()
        .find(|item| item["id"] == "check/released").unwrap();
    let bindings = plans["native"]["publications"].as_object().unwrap().values().map(|publication| {
        json!({"plan":publication["identity"]["plan"],
            "publication_request":publication["publication_request"],
            "content_revision":publication["revision"],"map_revision":publication["map_revision"]})
    }).collect::<Vec<_>>();
    let later_assignment = json!({"plan":2,"task":"later-owner","checks":[{
        "id":"check/released","item_revision":check["item_revision"]}]});
    let contract = json!({"phase":PHASE,"occurrence":plans["occurrence"],"plans":bindings,
        "allocation":[old_assignment.clone(),later_assignment.clone()]});
    let extended = call(project, "cadence_apply", json!({"operation":"execution-extend","request":{
        "request_id":"extend-later-owner","expected_set_version":1,"contract":contract.clone()}}));

    assert_eq!(extended["status"], "ok", "{extended}");
    assert_eq!(extended["receipt"]["set_version"], 2, "{extended}");
    let receipt = &extended["receipt"];
    assert_eq!(receipt["request"]["contract"], contract);
    assert_eq!(receipt["request"]["contract"]["allocation"][0], old_assignment);
    assert_eq!(receipt["request"]["contract"]["allocation"][1], later_assignment);
}
