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
