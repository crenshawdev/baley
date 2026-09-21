#[allow(dead_code)]
#[path = "support/serve.rs"]
mod serve;
#[path = "support/support_records.rs"]
mod support_records;
#[path = "support/task_fixtures.rs"]
mod task_fixtures;

use serde_json::{Value, json};
use serve::Client;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use task_fixtures::{TaskFixture, commit_file, listing, task_close, task_open};

/// T1: a task done with no planning root reports done with the risk
/// disposition stated and the record called unrecorded, creates nothing,
/// and a blocking risk still blocks.
#[test]
fn task_treeless_done_reports_unrecorded() {
    let fixture = TaskFixture::treeless();
    let project = fixture.project();
    let planning = project.join(".planning");
    let mut client = fixture.client();

    // A protected starting branch gets the branch policy's own disposition.
    let before = listing(&project);
    let protected = task_open(&mut client, "task-open-protected", "harmless-note", "inline", "add a harmless note");
    assert_eq!(listing(&project), before);
    assert_eq!(protected["status"], "refused", "{protected}");
    assert_eq!(protected["code"], "protected-branch");
    assert_eq!(protected["rule"], "branch-policy");
    assert_eq!(protected["details"]["branch"], "main");
    assert_eq!(protected["details"]["permission"], "ask");
    assert_eq!(protected["details"]["policy"], json!({"protected":["main"],"on_protected":"ask"}));
    assert_eq!(protected["details"]["gate"]["options"].as_array().unwrap().iter()
        .map(|option| option["id"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "proceed", "abort"]);
    assert!(!planning.exists());

    // The caller switches to an authorized branch before opening.
    serve::git(&project, &["switch", "-c", "task/harmless-note"]);
    let start = serve::git_value(&project, &["rev-parse", "HEAD"]);
    let before = listing(&project);
    let opened = task_open(&mut client, "task-open-harmless", "harmless-note", "inline", "add a harmless note");
    assert_eq!(listing(&project), before);
    assert_eq!(opened["status"], "ok", "{opened}");
    assert_eq!(opened["outcome"], "open");
    assert_eq!(opened["ephemeral"], true);
    assert_eq!(opened["task"]["slug"], "harmless-note");
    assert_eq!(opened["task"]["mode"], "inline");
    assert_eq!(opened["task"]["branch"], "task/harmless-note");
    assert_eq!(opened["task"]["start"], start);
    assert_eq!(opened["root"], json!({"kind":"absent","path":planning.to_str().unwrap()}));
    assert_eq!(opened["recording"]["kind"], "unrecorded");
    assert_eq!(opened["policy"], json!({"protected":["main"],"on_protected":"ask","permission":"pass"}));
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("task-"), "{token}");
    assert!(!planning.exists());

    // The caller's own commit is outside the before/after comparisons.
    let commit = commit_file(&project, "docs/note.txt", b"a harmless note\n", "docs: add a harmless note");

    // A close naming a file that does not exist is classified as that file.
    let missing = fixture.absent("what-shipped.txt");
    let before = listing(&project);
    let named = task_close(&mut client, "task-close-missing", "harmless-note", &token,
        json!({"kind":"file","path":missing.to_str().unwrap()}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(named["status"], "refused", "{named}");
    assert_eq!(named["code"], "missing-file");
    assert_eq!(named["rule"], "task-boundary");
    assert_eq!(named["slot"], "request.report.path");
    assert_eq!(named["details"]["path"], missing.to_str().unwrap());
    assert_eq!(named["details"]["root"]["kind"], "absent");
    let reason = named["reason"].as_str().unwrap();
    assert!(reason.contains(missing.to_str().unwrap()), "{reason}");
    assert!(!reason.contains("planning root") && !reason.contains("unrecorded"), "{reason}");
    assert!(!planning.exists());

    // The harmless close: done, risk clear, record unrecorded, nothing created.
    let before = listing(&project);
    let closed = task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(closed["status"], "ok", "{closed}");
    assert_eq!(closed["outcome"], "done");
    assert_eq!(closed["ephemeral"], true);
    let record = &closed["record"];
    assert_eq!(record["schema"], "task-1");
    assert_eq!(record["slug"], "harmless-note");
    assert_eq!(record["mode"], "inline");
    assert_eq!(record["description"], "add a harmless note");
    assert_eq!(record["token"], token);
    assert_eq!(record["root"]["kind"], "absent");
    assert_eq!(record["branch"], "task/harmless-note");
    assert_eq!(record["start"], start);
    assert_eq!(record["head"], commit);
    assert_eq!(record["commits"], json!([{"id":commit,"subject":"docs: add a harmless note","files":["docs/note.txt"]}]));
    assert_eq!(record["files"], json!(["docs/note.txt"]));
    assert_eq!(record["risk"]["kind"], "clear");
    assert_eq!(record["risk"]["surfaces"], json!(["auth"]));
    assert_eq!(record["risk"]["gate"], "blocking");
    assert_eq!(record["risk"]["scan"], json!({"checked":true,"categories":["auth"],"matches":[],"inconclusive":false,"empty":false}));
    assert_eq!(record["report"], "added docs/note.txt");
    assert_eq!(record["recording"]["kind"], "unrecorded");
    assert_eq!(record["recording"]["reason"], format!("no planning root at {}: git is the record", planning.display()));
    assert!(!planning.exists());
    assert!(!project.join("tasks/harmless-note").exists());
    assert!(!planning.join("tasks/harmless-note").exists());
    assert_eq!(fixture.task_material(), Vec::<String>::new());

    // The same request replays its answer; a changed body under it is refused;
    // the closed token is no longer an open task.
    assert_eq!(task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"])), closed);
    let reused = task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"different"}), Some(&["auth"]));
    assert_eq!(reused["code"], "request-reused", "{reused}");
    let again = task_close(&mut client, "task-close-again", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"]));
    assert_eq!(again["code"], "unknown-task", "{again}");
    client.finish();
    assert_eq!(listing(&project), before);
    assert!(!planning.exists());

    // A second independent repository whose change matches the auth surface.
    let fixture = TaskFixture::treeless();
    let project = fixture.project();
    let planning = project.join(".planning");
    serve::git(&project, &["switch", "-c", "task/token-check"]);
    let start = serve::git_value(&project, &["rev-parse", "HEAD"]);
    let mut client = fixture.client();
    let opened = task_open(&mut client, "task-open-auth", "token-check", "inline", "verify the session token");
    assert_eq!(opened["status"], "ok", "{opened}");
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    let commit = commit_file(&project, "src/risk.txt", b"jwt.verify(token)\n", "feat: verify the session token");
    let before = listing(&project);
    let blocked = task_close(&mut client, "task-close-auth", "token-check", &token,
        json!({"kind":"text","text":"verified the token"}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(blocked["status"], "refused", "{blocked}");
    assert_eq!(blocked["code"], "risk-blocked");
    assert_eq!(blocked["rule"], "risk-gate");
    let reason = blocked["reason"].as_str().unwrap();
    assert!(reason.contains("risk surface auth matched") && reason.contains("a JWT sign/verify call")
        && reason.contains("the blocking gate refuses done"), "{reason}");
    let record = &blocked["details"]["record"];
    assert_eq!(record["start"], start);
    assert_eq!(record["head"], commit);
    assert_eq!(record["commits"], json!([{"id":commit,"subject":"feat: verify the session token","files":["src/risk.txt"]}]));
    assert_eq!(record["risk"]["kind"], "blocked");
    assert_eq!(record["risk"]["gate"], "blocking");
    assert_eq!(record["risk"]["matched"], json!(["auth"]));
    assert_eq!(record["risk"]["scan"]["matches"], json!([{"category":"auth","signal":"changed line: a JWT sign/verify call"}]));
    assert_eq!(record["recording"]["kind"], "unrecorded");
    assert_eq!(record["root"]["kind"], "absent");
    // The matched material was held per run under the child's TMPDIR and is gone.
    let expected_diff = "diff --git a/src/risk.txt b/src/risk.txt\nnew file mode 100644\nindex 0000000..bb23d37\n\
        --- /dev/null\n+++ b/src/risk.txt\n@@ -0,0 +1 @@\n+jwt.verify(token)\n";
    let transient = &blocked["details"]["transient"];
    let location = Path::new(transient["location"].as_str().unwrap());
    assert!(location.starts_with(fixture.scratch()), "{}", location.display());
    assert_eq!(location.file_name().unwrap().to_str().unwrap(), format!("cadence-task-{token}"));
    assert_eq!(transient["file"], location.join("risk-task-token-check.diff").to_str().unwrap());
    assert_eq!(transient["bytes"], expected_diff.len());
    assert_eq!(transient["digest"], cadence::store::model::digest(expected_diff.as_bytes()));
    assert!(!location.exists(), "transient material survived the answer at {}", location.display());
    assert_eq!(fixture.task_material(), Vec::<String>::new());
    assert!(!planning.exists());
    assert!(!project.join("tasks/token-check").exists());
    client.finish();
    assert_eq!(listing(&project), before);
    assert!(!planning.exists());
}

/// The 14 tracked historical task directories at HEAD, copied into a fixture
/// as authored bytes; nothing here creates them and every operation leaves
/// them identical.
fn seed_historical(project: &Path) -> BTreeMap<String, Vec<u8>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listed = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "ls-files", "-z", ".planning/tasks"])
        .output().unwrap();
    assert!(listed.status.success(), "{}", String::from_utf8_lossy(&listed.stderr));
    let mut files = BTreeMap::new();
    let mut slugs = std::collections::BTreeSet::new();
    for entry in listed.stdout.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        let rel = std::str::from_utf8(entry).unwrap();
        let parts: Vec<&str> = rel.split('/').collect();
        assert_eq!((parts[0], parts[1]), (".planning", "tasks"), "{rel}");
        slugs.insert(parts[2].to_owned());
        let bytes = fs::read(repo.join(rel)).unwrap();
        let target = project.join(rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &bytes).unwrap();
        files.insert(rel.to_owned(), bytes);
    }
    assert_eq!(slugs.len(), 14, "expected 14 historical task directories, found {}: {slugs:?}", slugs.len());
    files
}

fn assert_historical(project: &Path, expected: &BTreeMap<String, Vec<u8>>) {
    for (rel, bytes) in expected {
        assert_eq!(&fs::read(project.join(rel)).unwrap(), bytes, "historical {rel} changed");
    }
}

/// The RECORD.md grammar, hand-written from the observed record and outcomes.
fn expected_record_md(record: &Value, outcomes: Option<&[(&str, &str)]>) -> String {
    let mut out = format!("# Task: {}\n\n{}\n\nMode: {}\nBranch: {}\nRange: {}..{}\n",
        record["slug"].as_str().unwrap(), record["description"].as_str().unwrap(),
        record["mode"].as_str().unwrap(), record["branch"].as_str().unwrap(),
        record["start"].as_str().unwrap(), record["head"].as_str().unwrap());
    out.push_str("\n## Commits\n");
    for commit in record["commits"].as_array().unwrap() {
        out.push_str(&format!("\n- {} {}\n", commit["id"].as_str().unwrap(), commit["subject"].as_str().unwrap()));
        for file in commit["files"].as_array().unwrap() {
            out.push_str(&format!("  - {}\n", file.as_str().unwrap()));
        }
    }
    out.push_str("\n## Files\n\n");
    let files = record["files"].as_array().unwrap();
    for file in files { out.push_str(&format!("- {}\n", file.as_str().unwrap())); }
    if files.is_empty() { out.push_str("- none\n"); }
    let risk = &record["risk"];
    let risk_line = match risk["kind"].as_str().unwrap() {
        "clear" => format!("clear (surfaces: {}; gate: {})",
            risk["surfaces"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect::<Vec<_>>().join(", "),
            risk["gate"].as_str().unwrap()),
        "skipped" => "skipped: no commits landed".to_string(),
        other => panic!("unexpected risk kind {other}"),
    };
    out.push_str(&format!("\n## Risk\n\n{risk_line}\n"));
    if let Some(outcomes) = outcomes {
        out.push_str("\n## Outcomes\n");
        for (task, result) in outcomes { out.push_str(&format!("\n- {task}: {result}\n")); }
    }
    out.push_str(&format!("\n## Report\n\n{}\n", record["report"].as_str().unwrap()));
    out
}

fn expected_plan_md(slug: &str, description: &str, steps: &[(&str, &str, &str)]) -> String {
    let mut out = format!("# Task plan: {slug}\n\n{description}\n\n## Steps\n");
    for (index, (id, action, verify)) in steps.iter().enumerate() {
        out.push_str(&format!("\n{}. {id}\n   Action: {action}\n   Verify: {verify}\n", index + 1));
    }
    out
}

/// T2: a rooted task is done only after its store record and protected Markdown
/// projections are acknowledged; a write failure names its path and refuses done.
#[test]
fn task_rooted_done_writes_the_record_before_done() {
    let repo = support_records::fixture();
    let project = repo.path();
    let historical = seed_historical(project);

    let mut client = Client::open(project);

    // A slug that already names a historical directory is authored history.
    let hist_slug = historical.keys().next().unwrap().split('/').nth(2).unwrap().to_owned();
    let refused = task_open(&mut client, "task-open-historical", &hist_slug, "inline", "reuse a historical slug");
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["code"], "task-history", "{refused}");
    assert_historical(project, &historical);

    // ---- Inline: two commits, then done writes the record and RECORD.md.
    let start = serve::git_value(project, &["rev-parse", "HEAD"]);
    let opened = task_open(&mut client, "task-open-rooted-inline", "rooted-inline", "inline", "add alpha and beta");
    assert_eq!(opened["status"], "ok", "{opened}");
    assert_eq!(opened["outcome"], "open");
    assert_eq!(opened["ephemeral"], false);
    assert_eq!(opened["root"]["kind"], "present");
    assert_eq!(opened["task"]["start"], start);
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    assert!(!project.join(".planning/tasks/rooted-inline/RECORD.md").exists());
    assert!(!project.join(".planning/tasks/rooted-inline/PLAN.md").exists());

    let alpha = commit_file(project, "alpha.txt", b"alpha\n", "feat: add alpha");
    let beta = commit_file(project, "beta.txt", b"beta\n", "feat: add beta");
    let head = serve::git_value(project, &["rev-parse", "HEAD"]);
    let closed = task_close(&mut client, "task-close-rooted-inline", "rooted-inline", &token,
        json!({"kind":"text","text":"added alpha.txt and beta.txt"}), Some(&["auth"]));
    assert_eq!(closed["status"], "ok", "{closed}");
    assert_eq!(closed["outcome"], "done");
    assert_eq!(closed["ephemeral"], false);
    let record = &closed["record"];

    // Both actual commit hashes in order and their exact filenames, from git.
    let ids: Vec<String> = serve::git_value(project, &["log", "--reverse", "--format=%H", &format!("{start}..{head}")])
        .lines().map(str::to_owned).collect();
    assert_eq!(ids, vec![alpha.clone(), beta.clone()]);
    assert_eq!(record["commits"], json!([
        {"id":alpha,"subject":"feat: add alpha","files":["alpha.txt"]},
        {"id":beta,"subject":"feat: add beta","files":["beta.txt"]}]));
    assert_eq!(record["files"], json!(["alpha.txt", "beta.txt"]));
    assert_eq!(record["start"], start);
    assert_eq!(record["head"], head);
    assert_eq!(record["risk"]["kind"], "clear");
    assert_eq!(record["recording"]["kind"], "recorded");

    let record_path = project.join(".planning/tasks/rooted-inline/RECORD.md");
    let installed = fs::read(&record_path).unwrap();
    assert_eq!(String::from_utf8(installed.clone()).unwrap(), expected_record_md(record, None), "RECORD.md grammar");
    // renderer output equals the installed bytes: the binary's own revision is their digest.
    assert_eq!(record["recording"]["revision"], cadence::store::model::digest(&installed));
    assert!(record["recording"]["path"].as_str().unwrap().ends_with(".planning/tasks/rooted-inline/RECORD.md"));
    assert_historical(project, &historical);

    // The guard refuses a host Write and Edit to the record.
    for tool in ["Write", "Edit"] {
        assert_eq!(support_records::guard(project, tool, record_path.to_str().unwrap())["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    // ---- Planned: PLAN.md at open, one outcome per plan step at close.
    let plan_start = serve::git_value(project, &["rev-parse", "HEAD"]);
    let plan_json = json!([
        {"id":"step-a","action":"write the parser","verify":"cargo test parser"},
        {"id":"step-b","action":"wire the parser in","verify":"cargo test wiring"}]);
    let plan_opened = client.call("cadence_apply", json!({"operation":"task-open","request":{
        "request_id":"task-open-rooted-planned","slug":"rooted-planned","mode":"planned",
        "description":"land the parser","plan":plan_json}}));
    assert_eq!(plan_opened["status"], "ok", "{plan_opened}");
    assert_eq!(plan_opened["task"]["mode"], "planned");
    let plan_token = plan_opened["task"]["token"].as_str().unwrap().to_owned();
    let plan_path = project.join(".planning/tasks/rooted-planned/PLAN.md");
    let plan_installed = fs::read_to_string(&plan_path).unwrap();
    assert_eq!(plan_installed, expected_plan_md("rooted-planned", "land the parser",
        &[("step-a", "write the parser", "cargo test parser"), ("step-b", "wire the parser in", "cargo test wiring")]));
    assert!(!project.join(".planning/tasks/rooted-planned/RECORD.md").exists());
    assert_historical(project, &historical);

    let gamma = commit_file(project, "gamma.txt", b"gamma\n", "feat: add gamma");
    let plan_head = serve::git_value(project, &["rev-parse", "HEAD"]);
    let outcomes = json!([{"task":"step-a","result":"parser written"}, {"task":"step-b","result":"parser wired"}]);
    let plan_closed = client.call("cadence_apply", json!({"operation":"task-close","request":{
        "request_id":"task-close-rooted-planned","slug":"rooted-planned","token":plan_token,
        "report":{"kind":"text","text":"landed the parser"},"surfaces":["auth"],"outcomes":outcomes}}));
    assert_eq!(plan_closed["status"], "ok", "{plan_closed}");
    assert_eq!(plan_closed["outcome"], "done");
    let plan_record = &plan_closed["record"];
    assert_eq!(plan_record["start"], plan_start);
    assert_eq!(plan_record["head"], plan_head);
    assert_eq!(plan_record["commits"][0]["id"], gamma);
    let plan_record_path = project.join(".planning/tasks/rooted-planned/RECORD.md");
    assert_eq!(fs::read_to_string(&plan_record_path).unwrap(),
        expected_record_md(plan_record, Some(&[("step-a", "parser written"), ("step-b", "parser wired")])));
    assert_eq!(fs::read_to_string(&plan_path).unwrap(), plan_installed, "the close leaves PLAN.md untouched");
    assert_historical(project, &historical);

    client.finish();

    // ---- done is followed by a successfully reopened acknowledged record.
    let reopened = serve::reopened(project);
    let saved = &reopened.snapshot.data["task"]["records"]["rooted-inline"];
    assert_eq!(saved["status"], "done");
    assert_eq!(saved["record"]["commits"], record["commits"]);
    assert_eq!(saved["record"]["recording"]["kind"], "recorded");
    let planned_saved = &reopened.snapshot.data["task"]["records"]["rooted-planned"];
    assert_eq!(planned_saved["status"], "done");
    assert_eq!(planned_saved["outcomes"], outcomes);
    assert_eq!(planned_saved["plan"].as_array().unwrap().len(), 2);
    assert_eq!(fs::read(&record_path).unwrap(), installed, "RECORD.md is unchanged across the restart");
    assert_historical(project, &historical);

    // The typed task document carries the observed commits and filenames.
    let mut client = Client::open(project);
    let doc = client.call("cadence_query", json!({"operation":"document",
        "identity":{"kind":"task-record","slug":"rooted-inline"},"part":"record"}));
    let body = doc["body"].as_str().unwrap_or_else(|| panic!("{doc}"));
    assert!(body.contains(&alpha) && body.contains(&beta), "{body}");
    assert!(body.contains("alpha.txt") && body.contains("beta.txt"), "{body}");
    let missing = client.call("cadence_query", json!({"operation":"document",
        "identity":{"kind":"task-record","slug":"absent-task"}}));
    assert_eq!(missing["status"], "refused", "{missing}");
    client.finish();

    write_failure_refuses_done();
}

/// A record write into an unwritable `.planning/tasks` names its path, refuses
/// done, and leaves no close receipt; historical siblings are untouched.
#[cfg(unix)]
fn write_failure_refuses_done() {
    use std::os::unix::fs::PermissionsExt;
    let uid = Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    assert_ne!(String::from_utf8(uid.stdout).unwrap().trim(), "0", "the write-failure case requires an unprivileged owner");

    let repo = support_records::fixture();
    let project = repo.path();
    let historical = seed_historical(project);
    let tasks_dir = project.join(".planning/tasks");
    let original = fs::metadata(&tasks_dir).unwrap().permissions();

    let mut client = Client::open(project);
    let start = serve::git_value(project, &["rev-parse", "HEAD"]);
    let opened = task_open(&mut client, "task-open-unwritable", "unwritable", "inline", "cannot be recorded");
    assert_eq!(opened["status"], "ok", "{opened}");
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    let commit = commit_file(project, "delta.txt", b"delta\n", "feat: add delta");
    assert_ne!(commit, start);

    // Make .planning/tasks unable to accept the record's new child directory.
    fs::set_permissions(&tasks_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let blocked = task_close(&mut client, "task-close-unwritable", "unwritable", &token,
        json!({"kind":"text","text":"never recorded"}), Some(&["auth"]));
    // Restore the mode before the temporary directory drops so cleanup works.
    fs::set_permissions(&tasks_dir, original).unwrap();

    assert_eq!(blocked["status"], "refused", "{blocked}");
    assert_eq!(blocked["code"], "task-record-unwritable", "{blocked}");
    assert_eq!(blocked["rule"], "rooted-record");
    assert!(blocked["reason"].as_str().unwrap().contains(".planning/tasks/unwritable/RECORD.md"), "{blocked}");
    assert!(blocked["details"]["path"].as_str().unwrap().ends_with(".planning/tasks/unwritable/RECORD.md"));
    assert!(!project.join(".planning/tasks/unwritable/RECORD.md").exists());
    client.finish();

    // No record and no close receipt survive the failed write.
    let reopened = serve::reopened(project);
    assert!(reopened.snapshot.data["task"]["records"].get("unwritable").is_none(),
        "a failed write left a record: {}", reopened.snapshot.data["task"]);
    assert!(reopened.snapshot.data["task"]["requests"].get("task-close-unwritable").is_none());
    assert_historical(project, &historical);
}

#[cfg(not(unix))]
fn write_failure_refuses_done() {}
