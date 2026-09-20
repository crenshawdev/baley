#![allow(dead_code)]
use crate::phase13::{self, Client, apply, query, git, git_value};
use serde_json::{Value, json};
use std::{fs, path::Path, process::{Command, Stdio}, os::unix::fs::PermissionsExt};

pub struct Fixture { pub temp: tempfile::TempDir }

fn ok(client: &mut Client, value: Value) -> Value {
    let answer = client.call("cadence_apply", value);
    assert_eq!(answer["status"], "ok", "{answer}");
    answer
}

impl Fixture {
    pub fn project(&self) -> &Path { self.temp.path() }
    pub fn new(phases: &[u32]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path();
        for path in [".planning/phases", "src", "tests", ".fixture-gnupg"] {
            fs::create_dir_all(project.join(path)).unwrap();
        }
        fs::set_permissions(project.join(".fixture-gnupg"), fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new("gpg").env("GNUPGHOME", project.join(".fixture-gnupg"))
            .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", "", "--quick-generate-key",
                "Cadence-Phase13 <phase13@example.invalid>", "ed25519", "sign", "0"])
            .stdin(Stdio::null()).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        git(project, &["init", "--initial-branch=fixture/landing"]);
        git_value(project, &["config", "user.signingkey", "phase13@example.invalid"]);
        fs::write(project.join(".gitignore"), ".planning/*\n!.planning/*.md\n!.planning/config.json\n!.planning/phases/\n.planning/phases/*/*\n!.planning/phases/*/*.md\n.fixture-gnupg/\n__pycache__/\n.git-trace\n").unwrap();
        fs::write(project.join(".planning/ROADMAP.md"), format!("## Phases\n{}", phases.iter().map(|p| format!("- [ ] **Phase {p}: Fixture {p}**\n")).collect::<String>())).unwrap();
        fs::write(project.join(".planning/config.json"), serde_json::to_vec(&json!({"review":{"triggers":{
            "plan":{"gate":"deferred"},"risk_surface":{"surfaces":cadence::rail::risk::CATEGORIES}}}})).unwrap()).unwrap();
        for phase in phases {
            fs::create_dir_all(project.join(format!(".planning/phases/{phase}"))).unwrap();
            fs::write(project.join(format!("src/p{phase}.txt")), "pending\n").unwrap();
            fs::write(project.join(format!("tests/p{phase}.py")), format!("import pathlib, unittest\nclass Check(unittest.TestCase):\n    def test_file(self):\n        self.assertEqual(pathlib.Path('src/p{phase}.txt').read_text(), 'ready\\n')\nif __name__ == '__main__':\n    unittest.main()\n")).unwrap();
        }
        git(project, &["add", "."]);
        git(project, &["commit", "-m", "Fixture baseline"]);
        for phase in phases { complete_phase(project, *phase); }
        git(project, &["add", ".planning"]);
        git(project, &["commit", "-m", "Fixture completed documents"]);
        assert_eq!(git_value(project, &["status", "--porcelain"]), "");
        Self { temp }
    }
}

fn state(client: &mut Client, phase: u32, key: &str) -> Value {
    client.call("cadence_query", json!({"operation":"execution-history","phase":phase}))[key][0].clone()
}

fn complete_phase(project: &Path, phase: u32) {
    let mut client = Client::open(project);
    let context = phase13::approve(json!({"operation":"context-submit","submission":{
        "phase":phase,"title":"Native milestone fixture","scope":"Complete one real artifact.",
        "durable_decisions":[],"decisions":[],"assumptions":[],"truths":[{
            "id":"T1","trigger":"the owner opens the artifact","observer":"the owner","verb":"sees",
            "outcome":"ready","kind":"property","observable":true,"fixed_oracle":true}]}}));
    ok(&mut client, context);
    let allocation = client.call("cadence_query", json!({"operation":"plan-read","phase":phase,"count":1}));
    let command = format!("python3 -B tests/p{phase}.py");
    let file = format!("src/p{phase}.txt");
    ok(&mut client, phase13::approve(json!({"operation":"plan-submit","submission":{
        "phase":phase,"occurrence":allocation["occurrence"],"request_id":format!("publish-{phase}"),
        "inventory_basis":allocation["inventory"]["basis"],"plans":[{"target":allocation["targets"][0],"content":{
            "phase":phase,"plan":1,"requirements":["T1"],"files":[file],"directories":[],
            "goal":"Deliver the ready artifact.","context":"Native completion fixture.","notes":"One artifact.",
            "tasks":[{"id":"task-ready","title":"Deliver ready","files":[file],"action":"Write the ready artifact.","verify":[command]}],
            "suite":command,"evidence_map":{"mode":"attached","items":[{"kind":"artifact","id":"artifact/ready",
                "spec":{"locators":[file],"substance":"The ready artifact contains ready."},"reason":"Observe the ready artifact.",
                "associations":[{"truth_id":"T1","truth_version":1,"reason":"Observe ready."}]},
                {"kind":"check","id":"check/ready","reason":"Read the real artifact.","spec":{"command":command,
                    "expected":{"kind":"literal","value":"ready followed by a newline"},"test":{"file":format!("tests/p{phase}.py"),"function":"Check.test_file"},
                    "setup":"A pending artifact.","call":"Read the artifact.","boundary":"real filesystem","fakes":[]},
                    "associations":[{"truth_id":"T1","truth_version":1,"reason":"Read ready."}]}]}}}]}})));
    git(project, &["add", ".planning"]);
    git(project, &["commit", "-m", &format!("Fixture phase {phase} publication")]);
    let read = client.call("cadence_query", json!({"operation":"plan-read","phase":phase}));
    let publication = &read["native"]["publications"]["1"];
    let map = client.call("cadence_query", json!({"operation":"evidence-read","phase":phase}));
    let item = map["items"].as_array().unwrap().iter().find(|i| i["kind"] == "check").unwrap();
    let check = json!({"id":item["id"],"item_revision":item["item_revision"]});
    ok(&mut client, json!({"operation":"execution-admit","request":{"request_id":format!("admit-{phase}"),"expected_set_version":0,
        "contract":{"phase":phase,"occurrence":read["occurrence"],"plans":[{"plan":1,"publication_request":publication["publication_request"],
            "content_revision":publication["revision"],"map_revision":publication["map_revision"]}],
            "allocation":[{"plan":1,"task":"task-ready","checks":[check]}]}}}));
    ok(&mut client, json!({"operation":"execution-authorize","phase":phase,"request_id":format!("authorize-{phase}"),
        "owner":"Fixture Owner","at":"2026-09-19T12:00:00Z","response":"Proceed with the fixture"}));
    let dispatch = client.call("cadence_query", json!({"operation":"execute-next","phase":phase}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-task-start","request":{"request_id":format!("start-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":0,"predecessor":null,"checks":[check]}}));
    git(project, &["add", &format!("tests/p{phase}.py")]);
    git(project, &["commit", "--allow-empty", "-m", &format!("test({phase}): expect ready task-ready")]);
    let red_commit = git_value(project, &["rev-parse", "HEAD"]);
    let task = state(&mut client, phase, "tasks");
    let red_run = format!("red-{phase}");
    ok(&mut client, json!({"operation":"execution-run","request":{"request_id":red_run,"task":task["task"],"attempt":"attempt-ready",
        "expected_version":task["state"]["version"],"command":command,"check":check,"stage":"red"}}));
    let red = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &red_run);
    assert_eq!(red["request"]["event"]["disposition"]["code"], 1, "{red}");
    fs::write(project.join(&file), "ready\n").unwrap();
    git(project, &["add", &file]);
    git_value(project, &["commit", "-S", "-m", &format!("feat({phase}): deliver ready task-ready")]);
    let completion = git_value(project, &["rev-parse", "HEAD"]);
    let task = state(&mut client, phase, "tasks");
    let run = format!("verify-{phase}");
    ok(&mut client, json!({"operation":"execution-run","request":{"request_id":run,"task":task["task"],"attempt":"attempt-ready",
        "expected_version":task["state"]["version"],"command":command,"check":check,"stage":"green"}}));
    let result = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &run);
    assert_eq!(result["request"]["event"]["disposition"]["code"], 0, "{result}");
    let launch = client.call("cadence_query", json!({"operation":"execution-history","phase":phase,"run":red_run}));
    let inspection = json!({"check":check,"test_digest":launch["launch"]["request"]["event"]["material"]["test_digest"],
        "evidence":[red_run,run],"no_subject_stub":true});
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-owner-attest","request":{"request_id":format!("inspect-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":task["state"]["version"],"statement":{"submission":inspection,
            "approval":{"approved":true,"owner":"Fixture Owner","at":"2026-09-19T12:00:00Z","submission":inspection}}}}));
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-task-close","request":{"request_id":format!("close-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":task["state"]["version"],"completion":completion,
        "checks":[{"check":check,"red_commit":red_commit,"green_commit":completion,"red_run":red_run,"green_run":run}],"verification":[run]}}));
    let plan = state(&mut client, phase, "plans");
    let suite = format!("suite-{phase}");
    ok(&mut client, json!({"operation":"execution-suite","request":{"request_id":suite,"plan":plan["plan"],"expected_version":plan["state"]["version"]}}));
    let result = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &suite);
    assert_eq!(result["request"]["event"]["disposition"]["code"], 0, "{result}");
    ok(&mut client, json!({"operation":"risk-check","request_id":format!("execution-risk-{phase}"),
        "scope":{"phase":phase,"occurrence":format!("phase-{phase}-execution"),"worker":"1"},
        "source":{"kind":"execution","plan":1,"dispatch_id":dispatch["dispatch_id"]},"surfaces":null}));
    let plan = state(&mut client, phase, "plans");
    ok(&mut client, json!({"operation":"execution-plan-complete","request":{"request_id":format!("complete-{phase}"),"plan":plan["plan"],"expected_version":plan["state"]["version"]}}));
    let dispatch = client.call("cadence_query", json!({"operation":"verify-next","phase":phase,"request_id":format!("verification-{phase}")}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let attempt = phase13::attempt_with(&mut client, &dispatch);
    let mut items = Vec::new();
    for item in attempt["map"]["items"].as_array().unwrap() {
        let mut runs = Vec::new();
        if item["kind"] == "check" {
            let run = format!("independent-{phase}");
            ok(&mut client, json!({"operation":"verification-run","request":{"request_id":run,"attempt":attempt["id"],"basis":attempt["basis"],
                "item":{"id":item["id"],"item_revision":item["item_revision"]}}}));
            let result = phase13::independent_result(&mut client, phase, &run);
            assert_eq!(result["disposition"]["code"], 0, "{result}");
            runs.push(run);
        }
        items.push(json!({"id":item["id"],"item_revision":item["item_revision"],"verdict":"accepted","observed":"The real artifact reads ready followed by a newline.","runs":runs}));
    }
    ok(&mut client, json!({"operation":"verification-submit","patch":{"request_id":format!("patch-{phase}"),"attempt":attempt["id"],
        "items":items}}));
    let read = client.call("cadence_query", json!({"operation":"verification-read","phase":phase}));
    ok(&mut client, json!({"operation":"verification-complete","request_id":format!("verified-{phase}"),"attempt":attempt["id"],
        "basis":read["current"]["observed"],"projections":{"roadmap":phase13::digest_of(&project.join(".planning/ROADMAP.md")),"requirements":null}}));
    client.finish();
}

pub fn risk(project: &Path, phase: u32, later_clear: bool) -> (Value, Value, Value) {
    let base = git_value(project, &["rev-parse", "HEAD"]);
    fs::write(project.join("src/risk.txt"), "jwt.verify(token)\n").unwrap();
    git(project, &["add", "src/risk.txt"]);
    git(project, &["commit", "-m", "Fixture risk material"]);
    let head = git_value(project, &["rev-parse", "HEAD"]);
    let scope = json!({"phase":phase,"occurrence":"milestone-risk","worker":null});
    let source = json!({"kind":"committed","base":base,"head":head});
    let scan = apply(project, json!({"operation":"risk-check","request_id":"risk-obligation","scope":scope,"source":source,"surfaces":["auth"]}));
    assert_eq!(scan["status"], "ok", "{scan}");
    assert!(!scan["observation"]["scan"]["matches"].as_array().unwrap().is_empty());
    let report = query(project, json!({"operation":"risk-status","scope":scope,"source":source,"surfaces":["auth"]}));
    let fire = json!({"id":"milestone-fire","binding":{"boundary":report["requirement"]["boundary"],
        "material":report["requirement"]["material"],"surfaces":["auth"],"observation":scan["confirmation"]},
        "review_scope":["src/risk.txt"],"rearm_of":null});
    let fired = apply(project, json!({"operation":"risk-fire","request_id":"fire-risk","fire":fire}));
    assert_eq!(fired["status"], "ok", "{fired}");
    if !later_clear { return (scan, fire, Value::Null); }
    fs::write(project.join("src/clear.txt"), "clear\n").unwrap();
    git(project, &["add", "src/clear.txt"]);
    git(project, &["commit", "-m", "Fixture later clear material"]);
    let clear = apply(project, json!({"operation":"risk-check","request_id":"later-clear","scope":scope,
        "source":{"kind":"committed","base":head,"head":git_value(project, &["rev-parse", "HEAD"])},"surfaces":["auth"]}));
    assert_eq!(clear["status"], "ok", "{clear}");
    assert_eq!(clear["observation"]["scan"]["matches"], json!([]));
    (scan, fire, clear)
}

pub fn deferred(project: &Path, phase: u32) -> String {
    let selected = query(project, json!({"operation":"review-select","command":"cad-review","arguments":["plan",phase.to_string()],"replay_key":"deferred-fixture"}));
    assert_eq!(selected["status"], "ok", "{selected}");
    let admitted = apply(project, json!({"operation":"review-admit","request":selected["result"]["admission"]}));
    assert_eq!(admitted["status"], "ok", "{admitted}");
    let fire = &admitted["result"]["fire"];
    let next = query(project, json!({"operation":"review-next","fire":fire}));
    assert_eq!(next["result"]["admission"]["gate"], "deferred", "{next}");
    assert!(next["result"]["attempt"].is_object(), "{next}");
    let queued = apply(project, json!({"operation":"review-enqueue","fire":fire}));
    assert_eq!(queued["result"]["state"], "unruled", "{queued}");
    queued["result"]["member"].as_str().unwrap().to_owned()
}

pub fn close(id: &str, phases: &[u32]) -> Value {
    json!({"operation":"milestone-close","request":{"request_id":id,"occurrence":"phase15-test","expected_generation":0,
        "selection":{"phases":phases,"label":"Fixture milestone"}}})
}

pub struct Publishing {
    pub project: tempfile::TempDir,
    pub remote: tempfile::TempDir,
    pub head: String,
    pub base: String,
}

impl Publishing {
    pub fn new(auto_close: bool) -> Self {
        let project = phase13::fixture();
        fs::write(project.path().join(".planning/config.json"), serde_json::to_vec(&json!({
            "git":{"auto_close":auto_close,"forge_provider":"github","forge_repo":"fixture/repo"}
        })).unwrap()).unwrap();
        Self::attach(project)
    }

    pub fn attach(project: tempfile::TempDir) -> Self {
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "--bare"]);
        git(project.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
        git(project.path(), &["push", "origin", "HEAD:refs/heads/main"]);
        let base = git_value(project.path(), &["rev-parse", "HEAD"]);
        fs::write(project.path().join("published.txt"), "publish this exact commit\n").unwrap();
        git(project.path(), &["add", "published.txt"]);
        git(project.path(), &["commit", "-m", "Fixture source"]);
        let head = git_value(project.path(), &["rev-parse", "HEAD"]);
        fs::create_dir_all(project.path().join(".run/bin")).unwrap();
        for program in ["gh", "glab", "tea"] {
            let path = project.path().join(".run/bin").join(program);
            let script = format!(r#"#!/usr/bin/env python3
import json, pathlib, sys
with pathlib.Path('.run/forge.log').open('a') as log:
    log.write(json.dumps(sys.argv) + '\n')
if 'GET' in sys.argv:
    print('[]')
elif any(arg.endswith('/merge') for arg in sys.argv):
    print('{{"merged":true,"sha":"{head}"}}')
else:
    print('{{"number":7,"iid":7,"html_url":"https://example.invalid/fixture/repo/pull/7","head":{{"sha":"{head}"}},"base":{{"ref":"main"}}}}')
"#);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { project, remote, head, base }
    }

    pub fn client(&self) -> Client {
        self.client_with_env(&[])
    }

    pub fn client_with_env(&self, env: &[(&str, &std::ffi::OsStr)]) -> Client {
        let mut paths = vec![self.project.path().join(".run/bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let path = std::env::join_paths(paths).unwrap();
        let mut vars = vec![("PATH", path.as_os_str())];
        vars.extend_from_slice(env);
        Client::open_with_env(self.project.path(), Path::new(env!("CARGO_BIN_EXE_cadence")), &vars)
    }

    pub fn start(&self, client: &mut Client, occurrence: &str) -> Value {
        ok(client, json!({"operation":"land-start","request":{
            "request_id":format!("start-{occurrence}"),"occurrence":occurrence,"expected_generation":0,
            "source":{"branch":git_value(self.project.path(), &["branch", "--show-current"]),"head":self.head},
            "base":{"branch":"main","head":self.base},
            "remote":{"name":"origin","url":self.remote.path().to_str().unwrap()}
        }}))["landing"].clone()
    }

    pub fn authorize(client: &mut Client, landing: &Value, id: &str, inputs: Value) -> Value {
        ok(client, json!({"operation":"land-authorize","request":{
            "request_id":format!("authorize-{id}"),"landing":landing["id"],"expected_generation":landing["generation"],
            "source":landing["source"],"base":landing["base"],"remote":landing["remote"],"inputs":inputs,
            "owner":"Fixture Owner","at":"2026-09-19T12:00:00Z"
        }}))["authorization"].clone()
    }

    pub fn invocations(&self) -> Vec<Value> {
        fs::read_to_string(self.project.path().join(".run/forge.log")).unwrap_or_default()
            .lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}

pub fn imported_auto_close(data: &Value) -> bool {
    match data {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "bytes" && serde_json::from_value::<Vec<u8>>(value.clone()).ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|raw| raw["git"]["auto_close"] == true)) || imported_auto_close(value)
        }),
        Value::Array(values) => values.iter().any(imported_auto_close),
        _ => false,
    }
}

impl Publishing {
    /// Merge in a separate real clone, leaving the landing worktree and base stale.
    pub fn cleanup_ready(&self, contained: bool) -> (Value, String) {
        self.durable_forge();
        git(self.project.path(), &["branch", "main", &self.base]);
        let mut client = self.client();
        let mut landing = self.start(&mut client, "cleanup-landing");
        let forge = json!({"provider":"github","repo":"fixture/repo","host":"github.com"});
        for (index, (operation, inputs)) in [
            ("land-publish", json!({"step":"push"})),
            ("land-open", json!({"step":"open","forge":forge,"title":"Cleanup","body":"Merged identity"})),
            ("land-merge", json!({"step":"merge","forge":forge,"pr":7})),
        ].into_iter().enumerate() {
            let auth = Self::authorize(&mut client, &landing, &format!("cleanup-grant-{index}"), inputs.clone());
            landing = ok(&mut client, json!({"operation":operation,"request":{
                "request_id":format!("cleanup-external-{index}"),"landing":landing["id"],
                "expected_generation":landing["generation"],"authorization":auth["id"],"inputs":inputs
            }}))["landing"].clone();
        }
        client.finish();
        let clone = self.project.path().join(".run/merge");
        git(self.project.path(), &["clone", "--branch", "main", self.remote.path().to_str().unwrap(), clone.to_str().unwrap()]);
        if contained {
            git(&clone, &["merge", "--no-ff", "origin/fixture/execution", "-m", "Fixture real merge"]);
        } else {
            git(&clone, &["commit", "--allow-empty", "-m", "Fixture unrelated base advance"]);
        }
        let merged = git_value(&clone, &["rev-parse", "HEAD"]);
        git(&clone, &["push", "origin", "main"]);
        let mut state = self.pr_state();
        state[0]["merge_commit_sha"] = json!(merged);
        self.set_pr_state(&state);
        assert_eq!(git_value(self.project.path(), &["rev-parse", "main"]), self.base);
        assert_eq!(git_value(self.project.path(), &["rev-parse", "HEAD"]), self.head);
        (landing, merged)
    }

    pub fn cleanup_commands(&self) -> Vec<Vec<String>> {
        self.trace().into_iter().filter(|event| event["event"] == "start")
            .filter_map(|event| serde_json::from_value::<Vec<String>>(event["argv"].clone()).ok())
            .filter(|args| matches!(args.get(1).map(String::as_str), Some("checkout" | "pull"))
                || (args.get(1).is_some_and(|s| s == "tag") && args.iter().any(|s| s == "-a"))
                || (args.get(1).is_some_and(|s| s == "branch") && args.iter().any(|s| s == "-d" || s == "-D")))
            .collect()
    }
}

// A real serve child exits only after its external command has returned.
pub fn effect_exit(mut client: Client, request: Value) {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.call("cadence_apply", request)));
    if answer.is_ok() {
        client.child.kill().unwrap();
    }
    assert_eq!(client.child.wait().unwrap().code(), Some(86), "serve must exit between effect and receipt");
    assert!(answer.is_err(), "an interrupted effect cannot return a receipt");
}

impl Publishing {
    pub fn durable_forge(&self) {
        git(self.remote.path(), &["config", "core.logAllRefUpdates", "true"]);
        let script = r#"#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv
endpoint = args[-1]
method = args[args.index('--method') + 1]
root = pathlib.Path('.run')
def durable(path, value):
    with path.open('w') as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())
with (root / 'forge.log').open('a') as log:
    log.write(json.dumps(args) + '\n')
    log.flush()
    os.fsync(log.fileno())
state_path = root / 'pr.json'
state = json.loads(state_path.read_text()) if state_path.exists() else []
if method == 'GET':
    if (root / 'read-fails').exists():
        sys.exit('fixture remote read unavailable')
    if '/issues' in endpoint:
        print('[]')
    elif endpoint.split('?')[0].endswith('/7'):
        if len(state) != 1:
            sys.exit('fixture PR identity unavailable')
        print(json.dumps(state[0]))
    elif '/pulls?' in endpoint:
        print(json.dumps(state))
    else:
        sys.exit('unexpected fixture read: ' + endpoint)
elif endpoint.endswith('/merge'):
    if (root / 'merged-once').exists():
        sys.exit('second merge forbidden')
    assert len(state) == 1 and state[0]['state'] == 'open'
    state[0]['state'] = 'closed'
    state[0]['merged'] = True
    state[0]['merged_at'] = '2026-09-19T12:00:00Z'
    durable(state_path, state)
    durable(root / 'merged-once', True)
    print('{"merged":true}')
elif method == 'POST' and endpoint.endswith('/pulls'):
    if (root / 'created-once').exists():
        sys.exit('second create forbidden')
    assert state == []
    state = [{"number":7,"state":"open","merged":False,"merged_at":None,
              "head":{"sha":"@HEAD@","ref":"fixture/execution","repo":{"full_name":"fixture/repo"}},
              "base":{"sha":"@BASE@","ref":"main","repo":{"full_name":"fixture/repo"}}}]
    durable(state_path, state)
    durable(root / 'created-once', True)
    print(json.dumps(state[0]))
else:
    sys.exit('unexpected fixture mutation: ' + endpoint)
"#.replace("@HEAD@", &self.head).replace("@BASE@", &self.base);
        for program in ["gh", "glab", "tea"] {
            fs::write(self.project.path().join(".run/bin").join(program), &script).unwrap();
        }
    }

    pub fn pr_state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.project.path().join(".run/pr.json")).unwrap()).unwrap()
    }

    pub fn set_pr_state(&self, state: &Value) {
        fs::write(self.project.path().join(".run/pr.json"), serde_json::to_vec(state).unwrap()).unwrap();
    }

    pub fn mutations(&self) -> Vec<Value> {
        self.invocations().into_iter().filter(|args| !args.as_array().unwrap().contains(&json!("GET"))).collect()
    }

    pub fn trace(&self) -> Vec<Value> {
        fs::read_to_string(self.project.path().join(".run/resume.trace")).unwrap_or_default()
            .lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}

pub struct UndoFixture {
    pub fixture: Fixture,
    pub hashes: Vec<String>,
    pub decoy: String,
}

impl UndoFixture {
    pub fn new(native: bool) -> Self {
        let fixture = undo_base();
        let project = fixture.project();
        git(project, &["config", "commit.gpgsign", "false"]);
        fs::create_dir_all(project.join(".planning/phases/13")).unwrap();
        fs::create_dir_all(project.join(".run")).unwrap();
        fs::write(project.join(".git/info/exclude"), ".run/\n").unwrap();
        fs::write(project.join(".planning/ROADMAP.md"), "## Phases\n- [ ] **Phase 13: Undo fixture**\n").unwrap();
        for path in ["src/p13.txt", "src/second.txt", "src/third.txt", "docs.txt"] {
            fs::write(project.join(path), "pending\n").unwrap();
        }
        fs::write(project.join("tests/p13.py"), "import pathlib, unittest\nclass Check(unittest.TestCase):\n    def test_file(self):\n        self.assertEqual(pathlib.Path('src/p13.txt').read_text(), 'ready\\n')\nif __name__ == '__main__':\n    unittest.main()\n").unwrap();
        git(project, &["add", "."]);
        git(project, &["commit", "-m", "Fixture undo baseline"]);
        let hashes = if native { complete_undo_phase(project) } else {
            fs::write(project.join(".planning/phases/13/PLAN-1.md"), "# Legacy plan\n").unwrap();
            let mut hashes = Vec::new();
            for (index, path) in ["src/p13.txt", "src/second.txt", "src/third.txt", "docs.txt"].iter().enumerate() {
                fs::write(project.join(path), "ready\n").unwrap();
                git(project, &["add", path]);
                git(project, &["commit", "-m", &format!("feat(13): deliver legacy {index}")]);
                hashes.push(git_value(project, &["rev-parse", "HEAD"]));
            }
            hashes
        };
        fs::write(project.join("decoy.txt"), "keep this unrelated commit\n").unwrap();
        git(project, &["add", "decoy.txt"]);
        git(project, &["commit", "-m", "feat(13): misleading phase commit"]);
        let decoy = git_value(project, &["rev-parse", "HEAD"]);
        let manifest = if native { vec![decoy.clone()] } else { hashes.clone() };
        fs::write(project.join(".planning/phases/13/SUMMARY.md"),
            format!("# Summary\n\n## Commits\n\n{}\n", manifest.iter().map(|h| format!("- `{h}`\n")).collect::<String>())).unwrap();
        fs::write(project.join(".planning/phases/13/UAT.md"), "### 1. Fixture\nstatus: pass\n").unwrap();
        git(project, &["add", ".planning"]);
        git(project, &["commit", "-m", "Fixture completed phase documents"]);
        Self { fixture, hashes, decoy }
    }
    pub fn project(&self) -> &Path { self.fixture.project() }
    pub fn client(&self) -> Client {
        Client::open_with_env(self.project(), Path::new(env!("CARGO_BIN_EXE_cadence")),
            &[("GIT_TRACE2_EVENT", self.project().join(".run/undo.trace").as_os_str())])
    }
    pub fn reverts(&self) -> Vec<String> {
        fs::read_to_string(self.project().join(".run/undo.trace")).unwrap_or_default().lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["event"] == "start")
            .filter_map(|event| {
                let argv = event["argv"].as_array()?;
                let pos = argv.iter().position(|arg| arg == "revert")?;
                argv.get(pos + 2)?.as_str().map(str::to_owned)
            }).collect()
    }
}

fn undo_task(client: &mut Client, id: &str) -> Value {
    let history = client.call("cadence_query", json!({"operation":"execution-history","phase":13}));
    history["tasks"].as_array().unwrap().iter().find(|t| t["task"]["task"] == id).unwrap().clone()
}

fn complete_undo_phase(project: &Path) -> Vec<String> {
    let phase = 13;
    let mut hashes = Vec::new();
    let mut client = Client::open(project);
    let context = phase13::approve(json!({"operation":"context-submit","submission":{
        "phase":phase,"title":"Native milestone fixture","scope":"Complete one real artifact.",
        "durable_decisions":[],"decisions":[],"assumptions":[],"truths":[{
            "id":"T1","trigger":"the owner opens the artifact","observer":"the owner","verb":"sees",
            "outcome":"ready","kind":"property","observable":true,"fixed_oracle":true}]}}));
    ok(&mut client, context);
    let allocation = client.call("cadence_query", json!({"operation":"plan-read","phase":phase,"count":1}));
    let command = format!("python3 -B tests/p{phase}.py");
    let file = format!("src/p{phase}.txt");
    ok(&mut client, phase13::approve(json!({"operation":"plan-submit","submission":{
        "phase":phase,"occurrence":allocation["occurrence"],"request_id":format!("publish-{phase}"),
        "inventory_basis":allocation["inventory"]["basis"],"plans":[{"target":allocation["targets"][0],"content":{
            "phase":phase,"plan":1,"requirements":["T1"],"files":[file,"src/second.txt","src/third.txt","docs.txt"],"directories":[],
            "goal":"Deliver the ready artifact.","context":"Native completion fixture.","notes":"One artifact.",
            "tasks":[{"id":"task-ready","title":"Deliver ready","files":[file],"action":"Write the ready artifact.","verify":[command]},
                {"id":"task-second","title":"Second implementation","files":["src/second.txt"],"action":"Deliver second.","verify":[command]},
                {"id":"task-third","title":"Third implementation","files":["src/third.txt"],"action":"Deliver third.","verify":[command]},
                {"id":"task-docs","title":"Documentation","files":["docs.txt"],"action":"Document the result.","verify":[command]}],
            "suite":command,"evidence_map":{"mode":"attached","items":[{"kind":"artifact","id":"artifact/ready",
                "spec":{"locators":[file],"substance":"The ready artifact contains ready."},"reason":"Observe the ready artifact.",
                "associations":[{"truth_id":"T1","truth_version":1,"reason":"Observe ready."}]},
                {"kind":"check","id":"check/ready","reason":"Read the real artifact.","spec":{"command":command,
                    "expected":{"kind":"literal","value":"ready followed by a newline"},"test":{"file":format!("tests/p{phase}.py"),"function":"Check.test_file"},
                    "setup":"A pending artifact.","call":"Read the artifact.","boundary":"real filesystem","fakes":[]},
                    "associations":[{"truth_id":"T1","truth_version":1,"reason":"Read ready."}]}]}}}]}})));
    git(project, &["add", ".planning"]);
    git(project, &["commit", "-m", &format!("Fixture phase {phase} publication")]);
    let read = client.call("cadence_query", json!({"operation":"plan-read","phase":phase}));
    let publication = &read["native"]["publications"]["1"];
    let map = client.call("cadence_query", json!({"operation":"evidence-read","phase":phase}));
    let item = map["items"].as_array().unwrap().iter().find(|i| i["kind"] == "check").unwrap();
    let check = json!({"id":item["id"],"item_revision":item["item_revision"]});
    ok(&mut client, json!({"operation":"execution-admit","request":{"request_id":format!("admit-{phase}"),"expected_set_version":0,
        "contract":{"phase":phase,"occurrence":read["occurrence"],"plans":[{"plan":1,"publication_request":publication["publication_request"],
            "content_revision":publication["revision"],"map_revision":publication["map_revision"]}],
            "allocation":[{"plan":1,"task":"task-ready","checks":[check]},
                {"plan":1,"task":"task-second","checks":[]},{"plan":1,"task":"task-third","checks":[]},{"plan":1,"task":"task-docs","checks":[]}]}}}));
    ok(&mut client, json!({"operation":"execution-authorize","phase":phase,"request_id":format!("authorize-{phase}"),
        "owner":"Fixture Owner","at":"2026-09-19T12:00:00Z","response":"Proceed with the fixture"}));
    let dispatch = client.call("cadence_query", json!({"operation":"execute-next","phase":phase}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-task-start","request":{"request_id":format!("start-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":0,"predecessor":null,"checks":[check]}}));
    git(project, &["add", &format!("tests/p{phase}.py")]);
    git(project, &["commit", "--allow-empty", "-m", &format!("test({phase}): expect ready task-ready")]);
    let red_commit = git_value(project, &["rev-parse", "HEAD"]);
    let task = state(&mut client, phase, "tasks");
    let red_run = format!("red-{phase}");
    ok(&mut client, json!({"operation":"execution-run","request":{"request_id":red_run,"task":task["task"],"attempt":"attempt-ready",
        "expected_version":task["state"]["version"],"command":command,"check":check,"stage":"red"}}));
    let red = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &red_run);
    assert_eq!(red["request"]["event"]["disposition"]["code"], 1, "{red}");
    fs::write(project.join(&file), "ready\n").unwrap();
    git(project, &["add", &file]);
    git_value(project, &["commit", "-S", "-m", &format!("feat({phase}): deliver ready task-ready")]);
    let completion = git_value(project, &["rev-parse", "HEAD"]);
    let task = state(&mut client, phase, "tasks");
    let run = format!("verify-{phase}");
    ok(&mut client, json!({"operation":"execution-run","request":{"request_id":run,"task":task["task"],"attempt":"attempt-ready",
        "expected_version":task["state"]["version"],"command":command,"check":check,"stage":"green"}}));
    let result = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &run);
    assert_eq!(result["request"]["event"]["disposition"]["code"], 0, "{result}");
    let launch = client.call("cadence_query", json!({"operation":"execution-history","phase":phase,"run":red_run}));
    let inspection = json!({"check":check,"test_digest":launch["launch"]["request"]["event"]["material"]["test_digest"],
        "evidence":[red_run,run],"no_subject_stub":true});
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-owner-attest","request":{"request_id":format!("inspect-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":task["state"]["version"],"statement":{"submission":inspection,
            "approval":{"approved":true,"owner":"Fixture Owner","at":"2026-09-19T12:00:00Z","submission":inspection}}}}));
    let task = state(&mut client, phase, "tasks");
    ok(&mut client, json!({"operation":"execution-task-close","request":{"request_id":format!("close-{phase}"),"task":task["task"],
        "attempt":"attempt-ready","expected_version":task["state"]["version"],"completion":completion,
        "checks":[{"check":check,"red_commit":red_commit,"green_commit":completion,"red_run":red_run,"green_run":run}],"verification":[run]}}));

    hashes.push(completion);
    for (id, path) in [("task-second", "src/second.txt"), ("task-third", "src/third.txt"), ("task-docs", "docs.txt")] {
        let task = undo_task(&mut client, id);
        ok(&mut client, json!({"operation":"execution-task-start","request":{"request_id":format!("undo-fixture-start-{id}"),
            "task":task["task"],"attempt":id,"expected_version":0,"predecessor":null,"checks":[]}}));
        fs::write(project.join(path), "ready\n").unwrap();
        git(project, &["add", path]);
        git_value(project, &["commit", "-S", "-m", &format!("feat(13): deliver fixture {id}")]);
        let completion = git_value(project, &["rev-parse", "HEAD"]);
        let task = undo_task(&mut client, id);
        let run = format!("undo-fixture-verify-{id}");
        ok(&mut client, json!({"operation":"execution-run","request":{"request_id":run,"task":task["task"],"attempt":id,
            "expected_version":task["state"]["version"],"command":command,"check":null,"stage":"verify"}}));
        let result = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &run);
        assert_eq!(result["request"]["event"]["disposition"]["code"], 0, "{result}");
        let task = undo_task(&mut client, id);
        ok(&mut client, json!({"operation":"execution-task-close","request":{"request_id":format!("undo-fixture-close-{id}"),
            "task":task["task"],"attempt":id,"expected_version":task["state"]["version"],
            "completion":completion,"checks":[],"verification":[run]}}));
        hashes.push(completion);
    }
    let plan = state(&mut client, phase, "plans");
    let suite = format!("suite-{phase}");
    ok(&mut client, json!({"operation":"execution-suite","request":{"request_id":suite,"plan":plan["plan"],"expected_version":plan["state"]["version"]}}));
    let result = phase13::native_result_with(|v| client.call("cadence_query", v), phase, &suite);
    assert_eq!(result["request"]["event"]["disposition"]["code"], 0, "{result}");
    ok(&mut client, json!({"operation":"risk-check","request_id":format!("execution-risk-{phase}"),
        "scope":{"phase":phase,"occurrence":format!("phase-{phase}-execution"),"worker":"1"},
        "source":{"kind":"execution","plan":1,"dispatch_id":dispatch["dispatch_id"]},"surfaces":null}));
    let plan = state(&mut client, phase, "plans");
    ok(&mut client, json!({"operation":"execution-plan-complete","request":{"request_id":format!("complete-{phase}"),"plan":plan["plan"],"expected_version":plan["state"]["version"]}}));
    let dispatch = client.call("cadence_query", json!({"operation":"verify-next","phase":phase,"request_id":format!("verification-{phase}")}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let attempt = phase13::attempt_with(&mut client, &dispatch);
    let mut items = Vec::new();
    for item in attempt["map"]["items"].as_array().unwrap() {
        let mut runs = Vec::new();
        if item["kind"] == "check" {
            let run = format!("independent-{phase}");
            ok(&mut client, json!({"operation":"verification-run","request":{"request_id":run,"attempt":attempt["id"],"basis":attempt["basis"],
                "item":{"id":item["id"],"item_revision":item["item_revision"]}}}));
            let result = phase13::independent_result(&mut client, phase, &run);
            assert_eq!(result["disposition"]["code"], 0, "{result}");
            runs.push(run);
        }
        items.push(json!({"id":item["id"],"item_revision":item["item_revision"],"verdict":"accepted","observed":"The real artifact reads ready followed by a newline.","runs":runs}));
    }
    ok(&mut client, json!({"operation":"verification-submit","patch":{"request_id":format!("patch-{phase}"),"attempt":attempt["id"],
        "items":items}}));
    let read = client.call("cadence_query", json!({"operation":"verification-read","phase":phase}));
    ok(&mut client, json!({"operation":"verification-complete","request_id":format!("verified-{phase}"),"attempt":attempt["id"],
        "basis":read["current"]["observed"],"projections":{"roadmap":phase13::digest_of(&project.join(".planning/ROADMAP.md")),"requirements":null}}));
    client.finish();
    hashes
}

fn undo_base() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    for path in [".planning/phases", "src", "tests", ".fixture-gnupg"] {
        fs::create_dir_all(project.join(path)).unwrap();
    }
    fs::set_permissions(project.join(".fixture-gnupg"), fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new("gpg").env("GNUPGHOME", project.join(".fixture-gnupg"))
        .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", "", "--quick-generate-key",
            "Cadence-Phase13 <phase13@example.invalid>", "ed25519", "sign", "0"])
        .stdin(Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    git(project, &["init", "--initial-branch=fixture/undo"]);
    git_value(project, &["config", "user.signingkey", "phase13@example.invalid"]);
    fs::write(project.join(".gitignore"), ".planning/*\n!.planning/*.md\n!.planning/config.json\n!.planning/phases/\n.planning/phases/*/*\n!.planning/phases/*/*.md\n.fixture-gnupg/\n__pycache__/\n.run/\n").unwrap();
    fs::write(project.join(".planning/config.json"), serde_json::to_vec(&json!({"review":{"triggers":{
        "plan":{"gate":"deferred"},"risk_surface":{"surfaces":cadence::rail::risk::CATEGORIES}}}})).unwrap()).unwrap();
    Fixture { temp }
}

