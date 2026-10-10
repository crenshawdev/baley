//! `baley install`: Claude Code's wiring at the stable path, from the content
//! `host_artifacts` renders, with ownership evidence in the ledger before
//! any replacement. Gathering is kept apart from judging.
//! One install runs at a time: the command holds an exclusive lock in Baley's
//! own home from reading the latest record to recording the result.
//! Stubs are applied and recorded here. Phase 15 plan 2 adds the registration,
//! hook and settings writes through the same plan and ownership facts.

/// Planned stub writes and their filesystem observations.
pub mod apply;
/// Command arguments, host selection and installation gathering.
pub mod command;
/// Installation ownership facts and the `install.recorded` event.
pub mod event;
/// Ordered artifact writes or a refusal from supplied file observations.
pub mod plan;
/// The install outcome and the owner's next steps, rendered from supplied facts.
pub mod receipt;
/// Recording and reading installation ownership evidence in the ledger.
pub mod record;
/// The registration install requests from Claude Code, decided from the file.
pub mod registration;
/// Claude Code's user settings file read, composed into and judged.
pub mod settings;
/// Stub ownership judged from observed bytes and the latest install record.
pub mod stubs;
/// The latest installation ownership record for each host.
pub mod view;

/// Shared inputs for the install tests: the placements derived from
/// `HOME` `/home/o`, Baley's folders and the sandbox programs found or not.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use crate::folders::{Environment, Folders};
    use crate::host_artifacts::installed::{self, Installed};
    use crate::host_artifacts::stubs;
    use crate::host_doctor::prerequisites::{self, Candidate, Judged, Observed, Search};

    /// The placements for `HOME` `/home/o` with `CLAUDE_CONFIG_DIR` unset.
    pub fn installed() -> Installed {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        installed::resolve(&env, None, &manifest).unwrap()
    }

    /// Linux with `bwrap` and `socat` both found.
    pub fn linux() -> Judged {
        prerequisites("linux", &["bwrap", "socat"])
    }

    /// The prerequisites judged from an observation for `os` in which each
    /// named program is a regular file with mode `0o755` in `/usr/bin`.
    pub fn prerequisites(os: &str, found: &[&str]) -> Judged {
        let search = |program: &str| Search {
            program: program.into(),
            candidates: vec![Candidate {
                folder: PathBuf::from("/usr/bin"),
                regular_file: found.contains(&program),
                mode: if found.contains(&program) { 0o755 } else { 0 },
            }],
        };
        prerequisites::judge(&Observed {
            os: os.into(),
            searches: vec![search("bwrap"), search("socat")],
        })
    }

    /// Baley's data and configuration folders under `/home/o`.
    pub fn folders() -> Folders {
        Folders {
            home: "/home/o/.local/share/crenshawdev/baley".into(),
            config: "/home/o/.config/crenshawdev/baley".into(),
        }
    }
    /// The matcher design 0012 gives the guard hook.
    pub const MATCHER: &str = "Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit";

    /// Baley's `PreToolUse` item for the stable path, written out by hand.
    pub fn hook_item() -> Value {
        json!({
            "matcher": MATCHER,
            "hooks": [{
                "type": "command",
                "command": "'/home/o/.local/bin/baley' guard",
                "timeout": 10,
            }],
        })
    }

    /// A settings document of a completed install, written by hand from
    /// design 0012: the owner's `model`, every Baley entry, and the deny
    /// rules in an order the renderer does not use.
    pub fn complete_settings() -> Value {
        json!({
            "model": "opus",
            "permissions": {"deny": [
                "Edit(//home/o/.local/bin/baley)",
                "Edit(//home/o/.claude/settings.json)",
                "Edit(//home/o/.claude.json)",
                "Edit(//home/o/.claude/skills/bal-help/SKILL.md)",
                "Edit(//home/o/.claude/skills/bal-capture/SKILL.md)",
                "Edit(//home/o/.local/lib/crenshawdev/baley/versions/**)",
                "Edit(//home/o/.config/crenshawdev/baley/**)",
                "Read(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.local/share/crenshawdev/baley/**)",
                "Read(//home/o/.local/share/crenshawdev/baley/**)",
            ]},
            "sandbox": {
                "enabled": true,
                "failIfUnavailable": true,
                "allowUnsandboxedCommands": false,
                "filesystem": {
                    "denyRead": [
                        "/home/o/.config/crenshawdev/baley",
                        "/home/o/.local/share/crenshawdev/baley",
                    ],
                    "denyWrite": [
                        "/home/o/.local/bin/baley",
                        "/home/o/.claude/settings.json",
                        "/home/o/.claude.json",
                        "/home/o/.claude/skills/bal-help/SKILL.md",
                        "/home/o/.claude/skills/bal-capture/SKILL.md",
                        "/home/o/.local/lib/crenshawdev/baley/versions",
                        "/home/o/.config/crenshawdev/baley",
                        "/home/o/.local/share/crenshawdev/baley",
                    ],
                },
            },
            "hooks": {"PreToolUse": [hook_item()]},
        })
    }

    /// The document pretty-printed with four-space indentation, the way an
    /// owner's editor might have left it.
    pub fn four_spaces(document: &Value) -> Vec<u8> {
        use serde::Serialize;
        let mut bytes = Vec::new();
        let format = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, format);
        document.serialize(&mut serializer).unwrap();
        bytes.push(b'\n');
        bytes
    }
}
