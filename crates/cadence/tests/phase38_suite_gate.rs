//! Phase 38 acceptance checks cross two real binaries, stdio, Git and durable
//! native execution state. No store record or renderer is seeded by a check.
#[path = "support/phase13.rs"]
#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};
use support::Client;

const PHASE: u32 = 38;
const COMMAND: &str = "python3 -B tests/retained_prompt.py";
const OWNER: &str = "Fixture Owner";
const AT: &str = "2026-09-15T14:00:00Z";

fn git(project: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Cadence-Phase38",
            "-c",
            "user.email=phase38@example.invalid",
        ])
        .args(args)
        .current_dir(project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GNUPGHOME", project.join(".fixture-gnupg"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn fixture() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    fs::create_dir_all(project.join(".planning/phases/38")).unwrap();
    fs::create_dir(project.join(".fixture-gnupg")).unwrap();
    fs::set_permissions(
        project.join(".fixture-gnupg"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let output = Command::new("gpg")
        .env("GNUPGHOME", project.join(".fixture-gnupg"))
        .args([
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-generate-key",
            "Cadence-Phase38 <phase38@example.invalid>",
            "ed25519",
            "sign",
            "0",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    fs::write(
        project.join(".planning/ROADMAP.md"),
        "## Phases\n- [ ] **Phase 38: Retained prompts**\n- [ ] **Phase 39: Next phase**\n",
    )
    .unwrap();
    fs::write(
        project.join(".planning/config.json"),
        serde_json::to_vec(&json!({
            "review":{"triggers":{"risk_surface":{"surfaces":cadence::rail::risk::CATEGORIES}}}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::create_dir(project.join("src")).unwrap();
    fs::write(project.join(".gitignore"), ".planning/\n.fixture-gnupg/\n__pycache__/\n").unwrap();
    fs::write(project.join("src/value.py"), "def answer():\n    return 1\n").unwrap();
    git(project, &["init", "--initial-branch=fixture/retained-prompt"]);
    git(project, &["config", "user.signingkey", "phase38@example.invalid"]);
    git(project, &["add", ".gitignore", "src/value.py"]);
    git(project, &["commit", "-S", "-m", "Fixture phase 38 subject"]);
    temp
}

fn call(project: &Path, tool: &str, request: Value) -> Value {
    let mut client = Client::open(project);
    let answer = client.call(tool, request);
    client.finish();
    answer
}

fn check() -> Value {
    json!({"kind":"check","id":"P38-T4-C",
        "reason":"Re-rendering would change or refuse the retained owner-visible prompt.",
        "spec":{"command":COMMAND,"expected":{"kind":"property","value":"the retained prompt survives a renderer change"},
            "test":{"file":"tests/retained_prompt.py","function":"RetainedPrompt.test_answer"},
            "setup":"Admit under one compiled renderer and read under another.",
            "call":"Request execute-next from the changed binary.",
            "boundary":"Two real Cadence stdio binaries and one durable project.","fakes":[]},
        "associations":[{"truth_id":"T4","truth_version":1,
            "reason":"This is the owner-visible retained prompt outcome."}]})
}

fn map() -> Value {
    json!({"mode":"attached","items":[check(),{
        "kind":"artifact","id":"P38-A-PROMPT",
        "reason":"The durable dispatch must carry the admitted prompt.",
        "spec":{"locators":["src/value.py"],"substance":"The fixture subject exists."},
        "associations":[{"truth_id":"T4","truth_version":1,
            "reason":"The dispatch retains the bytes used for this work."}]
    }]})
}

fn body(map: &Value) -> String {
    format!(
        "# Fixture plan\n\n## Evidence map\n\n```json\n{}\n```\n\n",
        support::section_json(map, 0)
    )
}

fn history(project: &Path) -> Value {
    let answer = call(
        project,
        "cadence_query",
        json!({"operation":"execution-history","phase":PHASE}),
    );
    assert_eq!(answer["status"], "ok", "{answer}");
    answer
}

fn task(project: &Path) -> Value {
    history(project)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["task"]["task"] == "retain-prompt")
        .unwrap()
        .clone()
}

fn run(project: &Path, id: &str, stage: &str, check: Value) -> Value {
    let current = task(project);
    let mut client = Client::open(project);
    let launched = client.call(
        "cadence_apply",
        json!({"operation":"execution-run","request":{"request_id":id,
            "task":current["task"],"attempt":"attempt-retain-prompt",
            "expected_version":current["state"]["version"],"command":COMMAND,
            "check":check,"stage":stage}}),
    );
    assert_eq!(launched["status"], "ok", "{launched}");
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let result = loop {
        let current = client.call(
            "cadence_query",
            json!({"operation":"execution-history","phase":PHASE}),
        );
        if let Some(record) = current["events"].as_array().unwrap().iter().find(|record| {
            record["request"]["event"]["kind"] == "result"
                && record["request"]["event"]["run_id"] == id
        }) {
            break record["request"]["event"].clone();
        }
        assert!(Instant::now() < deadline, "missing result {id}: {current}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    client.finish();
    result
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(source, target).unwrap();
        }
    }
}

fn changed_binary(temp: &Path) -> PathBuf {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = temp.join("changed-source");
    fs::create_dir_all(source.join("crates/cadence")).unwrap();
    fs::create_dir_all(source.join("cadence-core/references")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(repository.join(file), source.join(file)).unwrap();
    }
    fs::copy(
        repository.join("crates/cadence/Cargo.toml"),
        source.join("crates/cadence/Cargo.toml"),
    )
    .unwrap();
    copy_tree(
        &repository.join("crates/cadence/src"),
        &source.join("crates/cadence/src"),
    );
    copy_tree(
        &repository.join("crates/cadence/tests/fixtures/phase9"),
        &source.join("crates/cadence/tests/fixtures/phase9"),
    );
    fs::copy(
        repository.join("cadence-core/references/reviewer-brief.md"),
        source.join("cadence-core/references/reviewer-brief.md"),
    )
    .unwrap();
    let instructions = source.join("crates/cadence/src/execution/instructions.rs");
    let original = fs::read_to_string(&instructions).unwrap();
    let needle = "The dispatch's operational input is the binary's authority";
    assert_eq!(original.matches(needle).count(), 1);
    fs::write(
        &instructions,
        original.replace(
            needle,
            "The retained dispatch operational input is the binary's authority",
        ),
    )
    .unwrap();
    let target = temp.join("changed-target");
    let started = Instant::now();
    let output = Command::new("cargo")
        .args(["build", "--locked", "-p", "cadence", "--bin", "cadence"])
        .current_dir(&source)
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTC_WRAPPER", "")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!("PHASE38_SECOND_BINARY_BUILD_SECONDS={elapsed:.3}");
    assert!(
        output.status.success(),
        "changed binary build failed after {elapsed:.3}s:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    target.join("debug/cadence")
}

#[test]
fn phase38_retained_dispatch_prompt_survives_renderer_change() {
    let temp = fixture();
    let project = temp.path();
    let truth = json!({"id":"T4","trigger":"the binary renderer changes after dispatch admission",
        "observer":"the owner","verb":"gets","outcome":"the admitted dispatch prompt byte for byte",
        "kind":"property","observable":true,"fixed_oracle":true});
    let context = call(
        project,
        "cadence_apply",
        support::approve(json!({"operation":"context-submit","submission":{"phase":PHASE,
            "title":"Retained prompts","scope":"One durable dispatch.","durable_decisions":[],
            "decisions":[],"assumptions":[],"truths":[truth]}})),
    );
    assert_eq!(context["persisted"], true, "{context}");

    let evidence_map = map();
    let mut client = Client::open(project);
    let allocation = client.read("38", Some(1));
    assert_eq!(allocation["status"], "ok", "{allocation}");
    let target = allocation["targets"][0].clone();
    let submission = json!({"phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"publish-retained-prompt","inventory_basis":allocation["inventory"]["basis"],
        "plans":[{"target":target,"content":{"phase":PHASE,"plan":1,"requirements":["T4"],
            "files":["src/value.py","tests/retained_prompt.py"],"directories":[],
            "execution":{"schema":1,"suite":COMMAND,
                "tasks":[{"id":"retain-prompt","verify":[COMMAND]}]},
            "body":body(&evidence_map),"evidence_map":evidence_map}}]});
    let preview = client.call(
        "cadence_query",
        json!({"operation":"plan-read","phase_address":"38","submission":submission}),
    );
    assert_eq!(preview["status"], "ok", "{preview}");
    let published = client.call(
        "cadence_apply",
        support::approve(json!({"operation":"plan-submit","submission":preview["submission"]})),
    );
    assert_eq!(published["persisted"], true, "{published}");
    let plans = client.read("38", None);
    let evidence = client.call(
        "cadence_query",
        json!({"operation":"evidence-read","phase":PHASE}),
    );
    client.finish();
    let publication = &plans["native"]["publications"]["1"];
    let check_revision = evidence["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "P38-T4-C")
        .unwrap()["item_revision"]
        .clone();
    let assignment = json!({"plan":1,"task":"retain-prompt","checks":[{
        "id":"P38-T4-C","item_revision":check_revision}]});
    let contract = json!({"phase":PHASE,"occurrence":plans["occurrence"],"plans":[{
        "plan":1,"publication_request":publication["publication_request"],
        "content_revision":publication["revision"],"map_revision":publication["map_revision"]}],
        "allocation":[assignment]});
    let admitted = call(
        project,
        "cadence_apply",
        json!({"operation":"execution-admit","request":{"request_id":"admit-retained-prompt",
            "expected_set_version":0,"contract":contract}}),
    );
    assert_eq!(admitted["status"], "ok", "{admitted}");
    let authorized = call(
        project,
        "cadence_apply",
        json!({"operation":"execution-authorize","phase":PHASE,
            "request_id":"authorize-retained-prompt","owner":OWNER,"at":AT,
            "response":"Run the retained prompt check."}),
    );
    assert_eq!(authorized["status"], "ok", "{authorized}");
    let dispatch = call(
        project,
        "cadence_query",
        json!({"operation":"execute-next","phase":PHASE}),
    );
    assert_eq!(dispatch["outcome"], "dispatch", "{dispatch}");
    let retained_prompt = dispatch["prompt"].as_str().unwrap().to_owned();
    let admitted_digest = dispatch["dispatch"]["prompt_digest"].clone();
    let expected_digest = cadence::store::model::digest(retained_prompt.as_bytes());

    let current = task(project);
    let started = call(
        project,
        "cadence_apply",
        json!({"operation":"execution-task-start","request":{"request_id":"start-retained-prompt",
            "task":current["task"],"attempt":"attempt-retain-prompt",
            "expected_version":current["state"]["version"],"predecessor":null,
            "checks":[assignment["checks"][0].clone()]}}),
    );
    assert_eq!(started["status"], "ok", "{started}");
    fs::create_dir(project.join("tests")).unwrap();
    fs::write(project.join("tests/retained_prompt.py"),
        "import sys, unittest\nsys.path.insert(0, 'src')\nfrom value import answer\nunittest.runner.time.perf_counter = lambda: 0.0\nclass RetainedPrompt(unittest.TestCase):\n    def test_answer(self):\n        self.assertEqual(answer(), 2)\nif __name__ == '__main__':\n    unittest.main()\n").unwrap();
    git(project, &["add", "tests/retained_prompt.py"]);
    git(project, &["commit", "-S", "-m", "test(38): retain prompt red"]);
    let red_commit = git(project, &["rev-parse", "HEAD"]);
    let red = run(project, "retained-red", "red", assignment["checks"][0].clone());
    assert_eq!(red["disposition"], json!({"kind":"exited","code":1}), "{red}");
    fs::write(project.join("src/value.py"), "def answer():\n    return 2\n").unwrap();
    git(project, &["add", "src/value.py"]);
    git(project, &["commit", "-S", "-m", "feat: deliver retain-prompt"]);
    let green_commit = git(project, &["rev-parse", "HEAD"]);
    let green = run(project, "retained-green", "green", assignment["checks"][0].clone());
    assert_eq!(green["disposition"], json!({"kind":"exited","code":0}), "{green}");
    let verified = run(project, "retained-verify", "verify", Value::Null);
    assert_eq!(verified["disposition"], json!({"kind":"exited","code":0}), "{verified}");
    let events = history(project);
    let launch = events["events"].as_array().unwrap().iter().find(|record| {
        record["request"]["event"]["kind"] == "launch"
            && record["request"]["event"]["run_id"] == "retained-red"
    }).unwrap();
    let inspection = json!({"check":assignment["checks"][0],
        "test_digest":launch["request"]["event"]["material"]["test_digest"],
        "evidence":["retained-red","retained-green"],"no_subject_stub":true});
    let current = task(project);
    let attested = call(project, "cadence_apply", json!({"operation":"execution-owner-attest","request":{
        "request_id":"attest-retained-prompt","task":current["task"],"attempt":"attempt-retain-prompt",
        "expected_version":current["state"]["version"],"statement":{"submission":inspection,
        "supersedes":null,"approval":{"approved":true,"owner":OWNER,"at":AT,"submission":inspection}}}}));
    assert_eq!(attested["status"], "ok", "{attested}");
    let current = task(project);
    let closed = call(project, "cadence_apply", json!({"operation":"execution-task-close","request":{
        "request_id":"close-retained-prompt","task":current["task"],"attempt":"attempt-retain-prompt",
        "expected_version":current["state"]["version"],"completion":green_commit,
        "checks":[{"check":assignment["checks"][0],"red_commit":red_commit,
            "green_commit":green_commit,"red_run":"retained-red","green_run":"retained-green"}],
        "verification":["retained-verify"]}}));
    assert_eq!(closed["status"], "ok", "{closed}");
    let current = history(project);
    let plan = current["plans"].as_array().unwrap().iter().find(|entry| entry["plan"]["plan"] == 1).unwrap();
    let mut client = Client::open(project);
    let suite = client.call("cadence_apply", json!({"operation":"execution-suite","request":{
        "request_id":"suite-retained-prompt","plan":plan["plan"],
        "expected_version":plan["state"]["version"]}}));
    assert_eq!(suite["status"], "ok", "{suite}");
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let current = client.call("cadence_query", json!({"operation":"execution-history","phase":PHASE}));
        if let Some(result) = current["plan_events"].as_array().unwrap().iter().find(|record| {
            record["request"]["event"]["kind"] == "suite-result"
                && record["request"]["event"]["run_id"] == "suite-retained-prompt"
        }) {
            assert_eq!(result["request"]["event"]["disposition"], json!({"kind":"exited","code":0}));
            break;
        }
        assert!(Instant::now() < deadline, "missing green suite result: {current}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    client.finish();

    let changed = changed_binary(temp.path());
    let mut replacement = Client::open_with_program(project, &changed);
    let reopened = replacement.call(
        "cadence_query",
        json!({"operation":"execute-next","phase":PHASE}),
    );
    replacement.finish();
    assert_eq!(reopened["status"], "ok", "retained prompt was refused: {reopened}");
    assert_eq!(reopened["outcome"], "dispatch", "{reopened}");
    assert_eq!(reopened["prompt"].as_str().unwrap().as_bytes(), retained_prompt.as_bytes());
    assert_eq!(admitted_digest, expected_digest);
    assert_eq!(reopened["dispatch"]["prompt_digest"], expected_digest);
}

const RENDERED_COMMAND: &str = "python3 -B tests/rendered_skill.py";
const RENDERED_SKILL: &str = "skills/cad-executor-contract/SKILL.md";

fn rendered_check() -> Value {
    json!({"kind":"check","id":"P38-T5-C",
        "reason":"The real renderer, implicit lease and Write/Edit guard must agree on one owned file.",
        "spec":{"command":RENDERED_COMMAND,"expected":{"kind":"property","value":"the regenerated skill is accepted as implicit lease material"},
            "test":{"file":"tests/rendered_skill.py","function":"RenderedSkill.test_binary_output"},
            "setup":"Build the checked-in source and regenerate its executor skill through that binary.",
            "call":"Close the source and generated bytes, request the suite, and invoke the guard.",
            "boundary":"Compiled source through a real binary renderer, native close, suite and guard.","fakes":[]},
        "associations":[{"truth_id":"T5","truth_version":1,
            "reason":"This is the owner-visible regenerated-file outcome."}]})
}

fn rendered_map() -> Value {
    json!({"mode":"attached","items":[rendered_check(),{
        "kind":"artifact","id":"P38-A-RENDERED-SKILL",
        "reason":"The binary source and its rendered output form one owned change.",
        "spec":{"locators":["crates/cadence/src/execution/instructions.rs", RENDERED_SKILL],
            "substance":"The fixture changes compiled executor text and regenerates its skill."},
        "associations":[{"truth_id":"T5","truth_version":1,
            "reason":"The artifact is the source-to-render output exercised by the check."}]
    }]})
}

fn rendered_task(project: &Path) -> Value {
    history(project)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["task"]["task"] == "regenerate-skill")
        .unwrap()
        .clone()
}

fn rendered_run(project: &Path, id: &str, stage: &str, check: Value) -> Value {
    let current = rendered_task(project);
    let mut client = Client::open(project);
    let launched = client.call(
        "cadence_apply",
        json!({"operation":"execution-run","request":{"request_id":id,
            "task":current["task"],"attempt":"attempt-regenerate-skill",
            "expected_version":current["state"]["version"],"command":RENDERED_COMMAND,
            "check":check,"stage":stage}}),
    );
    assert_eq!(launched["status"], "ok", "{launched}");
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let result = loop {
        let current = client.call(
            "cadence_query",
            json!({"operation":"execution-history","phase":PHASE}),
        );
        if let Some(record) = current["events"].as_array().unwrap().iter().find(|record| {
            record["request"]["event"]["kind"] == "result"
                && record["request"]["event"]["run_id"] == id
        }) {
            break record["request"]["event"].clone();
        }
        assert!(Instant::now() < deadline, "missing result {id}: {current}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    client.finish();
    result
}

fn prepare_rendered_source(project: &Path) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::create_dir_all(project.join("crates/cadence")).unwrap();
    fs::create_dir_all(project.join("cadence-core/references")).unwrap();
    fs::create_dir_all(project.join("skills/cad-executor-contract")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(repository.join(file), project.join(file)).unwrap();
    }
    fs::copy(
        repository.join("crates/cadence/Cargo.toml"),
        project.join("crates/cadence/Cargo.toml"),
    )
    .unwrap();
    copy_tree(
        &repository.join("crates/cadence/src"),
        &project.join("crates/cadence/src"),
    );
    copy_tree(
        &repository.join("crates/cadence/tests/fixtures/phase9"),
        &project.join("crates/cadence/tests/fixtures/phase9"),
    );
    fs::copy(
        repository.join("cadence-core/references/reviewer-brief.md"),
        project.join("cadence-core/references/reviewer-brief.md"),
    )
    .unwrap();
    fs::copy(repository.join(RENDERED_SKILL), project.join(RENDERED_SKILL)).unwrap();
    fs::write(
        project.join(".gitignore"),
        ".planning/\n.fixture-gnupg/\n__pycache__/\ntarget/\n",
    )
    .unwrap();
    git(project, &["add", ".gitignore", "Cargo.toml", "Cargo.lock",
        "crates/cadence", "cadence-core/references/reviewer-brief.md", RENDERED_SKILL]);
    git(project, &["commit", "-S", "-m", "Fixture binary-rendered source"]);
}

fn build_rendered_binary(project: &Path) -> PathBuf {
    let started = Instant::now();
    let output = Command::new("cargo")
        .args(["build", "--locked", "-p", "cadence", "--bin", "cadence"])
        .current_dir(project)
        .env("CARGO_TARGET_DIR", project.join("target"))
        .env("RUSTC_WRAPPER", "")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!("PHASE38_REGENERATED_BINARY_BUILD_SECONDS={elapsed:.3}");
    assert!(output.status.success(), "fixture binary build failed after {elapsed:.3}s:\n{}",
        String::from_utf8_lossy(&output.stderr));
    project.join("target/debug/cadence")
}

fn guard_rendered_skill(project: &Path, binary: &Path) -> Value {
    let mut child = Command::new(binary)
        .arg("guard")
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(
        &serde_json::to_vec(&json!({
            "session_id":"phase38-rendered-guard",
            "hook_event_name":"PreToolUse",
            "tool_name":"Edit",
            "cwd":project,
            "tool_input":{"file_path":RENDERED_SKILL,"old_string":"old","new_string":"new"}
        })).unwrap(),
    ).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    if output.stdout.is_empty() { Value::Null } else { serde_json::from_slice(&output.stdout).unwrap() }
}

#[test]
fn phase38_regenerated_skill_is_implicit_lease_material() {
    let temp = fixture();
    let project = temp.path();
    prepare_rendered_source(project);
    let truth = json!({"id":"T5","trigger":"compiled executor text changes",
        "observer":"the owner","verb":"gets","outcome":"the binary-regenerated skill accepted in the task lease",
        "kind":"property","observable":true,"fixed_oracle":true});
    let context = call(project, "cadence_apply", support::approve(json!({"operation":"context-submit",
        "submission":{"phase":PHASE,"title":"Rendered lease material","scope":"One rendered skill.",
        "durable_decisions":[],"decisions":[],"assumptions":[],"truths":[truth]}})));
    assert_eq!(context["persisted"], true, "{context}");

    let evidence_map = rendered_map();
    let mut client = Client::open(project);
    let allocation = client.read("38", Some(1));
    assert_eq!(allocation["status"], "ok", "{allocation}");
    let submission = json!({"phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"publish-regenerated-skill","inventory_basis":allocation["inventory"]["basis"],
        "plans":[{"target":allocation["targets"][0],"content":{"phase":PHASE,"plan":1,
            "requirements":["T5"],
            "files":["crates/cadence/src/execution/instructions.rs","tests/rendered_skill.py"],
            "directories":[],"execution":{"schema":1,"suite":RENDERED_COMMAND,
                "tasks":[{"id":"regenerate-skill","verify":[RENDERED_COMMAND]}]},
            "body":body(&evidence_map),"evidence_map":evidence_map}}]});
    let preview = client.call("cadence_query",
        json!({"operation":"plan-read","phase_address":"38","submission":submission}));
    assert_eq!(preview["status"], "ok", "{preview}");
    let published = client.call("cadence_apply",
        support::approve(json!({"operation":"plan-submit","submission":preview["submission"]})));
    assert_eq!(published["persisted"], true, "{published}");
    let plans = client.read("38", None);
    let evidence = client.call("cadence_query", json!({"operation":"evidence-read","phase":PHASE}));
    client.finish();
    let publication = &plans["native"]["publications"]["1"];
    let check_revision = evidence["items"].as_array().unwrap().iter()
        .find(|item| item["id"] == "P38-T5-C").unwrap()["item_revision"].clone();
    let assignment = json!({"plan":1,"task":"regenerate-skill","checks":[{
        "id":"P38-T5-C","item_revision":check_revision}]});
    let contract = json!({"phase":PHASE,"occurrence":plans["occurrence"],"plans":[{
        "plan":1,"publication_request":publication["publication_request"],
        "content_revision":publication["revision"],"map_revision":publication["map_revision"]}],
        "allocation":[assignment]});
    let admitted = call(project, "cadence_apply", json!({"operation":"execution-admit","request":{
        "request_id":"admit-regenerated-skill","expected_set_version":0,"contract":contract}}));
    assert_eq!(admitted["status"], "ok", "{admitted}");
    let authorized = call(project, "cadence_apply", json!({"operation":"execution-authorize",
        "phase":PHASE,"request_id":"authorize-regenerated-skill","owner":OWNER,"at":AT,
        "response":"Regenerate the binary-owned skill."}));
    assert_eq!(authorized["status"], "ok", "{authorized}");
    let dispatch = call(project, "cadence_query", json!({"operation":"execute-next","phase":PHASE}));
    assert_eq!(dispatch["outcome"], "dispatch", "{dispatch}");

    let current = rendered_task(project);
    let started = call(project, "cadence_apply", json!({"operation":"execution-task-start","request":{
        "request_id":"start-regenerated-skill","task":current["task"],
        "attempt":"attempt-regenerate-skill","expected_version":current["state"]["version"],
        "predecessor":null,"checks":[assignment["checks"][0]]}}));
    assert_eq!(started["status"], "ok", "{started}");
    fs::create_dir(project.join("tests")).unwrap();
    fs::write(project.join("tests/rendered_skill.py"), format!(
        "import pathlib, subprocess, unittest\nunittest.runner.time.perf_counter = lambda: 0.0\nclass RenderedSkill(unittest.TestCase):\n    def test_binary_output(self):\n        rendered = subprocess.run(['target/debug/cadence', 'executor-instructions'], check=True, capture_output=True, text=True).stdout\n        self.assertEqual(rendered, pathlib.Path('{RENDERED_SKILL}').read_text())\n        self.assertIn(\"The retained dispatch operational input is the binary's authority\", rendered)\nif __name__ == '__main__':\n    unittest.main()\n"
    )).unwrap();
    git(project, &["add", "tests/rendered_skill.py"]);
    git(project, &["commit", "-S", "-m", "test(38): prove regenerated skill red"]);
    let red_commit = git(project, &["rev-parse", "HEAD"]);
    let _baseline_binary = build_rendered_binary(project);
    let red = rendered_run(project, "rendered-red", "red", assignment["checks"][0].clone());
    assert_eq!(red["disposition"], json!({"kind":"exited","code":1}), "{red}");

    let instructions = project.join("crates/cadence/src/execution/instructions.rs");
    let original = fs::read_to_string(&instructions).unwrap();
    let needle = "The dispatch's operational input is the binary's authority";
    assert_eq!(original.matches(needle).count(), 1);
    fs::write(&instructions, original.replace(needle,
        "The retained dispatch operational input is the binary's authority")).unwrap();
    let changed_binary = build_rendered_binary(project);
    let rendered = Command::new(&changed_binary).arg("executor-instructions")
        .current_dir(project).stdin(Stdio::null()).output().unwrap();
    assert!(rendered.status.success(), "{}", String::from_utf8_lossy(&rendered.stderr));
    fs::write(project.join(RENDERED_SKILL), &rendered.stdout).unwrap();
    git(project, &["add", "crates/cadence/src/execution/instructions.rs", RENDERED_SKILL]);
    git(project, &["commit", "-S", "-m", "feat: deliver regenerate-skill"]);
    let green_commit = git(project, &["rev-parse", "HEAD"]);
    let green = rendered_run(project, "rendered-green", "green", assignment["checks"][0].clone());
    assert_eq!(green["disposition"], json!({"kind":"exited","code":0}), "{green}");
    let verified = rendered_run(project, "rendered-verify", "verify", Value::Null);
    assert_eq!(verified["disposition"], json!({"kind":"exited","code":0}), "{verified}");
    let events = history(project);
    let launch = events["events"].as_array().unwrap().iter().find(|record| {
        record["request"]["event"]["kind"] == "launch"
            && record["request"]["event"]["run_id"] == "rendered-red"
    }).unwrap();
    let inspection = json!({"check":assignment["checks"][0],
        "test_digest":launch["request"]["event"]["material"]["test_digest"],
        "evidence":["rendered-red","rendered-green"],"no_subject_stub":true});
    let current = rendered_task(project);
    let attested = call(project, "cadence_apply", json!({"operation":"execution-owner-attest","request":{
        "request_id":"attest-regenerated-skill","task":current["task"],
        "attempt":"attempt-regenerate-skill","expected_version":current["state"]["version"],
        "statement":{"submission":inspection,"supersedes":null,
            "approval":{"approved":true,"owner":OWNER,"at":AT,"submission":inspection}}}}));
    assert_eq!(attested["status"], "ok", "{attested}");
    let current = rendered_task(project);
    let closed = call(project, "cadence_apply", json!({"operation":"execution-task-close","request":{
        "request_id":"close-regenerated-skill","task":current["task"],
        "attempt":"attempt-regenerate-skill","expected_version":current["state"]["version"],
        "completion":green_commit,"checks":[{"check":assignment["checks"][0],
            "red_commit":red_commit,"green_commit":green_commit,
            "red_run":"rendered-red","green_run":"rendered-green"}],
        "verification":["rendered-verify"]}}));
    assert_eq!(closed["status"], "ok", "{closed}");

    let current = history(project);
    let plan = current["plans"].as_array().unwrap().iter()
        .find(|entry| entry["plan"]["plan"] == 1).unwrap();
    let mut client = Client::open(project);
    let suite = client.call("cadence_apply", json!({"operation":"execution-suite","request":{
        "request_id":"suite-regenerated-skill","plan":plan["plan"],
        "expected_version":plan["state"]["version"]}}));
    assert_eq!(suite["status"], "ok", "{suite}");
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let suite_result = loop {
        let current = client.call("cadence_query", json!({"operation":"execution-history","phase":PHASE}));
        if let Some(result) = current["plan_events"].as_array().unwrap().iter().find(|record| {
            record["request"]["event"]["kind"] == "suite-result"
                && record["request"]["event"]["run_id"] == "suite-regenerated-skill"
        }) {
            break result["request"]["event"].clone();
        }
        assert!(Instant::now() < deadline, "missing green suite result: {current}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    client.finish();
    let guarded = guard_rendered_skill(project, &changed_binary);

    let files = dispatch["dispatch"]["files"].as_array().unwrap();
    assert!(files.iter().any(|path| path == RENDERED_SKILL),
        "the retained dispatch lease must include {RENDERED_SKILL}: {files:?}");
    assert!(closed["receipt"]["request"]["event"]["source"]["out_of_lease"]
        .as_object().is_none_or(|paths| paths.is_empty()),
        "binary-rendered material must not be retained as a deviation: {closed}");
    assert_eq!(fs::read(project.join(RENDERED_SKILL)).unwrap(), rendered.stdout);
    assert_eq!(suite_result["disposition"], json!({"kind":"exited","code":0}));
    assert_eq!(suite_result["observation"]["class"], "results-observed",
        "suite must carry a recognized result: {suite_result}");
    assert_eq!(guarded["hookSpecificOutput"]["permissionDecision"], "deny",
        "direct Edit of {RENDERED_SKILL} must be denied: {guarded}");
}
