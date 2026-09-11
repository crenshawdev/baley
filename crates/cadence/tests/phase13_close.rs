#[path = "support/phase13.rs"]
pub mod phase13;
use phase13::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, os::unix::fs::PermissionsExt, path::{Path, PathBuf}, process::{Command, Stdio}};

fn sha(raw: &[u8]) -> String { format!("{:x}", Sha256::digest(raw)) }

struct Settings {
    root: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
    hook: Vec<u8>,
    command: String,
}

impl Settings {
    fn new(parent: &Path, name: &str) -> Self {
        let root = parent.join(name);
        fs::create_dir_all(root.join("hooks")).unwrap();
        let command = format!("node {}", root.join("hooks/rules-gate.mjs").display());
        let target = serde_json::to_string(&command).unwrap();
        let near = serde_json::to_string(&format!("{command}.disabled")).unwrap();
        let flagged = serde_json::to_string(&format!("{command} --flag")).unwrap();
        let entry = format!(r#"{{"type":"command","command":{target}}}"#);
        let sibling = r#"{"type":"command","command":"node guard.mjs"}"#;
        let similar = format!(r#"{{"type":"command","command":{near}}}"#);
        let flags = format!(r#"{{"type":"command","command":{flagged}}}"#);
        // Handwritten output retains event/group/matcher order and untouched bytes.
        let before = format!(r#"{{
  "theme" : "unchanged", "hooks": {{
    "PreToolUse": [{{"matcher":"Write|Edit","hooks":[{entry}, {sibling}, {entry}, {similar}]}}],
    "Stop": [{{"matcher":"","hooks":[{flags}, {entry}]}}],
    "SessionStart": [{{"matcher":"all","hooks":[{{"type":"command","command":"node reviewer-stop-bridge.mjs"}}]}}]
  }}, "permissions" : {{ "allow" : ["Read", "Bash"] }}
}}
"#).into_bytes();
        let after = format!(r#"{{
  "theme" : "unchanged", "hooks": {{
    "PreToolUse": [{{"matcher":"Write|Edit","hooks":[{sibling}, {similar}]}}],
    "Stop": [{{"matcher":"","hooks":[{flags}]}}],
    "SessionStart": [{{"matcher":"all","hooks":[{{"type":"command","command":"node reviewer-stop-bridge.mjs"}}]}}]
  }}, "permissions" : {{ "allow" : ["Read", "Bash"] }}
}}
"#).into_bytes();
        let hook = b"// obsolete rules gate fixture\n".to_vec();
        fs::write(root.join("settings.json"), &before).unwrap();
        fs::write(root.join("hooks/rules-gate.mjs"), &hook).unwrap();
        fs::write(root.join("hooks/guard.mjs"), b"guard stays byte-for-byte\n").unwrap();
        fs::write(root.join("hooks/reviewer-stop-bridge.mjs"), b"bridge stays byte-for-byte\n").unwrap();
        Self {root,before,after,hook,command}
    }

    fn invoke(&self, action: &str, recovery: &Path, settings_hash: &str, hook_hash: &str) -> (i32, Value) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.planning/phases/13/close/retire-rules-gate.py").canonicalize().unwrap();
        let args = vec![action.to_owned(), "--settings-root".into(), self.root.display().to_string(),
            "--settings-sha256".into(), settings_hash.into(), "--hook-sha256".into(), hook_hash.into(),
            "--command".into(), self.command.clone(), "--recovery".into(), recovery.display().to_string()];
        let output = Command::new("python3").arg("-B").arg(&script).args(&args)
            .env("HOME", self.root.join("unused-home")).stdin(Stdio::null()).output().unwrap();
        assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let code = output.status.code().unwrap();
        // Bypass libtest capture so the report can quote this actual invocation.
        let mut output = std::io::stdout().lock();
        writeln!(output, "CLOSE_REHEARSAL {}", json!({
            "program":"python3","script":script,"args":args,"exit":code,"answer":value})).unwrap();
        (code, value)
    }

    fn guards(&self) {
        assert_eq!(fs::read(self.root.join("hooks/guard.mjs")).unwrap(), b"guard stays byte-for-byte\n");
        assert_eq!(fs::read(self.root.join("hooks/reviewer-stop-bridge.mjs")).unwrap(), b"bridge stays byte-for-byte\n");
    }
}

#[test]
fn phase13_rules_gate_retirement_rehearsal() {
    // The credited planner front door is project-free compiled text; its native
    // authoring/publication protocol is exercised over real serve/stdio below.
    let output = Command::new(env!("CARGO_BIN_EXE_cadence")).arg("plan-instructions")
        .stdin(Stdio::null()).output().unwrap();
    assert!(output.status.success());
    let planner = String::from_utf8(output.stdout).unwrap();
    assert!(planner.contains("**Planner.** For each truth, write its ONE check"));
    assert!(planner.contains("plan-submit"));
    let fixture = Completed::new();
    let project = fixture.project();
    let mut client = Client::open(project);
    let plans = client.read("13", None);
    assert_eq!(plans["status"], "ok", "{plans}");
    assert_eq!(plans["native_truths_approved"], true);
    assert_eq!(plans["plans"].as_array().unwrap().len(), 2);
    for plan in plans["plans"].as_array().unwrap() {
        assert_eq!(plan["classification"], "native-publication");
    }
    assert!(plans["contract"]["$defs"]["Submission"].is_object());
    assert_eq!(fixture.dispatches.len(), 2);
    for dispatch in &fixture.dispatches {
        assert_eq!(dispatch["outcome"], "dispatch");
        assert!(dispatch["prompt"].as_str().unwrap().contains("**Executor.**"));
    }
    let verify = client.call("cadence_query", json!({"operation":"verify-next","phase":13,"request_id":"close-prerequisite"}));
    assert_eq!(verify["status"], "ok", "{verify}");
    assert!(verify["attempt"]["prompt"].as_str().unwrap().contains("**Verifier.**"));
    client.finish();
    let baseline = tree(project);
    reopened(project);
    assert_eq!(query(project, json!({"operation":"verify-next","phase":13,"request_id":"close-prerequisite"})), verify);
    assert_eq!(tree(project), baseline);
    let mut output = std::io::stdout().lock();
    writeln!(output, "CLOSE_PREREQUISITE {}", json!({
        "planner":"plan-instructions plus real stdio native plan-read/plan-submit",
        "planner_bytes_sha256":sha(planner.as_bytes()),"executor_dispatches":fixture.dispatches.iter().map(|d| &d["dispatch"]["id"]).collect::<Vec<_>>(),
        "verifier_attempt":verify["attempt"]["id"],"prompt_digest":verify["attempt"]["prompt_digest"]})).unwrap();
    drop(output);

    let disposable = tempfile::tempdir().unwrap();
    let case = Settings::new(disposable.path(), "success");
    let recovery = disposable.path().join("success-recovery");
    let (code, result) = case.invoke("retire", &recovery, &sha(&case.before), &sha(&case.hook));
    assert_eq!((code, result["status"].as_str()), (0, Some("retired")));
    assert_eq!(result["removed"], 3);
    assert!(!case.root.join("hooks/rules-gate.mjs").exists());
    assert_eq!(fs::read(case.root.join("settings.json")).unwrap(), case.after);
    assert_eq!(fs::read(recovery.join("settings.original")).unwrap(), case.before);
    assert_eq!(fs::read(recovery.join("hook.original")).unwrap(), case.hook);
    case.guards();
    let (code, result) = case.invoke("retire", &recovery, &sha(&case.before), &sha(&case.hook));
    assert_eq!((code, result["status"].as_str()), (0, Some("already-retired")));
    let (code, result) = case.invoke("retire", &disposable.path().join("absent-recovery"), &sha(&case.after), "absent");
    assert_eq!((code, result["status"].as_str()), (0, Some("already-absent")));
    assert_eq!(fs::read(case.root.join("settings.json")).unwrap(), case.after);
    case.guards();

    let ambiguous = Settings::new(disposable.path(), "ambiguous");
    let raw = b"{\"hooks\":{},\"hooks\":{}}\n";
    fs::write(ambiguous.root.join("settings.json"), raw).unwrap();
    let recovery = disposable.path().join("ambiguous-recovery");
    let (code, result) = ambiguous.invoke("retire", &recovery, &sha(raw), &sha(&ambiguous.hook));
    assert_eq!(code, 1);
    assert_eq!(result["reason"], "ambiguous duplicate JSON key: hooks");
    assert!(!recovery.exists());
    assert_eq!(fs::read(ambiguous.root.join("settings.json")).unwrap(), raw);
    assert_eq!(fs::read(ambiguous.root.join("hooks/rules-gate.mjs")).unwrap(), ambiguous.hook);
    ambiguous.guards();

    let stale = Settings::new(disposable.path(), "stale");
    let recovery = disposable.path().join("stale-recovery");
    for (settings_hash, hook_hash) in [("0".repeat(64), sha(&stale.hook)), (sha(&stale.before), "0".repeat(64))] {
        let (code, result) = stale.invoke("retire", &recovery, &settings_hash, &hook_hash);
        assert_eq!(code, 1);
        assert_eq!(result["reason"], "stale inspected preimage");
        assert!(!recovery.exists());
    }
    assert_eq!(fs::read(stale.root.join("settings.json")).unwrap(), stale.before);
    assert_eq!(fs::read(stale.root.join("hooks/rules-gate.mjs")).unwrap(), stale.hook);
    stale.guards();

    let partial = Settings::new(disposable.path(), "partial");
    let recovery = disposable.path().join("partial-recovery");
    // Real filesystem denial after hook unlink, before settings replacement.
    // This permission-denial case requires the normal unprivileged executor.
    assert_ne!(unsafe { libc::geteuid() }, 0, "partial-failure rehearsal requires an unprivileged runner");
    fs::set_permissions(&partial.root, fs::Permissions::from_mode(0o500)).unwrap();
    let (code, result) = partial.invoke("retire", &recovery, &sha(&partial.before), &sha(&partial.hook));
    fs::set_permissions(&partial.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!((code, result["status"].as_str()), (2, Some("unfinished")));
    assert!(!partial.root.join("hooks/rules-gate.mjs").exists());
    assert_eq!(fs::read(partial.root.join("settings.json")).unwrap(), partial.before);
    assert_eq!(fs::read(recovery.join("settings.original")).unwrap(), partial.before);
    assert_eq!(fs::read(recovery.join("hook.original")).unwrap(), partial.hook);
    let (code, result) = partial.invoke("retire", &recovery, &sha(&partial.before), &sha(&partial.hook));
    assert_eq!(code, 1);
    assert!(result["reason"].as_str().unwrap().starts_with("unfinished"));
    let (code, result) = partial.invoke("recover", &recovery, &sha(&partial.before), &sha(&partial.hook));
    assert_eq!((code, result["status"].as_str()), (0, Some("recovered")));
    assert_eq!(fs::read(partial.root.join("settings.json")).unwrap(), partial.before);
    assert_eq!(fs::read(partial.root.join("hooks/rules-gate.mjs")).unwrap(), partial.hook);
    partial.guards();
    let (code, result) = partial.invoke("retire", &disposable.path().join("reinspected-recovery"), &sha(&partial.before), &sha(&partial.hook));
    assert_eq!((code, result["status"].as_str()), (0, Some("retired")));
    assert_eq!(fs::read(partial.root.join("settings.json")).unwrap(), partial.after);
    assert!(!partial.root.join("hooks/rules-gate.mjs").exists());
    partial.guards();
}
