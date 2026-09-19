use crate::{phase13::{Client, apply, query, git, git_value, reopened}, phase14, phase15};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, os::unix::fs::PermissionsExt};

const ROADMAP: &str = "# Roadmap\n\n## Phases\n- [x] **Phase 15: First**\n  wrapped first description\n- [x] **Phase 16: Second**\n  wrapped second description\n- [ ] **Phase 17: Open**\n  keep this wrapped description\n\n## Details\n### Phase 15: First\nRemove this detail.\n```md\n### Phase 17: Example inside selected detail\n```\n### Phase 16: Second\nRemove this too.\n### Phase 17: Open\nKeep this detail.\n\n## Deferred\n- Phase 15: keep deferred prose\n\n## Examples\n```md\n- [x] **Phase 15: Example**\n### Phase 16: Example\n```\n";
const ROADMAP_AFTER: &str = "# Roadmap\n\n## Phases\n- [ ] **Phase 17: Open**\n  keep this wrapped description\n\n## Details\n### Phase 17: Open\nKeep this detail.\n\n## Deferred\n- Phase 15: keep deferred prose\n\n## Examples\n```md\n- [x] **Phase 15: Example**\n### Phase 16: Example\n```\n";
const REQUIREMENTS: &str = "# Requirements\n\n## Active\n- [x] **REQ-15**: First\n  wrapped first requirement\n- [x] **REQ-16**: Second\n  wrapped second requirement\n- [ ] **REQ-17**: Open\n  wrapped open requirement\n\n## Traceability\n| Requirement | Phase | Status |\n|---|---|---|\n| REQ-15 | 15 | Complete |\n| REQ-16 | 16 | Complete |\n| REQ-17 | 17 | Pending |\n\n## Deferred\n- [ ] **REQ-15**: retain the deferred entry\n  and its continuation\n\n## Examples\n```md\n- [x] **REQ-15**: example\n| REQ-15 | 15 | Example |\n```\n";
const REQUIREMENTS_AFTER: &str = "# Requirements\n\n## Active\n- [ ] **REQ-17**: Open\n  wrapped open requirement\n\n## Traceability\n| Requirement | Phase | Status |\n|---|---|---|\n| REQ-17 | 17 | Pending |\n\n## Deferred\n- [ ] **REQ-15**: retain the deferred entry\n  and its continuation\n\n## Examples\n```md\n- [x] **REQ-15**: example\n| REQ-15 | 15 | Example |\n```\n";

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() { copy(&entry.path(), &dest); }
        else { fs::copy(entry.path(), dest).unwrap(); }
    }
}

// Preserve the actual root and .planning inodes: retained records bind them.
fn restore(saved: &Path, project: &Path) {
    for entry in fs::read_dir(project).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".planning" {
            for child in fs::read_dir(entry.path()).unwrap() {
                let child = child.unwrap();
                if child.file_type().unwrap().is_dir() { fs::remove_dir_all(child.path()).unwrap(); }
                else { fs::remove_file(child.path()).unwrap(); }
            }
        } else if entry.file_type().unwrap().is_dir() { fs::remove_dir_all(entry.path()).unwrap(); }
        else { fs::remove_file(entry.path()).unwrap(); }
    }
    copy(saved, project);
}

fn prune(close: &Value) -> Value {
    json!({"operation":"milestone-prune","request":{"request_id":"prune-both",
        "close":close["id"],"expected_generation":close["generation"],"selection":close["selection"]}})
}

fn client(project: &Path, stop: &str) -> Client {
    Client::open_with_env(project, Path::new(env!("CARGO_BIN_EXE_cadence")), &[
        ("CADENCE_PRUNE_STOP", stop.as_ref()),
        ("GIT_AUTHOR_DATE", "2026-09-19T12:00:00Z".as_ref()),
        ("GIT_COMMITTER_DATE", "2026-09-19T12:00:00Z".as_ref()),
    ])
}

fn call(project: &Path, request: &Value, stop: &str) -> Value {
    let mut client = client(project, stop);
    let answer = client.call("cadence_apply", request.clone());
    client.finish();
    answer
}

fn writes(project: &Path) -> Vec<String> {
    fn visit(project: &Path, relative: &str, points: &mut Vec<String>) {
        let mut children = fs::read_dir(project.join(relative)).unwrap().map(|e| e.unwrap()).collect::<Vec<_>>();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let path = format!("{relative}/{}", child.file_name().to_str().unwrap());
            if child.file_type().unwrap().is_dir() { visit(project, &path, points); }
            else { points.push(format!("delete:{path}")); }
        }
        points.push(format!("delete:{relative}"));
    }
    let mut points = vec!["intent".to_owned()];
    for p in [15,16] { visit(project, &format!(".planning/phases/{p}"), &mut points); }
    points.extend(["replace:.planning/ROADMAP.md", "replace:.planning/REQUIREMENTS.md",
        "objects", "commit", "ref", "index", "record", "clear"].map(str::to_owned));
    points.into_iter().flat_map(|p| [format!("{p}:before"), format!("{p}:after")]).collect()
}

fn open_documents(project: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    phase14::documents(project).into_iter().filter(|(p, _)| p.starts_with(".planning/phases/17")).collect()
}

fn assert_result(project: &Path, parent: &str, answer: &Value, before: &Value, open: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>) -> (String, String) {
    assert_eq!(answer["status"], "ok", "{answer}");
    assert_eq!(answer["prune"]["state"], "committed", "{answer}");
    assert_eq!(answer["prune"]["selection"]["phases"], json!([15,16]));
    assert_eq!(fs::read_to_string(project.join(".planning/ROADMAP.md")).unwrap(), ROADMAP_AFTER);
    assert_eq!(fs::read_to_string(project.join(".planning/REQUIREMENTS.md")).unwrap(), REQUIREMENTS_AFTER);
    for p in [15,16] { assert!(!project.join(format!(".planning/phases/{p}")).exists()); }
    assert!(!project.join(".planning/ARCHIVE.md").exists());
    assert_eq!(&open_documents(project), open);
    let after = reopened(project).snapshot.data;
    for (key, value) in before.as_object().unwrap() {
        assert_eq!(&after[key], value, "retained namespace {key}");
    }
    let head = git_value(project, &["rev-parse", "HEAD"]);
    assert_eq!(answer["prune"]["commit"], head);
    assert_eq!(git_value(project, &["show", "-s", "--format=%P", "HEAD"]), parent);
    assert_eq!(git_value(project, &["rev-list", "--count", &format!("{parent}..HEAD")]), "1");
    assert_eq!(git_value(project, &["status", "--porcelain"]), "");
    (head, git_value(project, &["rev-parse", "HEAD^{tree}"]))
}

pub fn exercise() {
    let fixture = phase15::Fixture::new(&[15,16]);
    let project = fixture.project();
    // Keep real risk, receipt and deferred records, on a phase outside selection.
    let (_, fire, _) = phase15::risk(project, 15, false);
    let settled = apply(project, json!({"operation":"risk-consequence","request_id":"settle-prune-risk","receipt":{
        "id":"prune-risk-settled","fire":fire,"consequence":{"kind":"gate-pass","evidence_id":"fixture-contracted-review"}}}));
    assert_eq!(settled["status"], "ok", "{settled}");
    fs::write(project.join(".planning/ROADMAP.md"), ROADMAP).unwrap();
    fs::create_dir_all(project.join(".planning/phases/17")).unwrap();
    fs::write(project.join(".planning/phases/17/NOTES.md"), "open phase stays byte exact\n").unwrap();
    let context = apply(project, crate::phase13::approve(json!({"operation":"context-submit","submission":{
        "phase":17,"title":"Open phase","scope":"Retain the deferred record.","durable_decisions":[],"decisions":[],"assumptions":[],
        "truths":[{"id":"T1","trigger":"the owner opens the next artifact","observer":"the owner","verb":"sees",
            "outcome":"ready","kind":"property","observable":true,"fixed_oracle":true}]}})));
    assert_eq!(context["status"], "ok", "{context}");
    let allocation = query(project, json!({"operation":"plan-read","phase":17,"count":1}));
    let plan = apply(project, crate::phase13::approve(json!({"operation":"plan-submit","submission":{
        "phase":17,"occurrence":allocation["occurrence"],"request_id":"publish-open",
        "inventory_basis":allocation["inventory"]["basis"],"plans":[{"target":allocation["targets"][0],"content":{
            "phase":17,"plan":1,"requirements":["T1"],"files":["src/p17.txt"],"directories":[],
            "goal":"Deliver the ready artifact.","context":"Open fixture.","notes":"One artifact.",
            "tasks":[{"id":"task-open","title":"Deliver ready","files":["src/p17.txt"],"action":"Write the artifact.","verify":["true"]}],
            "suite":"true","evidence_map":{"mode":"attached","items":[{"kind":"artifact","id":"artifact/open",
                "spec":{"locators":["src/p17.txt"],"substance":"The ready artifact contains ready."},"reason":"Observe ready.",
                "associations":[{"truth_id":"T1","truth_version":1,"reason":"Observe ready."}]}]}}}]}})));
    assert_eq!(plan["status"], "ok", "{plan}");
    let _member = phase15::deferred(project, 17);
    fs::write(project.join(".planning/REQUIREMENTS.md"), REQUIREMENTS).unwrap();
    let ready = apply(project, phase15::close("ready-prune", &[15,16]));
    assert_eq!(ready["status"], "ok", "{ready}");
    for p in [15,16] {
        let summary = fs::read_to_string(project.join(format!(".planning/phases/{p}/SUMMARY.md"))).unwrap();
        assert!(summary.contains("## Plan 1\n"));
        assert!(summary.contains("| Plan | Task | Status | Commit | Verification |"));
        assert!(summary.contains("| 1 | task-ready | completed |"));
        fs::write(project.join(format!(".planning/phases/{p}/UAT.md")), "# UAT\n\nNative artifact reads ready.\n").unwrap();
    }
    git(project, &["add", ".planning"]);
    git(project, &["commit", "-m", "Fixture prune inputs"]);
    git(project, &["tag", "v0.1.0"]);
    let parent = git_value(project, &["rev-parse", "HEAD"]);
    let before = reopened(project).snapshot.data;
    let request = prune(&ready["close"]);
    let points = writes(project);
    let saved = tempfile::tempdir().unwrap();
    copy(project, saved.path());
    let open = open_documents(project);
    let control = call(project, &request, "");
    let expected = assert_result(project, &parent, &control, &before, &open);
    for stop in points {
        restore(saved.path(), project);
        let stopped = call(project, &request, &stop);
        assert_eq!(stopped["status"], "refused", "stop {stop}: {stopped}");
        assert!(stopped.to_string().contains(&format!("injected prune stop: {stop}")), "{stopped}");
        let result = call(project, &request, "");
        assert_eq!(assert_result(project, &parent, &result, &before, &open), expected, "stop {stop}");
        let files = [".planning/ROADMAP.md", ".planning/REQUIREMENTS.md"].map(|p| fs::metadata(project.join(p)).unwrap().modified().unwrap());
        assert_eq!(call(project, &request, ""), result);
        assert_eq!([".planning/ROADMAP.md", ".planning/REQUIREMENTS.md"].map(|p| fs::metadata(project.join(p)).unwrap().modified().unwrap()), files);
        assert_eq!(git_value(project, &["rev-parse", "HEAD"]), expected.0);
    }
    let why = query(project, json!({"operation":"why","path":"src/p15.txt"}));
    let text = why["text"].as_str().unwrap();
    assert!(text.contains(&format!("an unlabelled close ({}) phase 15 (recovered from {}:.planning/phases/15)", &expected.0[..8], &parent[..8])), "{why}");
    assert!(text.contains("task-ready"), "{why}");
    assert!(text.contains("SUMMARY.md"), "{why}");
    git(project, &["commit", "--allow-empty", "-m", "Fixture later release"]);
    git(project, &["tag", "v4.0.0"]);
    let why = query(project, json!({"operation":"why","path":"src/p15.txt"}));
    assert!(why["text"].as_str().unwrap().contains(&format!("v4.0.0 phase 15 (recovered from {}:.planning/phases/15)", &parent[..8])), "{why}");

    restore(saved.path(), project);
    let original = phase14::documents(project);
    fs::set_permissions(project.join(".planning/REQUIREMENTS.md"), fs::Permissions::from_mode(0)).unwrap();
    // This real launch verifies the open fails under the same effective uid that
    // runs serve. chmod alone is not an unreadable-input test when cargo is root.
    let wrapper = project.join("unprivileged-serve");
    fs::write(&wrapper, format!("#!/usr/bin/env python3\nimport os,sys\nif os.geteuid()==0:\n os.setgroups([]); os.setgid(65534); os.setuid(65534)\ntry:\n open('.planning/REQUIREMENTS.md','rb')\nexcept PermissionError:\n pass\nelse:\n raise SystemExit('fixture open unexpectedly succeeded')\nos.execv({:?}, [{:?}]+sys.argv[1:])\n", env!("CARGO_BIN_EXE_cadence"), env!("CARGO_BIN_EXE_cadence"))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    if Command::new("id").arg("-u").output().unwrap().stdout == b"0\n" {
        assert!(Command::new("chown").args(["-R", "65534:65534"]).arg(project).status().unwrap().success());
    }
    let mut child = Client::open_with_program(project, &wrapper);
    let refused = child.call("cadence_apply", request);
    child.finish();
    assert_eq!(refused["status"], "refused", "{refused}");
    assert!(refused.to_string().contains("REQUIREMENTS.md"), "{refused}");
    fs::set_permissions(project.join(".planning/REQUIREMENTS.md"), fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(phase14::documents(project), original);
    // Git's owner check is intentionally unchanged; use the child owner to read
    // HEAD on root runners through the already-authored parent object bytes.
    assert_eq!(fs::read_to_string(project.join(".git/refs/heads/fixture/landing")).unwrap().trim(), parent);
}
