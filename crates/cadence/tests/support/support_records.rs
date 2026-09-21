#![allow(dead_code)]

use crate::phase13::{self, Client};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};

pub fn fixture() -> tempfile::TempDir {
    let temp = phase13::fixture();
    // Initialize the journal before any snapshot or stopped-process copy reads it.
    let mut client = Client::open(temp.path());
    client.call("cadence_query", json!({"operation":"progress"}));
    client.finish();
    phase13::reopened(temp.path());
    temp
}

pub fn risk_fixture() -> tempfile::TempDir {
    let temp = phase13::fixture();
    fs::write(temp.path().join(".planning/config.json"), serde_json::to_vec(&json!({
        "review":{"mode":"single","reviewers":["claude-subagent"],
            "triggers":{"risk_surface":{"surfaces":["auth"],"gate":"blocking"}}}
    })).unwrap()).unwrap();
    let mut client = Client::open(temp.path());
    client.call("cadence_query", json!({"operation":"progress"}));
    client.finish();
    phase13::reopened(temp.path());
    temp
}

pub fn change(project: &Path, path: &str, bytes: &[u8], staged: bool) {
    let target = project.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, bytes).unwrap();
    if staged { phase13::git(project, &["add", "--", path]); }
}

pub fn documents(project: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    phase13::tree(project).into_iter().filter_map(|(path, bytes)| {
        let document = path.extension().is_some_and(|ext| ext == "md")
            || path == Path::new(".planning/config.json");
        document.then_some(bytes).flatten().map(|bytes| (path, bytes))
    }).collect()
}

pub fn copy_stopped(project: &Path) -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() { copy(&entry.path(), &target); }
            else { fs::copy(entry.path(), target).unwrap(); }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    copy(project, temp.path());
    temp
}

pub fn staged_note(project: &Path) -> String {
    fs::write(project.join(".planning/config.json"), serde_json::to_vec(&json!({
        "review":{"triggers":{"risk_surface":{"surfaces":["auth"]}}}
    })).unwrap()).unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/session.js"), "jwt.verify(token)\n").unwrap();
    phase13::git(project, &["add", "src/session.js"]);
    phase13::git(project, &["commit", "-m", "Fixture auth baseline"]);
    fs::create_dir_all(project.join("docs")).unwrap();
    fs::write(project.join("docs/note.txt"), "plain note").unwrap();
    phase13::git(project, &["add", "docs/note.txt"]);
    phase13::git_value(project, &["write-tree"])
}

pub fn guard(project: &Path, tool: &str, target: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cadence"))
        .arg("guard").current_dir(project).env("CADENCE_GLOBAL_CONFIG", "")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let input = json!({"session_id":"support-record-guard","hook_event_name":"PreToolUse",
        "tool_name":tool,"cwd":project,"tool_input":if tool == "Write" {
            json!({"file_path":target,"content":"never written"})
        } else { json!({"file_path":target,"old_string":"old","new_string":"new"}) }});
    child.stdin.take().unwrap().write_all(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    if output.stdout.is_empty() { Value::Null } else { serde_json::from_slice(&output.stdout).unwrap() }
}

pub fn apply(client: &mut Client, operation: &str, request: Value) -> Value {
    let answer = client.call("cadence_apply", json!({"operation":operation,"request":request}));
    assert_eq!(answer["status"], "ok", "{answer}");
    answer
}

pub fn query(client: &mut Client, operation: &str, slug: &str) -> Value {
    client.call("cadence_query", json!({"operation":operation,"slug":slug}))
}

pub fn skill_frontmatter(text: &str) -> Value {
    let (frontmatter, _) = text.strip_prefix("---\n").unwrap().split_once("\n---\n").unwrap();
    serde_saphyr::from_str(frontmatter).unwrap()
}

pub fn render_skill(project: &Path, command: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cadence"))
        .args(command).current_dir(project).stdin(Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

pub fn resolve(client: &mut Client, slug: &str, id: &str, passed: bool) -> Value {
    let status = query(client, "debug-status", slug);
    client.call("cadence_apply", json!({"operation":"debug-resolve","request":{
        "request_id":id,"slug":slug,"expected_version":status["record"]["version"],
        "resolution":"repair token checks","reproduction":{"test":"repeat login",
        "result":"fixture reproduction","passed":passed}}}))
}

/// Play the caller over the issued H3 identity; no reviewer or saved record is faked.
pub fn return_findings(client: &mut Client, fire: &str, key: &str, findings: Value) -> Value {
    let next = client.call("cadence_query", json!({"operation":"review-next","fire":fire}));
    assert_eq!(next["result"]["state"], "dispatch", "{next}");
    let attempt = &next["result"]["attempt"];
    let launch = format!("{key}-launch");
    let returned = format!("{key}-return");
    for (kind, host_return) in [("launch", Value::Null), ("return", json!(returned))] {
        let answer = client.call("cadence_apply", json!({"operation":"review-observation","observation":{
            "observation":format!("{key}-{kind}-observation"),"attempt":attempt["attempt"],
            "launch":launch,"host_return":host_return,"kind":kind,"reference":format!("event:{key}-{kind}"),
            "observed_at":1,"host":"fixture","model":null,
            "usage":{"input":null,"output":null,"cost":null,"currency":null},"contract":attempt["contract"]}}));
        assert_eq!(answer["status"], "ok", "{answer}");
    }
    let answer = client.call("cadence_apply", json!({"operation":"review-return","identity":{
        "fire":attempt["fire"],"occurrence":attempt["occurrence"],"artifact":attempt["view"]["manifest"],
        "view":attempt["view"]["view"],"attempt":attempt["attempt"],"round":attempt["round"]},
        "launch":launch,"host_return":returned,"citations":[],"findings":findings}));
    assert_eq!(answer["result"]["terminal"], "accepted", "{answer}");
    let saved = client.call("cadence_query", json!({"operation":"review-attempt","attempt":attempt["attempt"]}));
    let original = client.call("cadence_query", json!({"operation":"review-original","original":saved["result"]["original"]}));
    assert_eq!(original["result"]["findings"], findings, "{original}");
    original["result"].clone()
}

pub fn consequence(client: &mut Client, id: &str, fire: &cadence::rail::receipts::Fire, consequence: Value) -> Value {
    client.call("cadence_apply", json!({"operation":"risk-consequence","request_id":format!("request-{id}"),
        "receipt":{"id":id,"fire":fire,"consequence":consequence}}))
}

pub fn debug_readback(client: &mut Client, project: &Path, slug: &str, needles: &[&str]) -> Value {
    let status = query(client, "debug-status", slug);
    assert_eq!(status["status"], "ok", "{status}");
    assert_eq!(query(client, "debug-continue", slug), status);
    let projection = fs::read_to_string(project.join(format!(".planning/debug/{slug}.md"))).unwrap();
    assert_eq!(status["projection"], projection);
    for needle in needles {
        assert!(status["record"]["review"].to_string().contains(needle), "missing {needle}: {status}");
        assert!(projection.contains(needle), "missing {needle}: {projection}");
    }
    status
}

pub fn recall_fixture(backend: Option<&str>) -> tempfile::TempDir {
    let temp = phase13::fixture();
    let root = temp.path().join(".planning");
    fs::write(root.join("ROADMAP.md"), "## Phases\n- [ ] **Phase 1: First**\n- [ ] **Phase 2: Second**\n- [ ] **Phase 13: Plan publication**\n- [ ] **Phase 28: Next phase**\n").unwrap();
    let config = backend.map_or_else(|| json!({}), |backend| json!({"memory":{"backend":backend}}));
    fs::write(root.join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let mut client = Client::open(temp.path());
    client.call("cadence_query", json!({"operation":"progress"}));
    for (id, kind, phase) in [("recall-phase-todo", "todo", Some(1)), ("recall-global-note", "note", None)] {
        let mut request = json!({"operation":"capture","request_id":id,"kind":kind,"text":"cache stale token"});
        if let Some(phase) = phase { request["phase"] = json!(phase); }
        let answer = client.call("cadence_apply", request);
        assert_eq!(answer["status"], "ok", "{answer}");
    }
    client.finish();
    phase13::reopened(temp.path());
    for (phase, reason) in [("1", "reused"), ("2", "changed"), ("1.1", "decimal")] {
        fs::create_dir_all(root.join(format!("phases/{phase}"))).unwrap();
        fs::write(root.join(format!("phases/{phase}/SUMMARY.md")), format!("## Deviations\n- cache stale token from {reason} key\n")).unwrap();
    }
    temp
}

#[cfg(unix)]
pub struct UnreadableFile {
    path: PathBuf,
    permissions: fs::Permissions,
}

#[cfg(unix)]
impl UnreadableFile {
    pub fn new(path: PathBuf) -> Option<Self> {
        use std::os::unix::fs::PermissionsExt;
        let uid = Command::new("id").arg("-u").output().unwrap();
        assert!(uid.status.success());
        if String::from_utf8(uid.stdout).unwrap().trim() == "0" { return None; }
        let permissions = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Some(Self { path, permissions })
    }
}

#[cfg(unix)]
impl Drop for UnreadableFile {
    fn drop(&mut self) { fs::set_permissions(&self.path, self.permissions.clone()).unwrap(); }
}

pub fn spike_history(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() { visit(root, &entry.path(), files); }
            else { files.insert(entry.path().strip_prefix(root).unwrap().into(), fs::read(entry.path()).unwrap()); }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

pub fn install_spike_history(project: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.planning/spikes");
    let files = spike_history(&source);
    assert_eq!(fs::read_dir(&source).unwrap().count(), 10);
    assert_eq!(files.len(), 15);
    assert_eq!(files.values().map(Vec::len).sum::<usize>(), 113_682);
    for (path, bytes) in &files {
        let target = project.join(".planning/spikes").join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    files
}
