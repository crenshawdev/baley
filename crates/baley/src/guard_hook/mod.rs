//! The guard hook (design 0010): one host tool call read from stdin, judged
//! with the guard's rules, recorded in the per-user `user` project when it
//! must be, and answered in the host's pre-tool hook form.
//!
//! Gathering and judging are kept apart. A gatherer reads the input, the
//! environment, the filesystem, git or the store and owns no policy, so it
//! has no unit test. A judge takes plain values and is tested directly.

mod branch;
mod context;
mod decide;
mod policy;
mod record;
mod render;
mod unrecordable;

pub use render::{Rendered, failed_write, render};

use crate::guard_budget::Budget;
use crate::hook_input::{self, HookInput, MAX_INPUT_BYTES};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::open;
use crate::process::System;
use crate::protected_paths::Disk;
use baley_core::guard::{Answer, AuditPrecondition, Redelivery, record_answer};
use baley_store::StoreError;
use baley_store_sqlite::SqliteStore;
use decide::{Next, Seen, Step};
use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;
use unrecordable::{not_recorded, store_failure};

/// Runs `baley guard`: reads one Claude Code pre-tool hook call from stdin,
/// judges it, records the answer when it must and writes it. It is the
/// hook's gatherer and has no unit test.
///
/// The budget starts before stdin is read, since the host's timeout counts
/// from the start. A bound command or PowerShell call first looks for the
/// answer recorded under its call id, on a short store open of its own,
/// and a replay is answered with no git or policy read. Only a bound commit
/// then reads a branch and a policy, in that order. Any answer selected for
/// recording takes a second short open for the audit transaction. Without
/// Baley's home folder or the call's identity, after a call id clash, or on
/// any store failure the decision is unrecordable, which a loud line says:
/// an ask becomes a deny.
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
    let envelope = match &input {
        HookInput::Command { envelope, .. }
        | HookInput::Path { envelope, .. }
        | HookInput::Watch(envelope)
        | HookInput::PowerShell(envelope) => Some(envelope),
        // Its answer never reads the context and is never recorded.
        HookInput::Deny(_) | HookInput::NoAnswer => None,
    };
    let cwd = envelope.map_or("", |envelope| envelope.cwd.as_str());
    let context = context::gather(cwd);
    // Who records, and where, or why nothing can be.
    let recorder = match (envelope, &context.protected) {
        (_, Err(refusal)) => Err(not_recorded(&format!(
            "Baley's home folder cannot be found ({refusal})"
        ))),
        (None, Ok(_)) => Err(not_recorded("the call carries no envelope")),
        (Some(envelope), Ok(paths)) => unrecordable::caller(envelope, &context.project_directory)
            .map(|caller| (caller, paths.home.as_path())),
    };
    // Set once the lookup makes the decision unrecordable.
    let mut failed = None;
    let mut seen = Seen::default();
    let decided = loop {
        match decide::next(&input, &context, &seen, &Disk) {
            Next::Decided(decided) => break *decided,
            Next::Do(Step::Lookup(digest)) => {
                seen.looked_up = true;
                let found = match &recorder {
                    Err(line) => Err(line.clone()),
                    Ok((caller, home)) => with_store(home, &mut budget, |store, _| {
                        record::lookup(store, caller, &digest)
                    })
                    .map_err(|error| store_failure(&error).1),
                };
                match found {
                    Ok(Redelivery::NoRecord) => {}
                    Ok(Redelivery::Replay(answer)) => {
                        return write(record_answer(answer, AuditPrecondition::Recorded));
                    }
                    Ok(Redelivery::Clash) => failed = Some(unrecordable::clash()),
                    Err(line) => failed = Some(line),
                }
            }
            Next::Do(Step::Branch) => {
                let at = Path::new(cwd);
                let git = branch::git_branch(at, &mut System, &mut budget);
                seen.branch = Some(branch::branch(git, branch::symbolic_head(at)));
            }
            Next::Do(Step::Policy(project)) => {
                let config = context
                    .protected
                    .as_ref()
                    .map(|paths| paths.config.as_path());
                let read = policy::gather(config, &project, &mut System, &mut budget);
                let (settings, excerpt) = policy::settings(read);
                seen.settings = Some(settings);
                seen.excerpt = excerpt;
            }
        }
    };
    let Some(selected) = &decided.record else {
        return write(decided.answer);
    };
    let recorded = match (failed, &recorder) {
        (Some(line), _) => Err(line),
        (None, Err(line)) => Err(line.clone()),
        (None, Ok((caller, home))) => {
            record::judged(decided.answer.clone(), selected, &seen, &context).and_then(|judged| {
                with_store(home, &mut budget, |store, at| {
                    record::record(store, caller, selected, &judged, new_request_id(), at)
                })
                .map_err(|error| store_failure(&error).1)
            })
        }
    };
    let (answer, precondition) = match recorded {
        Ok((answer, AuditPrecondition::Unrecordable)) => {
            loud(&unrecordable::clash());
            (answer, AuditPrecondition::Unrecordable)
        }
        Ok(recorded) => recorded,
        Err(line) => {
            loud(&line);
            (decided.answer, AuditPrecondition::Unrecordable)
        }
    };
    write(record_answer(answer, precondition))
}

/// Opens the guard store on the storage time the budget has left, runs
/// `work` on it with the time it was opened at, and charges the budget with
/// the time both took. Once that time is spent the store tries nothing and
/// answers busy.
fn with_store<T>(
    home: &Path,
    budget: &mut Budget,
    work: impl FnOnce(&SqliteStore, &str) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let started = Instant::now();
    let at = SystemClock::now();
    let result = open::store(home, &at, open::guard_options(budget.storage()))
        .and_then(|store| work(&store, &at));
    budget.charge_storage(started.elapsed());
    result
}

/// Writes one loud line to stderr.
fn loud(line: &str) {
    let _ = std::io::stderr().write_all(format!("{line}\n").as_bytes());
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
