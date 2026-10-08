//! Placed files and the executable, read and judged.
//!
//! Gathering and judging are apart, as `settings.rs` has them. Gathering
//! reads a path once through `settings::gather`, or asks the filesystem
//! about the executable, and owns no rule. Judging turns what was seen into
//! plain values: a file that is absent, faulty or holds bytes, a document
//! that is a JSON object or a fault, an executable that is present or has a
//! gap. A file that is not readable is never taken for an empty one, because
//! the coverage judge would turn an empty document into invented gaps.

use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::settings::{self, Seen};

/// Why a placed file cannot be used as found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// A link whose target does not exist.
    DanglingLink,
    /// Something other than a regular file is there.
    NotRegular,
    /// The file is there but could not be read; the system's cause.
    Unreadable(String),
    /// The bytes are not JSON; the parser's message.
    NotJson(String),
    /// The bytes are JSON but not an object.
    NotObject,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::DanglingLink => f.write_str("is a link to a missing file"),
            Fault::NotRegular => f.write_str("is not a regular file"),
            Fault::Unreadable(cause) => write!(f, "cannot be read: {cause}"),
            Fault::NotJson(message) => write!(f, "is not JSON: {message}"),
            Fault::NotObject => f.write_str("is JSON but not an object"),
        }
    }
}

/// What one read of a placed path found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// Nothing is there.
    Absent,
    /// Something is there that cannot be used.
    Fault(Fault),
    /// The bytes, exactly as read. They are never decoded as text, since a
    /// stub is compared and hashed byte for byte.
    Bytes(Vec<u8>),
}

/// Reads one placed path.
pub(crate) fn read(path: &Path) -> FileState {
    file_state(settings::gather(path))
}

/// Judges what a read found, by the rules `settings::judge` follows but
/// with this module's words.
pub(crate) fn file_state(seen: Seen) -> FileState {
    match seen {
        Seen::NotOpened { cause, link } => {
            let missing = cause.kind() == std::io::ErrorKind::NotFound;
            match link {
                Err(error) if missing && error.kind() == std::io::ErrorKind::NotFound => {
                    FileState::Absent
                }
                Ok(true) if missing => FileState::Fault(Fault::DanglingLink),
                _ => FileState::Fault(Fault::Unreadable(cause.to_string())),
            }
        }
        Seen::NotRegular => FileState::Fault(Fault::NotRegular),
        Seen::Unreadable { cause } => FileState::Fault(Fault::Unreadable(cause)),
        Seen::Opened(bytes) => FileState::Bytes(bytes),
    }
}

/// Reads bytes as a JSON object. Any other JSON value, and anything that
/// does not parse, is a fault and never an empty document.
pub fn document(bytes: &[u8]) -> Result<Value, Fault> {
    match serde_json::from_slice::<Value>(bytes) {
        Err(error) => Err(Fault::NotJson(error.to_string())),
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => Err(Fault::NotObject),
    }
}

/// Why the executable the hook and the registration run cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableGap {
    /// Nothing is at the path.
    Missing,
    /// A link whose target does not exist.
    DanglingLink,
    /// Something other than a regular file is there, a directory for one.
    NotRegular,
    /// It could not be examined; the system's cause.
    Unreadable(String),
}

impl fmt::Display for ExecutableGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutableGap::Missing => f.write_str("does not exist"),
            ExecutableGap::DanglingLink => f.write_str("is a link to a missing file"),
            ExecutableGap::NotRegular => f.write_str("is not a regular file"),
            ExecutableGap::Unreadable(cause) => write!(f, "cannot be examined: {cause}"),
        }
    }
}

/// What the filesystem said about the executable's path.
pub(crate) struct ExecutableSeen {
    /// Whether the path itself is a link, from `symlink_metadata`.
    pub(crate) link: std::io::Result<bool>,
    /// Whether the path, with links followed, is a regular file.
    pub(crate) target: std::io::Result<bool>,
}

/// Asks the filesystem about the executable. Nothing is opened or run.
pub(crate) fn observe_executable(path: &Path) -> Option<ExecutableGap> {
    executable_gap(ExecutableSeen {
        link: std::fs::symlink_metadata(path).map(|meta| meta.file_type().is_symlink()),
        target: std::fs::metadata(path).map(|meta| meta.is_file()),
    })
}

/// The executable's gap, or none when it is a regular file, reached through
/// a link or not.
pub(crate) fn executable_gap(seen: ExecutableSeen) -> Option<ExecutableGap> {
    match seen.target {
        Ok(true) => None,
        Ok(false) => Some(ExecutableGap::NotRegular),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match seen.link {
            Ok(true) => Some(ExecutableGap::DanglingLink),
            _ => Some(ExecutableGap::Missing),
        },
        Err(error) => Some(ExecutableGap::Unreadable(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    #[test]
    fn a_placed_directory_or_unreadable_file_reported_as_missing_or_present_is_caught() {
        let not_found = || std::io::Error::from(ErrorKind::NotFound);
        assert_eq!(
            file_state(Seen::NotOpened {
                cause: not_found(),
                link: Err(not_found()),
            }),
            FileState::Absent
        );
        assert_eq!(
            file_state(Seen::NotOpened {
                cause: not_found(),
                link: Ok(true),
            }),
            FileState::Fault(Fault::DanglingLink)
        );
        assert_eq!(
            file_state(Seen::NotRegular),
            FileState::Fault(Fault::NotRegular)
        );
        let cause = "Permission denied (os error 13)";
        assert_eq!(
            file_state(Seen::Unreadable {
                cause: cause.into()
            }),
            FileState::Fault(Fault::Unreadable(cause.into()))
        );
        assert_eq!(
            file_state(Seen::Opened(vec![0xff, 0x00, 0x78])),
            FileState::Bytes(vec![0xff, 0x00, 0x78])
        );
    }

    #[test]
    fn a_document_that_is_not_a_json_object_judged_as_empty_settings_is_caught() {
        let message = serde_json::from_str::<Value>("{").unwrap_err().to_string();
        assert_eq!(document(b"{"), Err(Fault::NotJson(message)));
        for scalar in ["[]", "null", "true", "3", "\"s\""] {
            assert_eq!(
                document(scalar.as_bytes()),
                Err(Fault::NotObject),
                "{scalar}"
            );
        }
        assert_eq!(document(b"{}"), Ok(Value::Object(Default::default())));
    }

    #[test]
    fn a_directory_or_dangling_link_counted_as_the_executable_is_caught() {
        let not_found = || std::io::Error::from(ErrorKind::NotFound);
        let seen = |link: std::io::Result<bool>, target: std::io::Result<bool>| {
            executable_gap(ExecutableSeen { link, target })
        };
        assert_eq!(
            seen(Err(not_found()), Err(not_found())),
            Some(ExecutableGap::Missing)
        );
        assert_eq!(
            seen(Ok(true), Err(not_found())),
            Some(ExecutableGap::DanglingLink)
        );
        assert_eq!(seen(Ok(false), Ok(false)), Some(ExecutableGap::NotRegular));
        assert_eq!(seen(Ok(true), Ok(true)), None);
        assert_eq!(seen(Ok(false), Ok(true)), None);
    }
}
