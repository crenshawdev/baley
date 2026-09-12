//! Real-binary fixtures for the phase 31 read-layer acceptance checks.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Client {
    pub fn open(project: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cadence"))
            .args(["serve", "--project-root", project.to_str().unwrap()])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "3")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
            .env("GIT_CONFIG_KEY_1", "user.name")
            .env("GIT_CONFIG_VALUE_1", "Cadence Phase31")
            .env("GIT_CONFIG_KEY_2", "user.email")
            .env("GIT_CONFIG_VALUE_2", "phase31@example.invalid")
            .current_dir(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut client = Self {
            stdin: child.stdin.take(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
        };
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"phase31-check","version":"1"}}}));
        assert!(client.recv()["result"]["serverInfo"].is_object());
        client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        client
    }

    fn send(&mut self, value: Value) {
        writeln!(self.stdin.as_mut().unwrap(), "{value}").unwrap();
        self.stdin.as_mut().unwrap().flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        assert!(self.stdout.read_line(&mut line).unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    }

    pub fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":tool,"arguments":arguments}}));
        let response = self.recv();
        assert!(response.get("error").is_none(), "{response}");
        assert_ne!(response["result"]["isError"], true, "{response}");
        let structured = response["result"]["structuredContent"].clone();
        let text: String = response["result"]["content"].as_array().unwrap().iter()
            .filter(|block| block["type"] == "text")
            .map(|block| block["text"].as_str().unwrap())
            .collect();
        assert_eq!(structured, serde_json::from_str::<Value>(&text).unwrap());
        structured
    }

    pub fn finish(mut self) {
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path();
        fs::create_dir_all(project.join(".planning/phases/13")).unwrap();
        fs::create_dir_all(project.join("src/nested")).unwrap();
        fs::create_dir_all(project.join("docs")).unwrap();
        fs::write(project.join(".planning/ROADMAP.md"), "## Phases\n- [ ] **Phase 13: Fixture**\n").unwrap();
        fs::write(project.join(".planning/config.json"), "{}").unwrap();
        fs::write(project.join(".gitignore"), "ignored/\n.planning/\n").unwrap();
        fs::create_dir_all(project.join("ignored")).unwrap();
        fs::write(project.join("ignored/sentinel.rs"), "fn ignored() { let needle = 0; }\n").unwrap();
        fs::write(project.join("src/units.rs"), "fn alpha() {\n    let needle = 1;\n    let needle_again = 2;\n}\n\nfn beta() {\n    let needle = 3;\n}\n\nfn untouched() {}\n").unwrap();
        fs::write(project.join("src/nested/mod.rs"), "fn outer() {\n    fn inner() { let needle = 4; }\n}\n").unwrap();
        fs::write(project.join("src/units.js"), "function javascriptUnit() {\n  const needle = 1;\n}\n").unwrap();
        fs::write(project.join("docs/units.md"), "# Markdown unit\nneedle\n").unwrap();
        fs::write(project.join("src/units.json"), "{\n  \"jsonUnit\": \"needle\"\n}\n").unwrap();
        fs::write(project.join("src/units.c"), "int c_unit(void) {\n  int needle = 1;\n  return needle;\n}\n").unwrap();
        Command::new("git").args(["init", "--initial-branch=fixture/read"]).current_dir(project).status().unwrap();
        Command::new("git").args(["add", "."]).current_dir(project).status().unwrap();
        Command::new("git").args(["-c", "commit.gpgsign=false", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture"]).current_dir(project).status().unwrap();
        Self { temp }
    }

    pub fn project(&self) -> &Path { self.temp.path() }
    pub fn path(&self, relative: &str) -> PathBuf { self.project().join(relative) }
}
