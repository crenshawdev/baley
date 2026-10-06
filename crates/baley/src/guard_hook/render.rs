//! The guard's answer in Claude Code's pre-tool hook form.

use baley_core::guard::Answer;
use serde::Serialize;

/// The most bytes of a reason the hook prints. The docs cap a hook's
/// `additionalContext`, `systemMessage`, `initialUserMessage` and plain
/// stdout at 10,000 characters and name no cap for the decision reason, so
/// the reason takes the same bound. Counted in bytes, it holds however the
/// host counts characters.
const REASON_CAP: usize = 10_000;

/// The exit code that blocks the call on its own, with stderr as the reason.
const BLOCKING_EXIT: u8 = 2;

/// What the hook writes and the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The bytes for stdout: one JSON object and a newline, or nothing.
    pub stdout: String,
    /// The text for stderr, or nothing.
    pub stderr: String,
    /// The exit status.
    pub exit: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output<'a> {
    hook_specific_output: Decision<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Decision<'a> {
    hook_event_name: &'static str,
    permission_decision: &'static str,
    permission_decision_reason: &'a str,
}

/// Renders one answer for Claude Code.
///
/// The form follows Claude Code's hooks reference
/// (code.claude.com/docs/en/hooks), read through Context7 on 2026-10-06:
///
/// - An ask or deny is `hookSpecificOutput` with `hookEventName`
///   `PreToolUse`, `permissionDecision` `ask` or `deny` and
///   `permissionDecisionReason`, on stdout with exit 0. The reason of an ask
///   is shown to the user, and of a deny to Claude.
/// - Exit 0 with no stdout reports no decision, and the host's own
///   permission rules decide. A plain pass is that, never `allow`, which
///   would skip the prompt those rules ask for.
/// - A pass on failure prints its reason as one stderr line and exits 0. The
///   docs say stderr on exit 0 goes to the debug log only.
/// - The reason is cut to `REASON_CAP` bytes on a character boundary.
pub fn render(answer: &Answer) -> Rendered {
    let (decision, reason) = match answer {
        Answer::Pass => return quiet(String::new()),
        Answer::PassOnFailure(reason) => return quiet(format!("{reason}\n")),
        Answer::Ask(reason) => ("ask", reason),
        Answer::Deny(reason) => ("deny", reason),
    };
    let reason = &reason[..reason.floor_char_boundary(REASON_CAP)];
    let output = Output {
        hook_specific_output: Decision {
            hook_event_name: "PreToolUse",
            permission_decision: decision,
            permission_decision_reason: reason,
        },
    };
    // Only strings go in, so serializing cannot fail.
    let json = serde_json::to_string(&output).expect("a hook decision serializes");
    Rendered {
        stdout: format!("{json}\n"),
        stderr: String::new(),
        exit: 0,
    }
}

/// The answer when an ask or deny could not be written to stdout: exit 2,
/// which blocks the call with stderr as the reason, so a failed write never
/// lets the call through. Any other nonzero exit would be a non-blocking
/// error and the call would run.
pub fn failed_write(reason: &str) -> Rendered {
    Rendered {
        stdout: String::new(),
        stderr: format!("{reason}\n"),
        exit: BLOCKING_EXIT,
    }
}

/// Nothing on stdout and exit 0, with `stderr` as given.
fn quiet(stderr: String) -> Rendered {
    Rendered {
        stdout: String::new(),
        stderr,
        exit: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_core::guard::reason;
    use serde_json::{Value, json};

    fn decision(rendered: &Rendered) -> Value {
        serde_json::from_str(&rendered.stdout).expect("one JSON object")
    }

    fn reason_of(rendered: &Rendered) -> String {
        decision(rendered)["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .expect("a reason")
            .to_owned()
    }

    #[test]
    fn an_ask_or_deny_with_a_wrong_field_or_decision_value_is_caught() {
        for (answer, value) in [
            (Answer::Ask("why".into()), "ask"),
            (Answer::Deny("why".into()), "deny"),
        ] {
            let rendered = render(&answer);
            assert_eq!(
                decision(&rendered),
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": value,
                        "permissionDecisionReason": "why",
                    }
                }),
                "{answer:?}"
            );
            assert!(rendered.stdout.ends_with("}\n"), "{rendered:?}");
            assert_eq!(rendered.stderr, "");
            assert_eq!(rendered.exit, 0);
        }
    }

    #[test]
    fn a_reason_over_the_cap_is_not_printed_whole_or_cut_inside_a_character() {
        // 4,000 three-byte characters: 12,000 bytes. The last whole one
        // under 10,000 bytes ends at 9,999.
        let long = "€".repeat(4_000);
        let cut = reason_of(&render(&Answer::Deny(long.clone())));
        assert_eq!(cut.len(), 9_999);
        assert!(long.starts_with(&cut));

        let exact = "a".repeat(10_000);
        assert_eq!(reason_of(&render(&Answer::Ask(exact.clone()))), exact);
    }

    #[test]
    fn a_protected_branch_deny_that_quotes_the_branch_or_drops_the_guidance_is_caught() {
        let shown = reason_of(&render(&Answer::Deny(reason::refuse_deny("feat/x"))));
        assert!(shown.contains(" feat/x "), "{shown}");
        assert!(
            !shown.contains("\"feat/x\"") && !shown.contains("'feat/x'"),
            "{shown}"
        );
        assert!(shown.ends_with("Create a task branch first."), "{shown}");
    }

    #[test]
    fn a_plain_pass_that_prints_anything_or_allows_is_caught() {
        let rendered = render(&Answer::Pass);
        assert_eq!(
            rendered,
            Rendered {
                stdout: String::new(),
                stderr: String::new(),
                exit: 0,
            }
        );
        assert!(!rendered.stdout.contains("allow"));
    }

    #[test]
    fn a_pass_on_failure_that_is_silent_or_prints_a_decision_is_caught() {
        let reason = "Baley guard: git could not read the branch (x), so this commit proceeds";
        let rendered = render(&Answer::PassOnFailure(reason.into()));
        assert_eq!(rendered.stdout, "");
        assert_eq!(rendered.stderr, format!("{reason}\n"));
        assert_eq!(rendered.exit, 0);
    }

    #[test]
    fn a_failed_answer_write_that_exits_other_than_the_blocking_code_is_caught() {
        let rendered = failed_write("Baley guard: denied");
        assert_eq!(rendered.exit, 2);
        assert_eq!(rendered.stderr, "Baley guard: denied\n");
        assert_eq!(rendered.stdout, "");
    }
}
