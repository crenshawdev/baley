//! Stub ownership from manifest bytes, a placed path, a supplied file read
//! and the latest install record. No file or environment value is read here.

use std::path::Path;

use serde_json::Value;

use crate::host_artifacts::stubs::Entry;
use crate::host_doctor::placed::FileState;
use crate::store::model::digest;

/// What installation may do with one placed stub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The path is absent, so the manifest bytes may be written.
    Write,
    /// The manifest bytes are already there, so no write is needed.
    Unchanged,
    /// The record proves the observed bytes belong to this stub.
    Replace {
        /// SHA-256 of the observed bytes, required by the replacement check.
        read_digest: String,
    },
}

/// Judges one stub without reading its path. Matching bytes stay unchanged,
/// including through a symbolic link. Other bytes need a recorded hash for
/// this identity, and a link may never be replaced. Conflicts name the artifact,
/// path, observed cause and how to retry.
pub fn judge(
    entry: &Entry,
    path: &Path,
    state: &FileState,
    symbolic_link: bool,
    latest: Option<&Value>,
) -> Result<Decision, String> {
    let conflict = |cause: &str| {
        format!(
            "install-ownership-conflict: stub `{}` at {} {cause}; move it aside and run `baley install` again",
            entry.identity,
            path.display()
        )
    };
    let link = "is a symbolic link, which Baley does not write through";
    match state {
        FileState::Absent if symbolic_link => Err(conflict(link)),
        FileState::Absent => Ok(Decision::Write),
        FileState::Fault(fault) => Err(conflict(&fault.to_string())),
        FileState::Bytes(bytes) if *bytes == entry.bytes => Ok(Decision::Unchanged),
        FileState::Bytes(_) if symbolic_link => Err(conflict(link)),
        FileState::Bytes(bytes) => {
            let read_digest = digest(bytes);
            let owned = latest
                .and_then(|record| record.get("stubs"))
                .and_then(Value::as_array)
                .is_some_and(|stubs| {
                    stubs.iter().any(|stub| {
                        stub.get("identity").and_then(Value::as_str) == Some(&entry.identity)
                            && stub.get("sha256").and_then(Value::as_str) == Some(&read_digest)
                    })
                });
            if owned {
                Ok(Decision::Replace { read_digest })
            } else {
                Err(conflict(
                    "holds bytes that match neither this binary's stub nor a recorded hash for this identity",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_artifacts::stubs::{front_doors, manifest};
    use crate::host_doctor::placed::Fault;
    use serde_json::json;

    const PATH: &str = "/home/o/.claude/skills/bal-help/SKILL.md";
    const OLD: &str = "cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4";

    fn help() -> Entry {
        manifest(&front_doors())
            .unwrap()
            .into_iter()
            .find(|entry| entry.identity == "bal-help")
            .unwrap()
    }

    #[test]
    fn a_stub_replaced_on_its_name_alone_is_caught() {
        let entry = help();
        let latest = json!({"stubs": [{
            "identity": "bal-help", "path": PATH, "sha256": OLD,
        }]});
        for (state, link, record, expected) in [
            (FileState::Absent, false, None, Decision::Write),
            (
                FileState::Bytes(entry.bytes.clone()),
                false,
                None,
                Decision::Unchanged,
            ),
            (
                FileState::Bytes(b"old".to_vec()),
                false,
                Some(&latest),
                Decision::Replace {
                    read_digest: OLD.into(),
                },
            ),
            (
                FileState::Bytes(entry.bytes.clone()),
                true,
                None,
                Decision::Unchanged,
            ),
        ] {
            assert_eq!(
                judge(&entry, Path::new(PATH), &state, link, record),
                Ok(expected),
                "{state:?}, link {link}"
            );
        }
        for (state, link, record, cause) in [
            (
                FileState::Bytes(b"mine".to_vec()),
                false,
                Some(&latest),
                "match neither",
            ),
            (
                FileState::Bytes(b"mine".to_vec()),
                false,
                None,
                "match neither",
            ),
            (
                FileState::Fault(Fault::NotRegular),
                false,
                None,
                "not a regular file",
            ),
            (
                FileState::Bytes(b"old".to_vec()),
                true,
                Some(&latest),
                "symbolic link",
            ),
            (
                FileState::Fault(Fault::DanglingLink),
                true,
                Some(&latest),
                "link to a missing file",
            ),
            (
                FileState::Fault(Fault::Unreadable("Permission denied".into())),
                false,
                Some(&latest),
                "Permission denied",
            ),
        ] {
            let refusal = judge(&entry, Path::new(PATH), &state, link, record)
                .expect_err("ownership conflict");
            assert!(
                refusal.starts_with(&format!(
                    "install-ownership-conflict: stub `bal-help` at {PATH} "
                )),
                "{refusal}"
            );
            assert!(refusal.contains(cause), "{refusal}");
            assert!(
                refusal.ends_with("move it aside and run `baley install` again"),
                "{refusal}"
            );
        }
    }

    #[test]
    fn a_recorded_hash_of_another_stub_taken_as_ownership_is_caught() {
        let latest = json!({"stubs": [{
            "identity": "bal-capture",
            "path": "/home/o/.claude/skills/bal-capture/SKILL.md",
            "sha256": "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881",
        }]});
        let refusal = judge(
            &help(),
            Path::new(PATH),
            &FileState::Bytes(b"x".to_vec()),
            false,
            Some(&latest),
        )
        .expect_err("another identity's hash is not ownership");
        assert!(refusal.starts_with("install-ownership-conflict:"));
        assert!(refusal.contains(PATH));
    }
}
