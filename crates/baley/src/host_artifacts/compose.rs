//! Composition: Baley's entries put into one existing settings document,
//! keeping everything the owner had and reporting each setting in it that
//! [`Conflict`] names as leaving Baley's protection off (D-18). A setting
//! outside that list, or one held in another settings file, is not seen
//! here.
//!
//! The settings layer's merge in `config` is not reused: it replaces arrays
//! and overwrites values without a word, and here a changed owner value must
//! always be reported. Composition deletes nothing. A conflict marks the
//! result incomplete, never successful, and `baley install` (Build 3 T15)
//! decides what to do with an incomplete result. A document is always
//! returned beside the report.
//!
//! `sandbox.network`, where the owner allows provider hosts, is an unrelated
//! key: kept as it is and never a conflict.

use serde_json::{Map, Value};

use super::coverage::{Reach, reach};
use super::executable::Executable;
use super::{hook, registration};
use crate::folders::Folders;

/// A setting in the composed document that leaves Baley's protection off or
/// differs from Baley's own entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// A value Baley sets held another value, and Baley's replaced it. When
    /// the proposal is composed, a sandbox turned off, `failIfUnavailable`
    /// false or `allowUnsandboxedCommands` true shows here; in a document the
    /// proposal is not composed into, it shows as [`Conflict::Disabled`].
    Replaced {
        /// The key's dotted path.
        key: String,
        /// The value it held.
        was: Value,
    },
    /// A key Baley adds entries under holds another JSON type, so nothing was
    /// added there. The dotted path, empty for the document itself.
    NotComposable(String),
    /// A setting that switches protection off, by its dotted key, holding a
    /// value other than its secure one: `disableAllHooks` or
    /// `sandbox.filesystem.disabled` true, or `sandbox.enabled`,
    /// `sandbox.failIfUnavailable` or `sandbox.allowUnsandboxedCommands` left
    /// insecure in a document the proposal is not composed into. Nothing
    /// composed here owns it, so it stays.
    Disabled(&'static str),
    /// A `PreToolUse` item runs a command whose last word is `guard` but is
    /// not Baley's hook item: another matcher, another executable or another
    /// field. It stays, and Baley's item is added beside it.
    OtherGuardHook(Value),
    /// `mcpServers.baley` runs another command or other arguments. It stays
    /// as it was.
    OtherRegistration(Value),
    /// An `allowRead` or `allowWrite` entry equals or lies inside the home or
    /// the config folder.
    Reopened {
        /// The list.
        list: &'static str,
        /// The entry as written.
        entry: String,
    },
    /// An `allowRead` or `allowWrite` entry that does not start with `/`.
    /// The host resolves it against the owner's home folder or the settings
    /// file's place, neither of which composition has, so whether it
    /// re-opens a folder cannot be judged. It stays in the document.
    Unjudged {
        /// The list.
        list: &'static str,
        /// The entry as written.
        entry: String,
    },
    /// A `sandbox.excludedCommands` entry, whatever it names. The host runs
    /// a listed command outside the sandbox even with
    /// `allowUnsandboxedCommands` false, so any entry, `sh`, `docker` or `*`,
    /// lets a command reach Baley's folders. An entry naming the executable
    /// is one of these (D-19). The entry stays in the document.
    Excluded(String),
}

/// The composed document and every conflict found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    /// The owner's document with Baley's entries added.
    pub document: Value,
    /// Every conflict, in the order found.
    pub conflicts: Vec<Conflict>,
}

impl Composed {
    /// Whether the result can be written as Baley's protection: true only
    /// when nothing conflicts.
    pub fn is_complete(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Composes Baley's values for one document (any of the registration, the
/// hook and the proposal, as the placement map groups them) into the
/// existing document, or into an empty object when there is none.
///
/// Every key Baley does not own keeps its value and its place, and a key
/// Baley adds goes last. Arrays gain each Baley entry not already present,
/// so composing the result again changes nothing. Baley's own values win,
/// and each one that replaced an owner value is reported. An existing
/// `mcpServers.baley` running something else is left as it was and
/// reported.
pub fn compose(
    existing: Option<&Value>,
    ours: &[Value],
    folders: &Folders,
    executable: &Executable,
) -> Composed {
    let mut document = existing
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut conflicts = Vec::new();
    for value in ours {
        merge(&mut document, value, "", &mut conflicts);
    }
    scan(&document, folders, executable, &mut conflicts);
    Composed {
        document,
        conflicts,
    }
}

fn merge(target: &mut Value, ours: &Value, key: &str, conflicts: &mut Vec<Conflict>) {
    match ours {
        Value::Object(fields) => {
            let Some(map) = target.as_object_mut() else {
                conflicts.push(Conflict::NotComposable(key.to_owned()));
                return;
            };
            for (name, value) in fields {
                let path = if key.is_empty() {
                    name.clone()
                } else {
                    format!("{key}.{name}")
                };
                match map.get_mut(name) {
                    None => {
                        map.insert(name.clone(), value.clone());
                    }
                    // Another server under Baley's key is reported by the scan.
                    Some(server)
                        if key == "mcpServers"
                            && name == registration::KEY
                            && !same_server(server, value) => {}
                    Some(existing) => merge(existing, value, &path, conflicts),
                }
            }
        }
        Value::Array(items) => {
            let Some(list) = target.as_array_mut() else {
                conflicts.push(Conflict::NotComposable(key.to_owned()));
                return;
            };
            for item in items {
                if !list.contains(item) {
                    list.push(item.clone());
                }
            }
        }
        value => {
            if target != value {
                conflicts.push(Conflict::Replaced {
                    key: key.to_owned(),
                    was: target.clone(),
                });
                *target = value.clone();
            }
        }
    }
}

/// Whether an existing `mcpServers` entry runs the same command with the
/// same arguments as ours. `alwaysLoad` and every other key are ignored, so
/// the doctor judges a registration by composition's own rule.
pub(crate) fn same_server(existing: &Value, ours: &Value) -> bool {
    existing.get("command") == ours.get("command") && existing.get("args") == ours.get("args")
}

fn strings<'v>(document: &'v Value, pointer: &str) -> Vec<&'v str> {
    document
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Reports what the document holds, whoever put it there, that turns
/// Baley's protection off or runs something else under Baley's name.
fn scan(
    document: &Value,
    folders: &Folders,
    executable: &Executable,
    conflicts: &mut Vec<Conflict>,
) {
    // When the proposal is composed the merge has already put Baley's value
    // here, so only a hook or registration document can still hold another.
    // An absent key may be set by another file, so it is not reported.
    for (pointer, setting, secure) in [
        ("/sandbox/enabled", "sandbox.enabled", true),
        (
            "/sandbox/failIfUnavailable",
            "sandbox.failIfUnavailable",
            true,
        ),
        (
            "/sandbox/allowUnsandboxedCommands",
            "sandbox.allowUnsandboxedCommands",
            false,
        ),
    ] {
        if document
            .pointer(pointer)
            .is_some_and(|value| *value != Value::Bool(secure))
        {
            conflicts.push(Conflict::Disabled(setting));
        }
    }
    for (pointer, setting) in [
        ("/disableAllHooks", "disableAllHooks"),
        (
            "/sandbox/filesystem/disabled",
            "sandbox.filesystem.disabled",
        ),
    ] {
        if document.pointer(pointer) == Some(&Value::Bool(true)) {
            conflicts.push(Conflict::Disabled(setting));
        }
    }

    let ours = hook::render(executable);
    let item = &ours["hooks"][hook::EVENT][0];
    let items = document
        .pointer(&format!("/hooks/{}", hook::EVENT))
        .and_then(Value::as_array);
    for existing in items.into_iter().flatten() {
        if existing != item && runs_guard(existing) {
            conflicts.push(Conflict::OtherGuardHook(existing.clone()));
        }
    }

    let server = document.pointer(&format!("/mcpServers/{}", registration::KEY));
    if let Some(server) = server {
        let ours = registration::render(executable, false);
        if !same_server(server, &ours["mcpServers"][registration::KEY]) {
            conflicts.push(Conflict::OtherRegistration(server.clone()));
        }
    }

    let folders: Vec<&str> = [&folders.home, &folders.config]
        .into_iter()
        .filter_map(|folder| folder.to_str())
        .collect();
    for list in ["allowRead", "allowWrite"] {
        for entry in strings(document, &format!("/sandbox/filesystem/{list}")) {
            let reaches: Vec<Reach> = folders.iter().map(|folder| reach(entry, folder)).collect();
            let entry = entry.to_owned();
            if reaches.contains(&Reach::Within) {
                conflicts.push(Conflict::Reopened { list, entry });
            } else if reaches.contains(&Reach::Unjudged) {
                conflicts.push(Conflict::Unjudged { list, entry });
            }
        }
    }

    for entry in strings(document, "/sandbox/excludedCommands") {
        conflicts.push(Conflict::Excluded(entry.to_owned()));
    }
}

/// Whether an item holds a handler whose command's last word is `guard`.
fn runs_guard(item: &Value) -> bool {
    item.get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|handler| handler.get("command").and_then(Value::as_str))
        .any(|command| command.split_whitespace().last() == Some("guard"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::host_artifacts::security::propose;

    const HOME: &str = "/home/o/.local/share/crenshawdev/baley";
    const CONFIG: &str = "/home/o/.config/crenshawdev/baley";
    const EXECUTABLE: &str = "/home/o/.local/bin/baley";

    fn folders() -> Folders {
        Folders {
            home: HOME.into(),
            config: CONFIG.into(),
        }
    }

    fn executable() -> Executable {
        Executable::new(EXECUTABLE).unwrap()
    }

    /// The proposal and the hook, as one settings document receives them.
    fn settings() -> Vec<Value> {
        vec![
            propose(&folders(), &executable(), &[]).settings,
            hook::render(&executable()),
        ]
    }

    fn composed(existing: &Value, ours: &[Value]) -> Composed {
        compose(Some(existing), ours, &folders(), &executable())
    }

    fn keys(value: &Value) -> Vec<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }

    fn assert_secure(document: &Value) {
        assert_eq!(document["sandbox"]["enabled"], true);
        assert_eq!(document["sandbox"]["failIfUnavailable"], true);
        assert_eq!(document["sandbox"]["allowUnsandboxedCommands"], false);
    }

    #[test]
    fn unrelated_settings_overwritten_reordered_or_replaced_is_caught() {
        let existing = json!({
            "model": "opus",
            "env": {"EDITOR": "vi"},
            "sandbox": {"network": {"allowedDomains": ["api.example.com"]}},
            "permissions": {"allow": ["Bash(ls *)"], "deny": ["Bash(rm *)"]},
        });
        let result = composed(&existing, &settings());
        assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
        let document = &result.document;
        assert_eq!(
            keys(document),
            ["model", "env", "sandbox", "permissions", "hooks"]
        );
        assert_eq!(document["model"], existing["model"]);
        assert_eq!(document["env"], existing["env"]);
        assert_eq!(
            keys(&document["sandbox"]),
            [
                "network",
                "enabled",
                "failIfUnavailable",
                "allowUnsandboxedCommands",
                "filesystem"
            ]
        );
        assert_eq!(
            document["sandbox"]["network"],
            existing["sandbox"]["network"]
        );
        assert_eq!(keys(&document["permissions"]), ["allow", "deny"]);
        assert_eq!(
            document["permissions"]["allow"],
            existing["permissions"]["allow"]
        );
        let deny = document["permissions"]["deny"].as_array().unwrap();
        let ours = settings()[0]["permissions"]["deny"].clone();
        assert_eq!(deny[0], "Bash(rm *)");
        assert_eq!(Value::Array(deny[1..].to_vec()), ours);
    }

    #[test]
    fn a_duplicated_hook_or_rule_is_caught() {
        let mut ours = settings();
        ours.push(registration::render(&executable(), false));
        let first = compose(None, &ours, &folders(), &executable());
        assert!(first.conflicts.is_empty(), "{:?}", first.conflicts);
        let second = composed(&first.document, &ours);
        assert!(second.conflicts.is_empty(), "{:?}", second.conflicts);
        assert_eq!(second.document, first.document);
        assert_eq!(
            first.document["hooks"][hook::EVENT]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let deny = first.document["permissions"]["deny"].as_array().unwrap();
        assert_eq!(
            deny,
            settings()[0]["permissions"]["deny"].as_array().unwrap()
        );
    }

    #[test]
    fn a_stale_or_narrowed_baley_hook_kept_as_success_is_caught() {
        let ours = hook::render(&executable());
        let same = composed(&ours, &settings());
        assert!(same.conflicts.is_empty(), "{:?}", same.conflicts);
        assert_eq!(same.document["hooks"], ours["hooks"]);

        let mut bash_only = ours.clone();
        bash_only["hooks"][hook::EVENT][0]["matcher"] = json!("Bash");
        let mut elsewhere = ours.clone();
        elsewhere["hooks"][hook::EVENT][0]["hooks"][0]["command"] = json!("'/opt/old/baley' guard");
        for existing in [bash_only, elsewhere] {
            let item = existing["hooks"][hook::EVENT][0].clone();
            let result = composed(&existing, &settings());
            assert_eq!(result.conflicts, [Conflict::OtherGuardHook(item.clone())]);
            assert!(!result.is_complete());
            assert_eq!(
                result.document["hooks"][hook::EVENT],
                json!([item, ours["hooks"][hook::EVENT][0]])
            );
        }
    }

    #[test]
    fn a_registration_with_other_arguments_kept_as_success_is_caught() {
        let existing = json!({"mcpServers": {
            "other": {"command": "/usr/bin/other", "args": []},
            "baley": {"command": EXECUTABLE, "args": ["serve", "--x"]},
        }});
        let result = composed(&existing, &[registration::render(&executable(), false)]);
        assert_eq!(
            result.conflicts,
            [Conflict::OtherRegistration(
                existing["mcpServers"]["baley"].clone()
            )]
        );
        assert!(!result.is_complete());
        assert_eq!(result.document, existing);
    }

    #[test]
    fn a_bypass_kept_as_success_is_caught() {
        let inside_home = "/home/o/.local/share/crenshawdev/baley/ledger";
        let excluded = "/home/o/.local/bin/baley *";
        for (existing, expected) in [
            (
                json!({"sandbox": {"enabled": false}}),
                Conflict::Replaced {
                    key: "sandbox.enabled".into(),
                    was: json!(false),
                },
            ),
            (
                json!({"sandbox": {"failIfUnavailable": false}}),
                Conflict::Replaced {
                    key: "sandbox.failIfUnavailable".into(),
                    was: json!(false),
                },
            ),
            (
                json!({"sandbox": {"allowUnsandboxedCommands": true}}),
                Conflict::Replaced {
                    key: "sandbox.allowUnsandboxedCommands".into(),
                    was: json!(true),
                },
            ),
            (
                json!({"disableAllHooks": true}),
                Conflict::Disabled("disableAllHooks"),
            ),
            (
                json!({"sandbox": {"filesystem": {"disabled": true}}}),
                Conflict::Disabled("sandbox.filesystem.disabled"),
            ),
            (
                json!({"sandbox": {"filesystem": {"allowWrite": [inside_home]}}}),
                Conflict::Reopened {
                    list: "allowWrite",
                    entry: inside_home.into(),
                },
            ),
            (
                json!({"sandbox": {"filesystem": {"allowRead": [CONFIG]}}}),
                Conflict::Reopened {
                    list: "allowRead",
                    entry: CONFIG.into(),
                },
            ),
            (
                json!({"sandbox": {"excludedCommands": [excluded]}}),
                Conflict::Excluded(excluded.into()),
            ),
        ] {
            let result = composed(&existing, &settings());
            assert_eq!(result.conflicts, [expected], "{existing}");
            assert!(!result.is_complete(), "{existing}");
            assert_secure(&result.document);
        }
    }

    #[test]
    fn a_bypass_already_in_the_document_kept_when_the_proposal_is_not_composed_is_caught() {
        let hook_only = vec![hook::render(&executable())];
        let registration_only = vec![registration::render(&executable(), false)];
        for (existing, ours, key) in [
            (
                json!({"sandbox": {"enabled": false}}),
                &hook_only,
                "sandbox.enabled",
            ),
            (
                json!({"sandbox": {"failIfUnavailable": false}}),
                &hook_only,
                "sandbox.failIfUnavailable",
            ),
            (
                json!({"sandbox": {"allowUnsandboxedCommands": true}}),
                &hook_only,
                "sandbox.allowUnsandboxedCommands",
            ),
            (
                json!({"sandbox": {"enabled": false}}),
                &registration_only,
                "sandbox.enabled",
            ),
            (
                json!({"sandbox": {"enabled": true, "failIfUnavailable": false}}),
                &hook_only,
                "sandbox.failIfUnavailable",
            ),
        ] {
            let result = composed(&existing, ours);
            assert_eq!(result.conflicts, [Conflict::Disabled(key)], "{existing}");
            assert!(!result.is_complete(), "{existing}");
            assert_eq!(result.document["sandbox"], existing["sandbox"]);
        }
    }

    #[test]
    fn a_non_absolute_allow_entry_kept_as_success_is_caught() {
        for (list, entry) in [
            ("allowRead", "~/.local/share/crenshawdev/baley/ledger"),
            ("allowWrite", "relative/path"),
        ] {
            let existing = json!({"sandbox": {"filesystem": {list: [entry]}}});
            let result = composed(&existing, &settings());
            assert_eq!(
                result.conflicts,
                [Conflict::Unjudged {
                    list,
                    entry: entry.into(),
                }]
            );
            assert!(!result.is_complete(), "{entry}");
        }
        let existing = json!({"sandbox": {"filesystem": {"allowRead": ["/home/o/projects"]}}});
        let result = composed(&existing, &settings());
        assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
    }

    #[test]
    fn an_excluded_command_other_than_the_executable_kept_as_success_is_caught() {
        let existing = json!({"sandbox": {"excludedCommands": ["sh", "bash *"]}});
        let result = composed(&existing, &settings());
        assert_eq!(
            result.conflicts,
            [
                Conflict::Excluded("sh".into()),
                Conflict::Excluded("bash *".into()),
            ]
        );
        assert!(!result.is_complete());
        assert_eq!(
            result.document["sandbox"]["excludedCommands"],
            json!(["sh", "bash *"])
        );

        let existing = json!({"sandbox": {"excludedCommands": ["sh"]}});
        let result = composed(&existing, &[hook::render(&executable())]);
        assert_eq!(result.conflicts, [Conflict::Excluded("sh".into())]);
        assert!(!result.is_complete());
        assert_eq!(
            result.document["sandbox"]["excludedCommands"],
            json!(["sh"])
        );
    }

    #[test]
    fn a_many_star_exclusion_left_unreported_or_stalling_composition_is_caught() {
        let entry = "********************Z";
        let existing = json!({"sandbox": {"excludedCommands": [entry]}});
        let result = composed(&existing, &settings());
        assert_eq!(result.conflicts, [Conflict::Excluded(entry.into())]);
        assert!(!result.is_complete());
    }

    #[test]
    fn provider_hosts_treated_as_a_conflict_or_changed_is_caught() {
        let network = json!({"allowedDomains": ["api.openai.com", "api.deepseek.com"]});
        let existing = json!({"sandbox": {"network": network}});
        let result = composed(&existing, &settings());
        assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
        assert!(result.is_complete());
        assert_eq!(result.document["sandbox"]["network"], network);
    }
}
