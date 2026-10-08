//! The skill stubs Claude Code loads, one per served front door, and the
//! manifest that lists them (design 0012 section 10, HST-R12).
//!
//! A stub carries no instruction text. Its body is one line that sends the
//! session to the registry for the instruction, so the words a model follows
//! are always the compiled, versioned text and never a copy on disk. The
//! stub's bytes depend on nothing but the help table and the identity, so the
//! manifest is the same wherever the stub is placed and whatever the server
//! is registered as (D-03, D-04).

use std::fmt;

use baley_core::policy::Host;
use serde_json::json;

use crate::help::table::COMMANDS;
use crate::instruction::{self, Lookup};
use crate::store::model::digest;

/// The front doors this binary serves, as (identity, description): every help
/// table row whose registry lookup is served, in the table's order (D-01).
/// `bal-read-contract` is served but is no help row, so it is not one.
pub fn front_doors() -> Vec<(&'static str, &'static str)> {
    COMMANDS
        .iter()
        .filter(|command| matches!(instruction::lookup(command.name), Lookup::Served { .. }))
        .map(|command| (command.name, command.description))
        .collect()
}

/// The text of one stub: YAML frontmatter holding the identity as `name` and
/// the description as `description`, then one body line (D-02).
///
/// The description is written as a JSON string literal, which YAML reads as
/// a double-quoted scalar, so a colon or a quote in it cannot break the
/// frontmatter. There is no `allowed-tools` line: the stub names
/// `baley_query` by its bare tool name (D-03).
pub fn render(identity: &str, description: &str) -> String {
    let description = serde_json::to_string(description).expect("a string serializes");
    let query = json!({"operation": "instruction", "identity": identity});
    format!(
        "---\nname: {identity}\ndescription: {description}\n---\n\nCall `baley_query` with `{query}` and follow the instructions it returns.\n"
    )
}

/// One manifest entry: a stub as the bytes the host loads (D-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The host that loads the stub.
    pub host: Host,
    /// The front door's registry identity.
    pub identity: String,
    /// The stub's exact bytes.
    pub bytes: Vec<u8>,
    /// Lowercase hex SHA-256 of exactly `bytes`.
    pub digest: String,
}

/// A supplied front-door list that names one identity twice (D-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateIdentity(pub String);

impl fmt::Display for DuplicateIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "front door `{}` is listed twice", self.0)
    }
}

/// The manifest for a supplied front-door list, one entry per door in the
/// list's order. Production passes [`front_doors`]; a test may pass its own.
/// Placement is not an input: the placement map links an identity to a path.
pub fn manifest(front_doors: &[(&str, &str)]) -> Result<Vec<Entry>, DuplicateIdentity> {
    let mut entries: Vec<Entry> = Vec::with_capacity(front_doors.len());
    for (identity, description) in front_doors {
        if entries.iter().any(|entry| entry.identity == *identity) {
            return Err(DuplicateIdentity((*identity).to_owned()));
        }
        let bytes = render(identity, description).into_bytes();
        let digest = digest(&bytes);
        entries.push(Entry {
            host: Host::ClaudeCode,
            identity: (*identity).to_owned(),
            bytes,
            digest,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sha2::{Digest, Sha256};

    use super::*;

    /// The nineteen front doors not served yet, written out by hand as
    /// `instruction/tests.rs` lists them.
    const UNAVAILABLE: &[&str] = &[
        "bal-context",
        "bal-plan",
        "bal-review",
        "bal-plan-review",
        "bal-decision-review",
        "bal-minimalism-review",
        "bal-execute",
        "bal-verify",
        "bal-audit",
        "bal-coverage",
        "bal-land",
        "bal-milestone",
        "bal-undo",
        "bal-progress",
        "bal-suggest",
        "bal-task",
        "bal-debug",
        "bal-spike",
        "bal-why",
    ];

    /// The frontmatter and the body of a rendered stub.
    fn split(stub: &str) -> (&str, &str) {
        stub.strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .expect("a frontmatter block")
    }

    #[test]
    fn an_unavailable_front_door_or_the_read_contract_given_a_stub_is_caught() {
        let entries = manifest(&front_doors()).unwrap();
        let identities: Vec<&str> = entries
            .iter()
            .map(|entry| entry.identity.as_str())
            .collect();
        assert_eq!(identities, ["bal-capture", "bal-help"]);
        assert_eq!(UNAVAILABLE.len(), 19);
        for identity in UNAVAILABLE.iter().chain(&["bal-read-contract"]) {
            assert!(!identities.contains(identity), "{identity} has a stub");
        }
        for entry in &entries {
            assert_eq!(entry.host, Host::ClaudeCode);
        }
    }

    #[test]
    fn an_allowed_tools_line_a_missing_field_or_a_broken_quote_in_the_frontmatter_is_caught() {
        let description = r#"Shows: the "thing", twice: once"#;
        let stub = render("bal-help", description);
        let (frontmatter, _) = split(&stub);
        let fields: BTreeMap<String, String> = serde_saphyr::from_str(frontmatter).unwrap();
        let expected = BTreeMap::from([
            ("name".to_owned(), "bal-help".to_owned()),
            ("description".to_owned(), description.to_owned()),
        ]);
        assert_eq!(fields, expected);
    }

    #[test]
    fn the_full_instructions_copied_into_any_shipped_stub_body_is_caught() {
        let mut judged = Vec::new();
        for entry in manifest(&front_doors()).unwrap() {
            let identity = entry.identity.as_str();
            let stub = std::str::from_utf8(&entry.bytes).unwrap();
            let (_, body) = split(stub);
            let lines: Vec<&str> = body
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            assert_eq!(lines.len(), 1, "{identity}: {body:?}");
            let query = format!(r#"{{"operation":"instruction","identity":"{identity}"}}"#);
            assert!(lines[0].contains("baley_query"), "{identity}");
            assert!(lines[0].contains(&query), "{identity}: {}", lines[0]);
            let Lookup::Served { text, .. } = instruction::lookup(identity) else {
                panic!("{identity} is not served");
            };
            for line in text.body.lines().filter(|line| !line.trim().is_empty()) {
                assert!(!stub.contains(line), "{identity} holds {line:?}");
            }
            judged.push(identity.to_owned());
        }
        judged.sort();
        assert_eq!(judged, ["bal-capture", "bal-help"]);
    }

    #[test]
    fn a_digest_over_less_than_the_whole_stub_or_an_unstable_render_is_caught() {
        let first = manifest(&front_doors()).unwrap();
        let second = manifest(&front_doors()).unwrap();
        assert_eq!(first, second);
        for entry in &first {
            let expected: String = Sha256::digest(&entry.bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(entry.digest, expected, "{}", entry.identity);
            assert_eq!(entry.digest.len(), 64);
            assert!(
                entry
                    .digest
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            );
        }
    }

    #[test]
    fn a_front_door_listed_twice_is_refused_by_name() {
        let refused =
            manifest(&[("bal-help", "a"), ("bal-capture", "b"), ("bal-help", "c")]).unwrap_err();
        assert_eq!(refused, DuplicateIdentity("bal-help".to_owned()));
        assert!(refused.to_string().contains("bal-help"));
    }
}
