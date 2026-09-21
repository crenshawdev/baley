#![allow(dead_code)]
//! Task fixtures: a genuine git repository the caller commits to, a branch
//! policy carried by a fixture global layer outside the project tree, and a
//! listing of the project that excludes only `.git`.

use crate::serve::{self, Client};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

/// One task fixture: `project/` is the repository, `global/config.json` the
/// only configuration layer it has, and `scratch/` the child's temporary root.
pub struct TaskFixture {
    temp: tempfile::TempDir,
}

impl TaskFixture {
    /// A repository on a protected `main` with one baseline commit and no
    /// `.planning/`. The branch policy is explicit fixture input; the risk
    /// surface answer is supplied per run on task-close.
    pub fn treeless() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/app.txt"), "baseline\n").unwrap();
        serve::git(&project, &["init", "--initial-branch=main"]);
        serve::git(&project, &["add", "."]);
        serve::git(&project, &["commit", "-m", "Fixture baseline"]);
        fs::create_dir_all(temp.path().join("global")).unwrap();
        fs::create_dir_all(temp.path().join("scratch")).unwrap();
        fs::write(temp.path().join("global/config.json"), serde_json::to_vec_pretty(&json!({
            "git": {"protected_branches": ["main"], "on_protected": "ask"}
        })).unwrap()).unwrap();
        assert!(!project.join(".planning").exists());
        Self { temp }
    }

    pub fn project(&self) -> PathBuf { self.temp.path().join("project") }
    pub fn global_config(&self) -> PathBuf { self.temp.path().join("global/config.json") }
    /// The child's `TMPDIR`: per-run task material lands here and nowhere else.
    pub fn scratch(&self) -> PathBuf { self.temp.path().join("scratch") }
    /// A path under the fixture that nothing creates.
    pub fn absent(&self, name: &str) -> PathBuf { self.temp.path().join("absent").join(name) }

    pub fn client(&self) -> Client {
        let global = self.global_config();
        let scratch = self.scratch();
        Client::open_with_env(&self.project(), Path::new(env!("CARGO_BIN_EXE_cadence")),
            &[("CADENCE_GLOBAL_CONFIG", global.as_os_str()), ("TMPDIR", scratch.as_os_str())])
    }

    /// Names under `scratch/` that a task run left behind.
    pub fn task_material(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.scratch()).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("cadence-task-")).collect();
        names.sort();
        names
    }
}

/// Every entry under the project except `.git`: a directory as `None`, a
/// file or symlink as its bytes.
pub fn listing(project: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(base: &Path, path: &Path, found: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let meta = fs::symlink_metadata(path).unwrap();
        let relative = path.strip_prefix(base).unwrap().to_path_buf();
        if meta.file_type().is_symlink() {
            found.insert(relative, Some(fs::read_link(path).unwrap().as_os_str().as_encoded_bytes().to_vec()));
        } else if meta.is_dir() {
            found.insert(relative, None);
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if path == base && entry.file_name() == ".git" { continue; }
                visit(base, &entry.path(), found);
            }
        } else {
            found.insert(relative, Some(fs::read(path).unwrap()));
        }
    }
    let mut found = BTreeMap::new();
    visit(project, project, &mut found);
    found
}

/// Write one file and commit it as the caller; returns the full commit id.
pub fn commit_file(project: &Path, path: &str, bytes: &[u8], subject: &str) -> String {
    let target = project.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, bytes).unwrap();
    serve::git(project, &["add", "--", path]);
    serve::git(project, &["commit", "-m", subject]);
    serve::git_value(project, &["rev-parse", "HEAD"])
}

pub fn task_open(client: &mut Client, request_id: &str, slug: &str, mode: &str, description: &str) -> Value {
    client.call("cadence_apply", json!({"operation":"task-open","request":{
        "request_id":request_id,"slug":slug,"mode":mode,"description":description}}))
}

pub fn task_close(client: &mut Client, request_id: &str, slug: &str, token: &str, report: Value, surfaces: Option<&[&str]>) -> Value {
    client.call("cadence_apply", json!({"operation":"task-close","request":{
        "request_id":request_id,"slug":slug,"token":token,"report":report,"surfaces":surfaces}}))
}
