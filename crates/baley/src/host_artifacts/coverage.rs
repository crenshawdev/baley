//! The coverage judge: what each of the nine guarded tools can read and
//! write in Baley's home and config folder, and whether each write-only file
//! is kept from writes, judged from a settings document and a hook document
//! (D-16; design 0010, GRD-R13; design 0001, EVD-R24). Build 3 T13's doctor
//! runs it over the documents it reads.
//!
//! The judge reads the two documents and the supplied values and nothing
//! else. It never assumes coverage from what the proposal would render, so a
//! composed document that turned the sandbox off reads as a gap. Its report
//! is about configuration, not enforcement: T12 measures what Claude Code
//! does with these settings, and T13 observes the machine.
//!
//! Three mechanisms carry the verdicts (ADR 0033):
//! - the sandbox, for Bash, Monitor and PowerShell commands and the
//!   processes they start;
//! - the `Read` deny rules and the guard hook, for Read, Grep and Glob;
//! - the `Edit` deny rules and the guard hook, for Write, Edit and
//!   NotebookEdit.
//!
//! The judge is stricter than the host, so it can report a gap Claude Code
//! would close but never reads as covered what the host leaves open:
//! - a list entry or rule counts only in the spelling `security` renders,
//!   although Claude Code also accepts `~/` paths and a trailing `/` or `/**`
//!   on a list entry, and copies `Read` and `Edit` deny rules into the
//!   sandbox lists;
//! - a hook matcher that Claude Code evaluates as a regular expression names
//!   no tool here;
//! - a hook counts as the guard only when it is the command hook `hook`
//!   renders, with no field that narrows, detaches or reshapes it and no
//!   timeout below the guard's budget;
//! - an `allowWrite` entry inside a folder is a gap, although Claude Code
//!   keeps a `denyWrite` entry inside an otherwise writable path. A narrower
//!   `allowRead` entry does re-open a `denyRead` folder.
//!
//! Host facts the judge depends on, read on 2026-10-07 from Claude Code
//! 2.1.293's bundle:
//! - A command listed in `sandbox.excludedCommands` runs outside the sandbox
//!   even when `allowUnsandboxedCommands` is false, and the host's own
//!   settings check names any non-empty list as exempting commands. So
//!   every entry, whatever it names, voids each verdict the sandbox carries.
//!
//! One gap runs the other way and is written down here: an `allowRead` or
//! `allowWrite` entry is judged only when it is absolute. A `~/` entry needs
//! the owner's home folder and a relative one the settings file's place, and
//! neither is an input yet, so such an entry pointing into a folder is not
//! caught. `baley install` knows both when it composes (Build 3 T15) and
//! adds them at [`Inputs`].

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::executable::Executable;
use super::hook;
use super::security::{Unrendered, edit_file_rule, edit_folder_rule, read_folder_rule, rule_path};
use crate::folders::Folders;
use crate::guard_budget::HOST_TIMEOUT;

/// A tool the guard hook's matcher names (GRD-R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Runs a shell command.
    Bash,
    /// Runs a command and watches its output.
    Monitor,
    /// Runs a PowerShell command.
    PowerShell,
    /// Reads a file.
    Read,
    /// Searches file contents.
    Grep,
    /// Lists paths by pattern.
    Glob,
    /// Writes a file.
    Write,
    /// Edits a file.
    Edit,
    /// Edits a notebook cell.
    NotebookEdit,
}

impl Tool {
    /// Every guarded tool, in the hook matcher's order.
    pub const ALL: [Tool; 9] = [
        Tool::Bash,
        Tool::Monitor,
        Tool::PowerShell,
        Tool::Read,
        Tool::Grep,
        Tool::Glob,
        Tool::Write,
        Tool::Edit,
        Tool::NotebookEdit,
    ];

    /// The name Claude Code gives the tool in a hook's input and matcher.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Bash => "Bash",
            Tool::Monitor => "Monitor",
            Tool::PowerShell => "PowerShell",
            Tool::Read => "Read",
            Tool::Grep => "Grep",
            Tool::Glob => "Glob",
            Tool::Write => "Write",
            Tool::Edit => "Edit",
            Tool::NotebookEdit => "NotebookEdit",
        }
    }

    /// The accesses judged for the tool: a command can read and write, the
    /// file readers only read and the file editors only write.
    pub fn accesses(self) -> &'static [Access] {
        match self {
            Tool::Bash | Tool::Monitor | Tool::PowerShell => &[Access::Read, Access::Write],
            Tool::Read | Tool::Grep | Tool::Glob => &[Access::Read],
            Tool::Write | Tool::Edit | Tool::NotebookEdit => &[Access::Write],
        }
    }
}

/// A read or a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reading a path.
    Read,
    /// Writing a path.
    Write,
}

/// A mechanism that carries a verdict, and that a caller can mark
/// unsupported on a machine (D-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// Claude Code's sandbox around commands.
    Sandbox,
    /// The `Read` and `Edit` deny rules.
    PermissionRules,
    /// The guard hook.
    Hook,
}

/// Why a tool's access, or a file, is not covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The caller marked the mechanism unsupported.
    Unsupported(Mechanism),
    /// A sandbox setting is missing or off its secure value: `enabled`,
    /// `failIfUnavailable`, `allowUnsandboxedCommands` or
    /// `filesystem.disabled`.
    SandboxSetting(&'static str),
    /// A `sandbox.excludedCommands` entry, as written. The host runs a
    /// listed command outside the sandbox even with
    /// `allowUnsandboxedCommands` false, and that program can reach any
    /// path, so no entry is safe by what it names.
    Excluded(String),
    /// A sandbox deny list (`denyRead` or `denyWrite`) does not name the
    /// path.
    NotDenied {
        /// The list.
        list: &'static str,
        /// The folder or file it should name.
        path: String,
    },
    /// An `allowRead` or `allowWrite` entry equals or lies inside the path,
    /// so it re-opens what the deny list closed.
    Reopened {
        /// The list.
        list: &'static str,
        /// The entry as written.
        entry: String,
    },
    /// `permissions.deny` lacks this rule.
    NoRule(String),
    /// No `PreToolUse` item runs the guard for the tool.
    NoGuard,
    /// `disableAllHooks` is true, so no hook runs.
    HooksDisabled,
    /// A supplied path cannot be written as a rule.
    Unrendered(Unrendered),
}

/// The verdict for one tool's access or one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every mechanism that carries the verdict is in place.
    Covered {
        /// The mechanisms, all of which hold.
        by: &'static [Mechanism],
        /// True for Grep and Glob: Claude Code applies `Read` rules to them
        /// only on a best-effort basis, and the guard refuses the rest.
        best_effort: bool,
    },
    /// Not covered, for every cause found.
    Gap(Vec<Cause>),
}

/// One tool's verdict for one access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolVerdict {
    /// The tool.
    pub tool: Tool,
    /// The access judged.
    pub access: Access,
    /// The verdict.
    pub verdict: Verdict,
}

/// One write-only file's verdict. Only writes are judged: these files are
/// readable by design (D-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVerdict {
    /// The file as supplied.
    pub path: PathBuf,
    /// Whether sandboxed commands and the file tools are kept from writing
    /// it.
    pub write: Verdict,
}

/// The judge's report. It covers the two folders and exactly the files it
/// was given: it promises nothing for a checkout whose `baley.toml` is not in
/// the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Every guarded tool's verdict for each access it has.
    pub tools: Vec<ToolVerdict>,
    /// Each supplied write-only file, then the executable.
    pub files: Vec<FileVerdict>,
}

impl Coverage {
    /// The verdict for one tool and access, if the tool has that access.
    pub fn verdict(&self, tool: Tool, access: Access) -> Option<&Verdict> {
        self.tools
            .iter()
            .find(|judged| judged.tool == tool && judged.access == access)
            .map(|judged| &judged.verdict)
    }
}

/// What the judge reads.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The settings document holding `sandbox` and `permissions`.
    pub settings: &'a Value,
    /// The document holding `hooks`; the same value as `settings` when one
    /// file holds both.
    pub hook: &'a Value,
    /// Baley's resolved folders.
    pub folders: &'a Folders,
    /// The executable the guard hook must run.
    pub executable: &'a Executable,
    /// The write-only files: the `baley.toml` paths the caller names and the
    /// placement map's protected list.
    pub write_only: &'a [PathBuf],
    /// The mechanisms the caller marks unsupported on this machine.
    pub unsupported: &'a [Mechanism],
}

const SANDBOX: &[Mechanism] = &[Mechanism::Sandbox];
const RULES_AND_GUARD: &[Mechanism] = &[Mechanism::PermissionRules, Mechanism::Hook];
const SANDBOX_AND_RULES: &[Mechanism] = &[Mechanism::Sandbox, Mechanism::PermissionRules];

/// Judges every tool's access to both folders and every write-only file's
/// writes, from the documents alone.
pub fn judge(inputs: &Inputs<'_>) -> Coverage {
    let mut folders: Vec<Result<String, Unrendered>> = Vec::new();
    for folder in [&inputs.folders.home, &inputs.folders.config] {
        let judged = rule_path(folder);
        if !folders.contains(&judged) {
            folders.push(judged);
        }
    }
    let sandbox = sandbox_settings(inputs);

    let mut tools = Vec::new();
    for tool in Tool::ALL {
        for &access in tool.accesses() {
            let (by, causes) = match tool {
                Tool::Bash | Tool::Monitor | Tool::PowerShell => {
                    let mut causes = sandbox.clone();
                    for folder in &folders {
                        match folder {
                            Ok(path) => causes.extend(sandbox_path(inputs.settings, access, path)),
                            Err(report) => causes.push(Cause::Unrendered(report.clone())),
                        }
                    }
                    (SANDBOX, causes)
                }
                Tool::Read | Tool::Grep | Tool::Glob => {
                    let mut causes = rules(inputs, &folders, read_folder_rule);
                    causes.extend(guard(inputs, tool));
                    (RULES_AND_GUARD, causes)
                }
                Tool::Write | Tool::Edit | Tool::NotebookEdit => {
                    let mut causes = rules(inputs, &folders, edit_folder_rule);
                    causes.extend(guard(inputs, tool));
                    (RULES_AND_GUARD, causes)
                }
            };
            let best_effort = matches!(tool, Tool::Grep | Tool::Glob);
            tools.push(ToolVerdict {
                tool,
                access,
                verdict: verdict(by, best_effort, causes),
            });
        }
    }

    let mut paths: Vec<&Path> = Vec::new();
    let supplied = inputs.write_only.iter().map(PathBuf::as_path);
    for path in supplied.chain([Path::new(inputs.executable.as_str())]) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    let files = paths
        .into_iter()
        .map(|path| {
            let causes = match rule_path(path) {
                Ok(text) => {
                    let mut causes = sandbox.clone();
                    causes.extend(sandbox_path(inputs.settings, Access::Write, &text));
                    causes.extend(rules(inputs, &[Ok(text)], edit_file_rule));
                    causes
                }
                Err(report) => vec![Cause::Unrendered(report)],
            };
            FileVerdict {
                path: path.to_path_buf(),
                write: verdict(SANDBOX_AND_RULES, false, causes),
            }
        })
        .collect();

    Coverage { tools, files }
}

fn verdict(by: &'static [Mechanism], best_effort: bool, causes: Vec<Cause>) -> Verdict {
    if causes.is_empty() {
        Verdict::Covered { by, best_effort }
    } else {
        Verdict::Gap(causes)
    }
}

/// The string items of the array at `pointer`, or none.
fn strings<'v>(document: &'v Value, pointer: &str) -> Vec<&'v str> {
    document
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The causes that void every sandbox entry, whatever path it names.
fn sandbox_settings(inputs: &Inputs<'_>) -> Vec<Cause> {
    let mut causes = Vec::new();
    if inputs.unsupported.contains(&Mechanism::Sandbox) {
        causes.push(Cause::Unsupported(Mechanism::Sandbox));
    }
    for (key, secure) in [
        ("enabled", true),
        ("failIfUnavailable", true),
        ("allowUnsandboxedCommands", false),
    ] {
        let pointer = format!("/sandbox/{key}");
        if inputs.settings.pointer(&pointer) != Some(&Value::Bool(secure)) {
            causes.push(Cause::SandboxSetting(key));
        }
    }
    if inputs.settings.pointer("/sandbox/filesystem/disabled") == Some(&Value::Bool(true)) {
        causes.push(Cause::SandboxSetting("filesystem.disabled"));
    }
    for entry in strings(inputs.settings, "/sandbox/excludedCommands") {
        causes.push(Cause::Excluded(entry.to_owned()));
    }
    causes
}

/// The causes that keep the sandbox from denying `access` to one path.
fn sandbox_path(settings: &Value, access: Access, path: &str) -> Vec<Cause> {
    let (deny, allow) = match access {
        Access::Read => ("denyRead", "allowRead"),
        Access::Write => ("denyWrite", "allowWrite"),
    };
    let mut causes = Vec::new();
    if !strings(settings, &format!("/sandbox/filesystem/{deny}")).contains(&path) {
        causes.push(Cause::NotDenied {
            list: deny,
            path: path.to_owned(),
        });
    }
    for entry in strings(settings, &format!("/sandbox/filesystem/{allow}")) {
        if lies_within(entry, path) {
            causes.push(Cause::Reopened {
                list: allow,
                entry: entry.to_owned(),
            });
        }
    }
    causes
}

/// The causes that keep `permissions.deny` from holding `rule` for each path.
fn rules(
    inputs: &Inputs<'_>,
    paths: &[Result<String, Unrendered>],
    rule: fn(&str) -> String,
) -> Vec<Cause> {
    let mut causes = Vec::new();
    if inputs.unsupported.contains(&Mechanism::PermissionRules) {
        causes.push(Cause::Unsupported(Mechanism::PermissionRules));
    }
    let deny = strings(inputs.settings, "/permissions/deny");
    for path in paths {
        match path {
            Ok(path) => {
                let wanted = rule(path);
                if !deny.contains(&wanted.as_str()) {
                    causes.push(Cause::NoRule(wanted));
                }
            }
            Err(report) => causes.push(Cause::Unrendered(report.clone())),
        }
    }
    causes
}

/// The causes that keep the guard from running before `tool`.
fn guard(inputs: &Inputs<'_>, tool: Tool) -> Vec<Cause> {
    let mut causes = Vec::new();
    if inputs.unsupported.contains(&Mechanism::Hook) {
        causes.push(Cause::Unsupported(Mechanism::Hook));
    }
    let disabled = [inputs.settings, inputs.hook]
        .iter()
        .any(|document| document.get("disableAllHooks") == Some(&Value::Bool(true)));
    if disabled {
        causes.push(Cause::HooksDisabled);
    }
    let command = hook::command(inputs.executable);
    let found = inputs
        .hook
        .pointer(&format!("/hooks/{}", hook::EVENT))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| {
            matcher_names(item.get("matcher"), tool)
                && item
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| is_guard(handler, &command))
                    })
        });
    if !found {
        causes.push(Cause::NoGuard);
    }
    causes
}

/// Whether a matcher names the tool. Claude Code matches everything for an
/// absent, empty or `*` matcher, and reads one holding only letters, digits,
/// `_`, `-`, spaces, `,` and `|` as a list of exact names. Anything else is
/// a regular expression, which this judge does not evaluate.
fn matcher_names(matcher: Option<&Value>, tool: Tool) -> bool {
    let text = match matcher {
        None => return true,
        Some(Value::String(text)) => text.as_str(),
        Some(_) => return false,
    };
    if text.is_empty() || text == "*" {
        return true;
    }
    let exact = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ' | ',' | '|'));
    exact
        && text
            .split(['|', ','])
            .any(|name| name.trim() == tool.name())
}

/// Whether a handler is the guard: a command hook running exactly `command`
/// through bash, with no field that narrows it (`if`), detaches it
/// (`async`), reshapes it (`args`) or is unknown, and no timeout below the
/// guard's budget, since a cut-off hook lets the call run.
fn is_guard(handler: &Value, command: &str) -> bool {
    let Some(fields) = handler.as_object() else {
        return false;
    };
    let known = fields.keys().all(|key| {
        matches!(
            key.as_str(),
            "type" | "command" | "timeout" | "statusMessage" | "shell"
        )
    });
    let timeout = fields.get("timeout").is_none_or(|value| {
        value
            .as_f64()
            .is_some_and(|secs| secs >= HOST_TIMEOUT.as_secs_f64())
    });
    let shell = fields.get("shell").is_none_or(|value| value == "bash");
    known
        && fields.get("type").is_some_and(|value| value == "command")
        && fields.get("command").and_then(Value::as_str) == Some(command)
        && timeout
        && shell
}

/// Whether a sandbox list entry names `path` or a path inside it. Only an
/// absolute entry is judged. `.` and `..` are resolved by name, and a
/// component holding a pattern character is taken to match any name, so no
/// spelling slips an entry past the check.
pub fn lies_within(entry: &str, path: &str) -> bool {
    let (Some(entry), Some(path)) = (components(entry), components(path)) else {
        return false;
    };
    for (index, name) in path.iter().enumerate() {
        match entry.get(index) {
            None => return false,
            Some(&"**") => return true,
            Some(part) if part == name || part.contains(['*', '?', '[']) => {}
            Some(_) => return false,
        }
    }
    true
}

fn components(path: &str) -> Option<Vec<&str>> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::host_artifacts::security::propose;

    const HOME: &str = "/home/o/.local/share/crenshawdev/baley";
    const CONFIG: &str = "/home/o/.config/crenshawdev/baley";
    const EXECUTABLE: &str = "/home/o/.local/bin/baley";
    const SETTINGS: &str = "/home/o/.claude/settings.json";
    const FILE_TOOLS: [Tool; 6] = [
        Tool::Read,
        Tool::Grep,
        Tool::Glob,
        Tool::Write,
        Tool::Edit,
        Tool::NotebookEdit,
    ];
    const SHELL_TOOLS: [Tool; 3] = [Tool::Bash, Tool::Monitor, Tool::PowerShell];

    fn folders() -> Folders {
        Folders {
            home: HOME.into(),
            config: CONFIG.into(),
        }
    }

    fn executable() -> Executable {
        Executable::new(EXECUTABLE).unwrap()
    }

    /// Baley's own proposal with the settings file write-only.
    fn ours() -> Value {
        propose(&folders(), &executable(), &[SETTINGS.into()]).settings
    }

    fn guard_hook() -> Value {
        hook::render(&executable())
    }

    fn judged(settings: &Value, hook: &Value, unsupported: &[Mechanism]) -> Coverage {
        judge(&Inputs {
            settings,
            hook,
            folders: &folders(),
            executable: &executable(),
            write_only: &[SETTINGS.into()],
            unsupported,
        })
    }

    fn gap(coverage: &Coverage, tool: Tool, access: Access) -> Vec<Cause> {
        match coverage.verdict(tool, access) {
            Some(Verdict::Gap(causes)) => causes.clone(),
            other => panic!("{tool:?} {access:?}: {other:?}"),
        }
    }

    fn covered(coverage: &Coverage, tool: Tool, access: Access) -> bool {
        matches!(
            coverage.verdict(tool, access),
            Some(Verdict::Covered { .. })
        )
    }

    fn file_tool_access(tool: Tool) -> Access {
        tool.accesses()[0]
    }

    /// Removes every string in the arrays at `pointers` that `drop` picks.
    fn without(mut document: Value, pointers: &[&str], drop: impl Fn(&str) -> bool) -> Value {
        for pointer in pointers {
            document
                .pointer_mut(pointer)
                .and_then(Value::as_array_mut)
                .unwrap()
                .retain(|item| !drop(item.as_str().unwrap()));
        }
        document
    }

    #[test]
    fn the_best_effort_label_lost_or_applied_to_read_is_caught() {
        let names: Vec<&str> = Tool::ALL.iter().map(|tool| tool.name()).collect();
        assert_eq!(names.join("|"), hook::MATCHER);
        let coverage = judged(&ours(), &guard_hook(), &[]);
        assert_eq!(coverage.tools.len(), 12);
        for judged in &coverage.tools {
            let expected = match judged.tool {
                Tool::Bash | Tool::Monitor | Tool::PowerShell => Verdict::Covered {
                    by: &[Mechanism::Sandbox],
                    best_effort: false,
                },
                Tool::Grep | Tool::Glob => Verdict::Covered {
                    by: &[Mechanism::PermissionRules, Mechanism::Hook],
                    best_effort: true,
                },
                _ => Verdict::Covered {
                    by: &[Mechanism::PermissionRules, Mechanism::Hook],
                    best_effort: false,
                },
            };
            assert_eq!(judged.verdict, expected, "{judged:?}");
        }
        for file in &coverage.files {
            assert_eq!(
                file.write,
                Verdict::Covered {
                    by: &[Mechanism::Sandbox, Mechanism::PermissionRules],
                    best_effort: false,
                },
                "{file:?}"
            );
        }
    }

    #[test]
    fn home_only_protection_read_as_covering_the_config_folder_is_caught() {
        let home_only = without(
            ours(),
            &[
                "/sandbox/filesystem/denyRead",
                "/sandbox/filesystem/denyWrite",
                "/permissions/deny",
            ],
            |item| item.contains("/.config/"),
        );
        let coverage = judged(&home_only, &guard_hook(), &[]);
        for tool in SHELL_TOOLS {
            for (access, list) in [(Access::Read, "denyRead"), (Access::Write, "denyWrite")] {
                assert_eq!(
                    gap(&coverage, tool, access),
                    [Cause::NotDenied {
                        list,
                        path: CONFIG.into(),
                    }],
                    "{tool:?}"
                );
            }
        }
        for tool in [Tool::Read, Tool::Grep, Tool::Glob] {
            assert_eq!(
                gap(&coverage, tool, Access::Read),
                [Cause::NoRule(
                    "Read(//home/o/.config/crenshawdev/baley/**)".into()
                )]
            );
        }
        for tool in [Tool::Write, Tool::Edit, Tool::NotebookEdit] {
            assert_eq!(
                gap(&coverage, tool, Access::Write),
                [Cause::NoRule(
                    "Edit(//home/o/.config/crenshawdev/baley/**)".into()
                )]
            );
        }
    }

    #[test]
    fn write_protection_claimed_as_read_protection_is_caught() {
        let write_only = without(ours(), &["/sandbox/filesystem/denyRead"], |_| true);
        let coverage = judged(&write_only, &guard_hook(), &[]);
        for tool in SHELL_TOOLS {
            assert_eq!(
                gap(&coverage, tool, Access::Read),
                [
                    Cause::NotDenied {
                        list: "denyRead",
                        path: HOME.into(),
                    },
                    Cause::NotDenied {
                        list: "denyRead",
                        path: CONFIG.into(),
                    },
                ]
            );
            assert!(covered(&coverage, tool, Access::Write), "{tool:?}");
        }
    }

    #[test]
    fn a_bash_only_guard_read_as_guarding_the_file_tools_is_caught() {
        let mut bash_only = guard_hook();
        bash_only["hooks"][hook::EVENT][0]["matcher"] = json!("Bash");
        let coverage = judged(&ours(), &bash_only, &[]);
        for tool in FILE_TOOLS {
            let access = file_tool_access(tool);
            assert_eq!(gap(&coverage, tool, access), [Cause::NoGuard], "{tool:?}");
        }
        for tool in SHELL_TOOLS {
            assert!(covered(&coverage, tool, Access::Read));
            assert!(covered(&coverage, tool, Access::Write));
        }
    }

    #[test]
    fn a_file_declared_protected_by_its_list_entries_while_the_sandbox_is_off_is_caught() {
        for (pointer, value, key) in [
            ("/sandbox/enabled", json!(false), "enabled"),
            (
                "/sandbox/failIfUnavailable",
                json!(false),
                "failIfUnavailable",
            ),
            (
                "/sandbox/allowUnsandboxedCommands",
                json!(true),
                "allowUnsandboxedCommands",
            ),
        ] {
            let mut off = ours();
            *off.pointer_mut(pointer).unwrap() = value;
            let coverage = judged(&off, &guard_hook(), &[]);
            for tool in SHELL_TOOLS {
                for access in [Access::Read, Access::Write] {
                    assert_eq!(
                        gap(&coverage, tool, access),
                        [Cause::SandboxSetting(key)],
                        "{key} {tool:?}"
                    );
                }
            }
            assert_eq!(coverage.files.len(), 2);
            for file in &coverage.files {
                assert_eq!(
                    file.write,
                    Verdict::Gap(vec![Cause::SandboxSetting(key)]),
                    "{key}"
                );
            }
            assert!(covered(&coverage, Tool::Read, Access::Read), "{key}");
        }
        let mut no_filesystem = ours();
        no_filesystem["sandbox"]["filesystem"]["disabled"] = json!(true);
        let coverage = judged(&no_filesystem, &guard_hook(), &[]);
        assert_eq!(
            gap(&coverage, Tool::Bash, Access::Read),
            [Cause::SandboxSetting("filesystem.disabled")]
        );
    }

    #[test]
    fn an_excluded_command_read_as_sandbox_coverage_is_caught() {
        let excluding = |entries: Value| {
            let mut settings = ours();
            settings["sandbox"]["excludedCommands"] = entries;
            judged(&settings, &guard_hook(), &[])
        };
        let cases = [
            (json!(["*"]), vec![Cause::Excluded("*".into())]),
            (json!(["sh"]), vec![Cause::Excluded("sh".into())]),
            (
                json!(["sh", "docker"]),
                vec![
                    Cause::Excluded("sh".into()),
                    Cause::Excluded("docker".into()),
                ],
            ),
        ];
        for (entries, expected) in cases {
            let coverage = excluding(entries.clone());
            for tool in SHELL_TOOLS {
                for access in [Access::Read, Access::Write] {
                    assert_eq!(
                        gap(&coverage, tool, access),
                        expected,
                        "{entries} {tool:?} {access:?}"
                    );
                }
            }
            assert_eq!(coverage.files.len(), 2);
            for file in &coverage.files {
                assert_eq!(file.write, Verdict::Gap(expected.clone()), "{entries}");
            }
            for tool in FILE_TOOLS {
                let access = file_tool_access(tool);
                assert!(covered(&coverage, tool, access), "{entries} {tool:?}");
            }
        }
        let coverage = excluding(json!([]));
        for judged in &coverage.tools {
            assert!(
                matches!(judged.verdict, Verdict::Covered { .. }),
                "{judged:?}"
            );
        }
        for file in &coverage.files {
            assert!(matches!(file.write, Verdict::Covered { .. }), "{file:?}");
        }
    }

    #[test]
    fn a_guard_hook_identified_by_its_last_word_or_narrowed_or_cut_short_is_caught() {
        let genuine = hook::command(&executable());
        for handler in [
            json!({"type": "command", "command": "/bin/echo guard", "timeout": 10}),
            json!({"type": "command", "command": genuine, "timeout": 10, "if": "Bash(*)"}),
            json!({"type": "command", "command": genuine, "timeout": 10, "async": true}),
            json!({"type": "command", "command": genuine, "timeout": 1}),
        ] {
            let mut other = guard_hook();
            other["hooks"][hook::EVENT][0]["hooks"] = json!([handler]);
            let coverage = judged(&ours(), &other, &[]);
            for tool in FILE_TOOLS {
                let access = file_tool_access(tool);
                assert_eq!(gap(&coverage, tool, access), [Cause::NoGuard], "{handler}");
            }
        }
    }

    #[test]
    fn hooks_turned_off_by_a_setting_or_marked_unsupported_read_as_guarded_is_caught() {
        let mut off = ours();
        off["disableAllHooks"] = json!(true);
        let coverage = judged(&off, &guard_hook(), &[]);
        for tool in FILE_TOOLS {
            let access = file_tool_access(tool);
            assert_eq!(gap(&coverage, tool, access), [Cause::HooksDisabled]);
        }
        let coverage = judged(&ours(), &guard_hook(), &[Mechanism::Hook]);
        for tool in FILE_TOOLS {
            let access = file_tool_access(tool);
            assert_eq!(
                gap(&coverage, tool, access),
                [Cause::Unsupported(Mechanism::Hook)]
            );
        }
    }

    #[test]
    fn a_read_reopened_inside_a_folder_under_any_spelling_is_caught() {
        let ledger = "/home/o/.local/share/crenshawdev/baley/ledger";
        let glob = "/home/o/.local/share/*/baley";
        let dotted = "/home/o/.config/crenshawdev/baley/../baley";
        let mut reopened = ours();
        reopened["sandbox"]["filesystem"]["allowRead"] =
            json!([ledger, "/home/o", dotted, glob, "/home/o/projects"]);
        let coverage = judged(&reopened, &guard_hook(), &[]);
        for tool in SHELL_TOOLS {
            let entries: Vec<Cause> = [ledger, glob, dotted]
                .iter()
                .map(|entry| Cause::Reopened {
                    list: "allowRead",
                    entry: (*entry).into(),
                })
                .collect();
            assert_eq!(gap(&coverage, tool, Access::Read), entries);
            assert!(covered(&coverage, tool, Access::Write));
        }
    }

    #[test]
    fn an_entry_reopening_a_folder_only_once_dot_dot_is_resolved_read_as_harmless_is_caught() {
        let entry = "/home/o/.config/x/../crenshawdev/baley";
        let mut reopened = ours();
        reopened["sandbox"]["filesystem"]["allowRead"] = json!([entry]);
        let coverage = judged(&reopened, &guard_hook(), &[]);
        for tool in SHELL_TOOLS {
            assert_eq!(
                gap(&coverage, tool, Access::Read),
                [Cause::Reopened {
                    list: "allowRead",
                    entry: entry.into(),
                }],
                "{tool:?}"
            );
            assert!(covered(&coverage, tool, Access::Write), "{tool:?}");
        }
    }

    #[test]
    fn a_write_rule_accepted_in_place_of_an_edit_rule_is_caught() {
        let mut misspelled = ours();
        for rule in misspelled["permissions"]["deny"].as_array_mut().unwrap() {
            let text = rule.as_str().unwrap();
            if text.starts_with("Edit(") && text.ends_with("/**)") {
                *rule = json!(text.replacen("Edit(", "Write(", 1));
            }
        }
        let coverage = judged(&misspelled, &guard_hook(), &[]);
        for tool in [Tool::Write, Tool::Edit, Tool::NotebookEdit] {
            assert_eq!(
                gap(&coverage, tool, Access::Write),
                [
                    Cause::NoRule("Edit(//home/o/.local/share/crenshawdev/baley/**)".into()),
                    Cause::NoRule("Edit(//home/o/.config/crenshawdev/baley/**)".into()),
                ]
            );
        }
    }

    #[test]
    fn a_mechanism_marked_unsupported_read_as_covered_is_caught() {
        let coverage = judged(&ours(), &guard_hook(), &[Mechanism::Sandbox]);
        for tool in SHELL_TOOLS {
            for access in [Access::Read, Access::Write] {
                assert_eq!(
                    gap(&coverage, tool, access),
                    [Cause::Unsupported(Mechanism::Sandbox)]
                );
            }
        }
        for file in &coverage.files {
            assert_eq!(
                file.write,
                Verdict::Gap(vec![Cause::Unsupported(Mechanism::Sandbox)])
            );
        }
        let coverage = judged(&ours(), &guard_hook(), &[Mechanism::PermissionRules]);
        for tool in FILE_TOOLS {
            let access = file_tool_access(tool);
            assert_eq!(
                gap(&coverage, tool, access),
                [Cause::Unsupported(Mechanism::PermissionRules)]
            );
        }
    }

    #[test]
    fn read_protection_required_of_a_write_only_file_or_an_unlisted_file_judged_is_caught() {
        let checkout = "/work/a/baley.toml";
        let write_only: Vec<PathBuf> = vec![SETTINGS.into(), checkout.into()];
        let settings = propose(&folders(), &executable(), &write_only).settings;
        let coverage = judge(&Inputs {
            settings: &settings,
            hook: &guard_hook(),
            folders: &folders(),
            executable: &executable(),
            write_only: &write_only,
            unsupported: &[],
        });
        let paths: Vec<&Path> = coverage
            .files
            .iter()
            .map(|file| file.path.as_path())
            .collect();
        assert_eq!(
            paths,
            [
                Path::new(SETTINGS),
                Path::new(checkout),
                Path::new(EXECUTABLE)
            ]
        );
        for file in &coverage.files {
            assert_eq!(
                file.write,
                Verdict::Covered {
                    by: &[Mechanism::Sandbox, Mechanism::PermissionRules],
                    best_effort: false,
                },
                "{file:?}"
            );
        }
    }
}
