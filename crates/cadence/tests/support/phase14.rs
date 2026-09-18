use crate::phase13::{Client, Completed, admit_request, apply, contract, digest_of, git, git_value, query, tree, verify};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, time::{Duration, Instant}};

pub const OPEN: &str = "## Phases\n- [ ] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";
pub const TICKED: &str = "## Phases\n- [x] **Phase 5: Legacy**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n";

/// The adoption tree's roadmap: phases 1 to 3 ticked, 4 open.
pub const LEGACY: &str = "## Phases\n- [x] **Phase 1: First**\n- [x] **Phase 2: Second**\n- [x] **Phase 3: Third**\n- [ ] **Phase 4: Fourth**\n";
/// The same roadmap after the owner ticks phase 4 by hand.
pub const LEGACY_TICKED: &str = "## Phases\n- [x] **Phase 1: First**\n- [x] **Phase 2: Second**\n- [x] **Phase 3: Third**\n- [x] **Phase 4: Fourth**\n";

pub fn progress_fixture() -> Completed {
    let fixture = Completed::new();
    let root = fixture.project().join(".planning");
    fs::write(root.join("ROADMAP.md"), OPEN).unwrap();
    fs::create_dir_all(root.join("phases/5")).unwrap();
    fs::write(root.join("phases/5/PLAN-1.md"), "---\nphase: 5\nplan: 1\n---\n# Legacy plan\n\n## Tasks\n\n### Task 1: Deliver legacy work\n\n- **Files:** src/legacy.rs\n- **Action:** Deliver the legacy work.\n- **Verify:** cargo test\n").unwrap();
    fs::create_dir_all(root.join("deferred/5")).unwrap();
    fs::write(root.join("deferred/5/DEFERRED-diff-1.json"),
        r#"{"phase":"5","trigger":"diff","discriminator":"1","round":1,"findings":[{"description":"Review the legacy work"}]}"#).unwrap();
    fixture
}

/// A legacy tree no binary has touched: phase 1 derives complete (SUMMARY and
/// a passing UAT), phase 2 is ticked with one failing UAT item, phase 3 is
/// ticked with a plan only, phase 4 is open with a plan. One baseline commit.
pub fn legacy_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".planning");
    for phase in 1..=4 {
        fs::create_dir_all(root.join(format!("phases/{phase}"))).unwrap();
        fs::write(root.join(format!("phases/{phase}/PLAN-1.md")),
            format!("---\nphase: {phase}\nplan: 1\n---\n# Phase {phase} plan\n\n## Tasks\n\n### Task 1: Deliver phase {phase}\n\n- **Files:** src/phase{phase}.rs\n- **Action:** Deliver the phase.\n- **Verify:** cargo test\n")).unwrap();
    }
    fs::write(root.join("ROADMAP.md"), LEGACY).unwrap();
    fs::write(root.join("config.json"), "{}\n").unwrap();
    fs::write(root.join("phases/1/SUMMARY.md"), "# Phase 1 summary\n").unwrap();
    fs::write(root.join("phases/1/UAT.md"), "## Items\n\n### 1. Done\nstatus: pass\n").unwrap();
    fs::write(root.join("phases/2/SUMMARY.md"), "# Phase 2 summary\n").unwrap();
    fs::write(root.join("phases/2/UAT.md"), "## Items\n\n### 1. A\nstatus: pass\n\n### 2. B\nstatus: pass\n\n### 3. C\nstatus: fail\n").unwrap();
    git(temp.path(), &["init", "--initial-branch=fixture/adoption"]);
    fs::write(temp.path().join(".gitignore"), ".planning/\n").unwrap();
    git(temp.path(), &["add", ".gitignore"]);
    git(temp.path(), &["commit", "-m", "Adoption fixture baseline"]);
    temp
}

/// A second project verified and natively completed the way
/// phase13_verification completes one; the completion record's id comes back
/// with it.
pub fn natively_completed() -> (Completed, String) {
    let fixture = Completed::new();
    let project = fixture.project();
    let (accepted, _) = verify(project, "accepted", &[]);
    let basis = query(project, json!({"operation":"verification-read","phase":13}))["current"]["observed"].clone();
    assert!(basis.is_object(), "{basis}");
    let root = project.join(".planning");
    let requirements = root.join("REQUIREMENTS.md");
    let done = apply(project, json!({"operation":"verification-complete","request_id":"complete-13","attempt":accepted["id"],"basis":basis,
        "projections":{"roadmap":digest_of(&root.join("ROADMAP.md")),"requirements":requirements.exists().then(|| digest_of(&requirements))}}));
    assert_eq!(done["status"], "ok", "{done}");
    let id = done["receipt"]["record"]["id"].as_str().unwrap().to_owned();
    (fixture, id)
}

fn task_state(client: &mut Client, plan: u32) -> Value {
    let history = client.call("cadence_query", json!({"operation":"execution-history","phase":13}));
    history["tasks"].as_array().unwrap_or(&Vec::new()).iter()
        .find(|entry| entry["task"]["plan"] == plan)
        .unwrap_or_else(|| panic!("plan {plan} has no open task: {history}")).clone()
}

fn task_run(client: &mut Client, id: &str, check: &Value, stage: &str, command: &str) -> Value {
    let state = task_state(client, 1);
    let launch = client.call("cadence_apply", json!({"operation":"execution-run","request":{
        "request_id":id,"task":state["task"],"attempt":"attempt-1",
        "expected_version":state["state"]["version"],"check":check,"stage":stage,"command":command}}));
    assert_eq!(launch["status"], "ok", "{launch}");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let history = client.call("cadence_query", json!({"operation":"execution-history","phase":13,"run":id}));
        if history["result"]["request"]["event"]["kind"] == "result" {
            return history["result"]["request"]["event"].clone();
        }
        assert!(Instant::now() < deadline, "missing {id}: {history}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One published plan carried to the edge of its close: admitted, authorized,
/// dispatched, with a committed red run and a signed green run recorded. The
/// close request comes back unsent, so the caller decides what the worktree
/// looks like when it asks for it.
pub fn dispatched_plan() -> (Completed, Value) {
    let fixture = Completed::published(false, |_| {});
    let project = fixture.project();
    let contract = contract(project);
    let check = contract["allocation"].as_array().unwrap().iter()
        .find(|entry| entry["plan"] == 1).unwrap()["checks"][0].clone();
    assert_eq!(check["id"], "check/A", "{contract}");
    let mut client = Client::open(project);
    let admitted = client.call("cadence_apply", admit_request(contract, "admit-one", 0));
    assert_eq!(admitted["status"], "ok", "{admitted}");
    let authorized = client.call("cadence_apply", json!({"operation":"execution-authorize","phase":13,
        "request_id":"authorize-1","owner":"Fixture Owner","at":"2026-09-11T12:00:00Z",
        "response":"Proceed with native execution"}));
    assert_eq!(authorized["status"], "ok", "{authorized}");
    let dispatch = client.call("cadence_query", json!({"operation":"execute-next","phase":13}));
    assert_eq!(dispatch["status"], "ok", "{dispatch}");
    let task = task_state(&mut client, 1);
    let started = client.call("cadence_apply", json!({"operation":"execution-task-start","request":{
        "request_id":"start-1","task":task["task"],"attempt":"attempt-1","expected_version":0,
        "predecessor":null,"checks":[check.clone()]}}));
    assert_eq!(started["status"], "ok", "{started}");
    let command = "python3 -B tests/a.py";
    fs::write(project.join("tests/a.py"),
        "import sys, unittest\nsys.path.insert(0, 'src')\nfrom a import answer\n\
         unittest.runner.time.perf_counter = lambda: 0.0\nclass Check(unittest.TestCase):\n    \
         def test_answer(self):\n        self.assertEqual(answer(), 7)\n\
         if __name__ == '__main__':\n    unittest.main()\n").unwrap();
    git_value(project, &["add", "tests/a.py"]);
    git_value(project, &["commit", "-m", "test(13): red task-a"]);
    let red_commit = git_value(project, &["rev-parse", "HEAD"]);
    let red = task_run(&mut client, "red-1", &check, "red", command);
    assert_eq!(red["disposition"], json!({"kind":"exited","code":1}), "{red}");
    fs::write(project.join("src/a.py"), "def answer():\n    return 7\n").unwrap();
    git_value(project, &["add", "src/a.py"]);
    git_value(project, &["commit", "-S", "-m", "feat(13): green task-a"]);
    let green_commit = git_value(project, &["rev-parse", "HEAD"]);
    let green = task_run(&mut client, "green-1", &check, "green", command);
    assert_eq!(green["disposition"], json!({"kind":"exited","code":0}), "{green}");
    let task = task_state(&mut client, 1);
    let close = json!({"operation":"execution-task-close","request":{"request_id":"close-1",
        "task":task["task"],"attempt":"attempt-1","expected_version":task["state"]["version"],
        "completion":green_commit,"checks":[{"check":check,"red_commit":red_commit,
        "green_commit":green_commit,"red_run":"red-1","green_run":"green-1"}],
        "verification":["green-1"]}});
    client.finish();
    (fixture, close)
}

pub fn documents(project: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(project).into_iter().filter_map(|(path, bytes)| {
        // Store journals and snapshots may change; every authored document and
        // the deferred input must remain byte-exact across the progress reads.
        let document = path.extension().is_some_and(|ext| ext == "md")
            || path.starts_with(".planning/deferred")
            || path == Path::new(".planning/config.json");
        document.then_some(bytes).flatten().map(|bytes| (path, bytes))
    }).collect()
}
