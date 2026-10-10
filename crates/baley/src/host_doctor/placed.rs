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
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::Value;

use crate::host_artifacts::compose::same_server;
use crate::host_artifacts::executable::Executable;
use crate::host_artifacts::registration;
use crate::host_artifacts::stubs::Entry;
use crate::settings::{self, Seen};
use crate::store::model::digest;

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
    /// A regular file is there but no execute bit is set for anyone, so the
    /// hook and the registration fail to start it.
    NotExecutable,
    /// It could not be examined; the system's cause.
    Unreadable(String),
}

impl fmt::Display for ExecutableGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutableGap::Missing => f.write_str("does not exist"),
            ExecutableGap::DanglingLink => f.write_str("is a link to a missing file"),
            ExecutableGap::NotRegular => f.write_str("is not a regular file"),
            ExecutableGap::NotExecutable => f.write_str("has no execute permission"),
            ExecutableGap::Unreadable(cause) => write!(f, "cannot be examined: {cause}"),
        }
    }
}

/// What the filesystem said about the executable's path.
pub(crate) struct ExecutableSeen {
    /// Whether the path itself is a link, from `symlink_metadata`.
    pub(crate) link: std::io::Result<bool>,
    /// The file at the path, with links followed.
    pub(crate) target: std::io::Result<Target>,
}

/// The file an executable's path leads to.
pub(crate) struct Target {
    /// Whether it is a regular file.
    pub(crate) regular_file: bool,
    /// Its mode bits.
    pub(crate) mode: u32,
}

/// Asks the filesystem about the executable. Nothing is opened or run.
pub(crate) fn observe_executable(path: &Path) -> Option<ExecutableGap> {
    executable_gap(ExecutableSeen {
        link: std::fs::symlink_metadata(path).map(|meta| meta.file_type().is_symlink()),
        target: std::fs::metadata(path).map(|meta| Target {
            regular_file: meta.is_file(),
            mode: meta.permissions().mode(),
        }),
    })
}

/// The executable's gap, or none when it is a regular file with any execute
/// bit set, reached through a link or not. Any bit counts, as it does for
/// the sandbox programs in `prerequisites.rs`.
pub(crate) fn executable_gap(seen: ExecutableSeen) -> Option<ExecutableGap> {
    match seen.target {
        Ok(target) if !target.regular_file => Some(ExecutableGap::NotRegular),
        Ok(target) if target.mode & 0o111 == 0 => Some(ExecutableGap::NotExecutable),
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match seen.link {
            Ok(true) => Some(ExecutableGap::DanglingLink),
            _ => Some(ExecutableGap::Missing),
        },
        Err(error) => Some(ExecutableGap::Unreadable(error.to_string())),
    }
}

/// How a placed stub compares with the manifest's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StubJudgement {
    /// The bytes are the manifest entry's, byte for byte.
    Matches,
    /// The bytes differ.
    Differs {
        /// Lowercase hex SHA-256 of the bytes found, to set beside the
        /// entry's own digest.
        found_digest: String,
    },
}

/// Compares the bytes found at a stub's place with the manifest entry's
/// bytes. Only equality counts: a stub that is longer, shorter or changed
/// by one byte is a different stub.
pub fn stub(entry: &Entry, found: &[u8]) -> StubJudgement {
    if found == entry.bytes.as_slice() {
        StubJudgement::Matches
    } else {
        StubJudgement::Differs {
            found_digest: digest(found),
        }
    }
}

/// How a registration document compares with the entry Baley renders.
#[derive(Debug, Clone, PartialEq)]
pub enum RegistrationJudgement {
    /// The `mcpServers` entry under Baley's key runs the same command with
    /// the same arguments and sets no environment entry.
    Matches,
    /// The document has no entry under Baley's key.
    Missing,
    /// The entry runs another command or other arguments, or sets
    /// environment entries; the entry found.
    Differs(Value),
}

/// Judges the entry under `mcpServers.baley` by the rule `baley install` and
/// composition use: equal `command` and `args` and no environment entry, with
/// `alwaysLoad` and any other key ignored.
pub fn registration(document: &Value, executable: &Executable) -> RegistrationJudgement {
    let key = registration::KEY;
    let Some(found) = document.pointer(&format!("/mcpServers/{key}")) else {
        return RegistrationJudgement::Missing;
    };
    let ours = registration::render(executable, false);
    if same_server(found, &ours["mcpServers"][key]) {
        RegistrationJudgement::Matches
    } else {
        RegistrationJudgement::Differs(found.clone())
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
        let seen = |link: std::io::Result<bool>, target: std::io::Result<Target>| {
            executable_gap(ExecutableSeen { link, target })
        };
        let file = |regular_file: bool| {
            Ok(Target {
                regular_file,
                mode: 0o755,
            })
        };
        assert_eq!(
            seen(Err(not_found()), Err(not_found())),
            Some(ExecutableGap::Missing)
        );
        assert_eq!(
            seen(Ok(true), Err(not_found())),
            Some(ExecutableGap::DanglingLink)
        );
        assert_eq!(
            seen(Ok(false), file(false)),
            Some(ExecutableGap::NotRegular)
        );
        assert_eq!(seen(Ok(true), file(true)), None);
        assert_eq!(seen(Ok(false), file(true)), None);
    }

    #[test]
    fn a_regular_file_with_no_execute_bit_counted_as_the_executable_is_caught() {
        let seen = |mode: u32| {
            executable_gap(ExecutableSeen {
                link: Ok(false),
                target: Ok(Target {
                    regular_file: true,
                    mode,
                }),
            })
        };
        for mode in [0o644, 0o600, 0o000] {
            assert_eq!(seen(mode), Some(ExecutableGap::NotExecutable), "{mode:o}");
        }
        // Any one bit lets someone start it, the rule the sandbox programs use.
        for mode in [0o755, 0o100, 0o010, 0o001] {
            assert_eq!(seen(mode), None, "{mode:o}");
        }
        let directory = executable_gap(ExecutableSeen {
            link: Ok(false),
            target: Ok(Target {
                regular_file: false,
                mode: 0o000,
            }),
        });
        assert_eq!(directory, Some(ExecutableGap::NotRegular));
    }
    fn entry(identity: &str) -> Entry {
        let manifest =
            crate::host_artifacts::stubs::manifest(&crate::host_artifacts::stubs::front_doors())
                .unwrap();
        manifest
            .into_iter()
            .find(|entry| entry.identity == identity)
            .unwrap()
    }

    #[test]
    fn a_stub_judged_by_length_or_prefix_is_caught() {
        let help = entry("bal-help");
        let mut changed = help.bytes.clone();
        changed[0] ^= 1;
        let mut appended = help.bytes.clone();
        appended.push(b'\n');
        let mut cut = help.bytes.clone();
        cut.pop();
        assert_eq!(stub(&help, &help.bytes), StubJudgement::Matches);
        for found in [changed, appended, cut] {
            assert_eq!(
                stub(&help, &found),
                StubJudgement::Differs {
                    found_digest: digest(&found)
                }
            );
        }
        let capture = entry("bal-capture");
        assert_eq!(stub(&capture, &capture.bytes), StubJudgement::Matches);
    }

    #[test]
    fn a_registration_with_always_load_called_different_or_other_arguments_accepted_is_caught() {
        let exe = Executable::new("/usr/local/bin/baley").unwrap();
        let entry_of = |document: &Value| document["mcpServers"][registration::KEY].clone();
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut document = registration::render(&exe, false);
            edit(&mut document["mcpServers"][registration::KEY]);
            document
        };
        let described = with(&|entry| entry["description"] = "x".into());
        let other_args = with(&|entry| entry["args"] = serde_json::json!(["serve", "--x"]));
        let other_command = with(&|entry| entry["command"] = "/other/baley".into());
        assert_eq!(
            registration(&registration::render(&exe, true), &exe),
            RegistrationJudgement::Matches
        );
        assert_eq!(
            registration(&described, &exe),
            RegistrationJudgement::Matches
        );
        assert_eq!(
            registration(&other_args, &exe),
            RegistrationJudgement::Differs(entry_of(&other_args))
        );
        assert_eq!(
            registration(&other_command, &exe),
            RegistrationJudgement::Differs(entry_of(&other_command))
        );
        assert_eq!(
            registration(&serde_json::json!({}), &exe),
            RegistrationJudgement::Missing
        );
    }
    #[test]
    fn a_registration_install_would_refuse_called_matching_by_the_doctor_is_caught() {
        let executable = Executable::new("/home/o/.local/bin/baley").unwrap();
        let stored = |env: Value| {
            serde_json::json!({"mcpServers": {"baley": {
                "type": "stdio",
                "command": "/home/o/.local/bin/baley",
                "args": ["serve"],
                "env": env,
            }}})
        };

        let elsewhere = stored(serde_json::json!({"BALEY_HOME": "/elsewhere"}));
        assert_eq!(
            registration(&elsewhere, &executable),
            RegistrationJudgement::Differs(elsewhere["mcpServers"]["baley"].clone())
        );
        assert_eq!(
            registration(&stored(serde_json::json!({})), &executable),
            RegistrationJudgement::Matches
        );
    }
}
