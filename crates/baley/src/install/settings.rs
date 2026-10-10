//! Claude Code's user settings file for `baley install`: what a read of it
//! found, which older Baley entries leave it on the install record's
//! evidence, and what Baley's entries compose into it. Nothing here reads or
//! writes a file.

use std::path::Path;

use serde_json::{Map, Value};

use crate::host_doctor::placed::{self, FileState};
use crate::store::model::digest;

use super::plan::Observation;

/// The settings file as a read found it, ready to compose into.
#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    /// The JSON object the file holds, or an empty one when it is absent.
    pub document: Value,
    /// Whether the settings path is itself a symbolic link. A link whose
    /// target reads as an object is accepted here and refused only when a
    /// write is needed.
    pub symbolic_link: bool,
    /// SHA-256 of the bytes read, or none when the file is absent.
    pub read_digest: Option<String>,
}

/// The refusal for a settings file that cannot be composed into.
fn conflict(path: &Path, cause: &str) -> String {
    format!(
        "install-settings-conflict: {} {cause}; fix it by hand, then run baley install again",
        path.display()
    )
}

/// Judges what the read found. An absent file is an empty document, and a
/// file that is not a JSON object, or is not a readable file, is refused:
/// it is never read as an empty one, since composing into it would write
/// over the owner's file.
pub fn current(path: &Path, seen: &Observation) -> Result<Current, String> {
    match &seen.state {
        FileState::Absent => Ok(Current {
            document: Value::Object(Map::new()),
            symbolic_link: seen.symbolic_link,
            read_digest: None,
        }),
        FileState::Fault(fault) => Err(conflict(path, &fault.to_string())),
        FileState::Bytes(bytes) => {
            let document =
                placed::document(bytes).map_err(|fault| conflict(path, &fault.to_string()))?;
            Ok(Current {
                document,
                symbolic_link: seen.symbolic_link,
                read_digest: Some(digest(bytes)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::host_doctor::placed::{Fault, FileState};
    use crate::install::plan::Observation;

    const PATH: &str = "/home/o/.claude/settings.json";

    fn seen(state: FileState) -> Observation {
        Observation {
            state,
            symbolic_link: false,
        }
    }

    #[test]
    fn an_unparseable_settings_file_composed_into_is_caught() {
        let bytes = br#"{"model": "opus","#.to_vec();
        let refusal = current(Path::new(PATH), &seen(FileState::Bytes(bytes)))
            .expect_err("bytes that are not JSON give no document to compose into");
        assert!(
            refusal.starts_with("install-settings-conflict: "),
            "{refusal}"
        );
        assert!(refusal.contains(PATH), "{refusal}");
        assert!(refusal.contains("is not JSON"), "{refusal}");
        assert_eq!(refusal.lines().count(), 1);
    }

    #[test]
    fn a_settings_file_that_is_not_an_object_or_not_a_file_composed_into_is_caught() {
        for (state, cause) in [
            (
                FileState::Bytes(b"[1]".to_vec()),
                "is JSON but not an object",
            ),
            (
                FileState::Bytes(br#""x""#.to_vec()),
                "is JSON but not an object",
            ),
            (FileState::Fault(Fault::NotRegular), "is not a regular file"),
            (
                FileState::Fault(Fault::DanglingLink),
                "is a link to a missing file",
            ),
            (
                FileState::Fault(Fault::Unreadable("Permission denied".into())),
                "Permission denied",
            ),
        ] {
            let refusal = current(Path::new(PATH), &seen(state.clone()))
                .expect_err("a file that is not a settings document is refused");
            assert!(
                refusal.starts_with("install-settings-conflict: "),
                "{refusal}"
            );
            assert!(refusal.contains(PATH), "{refusal}");
            assert!(refusal.contains(cause), "{refusal}");
        }
        let empty = current(Path::new(PATH), &seen(FileState::Absent)).expect("absent is empty");
        assert_eq!(empty.document, serde_json::json!({}));
        assert_eq!(empty.read_digest, None);
    }
}
