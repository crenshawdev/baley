//! The guard hook (design 0010): one host tool call read from stdin, judged
//! with the guard's rules and answered in the host's pre-tool hook form.
//!
//! Gathering and judging are kept apart. A gatherer reads the input, the
//! environment, the filesystem or git and owns no policy, so it has no unit
//! test. A judge takes plain values and is tested directly.

mod branch;
mod context;
mod decide;
mod policy;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Build 3 T10's recording step in run is its caller"
    )
)]
mod record;
mod render;

pub use render::{Rendered, failed_write, render};

use crate::guard_budget::Budget;
use crate::hook_input::{self, HookInput, MAX_INPUT_BYTES};
use crate::process::System;
use crate::protected_paths::Disk;
use baley_core::guard::{Answer, AuditPrecondition, record_answer};
use decide::{Next, Seen, Step};
use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

/// Runs `baley guard`: reads one Claude Code pre-tool hook call from stdin,
/// judges it and writes the answer. It is the hook's gatherer and has no
/// unit test.
///
/// The budget starts before stdin is read, since the host's timeout counts
/// from the start. Only a bound commit reads a branch and a policy, in that
/// order. No answer is recorded yet, so every answer selected for recording
/// meets the store as unrecordable: an ask becomes a deny.
pub fn run() -> ExitCode {
    let mut budget = Budget::start();
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin()
        .lock()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
    {
        // The tool is unknown, and a path tool must not pass.
        return write(Answer::Deny(format!(
            "Baley guard: the hook input cannot be read ({error}), so the call is refused"
        )));
    }
    let input = hook_input::classify(&bytes);
    let cwd = match &input {
        HookInput::Command { envelope, .. }
        | HookInput::Path { envelope, .. }
        | HookInput::Watch(envelope)
        | HookInput::PowerShell(envelope) => envelope.cwd.clone(),
        // Its answer never reads the context.
        HookInput::Deny(_) | HookInput::NoAnswer => String::new(),
    };
    let context = context::gather(&cwd);
    let mut seen = Seen::default();
    let decided = loop {
        match decide::next(&input, &context, &seen, &Disk) {
            Next::Decided(decided) => break *decided,
            Next::Do(Step::Branch) => {
                let at = Path::new(&cwd);
                let git = branch::git_branch(at, &mut System, &mut budget);
                seen.branch = Some(branch::branch(git, branch::symbolic_head(at)));
            }
            Next::Do(Step::Policy(project)) => {
                let config = context
                    .protected
                    .as_ref()
                    .map(|paths| paths.config.as_path());
                let read = policy::gather(config, &project, &mut System, &mut budget);
                seen.settings = Some(policy::settings(read).0);
            }
        }
    };
    let answer = match decided.record {
        Some(_) => record_answer(decided.answer, AuditPrecondition::Unrecordable),
        None => decided.answer,
    };
    write(answer)
}

/// Writes the rendered answer. When an ask or deny cannot be written to
/// stdout, the blocking exit carries its reason instead.
fn write(answer: Answer) -> ExitCode {
    let rendered = render(&answer);
    if !rendered.stdout.is_empty() {
        let mut stdout = std::io::stdout().lock();
        let written = stdout
            .write_all(rendered.stdout.as_bytes())
            .and_then(|()| stdout.flush());
        if written.is_err() {
            let reason = match &answer {
                Answer::Ask(reason) | Answer::Deny(reason) => reason.as_str(),
                Answer::Pass | Answer::PassOnFailure(_) => "",
            };
            let fallback = failed_write(reason);
            let _ = std::io::stderr().write_all(fallback.stderr.as_bytes());
            return ExitCode::from(fallback.exit);
        }
    }
    if !rendered.stderr.is_empty() {
        let _ = std::io::stderr().write_all(rendered.stderr.as_bytes());
    }
    ExitCode::from(rendered.exit)
}
