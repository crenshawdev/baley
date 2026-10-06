//! The guard's two events, the command kind every guard write is recorded
//! under, the input digest of a classified call and the payload of each
//! event (design 0010 section 6, design 0001 Payloads).
//!
//! Both events go to the per-user `user` project on stream `guard`, at
//! policy version 0. The session project and the target path are facts in
//! the payload and the caller, never the ledger project, so a guard record
//! needs no project ledger.

use baley_store::request_digest;
use serde_json::{Map, Value, json};

use super::answer::Answer;
use super::remembered::DenialParts;
use super::scan::GitVerb;
use super::settings::GuardSettings;
use crate::registry::{Registry, RegistryError};

/// One recorded guard answer: an ask, a deny or a pass on failure, with the
/// call it answered and the facts it was judged on. Built by
/// [`answered_payload`].
pub const GUARD_ANSWERED: &str = "guard.answered";
/// The current `guard.answered` payload version.
pub const GUARD_ANSWERED_VERSION: u32 = 1;
/// The denials of the last complete policy for one session project, target
/// checkout and host, kept for torn settings (GRD-R7). Built by
/// [`policy_recorded_payload`].
pub const GUARD_POLICY_RECORDED: &str = "guard.policy_recorded";
/// The current `guard.policy_recorded` payload version.
pub const GUARD_POLICY_RECORDED_VERSION: u32 = 1;
/// The one stream in `user` both guard events go on.
pub const GUARD_STREAM: &str = "guard";
/// The command kind every guard write is recorded under, so the request
/// digest and the store's `command/<kind>` stream share one spelling.
pub const GUARD_COMMAND: &str = "guard.record";

/// Registers `guard.answered` and `guard.policy_recorded` at version 1, with
/// no upcasters.
pub fn register_guard_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(GUARD_ANSWERED, GUARD_ANSWERED_VERSION, [])?;
    registry.register(GUARD_POLICY_RECORDED, GUARD_POLICY_RECORDED_VERSION, [])
}

/// The `tool_input` fields the guard reads, each absent unless the call
/// sent it. Bash and Monitor give `command`, the path tools give the path
/// fields their tool names, and PowerShell gives none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolInput<'a> {
    /// The shell command of a Bash or Monitor call.
    pub command: Option<&'a str>,
    /// The file a Read, Write or Edit call names.
    pub file_path: Option<&'a str>,
    /// The notebook a NotebookEdit call names.
    pub notebook_path: Option<&'a str>,
    /// The folder a Grep or Glob call searches.
    pub path: Option<&'a str>,
    /// The file filter of a Grep call.
    pub glob: Option<&'a str>,
    /// The pattern of a Glob call.
    pub pattern: Option<&'a str>,
}

/// The input digest of a classified call: the lowercase hex SHA-256 of the
/// canonical JSON of the tool's name and each field in `input` by name. An
/// absent field is left out, so it never matches an empty one.
///
/// It is not the digest of the raw stdin bytes, because a genuine
/// redelivery can differ in fields the guard ignores, such as the
/// permission mode or the transcript path, and would read as different
/// input. Those fields cannot reach this digest, since it never takes them.
/// The project directory and the cwd are compared on their own.
pub fn input_digest(tool: &str, input: &ToolInput<'_>) -> String {
    let fields = [
        ("command", input.command),
        ("file_path", input.file_path),
        ("notebook_path", input.notebook_path),
        ("path", input.path),
        ("glob", input.glob),
        ("pattern", input.pattern),
    ];
    let mut named = Map::new();
    for (name, value) in fields {
        if let Some(value) = value {
            named.insert(name.into(), json!(value));
        }
    }
    request_digest(&json!({"tool": tool, "input": named}))
        // Strings always have a canonical form.
        .expect("an input digest input is canonical")
        .to_hex()
}

/// The settings an answer was judged under, as the record keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFact<'a> {
    /// A complete policy's guard settings.
    Complete(&'a GuardSettings),
    /// The settings file that gave no policy, in words.
    Torn(&'a str),
    /// No settings were read, including a commit with an unestablished target.
    Absent,
}

/// What a `guard.answered` records besides the answer itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsweredFacts<'a> {
    /// The host's name, `claude-code` for Claude Code.
    pub host: &'a str,
    /// The host's own session id, when it sent one.
    pub session: Option<&'a str>,
    /// The host's id for the call.
    pub call: &'a str,
    /// The project directory as the host gave it, when it gave one.
    pub project_directory: Option<&'a str>,
    /// The directory the host ran the call from.
    pub cwd: &'a str,
    /// The tool's name as the host gives it.
    pub tool: &'a str,
    /// The call's [`input_digest`]. The command itself is never recorded.
    pub input_digest: &'a str,
    /// A path tool's resolved target or a redirected commit's directory.
    pub target: Option<&'a str>,
    /// The git verb the command runs, when it runs one.
    pub verb: Option<GitVerb>,
    /// The branch, when one was read.
    pub branch: Option<&'a str>,
    /// The settings the answer was judged under.
    pub settings: SettingsFact<'a>,
}

/// The payload of `guard.answered`: every fact in `facts`, the outcome
/// (`ask`, `deny` or `pass-on-failure`) and the reason. It holds no command
/// text, no file content and nothing a tool printed beyond the reason, and
/// every fact the `guard` view keys and compares on is inline, as EVD-R10
/// requires. A plain pass is never recorded, so it has no payload.
pub fn answered_payload(facts: &AnsweredFacts<'_>, answer: &Answer) -> Option<Value> {
    let (outcome, reason) = outcome(answer)?;
    let settings = match facts.settings {
        SettingsFact::Complete(settings) => json!({
            "protected_branches": settings.protected_branches,
            "on_protected": settings.on_protected.name(),
            "hard_fail": settings.hard_fail,
        }),
        SettingsFact::Torn(torn) => json!({"torn": torn}),
        SettingsFact::Absent => Value::Null,
    };
    Some(json!({
        "host": facts.host,
        "session": facts.session,
        "call": facts.call,
        "project_directory": facts.project_directory,
        "cwd": facts.cwd,
        "tool": facts.tool,
        "input_digest": facts.input_digest,
        "target": facts.target,
        "verb": facts.verb.map(verb_name),
        "branch": facts.branch,
        "settings": settings,
        "outcome": outcome,
        "reason": reason,
    }))
}

/// The payload of `guard.policy_recorded`: the key it is remembered under
/// and the denials kept. `checkout_root` is null when the commit target is in no
/// checkout.
pub fn policy_recorded_payload(
    project_root: &str,
    checkout_root: Option<&str>,
    host: &str,
    parts: &DenialParts,
) -> Value {
    json!({
        "project_root": project_root,
        "checkout_root": checkout_root,
        "host": host,
        "refuse": parts.refuse,
        "hard_fail": parts.hard_fail,
        "protected_branches": parts.protected_branches,
    })
}

/// The outcome's name and the reason of a recordable answer.
pub(super) fn outcome(answer: &Answer) -> Option<(&'static str, &str)> {
    match answer {
        Answer::Pass => None,
        Answer::Ask(reason) => Some((ASK, reason)),
        Answer::Deny(reason) => Some((DENY, reason)),
        Answer::PassOnFailure(reason) => Some((PASS_ON_FAILURE, reason)),
    }
}

/// The recorded outcome of an ask.
pub(super) const ASK: &str = "ask";
/// The recorded outcome of a deny.
pub(super) const DENY: &str = "deny";
/// The recorded outcome of a pass on failure.
pub(super) const PASS_ON_FAILURE: &str = "pass-on-failure";

/// The verb as the record spells it.
fn verb_name(verb: GitVerb) -> &'static str {
    match verb {
        GitVerb::Commit => "commit",
        GitVerb::Push => "push",
    }
}
