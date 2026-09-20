//! Binary-owned subprocesses. Callers journal the invocation before launch.
use super::model::{Authorize, ExternalInput, Landing, Step};
use crate::{rail::branch, store::{Error, Result}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, os::unix::process::CommandExt, path::Path, process::{Command, Stdio}, time::{Duration, Instant}};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation { pub program: String, pub args: Vec<String> }

const OUTPUT_BOUND: usize = 65_536;

fn capture(mut stream: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut overflow = false;
    let mut chunk = [0; 4096];
    loop {
        let size = stream.read(&mut chunk)?;
        if size == 0 { return Ok((bytes, overflow)); }
        let keep = size.min(OUTPUT_BOUND.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..keep]);
        overflow |= keep != size;
    }
}

pub fn run(root: &Path, invocation: &Invocation) -> Result<String> {
    let mut child = Command::new(&invocation.program).args(&invocation.args).current_dir(root)
        .process_group(0)
        .env("GIT_TERMINAL_PROMPT", "0").env("GH_PROMPT_DISABLED", "1").env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || capture(stdout));
    let err = std::thread::spawn(move || capture(stderr));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? { break status; }
        if started.elapsed() >= Duration::from_secs(60) {
            child.kill()?;
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Also close inherited pipes in descendants on timeout or parent exit.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
    let (stdout, out_overflow) = out.join().map_err(|_| Error::Invalid("stdout reader failed".into()))??;
    let (stderr, err_overflow) = err.join().map_err(|_| Error::Invalid("stderr reader failed".into()))??;
    if !status.success() || out_overflow || err_overflow {
        return Err(Error::Invalid(format!("{} returned {status}; output exceeded bound: {}; {}",
            invocation.program, out_overflow || err_overflow, String::from_utf8_lossy(&stderr))));
    }
    String::from_utf8(stdout).map_err(|_| Error::Invalid("subprocess output is not UTF-8".into()))
}

pub fn observe(root: &Path, args: &[&str]) -> Result<String> {
    run(root, &Invocation { program: "git".into(), args: args.iter().map(|s| (*s).into()).collect() })
        .map(|s| s.trim_end().into())
}

pub fn valid_ref(name: &str) -> bool {
    name.starts_with("refs/") && !name.ends_with('.') && !name.contains("..") && !name.contains("@{")
        && !name.chars().any(|c| c.is_control() || " ~^:?*[\\".contains(c))
        && name.split('/').all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

pub fn source_matches(root: &Path, landing: &Landing) -> Result<bool> {
    Ok(observe(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])? == landing.source.branch
        && observe(root, &["rev-parse", "--verify", "HEAD^{commit}"])? == landing.source.head
        && observe(root, &["rev-parse", "--verify", "--end-of-options", &format!("refs/heads/{}^{{commit}}", landing.source.branch)])? == landing.source.head
        && observe(root, &["remote", "get-url", "--push", "--all", &landing.remote.name])? == landing.remote.url
        && observe(root, &["remote", "get-url", "--all", &landing.remote.name])? == landing.remote.url)
}

pub fn remote_head(root: &Path, landing: &Landing, reference: &str) -> Result<Option<String>> {
    let output = observe(root, &["ls-remote", "--refs", "--", &landing.remote.url, reference])?;
    let mut found = None;
    for line in output.lines() {
        let (sha, name) = line.split_once('\t').ok_or_else(|| Error::Invalid(format!("malformed remote ref: {line}")))?;
        if name != reference || found.is_some() || !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!("ambiguous remote ref {reference}: {output}")));
        }
        found = Some(sha.to_owned());
    }
    Ok(found)
}

pub fn policy(landing: &Landing, inputs: &ExternalInput, config: &Value) -> Result<()> {
    let step = inputs.step();
    let target = if step == Step::Merge { &landing.base.branch } else { &landing.source.branch };
    let protected = branch::protected_branches(config.pointer("/git/protected_branches"));
    let policy = config.pointer("/git/on_protected").and_then(Value::as_str).unwrap_or("ask");
    // Ask is fulfilled by the exact owner authorization already required here.
    if branch::permission(&protected, policy, target)? == branch::Permission::Deny {
        return Err(Error::Policy(format!("protected branch {target} forbids this step")));
    }
    for branch in [&landing.source.branch, &landing.base.branch] {
        if !valid_ref(&format!("refs/heads/{branch}")) { return Err(Error::Invalid("invalid landing branch".into())); }
    }
    Ok(())
}

pub fn prepare(root: &Path, landing: &Landing, inputs: &ExternalInput, config: &Value) -> Result<Invocation> {
    policy(landing, inputs, config)?;
    match inputs {
        ExternalInput::Push => Ok(Invocation { program: "git".into(), args: vec!["push".into(), "--porcelain".into(), "--".into(),
            landing.remote.url.clone(), format!("{}:refs/heads/{}", landing.source.head, landing.source.branch)] }),
        ExternalInput::TagPush { tag, head } => {
            let reference = format!("refs/tags/{tag}");
            if observe(root, &["rev-parse", "--verify", "--end-of-options", &reference])? != *head {
                return Err(Error::Invalid("tag object changed since authorization".into()));
            }
            Ok(Invocation { program: "git".into(), args: vec!["push".into(), "--porcelain".into(), "--".into(),
                landing.remote.url.clone(), format!("{head}:{reference}")] })
        }
        ExternalInput::Open { forge, .. } => {
            if !landing.steps.iter().any(|s| s.step == Step::Publish && s.receipt.is_some())
                || remote_head(root, landing, &format!("refs/heads/{}", landing.source.branch))?.as_deref() != Some(&landing.source.head) {
                return Err(Error::Invalid("opening requires the recorded push and its exact remote source head".into()));
            }
            super::forge::configured(forge, config)?;
            require_base(root, landing)?;
            super::forge::mutation(landing, inputs)
        }
        ExternalInput::Merge { forge, pr } => {
            let open = landing.steps.iter().find(|s| s.step == Step::Open).and_then(|s| s.receipt.as_ref())
                .ok_or_else(|| Error::Invalid("merge requires the recorded PR identity".into()))?;
            if open["result"]["number"].as_u64().or_else(|| open["result"]["iid"].as_u64()) != Some(*pr)
                || open["inputs"]["forge"] != serde_json::to_value(forge)? {
                return Err(Error::Invalid("merge identity differs from the recorded PR".into()));
            }
            super::forge::configured(forge, config)?;
            require_base(root, landing)?;
            super::forge::mutation(landing, inputs)
        }
    }
}

fn require_base(root: &Path, landing: &Landing) -> Result<()> {
    if remote_head(root, landing, &format!("refs/heads/{}", landing.base.branch))?.as_deref() != Some(&landing.base.head) {
        return Err(Error::Invalid("remote base differs from the authorized base commit".into()));
    }
    Ok(())
}

pub fn perform(root: &Path, invocation: &Invocation, authorization: &Authorize) -> Result<Value> {
    let output = run(root, invocation)?;
    match &authorization.inputs {
        ExternalInput::Push | ExternalInput::TagPush { .. } => Ok(json!({"output":output})),
        ExternalInput::Open { forge, .. } => {
            let result: Value = serde_json::from_str(&output)?;
            if result["number"].as_u64().or_else(|| result["iid"].as_u64()).is_none_or(|id| id == 0) {
                return Err(Error::Invalid("forge did not return a PR identity; reconciliation required".into()));
            }
            let (head, base) = if forge.provider == "gitlab" { (&result["sha"], &result["target_branch"]) }
                else { (&result["head"]["sha"], &result["base"]["ref"]) };
            if head != &authorization.source.head || base != &authorization.base.branch {
                return Err(Error::Invalid("created PR differs from the authorized source or base; reconciliation required".into()));
            }
            Ok(result)
        }
        ExternalInput::Merge { forge, .. } => {
            // Gitea's successful merge is HTTP 200 with an empty response body.
            if forge.provider == "forgejo" && output.trim().is_empty() { return Ok(json!({"merged":true})); }
            let result: Value = serde_json::from_str(&output)?;
            if result["merged"] != true && result["state"] != "merged" {
                return Err(Error::Invalid("forge did not confirm merge; reconciliation required".into()));
            }
            Ok(result)
        }
    }
}
