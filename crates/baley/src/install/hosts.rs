//! The settings-file part of the installation writer that the interview's host
//! step uses (design 0003 section 5, CFG-R28). It adds the chosen providers'
//! API hosts to `sandbox.network.allowedDomains` in Claude Code's settings
//! file and nothing else: no install lock, no install record, and no
//! composition of Baley's hook or sandbox block.

use std::fs;
use std::path::{Path, PathBuf};

use baley_core::catalog::Provider;
use serde_json::{Map, Value};

use super::plan::Observation;
use super::settings::{self, Current};
use crate::folders::Environment;
use crate::host_artifacts::{installed, stubs};
use crate::host_doctor::placed;
use crate::ledger::display::Render;
use crate::replace::{self, Failure};

/// The settings document the host step writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Allowance {
    /// The document with the hosts added.
    pub document: Value,
    /// The document pretty-printed with a trailing newline.
    pub bytes: Vec<u8>,
    /// SHA-256 of the bytes the document was read from, or none when the file
    /// was absent.
    pub read_digest: Option<String>,
    /// The hosts this write adds, in the order they are appended.
    pub added: Vec<&'static str>,
}

/// The key that holds the list of allowed hosts.
const DOMAINS: &str = "allowedDomains";

/// The refusal for a settings file whose network setting cannot take hosts,
/// worded as `settings::judge` words a value Baley cannot add to.
fn not_composable(path: &Path, key: &str) -> String {
    format!(
        "install-settings-conflict: {} holds `{key}` as a kind of value Baley cannot add hosts to; fix it by hand, then run the interview again",
        path.display()
    )
}

/// The object a slot holds, or `key` when it holds anything else.
fn object<'a>(
    slot: &'a mut Value,
    key: &'static str,
) -> Result<&'a mut Map<String, Value>, &'static str> {
    slot.as_object_mut().ok_or(key)
}

/// Appends the wanted hosts the list does not hold, making `sandbox`,
/// `sandbox.network` and the list when they are missing. The hosts it added,
/// or the key that holds the wrong kind of value.
fn add_hosts(
    document: &mut Value,
    wanted: &[&'static str],
) -> Result<Vec<&'static str>, &'static str> {
    let empty = || Value::Object(Map::new());
    let root = object(document, "sandbox")?;
    let sandbox = object(root.entry("sandbox").or_insert_with(empty), "sandbox")?;
    let network = object(
        sandbox.entry("network").or_insert_with(empty),
        "sandbox.network",
    )?;
    let Value::Array(list) = network
        .entry(DOMAINS)
        .or_insert_with(|| Value::Array(Vec::new()))
    else {
        return Err("sandbox.network.allowedDomains");
    };
    let missing: Vec<&'static str> = wanted
        .iter()
        .copied()
        .filter(|host| !list.iter().any(|held| held.as_str() == Some(host)))
        .collect();
    list.extend(missing.iter().map(|host| Value::String((*host).to_owned())));
    Ok(missing)
}

/// Decides what to write so each chosen provider's API host is allowed. Hosts
/// are only added: every other key, value and host stays as it was, and a host
/// the file already holds is not added again. A new host goes after the
/// entries already there, in the catalog's order. `None` means nothing to
/// write. A file whose `sandbox`, `sandbox.network` or `allowedDomains` holds
/// the wrong kind of value is refused when a host is chosen, and a settings
/// path that is a symbolic link is refused only when a write is needed.
pub fn judge(
    path: &Path,
    current: &Current,
    chosen: &[Provider],
) -> Result<Option<Allowance>, String> {
    let wanted: Vec<&'static str> = Provider::ALL
        .into_iter()
        .filter(|provider| chosen.contains(provider))
        .map(Provider::api_host)
        .collect();
    if wanted.is_empty() {
        return Ok(None);
    }
    let mut document = current.document.clone();
    let added = add_hosts(&mut document, &wanted).map_err(|key| not_composable(path, key))?;
    if added.is_empty() {
        return Ok(None);
    }
    if current.symbolic_link {
        let link = replace::Conflict::Link { path: path.into() };
        return Err(settings::replace_refusal(&link));
    }
    let mut bytes = serde_json::to_vec_pretty(&document).expect("a JSON document serializes");
    bytes.push(b'\n');
    Ok(Some(Allowance {
        document,
        bytes,
        read_digest: current.read_digest.clone(),
        added,
    }))
}

/// Claude Code's settings file as the interview read it before its questions.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeSettings {
    /// The settings file's path.
    pub path: PathBuf,
    /// What the read found.
    pub current: Current,
}

/// Reads Claude Code's settings file where `baley install` places it, from
/// the environment and `CLAUDE_CONFIG_DIR`. The refusal is the line that says
/// why the path or the file cannot be used.
#[expect(
    dead_code,
    reason = "the interview calls it once the ordered writes are wired"
)]
pub(crate) fn read() -> Result<ClaudeSettings, String> {
    let manifest = stubs::manifest(&stubs::front_doors()).expect("unique compiled front doors");
    let placed_at = installed::resolve(
        &Environment::read(),
        std::env::var_os("CLAUDE_CONFIG_DIR"),
        &manifest,
    )
    .map_err(|refusal| refusal.to_string())?;
    let path = placed_at.settings_file;
    let seen = Observation {
        state: placed::read(&path),
        symbolic_link: fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_symlink()),
    };
    let current = settings::current(&path, &seen)?;
    Ok(ClaudeSettings { path, current })
}

/// Writes the decided document through the digest-checked replacement, so a
/// file changed since it was read is refused. The settings file's folder is
/// made first when it is missing, with the process's default mode since it is
/// Claude Code's folder. The note is the sync failure's text when the file was
/// replaced but its folder could not be synced.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the interview calls it once the ordered writes are wired"
    )
)]
pub(crate) fn write(path: &Path, allowance: &Allowance) -> Result<Option<String>, Render> {
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let made = fs::create_dir_all(folder).map_err(|cause| Failure::Unchanged {
        path: path.into(),
        cause,
    });
    let result = made
        .and_then(|()| replace::replace(path, &allowance.bytes, allowance.read_digest.as_deref()));
    written(result)
}

/// What a write's result means to the owner: a refused replacement is
/// `install-settings-conflict` with exit 2, a file or folder that could not be
/// written is `not-writable` with exit 3, and a file replaced but not synced
/// counts as written and keeps the sync failure's text.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the interview reaches it through write once the ordered writes are wired"
    )
)]
pub(crate) fn written(result: Result<(), Failure>) -> Result<Option<String>, Render> {
    match result {
        Ok(()) => Ok(None),
        Err(Failure::Unsynced { path, cause }) => {
            Ok(Some(Failure::Unsynced { path, cause }.to_string()))
        }
        Err(Failure::Refused(conflict)) => {
            Err(Render::refusal(settings::replace_refusal(&conflict)))
        }
        Err(Failure::Unchanged { path, cause }) => Err(Render {
            lines: vec![format!("not-writable: {}: {cause}", path.display())],
            code: 3,
            error: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    use crate::host_doctor::placed::FileState;
    use crate::install::plan::Observation;
    use serde_json::json;

    const PATH: &str = "/home/o/.claude/settings.json";

    fn read(document: &Value) -> Current {
        let seen = Observation {
            state: FileState::Bytes(serde_json::to_vec(document).unwrap()),
            symbolic_link: false,
        };
        settings::current(Path::new(PATH), &seen).unwrap()
    }

    fn pretty(document: &Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(document).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn a_provider_host_added_by_dropping_an_owner_setting_or_host_is_caught() {
        let before = json!({
            "model": "opus",
            "permissions": {"deny": ["Read(//home/o/.ssh/**)"]},
            "sandbox": {
                "enabled": true,
                "network": {
                    "allowedDomains": ["example.com"],
                    "allowUnixSockets": ["/run/x.sock"],
                },
            },
        });
        let current = read(&before);

        let one = judge(Path::new(PATH), &current, &[Provider::OpenAi])
            .unwrap()
            .expect("a host to add");
        let mut expected = before.clone();
        expected["sandbox"]["network"]["allowedDomains"] = json!(["example.com", "api.openai.com"]);
        assert_eq!(one.document, expected);
        assert_eq!(one.bytes, pretty(&expected));
        assert_eq!(one.read_digest, current.read_digest);
        assert!(one.read_digest.is_some());

        let both = judge(Path::new(PATH), &current, &Provider::ALL)
            .unwrap()
            .expect("hosts to add");
        let mut expected = before;
        expected["sandbox"]["network"]["allowedDomains"] =
            json!(["example.com", "api.openai.com", "api.deepseek.com"]);
        assert_eq!(both.document, expected);
    }
    fn linked(document: &Value) -> Current {
        let seen = Observation {
            state: FileState::Bytes(serde_json::to_vec(document).unwrap()),
            symbolic_link: true,
        };
        settings::current(Path::new(PATH), &seen).unwrap()
    }

    #[test]
    fn an_allowed_host_written_again_or_listed_twice_is_caught() {
        let held = read(&json!({"sandbox": {"network": {"allowedDomains": ["api.openai.com"]}}}));

        assert_eq!(judge(Path::new(PATH), &held, &[Provider::OpenAi]), Ok(None));
        let both = judge(Path::new(PATH), &held, &Provider::ALL)
            .unwrap()
            .expect("the second host to add");
        assert_eq!(
            both.document["sandbox"]["network"]["allowedDomains"],
            json!(["api.openai.com", "api.deepseek.com"])
        );
    }

    #[test]
    fn a_network_setting_that_cannot_take_hosts_written_anyway_is_caught() {
        let cases = [
            (json!({"sandbox": "off"}), "sandbox"),
            (json!({"sandbox": {"network": ["a"]}}), "sandbox.network"),
            (
                json!({"sandbox": {"network": {"allowedDomains": {"a": 1}}}}),
                "sandbox.network.allowedDomains",
            ),
            (json!({"sandbox": null}), "sandbox"),
            (json!({"sandbox": {"network": null}}), "sandbox.network"),
            (
                json!({"sandbox": {"network": {"allowedDomains": null}}}),
                "sandbox.network.allowedDomains",
            ),
        ];
        for (document, key) in cases {
            let line = judge(Path::new(PATH), &read(&document), &[Provider::OpenAi])
                .expect_err(&document.to_string());

            assert!(line.starts_with("install-settings-conflict: "), "{line}");
            assert!(line.contains(PATH), "{line}");
            assert!(line.contains(&format!("`{key}`")), "{line}");
        }
    }

    #[test]
    fn a_linked_settings_file_written_through_for_hosts_is_caught() {
        let refused = judge(
            Path::new(PATH),
            &linked(&json!({"model": "opus"})),
            &[Provider::OpenAi],
        );
        assert_eq!(
            refused,
            Err("install-settings-conflict: /home/o/.claude/settings.json is a symbolic link, which Baley does not write through (fix: edit the file it points to by hand, or replace the link with a regular file)".to_owned())
        );

        let held = linked(&json!({"sandbox": {"network": {"allowedDomains": ["api.openai.com"]}}}));
        assert_eq!(judge(Path::new(PATH), &held, &[Provider::OpenAi]), Ok(None));
    }

    #[test]
    fn an_absent_settings_file_given_more_than_the_hosts_is_caught() {
        let absent = settings::current(
            Path::new(PATH),
            &Observation {
                state: FileState::Absent,
                symbolic_link: false,
            },
        )
        .unwrap();

        let decided = judge(Path::new(PATH), &absent, &[Provider::OpenAi])
            .unwrap()
            .expect("a first allowance");

        assert_eq!(
            decided.document,
            json!({"sandbox": {"network": {"allowedDomains": ["api.openai.com"]}}})
        );
        assert_eq!(decided.read_digest, None);
    }
    #[test]
    fn a_host_write_failure_reported_under_the_wrong_code_is_caught() {
        let path = PathBuf::from(PATH);
        let conflict = replace::Conflict::Changed { path: path.clone() };
        let render = written(Err(Failure::Refused(conflict))).unwrap_err();
        assert_eq!(render.code, 2);
        assert!(render.error);
        assert!(
            render.lines[0].starts_with("install-settings-conflict: "),
            "{render:?}"
        );

        let denied = io::Error::from_raw_os_error(13);
        let temporary = PathBuf::from("/home/o/.claude/.settings.json.7-0.tmp");
        let render = written(Err(Failure::Unchanged {
            path: temporary,
            cause: denied,
        }))
        .unwrap_err();
        assert_eq!(render.code, 3);
        assert!(render.error);
        assert_eq!(
            render.lines,
            [
                "not-writable: /home/o/.claude/.settings.json.7-0.tmp: Permission denied (os error 13)"
            ]
        );
        assert!(render.lines[0].contains("Permission denied"), "{render:?}");

        let unsynced = written(Err(Failure::Unsynced {
            path: path.clone(),
            cause: io::Error::from_raw_os_error(5),
        }))
        .unwrap();
        let note = unsynced.expect("the sync failure's text is kept");
        assert!(note.contains(PATH), "{note}");
        assert!(note.contains("Input/output error"), "{note}");

        let folder = written(Err(Failure::Unchanged {
            path,
            cause: io::Error::from_raw_os_error(13),
        }))
        .unwrap_err();
        assert_eq!(folder.code, 3);
        assert!(folder.lines[0].starts_with("not-writable: "), "{folder:?}");

        assert_eq!(written(Ok(())).unwrap(), None);
    }

    /// Reads and decides for OpenAI from the file at `path`, as the interview
    /// does before its questions.
    fn decided(path: &Path) -> Allowance {
        let seen = Observation {
            state: placed::read(path),
            symbolic_link: false,
        };
        let current = settings::current(path, &seen).unwrap();
        judge(path, &current, &[Provider::OpenAi])
            .unwrap()
            .expect("a host to add")
    }

    fn parsed(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn a_host_write_over_a_settings_file_changed_since_it_was_read_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"model":"opus"}"#).unwrap();
        let allowance = decided(&path);
        fs::write(&path, r#"{"model":"sonnet"}"#).unwrap();

        let refused = write(&path, &allowance).unwrap_err();

        assert_eq!(refused.code, 2);
        assert!(
            refused.lines[0].starts_with("install-settings-conflict: "),
            "{refused:?}"
        );
        assert_eq!(parsed(&path), json!({"model": "sonnet"}));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"model":"opus"}"#).unwrap();
        let allowance = decided(&path);

        write(&path, &allowance).unwrap();

        assert_eq!(
            parsed(&path),
            json!({"model": "opus", "sandbox": {"network": {"allowedDomains": ["api.openai.com"]}}})
        );
    }

    #[test]
    fn a_missing_claude_folder_left_uncreated_for_the_hosts_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude").join("settings.json");
        let allowance = decided(&path);

        write(&path, &allowance).unwrap();

        assert_eq!(parsed(&path), allowance.document);

        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("claude");
        let path = folder.join("settings.json");
        let allowance = decided(&path);
        fs::create_dir(&folder).unwrap();
        fs::write(&path, r#"{"model":"sonnet"}"#).unwrap();

        let refused = write(&path, &allowance).unwrap_err();

        assert_eq!(refused.code, 2);
        assert!(
            refused.lines[0].starts_with("install-settings-conflict: "),
            "{refused:?}"
        );
        assert_eq!(parsed(&path), json!({"model": "sonnet"}));
    }
}
