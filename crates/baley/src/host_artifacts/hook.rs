//! Claude Code's pre-tool hook for `baley guard` (design 0010, GRD-R1; D-10).
//!
//! The hook item has no `args`, so it is the shell form: Claude Code hands
//! its `command` string to a shell. The executable is therefore POSIX
//! single-quoted (D-12), and no path is refused for needing quotes, because
//! an owner's home folder can hold a space or a quote.

use serde_json::{Value, json};

use super::executable::Executable;
use crate::guard_budget::HOST_TIMEOUT;

/// The event the guard runs on. There is no session-start hook.
pub const EVENT: &str = "PreToolUse";

/// The tools the guard judges, as the hook's matcher: every tool that runs a
/// command or touches a path (GRD-R1). The coverage judge and the doctor
/// read it from here.
pub const MATCHER: &str = "Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit";

/// The hook: `hooks.PreToolUse` holding one item with [`MATCHER`] and one
/// inner command hook that runs [`command`] within [`HOST_TIMEOUT`] in whole
/// seconds. No other event and no second hook.
pub fn render(executable: &Executable) -> Value {
    json!({
        "hooks": {
            EVENT: [{
                "matcher": MATCHER,
                "hooks": [{
                    "type": "command",
                    "command": command(executable),
                    "timeout": HOST_TIMEOUT.as_secs(),
                }],
            }],
        },
    })
}

/// The shell string the hook runs: the executable single-quoted for POSIX
/// `sh`, then ` guard`.
pub fn command(executable: &Executable) -> String {
    format!("{} guard", shell_quote(executable.as_str()))
}

/// POSIX single quoting: wrapped in `'`, with each inner `'` written as
/// `'\''`. A single-quoted word has no other escape, so a space, `$`, a
/// backtick or a backslash in the path stays literal.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(path: &str) -> Value {
        render(&Executable::new(path).unwrap())
    }

    #[test]
    fn a_bash_only_or_six_tool_matcher_is_caught() {
        let tools: Vec<&str> = MATCHER.split('|').collect();
        assert_eq!(
            tools,
            [
                "Bash",
                "Monitor",
                "PowerShell",
                "Read",
                "Grep",
                "Glob",
                "Write",
                "Edit",
                "NotebookEdit"
            ]
        );
        assert_eq!(
            rendered("/usr/local/bin/baley")["hooks"][EVENT][0]["matcher"],
            MATCHER
        );
    }

    #[test]
    fn a_space_or_a_quote_in_the_executable_quoted_wrongly_for_the_shell_is_caught() {
        // Expected strings follow POSIX single-quoting rules, not the code.
        for (path, expected) in [
            ("/home/o w/baley", "'/home/o w/baley' guard"),
            ("/a'b/baley", r"'/a'\''b/baley' guard"),
        ] {
            assert_eq!(
                rendered(path)["hooks"][EVENT][0]["hooks"][0]["command"],
                expected
            );
        }
    }

    #[test]
    fn a_timeout_that_no_longer_matches_the_guards_budget_is_caught() {
        let timeout = &rendered("/usr/local/bin/baley")["hooks"][EVENT][0]["hooks"][0]["timeout"];
        assert_eq!(*timeout, 10);
        assert_eq!(timeout.as_u64(), Some(HOST_TIMEOUT.as_secs()));
    }

    #[test]
    fn drift_between_the_tracked_hooks_json_and_the_renderer_is_caught() {
        // The tracked file is what T12 runs by hand; its command is a bare
        // `baley` on purpose and is not compared. T15 removes the file and
        // this test once `baley install` writes the hook from `render`.
        let tracked: Value = serde_json::from_str(include_str!("../../../../hooks/hooks.json"))
            .expect("hooks/hooks.json parses");
        let item = &tracked["hooks"][EVENT][0];
        let ours = rendered("/usr/local/bin/baley");
        assert_eq!(item["matcher"], ours["hooks"][EVENT][0]["matcher"]);
        assert_eq!(
            item["hooks"][0]["timeout"],
            ours["hooks"][EVENT][0]["hooks"][0]["timeout"]
        );
    }

    #[test]
    fn a_second_hook_or_another_event_is_caught() {
        let ours = rendered("/usr/local/bin/baley");
        let top: Vec<&String> = ours.as_object().unwrap().keys().collect();
        assert_eq!(top, ["hooks"]);
        let events: Vec<&String> = ours["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(events, [EVENT]);
        let items = ours["hooks"][EVENT].as_array().unwrap();
        assert_eq!(items.len(), 1);
        let inner = items[0]["hooks"].as_array().unwrap();
        assert_eq!(inner.len(), 1);
        let keys: Vec<&String> = inner[0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["type", "command", "timeout"]);
        assert_eq!(inner[0]["type"], "command");
    }
}
