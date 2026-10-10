//! The settings-file part of the installation writer that the interview's host
//! step uses (design 0003 section 5, CFG-R28). It adds the chosen providers'
//! API hosts to `sandbox.network.allowedDomains` in Claude Code's settings
//! file and nothing else: no install lock, no install record, and no
//! composition of Baley's hook or sandbox block.

use std::path::Path;

use baley_core::catalog::Provider;
use serde_json::{Map, Value};

use super::settings::{self, Current};
use crate::replace;

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
/// `sandbox.network` and the list when they are missing. Whether it added any,
/// or the key that holds the wrong kind of value.
fn add_hosts(document: &mut Value, wanted: &[&str]) -> Result<bool, &'static str> {
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
    let missing: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|host| !list.iter().any(|held| held.as_str() == Some(host)))
        .collect();
    list.extend(missing.iter().map(|host| Value::String((*host).to_owned())));
    Ok(!missing.is_empty())
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
    let wanted: Vec<&str> = Provider::ALL
        .into_iter()
        .filter(|provider| chosen.contains(provider))
        .map(Provider::api_host)
        .collect();
    if wanted.is_empty() {
        return Ok(None);
    }
    let mut document = current.document.clone();
    if !add_hosts(&mut document, &wanted).map_err(|key| not_composable(path, key))? {
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
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
