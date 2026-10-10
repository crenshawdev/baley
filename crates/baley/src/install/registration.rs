//! The MCP registration install requests from Claude Code. Claude Code is
//! the only writer of its own configuration file, so install reads the file,
//! judges whether the `mcpServers.baley` entry is Baley's, and requests the
//! `claude mcp` commands that place this binary's entry. The decision is pure:
//! it returns launches and never runs one.

use std::io;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::host_artifacts::compose::same_server;
use crate::host_artifacts::executable::Executable;
use crate::host_artifacts::registration::{self, KEY};
use crate::host_doctor::placed::{self, FileState};
use crate::process::{Launch, Output};

/// How long one `claude` command may run.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How much of each output stream is kept.
const LIMIT: usize = 64 * 1024;

/// What install does about the registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The entry is this binary's already, so nothing runs.
    Unchanged,
    /// The `claude` commands to run in order: the add alone, or the remove of
    /// Baley's older entry and then the add, since `claude mcp add-json`
    /// refuses a name that already exists in the scope.
    Run(Vec<Launch>),
}

/// This binary's entry: the stable path and `serve`.
pub fn entry(executable: &Executable) -> Value {
    registration::render(executable, false)["mcpServers"][KEY].clone()
}

fn claude(args: [&str; 5]) -> Launch {
    Launch::new("claude")
        .args(args)
        .own_group()
        .timeout(TIMEOUT)
        .limit(LIMIT)
}

/// The command that registers `entry` at user scope.
pub fn add_launch(entry: &Value) -> Launch {
    let compact = serde_json::to_string(entry).expect("a JSON entry serializes");
    claude(["mcp", "add-json", "--scope", "user", KEY]).arg(compact)
}

/// The command that removes the user-scope entry.
pub fn remove_launch() -> Launch {
    claude(["mcp", "remove", "--scope", "user", KEY])
}

fn unusable(path: &Path, cause: &str) -> String {
    format!(
        "install-settings-conflict: {} {cause}; fix it by hand, then run baley install again",
        path.display()
    )
}

/// Decides from what a read of Claude Code's configuration file found, the
/// latest install record and this binary's entry. A file that is not a JSON
/// object, or whose `mcpServers` is present but not an object, is refused as
/// a settings conflict. With no `mcpServers.baley` entry the add is
/// requested. An entry equal to this binary's by [`same_server`] is
/// unchanged, since Claude Code stores keys of its own such as `type`. An
/// entry equal by that rule to the one the record holds is Baley's older
/// registration and is removed and added again. Any other entry is not
/// Baley's and is refused as an ownership conflict.
pub fn judge(
    path: &Path,
    state: &FileState,
    latest: Option<&Value>,
    executable: &Executable,
) -> Result<Step, String> {
    let ours = entry(executable);
    let document = match state {
        FileState::Absent => return Ok(Step::Run(vec![add_launch(&ours)])),
        FileState::Fault(fault) => return Err(unusable(path, &fault.to_string())),
        FileState::Bytes(bytes) => {
            placed::document(bytes).map_err(|fault| unusable(path, &fault.to_string()))?
        }
    };
    let servers = match document.get("mcpServers") {
        None => return Ok(Step::Run(vec![add_launch(&ours)])),
        Some(Value::Object(servers)) => servers,
        Some(_) => {
            return Err(unusable(
                path,
                "holds mcpServers that is not a JSON object, so no entry can be judged in it",
            ));
        }
    };
    let Some(found) = servers.get(KEY) else {
        return Ok(Step::Run(vec![add_launch(&ours)]));
    };
    if same_server(found, &ours) {
        return Ok(Step::Unchanged);
    }
    let recorded = latest.and_then(|latest| latest.pointer("/registered/registration/entry"));
    if recorded.is_some_and(|recorded| same_server(found, recorded)) {
        return Ok(Step::Run(vec![remove_launch(), add_launch(&ours)]));
    }
    Err(format!(
        "install-ownership-conflict: mcpServers.baley in {} is not an entry Baley wrote: {found}; remove it with `claude mcp remove --scope user baley` if it is yours to remove, then run `baley install` again",
        path.display()
    ))
}

/// The entry a read found under `mcpServers.baley`, when the file reads as a
/// JSON object that holds one.
pub fn held(state: &FileState) -> Option<Value> {
    let FileState::Bytes(bytes) = state else {
        return None;
    };
    placed::document(bytes)
        .ok()?
        .pointer(&format!("/mcpServers/{KEY}"))
        .cloned()
}

/// What the registration step did during a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// The entry was Baley's already, so nothing ran.
    Unchanged,
    /// `claude` added this binary's entry.
    Registered,
    /// `claude` removed Baley's older entry and added this binary's.
    Replaced,
    /// The file read just before a remove no longer showed an entry Baley
    /// may replace, so nothing ran. The refusal's own text.
    Refused(String),
    /// A `claude` command failed, as a `not-writable` line naming the file.
    Failed(String),
    /// The step was not reached, for the cause.
    NotReached(String),
}

/// The registration step's result with the evidence the record needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The `mcpServers.baley` entry the latest read of the file held: the
    /// read before the file writes, or the read just before the remove.
    pub seen: Option<Value>,
    /// Whether a remove launch succeeded after that read.
    pub removed: bool,
    /// What the step did.
    pub result: Registered,
}

/// Reads one launch's result. Exit status 0 is done. Another exit status
/// gives its first line of standard error, or the status when that is empty.
/// A signal, a timeout and a start error each give their cause. A failure is
/// a `not-writable` line naming the registration file.
pub fn interpret(path: &Path, result: io::Result<Output>) -> Result<(), String> {
    let cause = match result {
        Ok(output) if output.success() => return Ok(()),
        Ok(output) => match (output.code(), output.signal()) {
            (Some(code), _) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                match stderr.lines().next() {
                    Some(first) if !first.trim().is_empty() => first.trim().to_owned(),
                    _ => format!("exit status {code}"),
                }
            }
            (None, Some(signal)) => format!("signal {signal}"),
            (None, None) => "process ended without an exit status".to_owned(),
        },
        Err(error) if error.kind() == io::ErrorKind::TimedOut => format!("timed out: {error}"),
        Err(error) => format!("could not start claude: {error}"),
    };
    Err(format!("not-writable: {}: {cause}", path.display()))
}

/// What to do after the file is read again just before a remove launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Run these launches.
    Run(Vec<Launch>),
    /// The entry is this binary's now, so nothing runs.
    Unchanged,
    /// The entry is not one Baley may replace, so nothing runs.
    Refused(String),
}

/// The second look and the entry it saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recheck {
    /// The entry the second read held.
    pub seen: Option<Value>,
    /// The new answer, from the same decision the plan used.
    pub next: Next,
}

/// Judges the second read. A remove deletes an entry, so it never runs on
/// evidence read before the file writes: the entry may have changed since.
pub fn recheck(
    path: &Path,
    reread: &FileState,
    latest: Option<&Value>,
    executable: &Executable,
) -> Recheck {
    let next = match judge(path, reread, latest, executable) {
        Ok(Step::Run(launches)) => Next::Run(launches),
        Ok(Step::Unchanged) => Next::Unchanged,
        Err(refusal) => Next::Refused(refusal),
    };
    Recheck {
        seen: held(reread),
        next,
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The command an owner can run by hand to place the entry, with the entry
/// single-quoted for a POSIX shell.
pub fn hand_command(entry: &Value) -> String {
    let compact = serde_json::to_string(entry).expect("a JSON entry serializes");
    format!(
        "claude mcp add-json --scope user {KEY} {}",
        shell_quote(&compact)
    )
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;
    use crate::host_artifacts::executable::Executable;
    use crate::host_doctor::placed::{Fault, FileState};
    use crate::process::{Launch, Output, StdioPlan, Stream, stdio_plan};

    const PATH: &str = "/home/o/.claude.json";
    const EXECUTABLE: &str = "/home/o/.local/bin/baley";

    fn executable() -> Executable {
        Executable::new(EXECUTABLE).unwrap()
    }

    fn file(document: Value) -> FileState {
        FileState::Bytes(serde_json::to_vec(&document).unwrap())
    }

    fn older() -> Value {
        json!({"command": EXECUTABLE, "args": ["serve", "--old"]})
    }

    fn record(entry: &Value) -> Value {
        json!({"registered": {"registration": {"path": PATH, "entry": entry}}})
    }

    fn decide(state: &FileState, latest: Option<&Value>) -> Result<Step, String> {
        judge(Path::new(PATH), state, latest, &executable())
    }

    fn launches_of(step: Result<Step, String>) -> Vec<Launch> {
        match step.expect("no refusal") {
            Step::Run(launches) => launches,
            Step::Unchanged => panic!("a launch was expected"),
        }
    }

    fn args(launch: &Launch) -> Vec<&str> {
        launch
            .args
            .iter()
            .map(|arg| arg.to_str().unwrap())
            .collect()
    }

    fn add_args() -> [&'static str; 5] {
        ["mcp", "add-json", "--scope", "user", "baley"]
    }

    #[test]
    fn a_registration_off_user_scope_or_without_the_stable_path_is_caught() {
        let launches = launches_of(decide(&FileState::Absent, None));

        assert_eq!(launches.len(), 1);
        let launch = &launches[0];
        assert_eq!(launch.program, "claude");
        let arguments = args(launch);
        assert_eq!(arguments.len(), 6, "{arguments:?}");
        assert_eq!(arguments[..5], add_args());
        let entry: Value = serde_json::from_str(arguments[5]).expect("the sixth is JSON");
        assert_eq!(entry, json!({"command": EXECUTABLE, "args": ["serve"]}));
    }

    #[test]
    fn a_registration_replaced_without_record_evidence_is_caught() {
        let remove = ["mcp", "remove", "--scope", "user", "baley"];
        let recorded = record(&older());

        let held = file(json!({"mcpServers": {"baley": older()}}));
        let launches = launches_of(decide(&held, Some(&recorded)));
        assert_eq!(launches.len(), 2);
        assert_eq!(args(&launches[0]), remove);
        assert_eq!(args(&launches[1])[..5], add_args());

        let refusal = decide(&held, None).expect_err("no record, no replacement");
        assert!(
            refusal.starts_with("install-ownership-conflict: "),
            "{refusal}"
        );
        assert!(refusal.contains("mcpServers.baley"), "{refusal}");
        assert!(refusal.contains(PATH), "{refusal}");

        let stored = file(json!({"mcpServers": {"baley": {
            "type": "stdio",
            "command": EXECUTABLE,
            "args": ["serve", "--old"],
            "env": {},
        }}}));
        let launches = launches_of(decide(&stored, Some(&recorded)));
        assert_eq!(launches.len(), 2);
        assert_eq!(args(&launches[0]), remove);
        assert_eq!(args(&launches[1])[..5], add_args());
    }

    #[test]
    fn an_equal_registration_registered_again_is_caught() {
        let stored = |env: Value| {
            file(json!({"mcpServers": {"baley": {
                "type": "stdio",
                "command": EXECUTABLE,
                "args": ["serve"],
                "env": env,
            }}}))
        };

        assert_eq!(decide(&stored(json!({})), None), Ok(Step::Unchanged));

        let refusal = decide(&stored(json!({"BALEY_HOME": "/elsewhere"})), None)
            .expect_err("an entry that sets environment entries is not Baley's");
        assert!(
            refusal.starts_with("install-ownership-conflict: "),
            "{refusal}"
        );
        assert!(refusal.contains("mcpServers.baley"), "{refusal}");
        assert!(refusal.contains(PATH), "{refusal}");
    }

    #[test]
    fn a_claude_json_that_cannot_be_read_taken_as_no_registration_is_caught() {
        for state in [
            FileState::Bytes(br#"{"mcpServers":"#.to_vec()),
            FileState::Fault(Fault::NotRegular),
        ] {
            let refusal = decide(&state, None).expect_err("an unreadable file is refused");
            assert!(
                refusal.starts_with("install-settings-conflict: "),
                "{refusal}"
            );
            assert!(refusal.contains(PATH), "{refusal}");
        }

        let launches = launches_of(decide(&file(json!({"projects": {}})), None));
        assert_eq!(args(&launches[0])[..5], add_args());

        for servers in [json!([]), json!("x")] {
            let refusal = decide(&file(json!({"mcpServers": servers})), None)
                .expect_err("a malformed mcpServers is refused");
            assert!(
                refusal.starts_with("install-settings-conflict: "),
                "{refusal}"
            );
            assert!(refusal.contains(PATH), "{refusal}");
            assert!(refusal.contains("mcpServers"), "{refusal}");
        }
    }

    #[test]
    fn a_registration_launch_that_could_hang_or_flood_is_caught() {
        let held = file(json!({"mcpServers": {"baley": older()}}));
        let recorded = record(&older());
        for launches in [
            launches_of(decide(&FileState::Absent, None)),
            launches_of(decide(&held, Some(&recorded))),
        ] {
            for launch in &launches {
                assert!(launch.own_group);
                assert_eq!(launch.timeout, Some(Duration::from_secs(30)));
                assert_eq!(launch.limit, 64 * 1024);
                assert!(launch.env.is_empty());
                assert_eq!(
                    stdio_plan(launch),
                    StdioPlan {
                        stdin: Stream::Null,
                        stdout: Stream::Piped,
                        stderr: Stream::Piped,
                    }
                );
            }
        }
    }
    #[test]
    fn a_failed_claude_command_reported_as_registered_is_caught() {
        let path = Path::new(PATH);
        assert_eq!(interpret(path, Ok(Output::exited(0, "", ""))), Ok(()));

        for (result, cause) in [
            (
                Ok(Output::exited(
                    1,
                    "",
                    "MCP server baley already exists in user config\nsecond line\n",
                )),
                "MCP server baley already exists in user config",
            ),
            (Ok(Output::exited(2, "", "")), "exit status 2"),
            (Ok(Output::signaled(9)), "signal 9"),
            (
                Err(io::Error::new(io::ErrorKind::TimedOut, "deadline reached")),
                "timed out",
            ),
            (
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "No such file or directory",
                )),
                "could not start claude: No such file or directory",
            ),
        ] {
            let line = interpret(path, result).expect_err("a failed command is a failure");
            assert!(line.starts_with("not-writable: "), "{line}");
            assert!(line.contains(PATH), "{line}");
            assert!(line.contains(cause), "{line}");
            assert!(!line.contains("second line"), "{line}");
        }
    }

    #[test]
    fn a_registration_removed_on_stale_ownership_evidence_is_caught() {
        let latest = record(&older());
        let look = |document: Value| {
            recheck(
                Path::new(PATH),
                &file(document),
                Some(&latest),
                &executable(),
            )
        };

        let foreign = json!({"command": "/opt/other/baley", "args": ["serve"]});
        let first = look(json!({"mcpServers": {"baley": foreign}}));
        assert_eq!(first.seen, Some(foreign));
        let Next::Refused(line) = first.next else {
            panic!("a foreign entry must run nothing");
        };
        assert!(line.starts_with("install-ownership-conflict: "), "{line}");
        assert!(line.contains("mcpServers.baley"), "{line}");
        assert!(line.contains(PATH), "{line}");

        let Next::Run(launches) = look(json!({"mcpServers": {"baley": older()}})).next else {
            panic!("the recorded older entry is replaced");
        };
        assert_eq!(launches.len(), 2);
        assert_eq!(
            args(&launches[0]),
            ["mcp", "remove", "--scope", "user", "baley"]
        );
        assert_eq!(args(&launches[1])[..5], add_args());

        let ours = json!({"command": EXECUTABLE, "args": ["serve"]});
        assert_eq!(
            look(json!({"mcpServers": {"baley": ours}})).next,
            Next::Unchanged
        );

        let Next::Run(launches) = look(json!({"mcpServers": {}})).next else {
            panic!("an absent entry is added");
        };
        assert_eq!(launches.len(), 1);
        assert_eq!(args(&launches[0])[..5], add_args());
    }
}
