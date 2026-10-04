//! The reason text of the guard's answers (design 0010). Every function takes
//! plain values and returns owned text, so the hook and the live guard say the
//! same thing. A branch is written exactly as given and never quoted, and a
//! branch nobody read is named in words.

/// The guidance every commit answer that names a protected branch ends with.
const DENY_GUIDANCE: &str = "Create a task branch first.";

/// The words for a branch, or for the lack of one.
fn branch_words(branch: Option<&str>) -> String {
    match branch {
        Some(name) => name.to_owned(),
        None => "an unknown branch (git could not read it)".to_owned(),
    }
}

/// Why every `git push` in a project asks.
pub fn push_ask() -> String {
    "Baley guard: every git push asks for permission. Approve only if you are deliberately publishing."
        .to_owned()
}

/// A commit on a protected branch under `ask`.
pub fn protected_ask(branch: &str) -> String {
    format!(
        "Baley guard: {branch} is a protected branch (git.protected_branches). Create a task branch first, or approve to commit here deliberately."
    )
}

/// A commit on a protected branch under `refuse`.
pub fn refuse_deny(branch: &str) -> String {
    format!(
        "Baley guard: git.on_protected is refuse, so a commit on the protected branch {branch} is denied. {DENY_GUIDANCE}"
    )
}

/// A commit denied under hard fail: `failure` says what could not be read,
/// and `.git/HEAD` still names the protected `branch`.
pub fn hard_fail_deny(branch: &str, failure: &str) -> String {
    format!(
        "Baley guard: git.guard_hard_fail is true, {failure}, and .git/HEAD names the protected branch {branch}, so this commit is denied. {DENY_GUIDANCE}"
    )
}

/// A commit denied because the settings are torn and the last complete
/// settings said `refuse`.
pub fn remembered_refuse_deny(torn: &str, branch: &str) -> String {
    format!(
        "Baley guard: the settings are unavailable ({torn}), and the last complete settings had git.on_protected set to refuse, so a commit on the protected branch {branch} is denied. {DENY_GUIDANCE}"
    )
}

/// A commit denied because the settings are torn and the last complete
/// settings had hard fail on: `failure` says what could not be read.
pub fn remembered_hard_fail_deny(torn: &str, failure: &str, branch: &str) -> String {
    format!(
        "Baley guard: the settings are unavailable ({torn}), the last complete settings had git.guard_hard_fail set to true, {failure}, and .git/HEAD names the protected branch {branch}, so this commit is denied. {DENY_GUIDANCE}"
    )
}

/// A commit asked about because the settings are torn. It claims nothing
/// about the branch, so it carries no task-branch guidance.
pub fn torn_ask(torn: &str, branch: Option<&str>) -> String {
    format!(
        "Baley guard: the settings are unavailable ({torn}), so Baley cannot check the protected-branch rules for a commit on {}. Fix the file, or approve to commit here deliberately.",
        branch_words(branch)
    )
}

/// A commit that proceeds because an input could not be read.
pub fn failure_pass(unreadable: &str) -> String {
    format!(
        "Baley guard: {unreadable}, so this commit proceeds without Baley's protected-branch check. This is not policy approval."
    )
}

/// Why every PowerShell call in a project asks. The commit and push check
/// reads POSIX shell grammar only, so it says nothing about this command.
pub fn powershell_ask() -> String {
    "Baley guard: this is a PowerShell call, and Baley's commit and push check reads POSIX shell grammar only, so it cannot judge this command. Approve only if it runs no git commit or push you did not intend."
        .to_owned()
}
