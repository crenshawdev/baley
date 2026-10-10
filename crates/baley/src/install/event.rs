//! Ownership facts for `install.recorded`, version 1, on `user`'s `install`
//! stream. Builders use supplied values only. The caller records the event
//! after judging which artifacts remain Baley's and whether the run completed.

use baley_core::policy::Host;
use baley_core::{Registry, RegistryError};
use baley_store::{NewEvent, StreamName};
use serde::Serialize;
use serde_json::{Value, json};

use crate::update::events::INSTALL_STREAM;

/// An installation's ownership evidence after its writes finish.
pub const INSTALL_RECORDED: &str = "install.recorded";
/// The current `install.recorded` payload version.
pub const INSTALL_RECORDED_VERSION: u32 = 1;

/// Registers `install.recorded` at version 1, with no upcasters.
pub fn register_install_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(INSTALL_RECORDED, INSTALL_RECORDED_VERSION, [])
}

/// The whole MCP entry Baley owns, even when this run left it unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Registration {
    /// The user-scope registration file's absolute path.
    pub path: String,
    /// The complete `mcpServers.baley` entry after the run.
    pub entry: Value,
}

/// The whole pre-tool hook item Baley owns after the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hook {
    /// The settings file's absolute path.
    pub path: String,
    /// Baley's complete `PreToolUse` item, including its matcher and hooks.
    pub item: Value,
}

/// A stub whose bytes are Baley's after the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stub {
    /// The served front door's identity.
    pub identity: String,
    /// The stub's absolute path.
    pub path: String,
    /// The lowercase hex SHA-256 of the bytes left at that path.
    pub sha256: String,
}

/// The protected paths and whole security entries Baley owns after the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sandbox {
    /// The settings file's absolute path.
    pub settings_path: String,
    /// The lowercase hex SHA-256 of the settings file's bytes after the run.
    pub sha256: String,
    /// Baley's resolved data folder.
    pub home: String,
    /// Baley's resolved configuration folder.
    pub config: String,
    /// Files protected from writes.
    pub write_only_files: Vec<String>,
    /// Folders protected from writes.
    pub write_only_folders: Vec<String>,
    /// Every `permissions.deny` entry Baley owns after the run.
    pub permissions_deny: Vec<String>,
    /// Every `sandbox.filesystem.denyRead` entry Baley owns after the run.
    pub deny_read: Vec<String>,
    /// Every `sandbox.filesystem.denyWrite` entry Baley owns after the run.
    pub deny_write: Vec<String>,
    /// Why the sandbox block was not written, or none when it was.
    pub held_back: Option<String>,
}

/// The effective update setting and the version the stable path runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Updates {
    /// Effective `updates.auto`, or none when global settings could not be read.
    pub auto: Option<bool>,
    /// The version the stable path runs, or none when it names none.
    pub staged_version: Option<String>,
}

/// Supplied facts about all artifacts Baley owns after the run, including
/// unchanged entries and older artifacts whose replacement did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The host whose wiring was installed.
    pub host: Host,
    /// The installing binary's version as text.
    pub binary_version: String,
    /// The stable executable path as absolute UTF-8 text.
    pub binary_path: String,
    /// True only with every artifact owned, no gap and the catalog seed recorded.
    /// The caller judges this from the same outcomes used by the receipt.
    pub complete: bool,
    /// The registration Baley owns, or none when none is present.
    pub registration: Option<Registration>,
    /// The hook Baley owns, or none when none is present.
    pub hook: Option<Hook>,
    /// Every stub Baley owns after the run, in manifest order.
    pub stubs: Vec<Stub>,
    /// Baley's security entries, or none when the settings file holds none.
    pub sandbox: Option<Sandbox>,
    /// The effective update setting and active staged version.
    pub updates: Updates,
}

/// Builds the payload without applying any setting or acknowledgement.
/// Whole ownership entries are preserved so a later run can replace them.
pub fn payload(facts: &Facts) -> Value {
    json!({
        "host": facts.host.name(),
        "binary_version": facts.binary_version,
        "binary_path": facts.binary_path,
        "complete": facts.complete,
        "registered": {
            "registration": facts.registration,
            "hook": facts.hook,
        },
        "stubs": facts.stubs,
        "sandbox": facts.sandbox,
        "defaults": {},
        "updates": facts.updates,
    })
}

/// Builds an attachment-free event for the caller to append in project `user`.
pub fn recorded_event(facts: &Facts) -> NewEvent {
    NewEvent {
        stream: StreamName(INSTALL_STREAM.into()),
        type_name: INSTALL_RECORDED.into(),
        type_version: INSTALL_RECORDED_VERSION,
        git: None,
        payload: payload(facts),
        attachments: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_payload_missing_a_design_field_or_an_acknowledgement_slipped_in_is_caught() {
        let facts = Facts {
            host: Host::ClaudeCode,
            binary_version: "0.2.0".into(),
            binary_path: "/home/o/.local/bin/baley".into(),
            complete: false,
            registration: None,
            hook: Some(Hook {
                path: "/home/o/.claude/settings.json".into(),
                item: json!({
                    "matcher": "Write|Edit",
                    "hooks": [{"type": "command", "command": "/home/o/.local/bin/baley guard", "timeout": 5}],
                }),
            }),
            stubs: vec![
                Stub {
                    identity: "bal-capture".into(),
                    path: "/home/o/.claude/skills/bal-capture/SKILL.md".into(),
                    sha256: "a".repeat(64),
                },
                Stub {
                    identity: "bal-help".into(),
                    path: "/home/o/.claude/skills/bal-help/SKILL.md".into(),
                    sha256: "b".repeat(64),
                },
            ],
            sandbox: Some(Sandbox {
                settings_path: "/home/o/.claude/settings.json".into(),
                sha256: "c".repeat(64),
                home: "/home/o/.local/share/crenshawdev/baley".into(),
                config: "/home/o/.config/crenshawdev/baley".into(),
                write_only_files: vec!["/home/o/.local/bin/baley".into()],
                write_only_folders: vec!["/home/o/.local/lib/crenshawdev/baley/versions".into()],
                permissions_deny: vec!["Edit(//home/o/.local/bin/baley)".into()],
                deny_read: vec![],
                deny_write: vec![],
                held_back: Some("socat is missing; install socat".into()),
            }),
            updates: Updates {
                auto: None,
                staged_version: Some("0.2.0".into()),
            },
        };
        let event = recorded_event(&facts);
        assert_eq!(event.type_name, "install.recorded");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.stream, StreamName("install".into()));
        assert!(event.attachments.is_empty());
        assert_eq!(event.git, None);
        assert_eq!(
            event.payload,
            json!({
                "host": "claude-code",
                "binary_version": "0.2.0",
                "binary_path": "/home/o/.local/bin/baley",
                "complete": false,
                "registered": {
                    "registration": null,
                    "hook": {
                        "path": "/home/o/.claude/settings.json",
                        "item": {
                            "matcher": "Write|Edit",
                            "hooks": [{"type": "command", "command": "/home/o/.local/bin/baley guard", "timeout": 5}],
                        },
                    },
                },
                "stubs": [
                    {"identity": "bal-capture", "path": "/home/o/.claude/skills/bal-capture/SKILL.md", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                    {"identity": "bal-help", "path": "/home/o/.claude/skills/bal-help/SKILL.md", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                ],
                "sandbox": {
                    "settings_path": "/home/o/.claude/settings.json",
                    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "home": "/home/o/.local/share/crenshawdev/baley",
                    "config": "/home/o/.config/crenshawdev/baley",
                    "write_only_files": ["/home/o/.local/bin/baley"],
                    "write_only_folders": ["/home/o/.local/lib/crenshawdev/baley/versions"],
                    "permissions_deny": ["Edit(//home/o/.local/bin/baley)"],
                    "deny_read": [],
                    "deny_write": [],
                    "held_back": "socat is missing; install socat",
                },
                "defaults": {},
                "updates": {"auto": null, "staged_version": "0.2.0"},
            })
        );
    }
}
