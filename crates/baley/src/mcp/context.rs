//! The session context: gathered from the process once at startup, then
//! judged as plain values.
//!
//! Gathering reads the environment and the filesystem and owns no policy.
//! Judging applies the rules and never prints or exits, so a bad project does
//! not end the process: project calls answer `failed` and project-independent
//! reads keep answering.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

use baley_store::{MAX_DIRECTORY_BYTES, MAX_HOST_SESSION_BYTES};

/// What the process told us at startup, before any rule is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// `CLAUDE_PROJECT_DIR`, or `None` when unset.
    pub project_dir: Option<OsString>,
    /// Whether that path is an existing directory (a link is followed).
    pub project_is_directory: bool,
    /// The startup working directory, or `None` when it cannot be read.
    pub working_dir: Option<OsString>,
    /// `CLAUDE_CODE_SESSION_ID`, or `None` when unset.
    pub host_session: Option<OsString>,
    /// A freshly minted lower-case hyphenated UUID version 4.
    pub minted_session: String,
}

impl Observation {
    /// Reads the process once. It holds no decision, so it has no unit test.
    pub fn gather() -> Self {
        let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR");
        let project_is_directory = project_dir
            .as_deref()
            .is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_dir()));
        Self {
            project_dir,
            project_is_directory,
            working_dir: std::env::current_dir().ok().map(OsString::from),
            host_session: std::env::var_os("CLAUDE_CODE_SESSION_ID"),
            minted_session: uuid::Uuid::new_v4().hyphenated().to_string(),
        }
    }
}

/// Why a directory value cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryFault {
    /// The value is empty.
    Empty,
    /// The value is not valid UTF-8.
    NotUtf8,
    /// The value is not an absolute path.
    Relative,
    /// The value is over the directory limit.
    TooLong {
        /// The value's length in bytes.
        bytes: usize,
    },
    /// The path is not an existing directory.
    NotADirectory,
    /// The value could not be read at all.
    Unreadable,
}

impl fmt::Display for DirectoryFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("it is empty"),
            Self::NotUtf8 => f.write_str("it is not valid UTF-8"),
            Self::Relative => f.write_str("it is not an absolute path"),
            Self::TooLong { bytes } => {
                write!(
                    f,
                    "it is {bytes} bytes, over the {MAX_DIRECTORY_BYTES}-byte limit"
                )
            }
            Self::NotADirectory => f.write_str("it is not an existing directory"),
            Self::Unreadable => f.write_str("it could not be read"),
        }
    }
}

/// The project the session was started for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectContext {
    /// `CLAUDE_PROJECT_DIR` exactly as given.
    Valid(String),
    /// `CLAUDE_PROJECT_DIR` is not set.
    Missing,
    /// `CLAUDE_PROJECT_DIR` is set and cannot be the project.
    Invalid(DirectoryFault),
}

/// The directory the server started in, recorded apart from the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingDirectory {
    /// A usable absolute path.
    Usable(String),
    /// A caller needs a usable one, so a call that records fails on this.
    Unusable(DirectoryFault),
}

/// What one server process is: its project, its directory and its ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    /// The project, as given and never canonicalized, so the hook records the
    /// same text.
    pub project: ProjectContext,
    /// The startup working directory. It may differ from the project.
    pub working_directory: WorkingDirectory,
    /// The Baley session: always the minted UUID, whichever agent calls.
    pub baley_session: String,
    /// The host's own session id, kept only when usable and compared with
    /// nothing.
    pub host_session: Option<String>,
    /// Startup notes for the process to print. Judging prints nothing.
    pub notes: Vec<String>,
}

/// Decides the session context from what the process told us.
pub fn judge(observation: Observation) -> SessionContext {
    let mut notes = Vec::new();
    let project = match &observation.project_dir {
        None => ProjectContext::Missing,
        Some(value) => match directory(value) {
            Ok(_) if !observation.project_is_directory => {
                ProjectContext::Invalid(DirectoryFault::NotADirectory)
            }
            Ok(text) => ProjectContext::Valid(text),
            Err(fault) => ProjectContext::Invalid(fault),
        },
    };
    let working_directory = match &observation.working_dir {
        None => WorkingDirectory::Unusable(DirectoryFault::Unreadable),
        Some(value) => match directory(value) {
            Ok(text) => WorkingDirectory::Usable(text),
            Err(fault) => WorkingDirectory::Unusable(fault),
        },
    };
    let host_session = observation
        .host_session
        .as_deref()
        .and_then(|value| host_session(value, &mut notes));
    SessionContext {
        project,
        working_directory,
        baley_session: observation.minted_session,
        host_session,
        notes,
    }
}

/// A directory value as text: non-empty, UTF-8, absolute and within the limit.
fn directory(value: &OsStr) -> Result<String, DirectoryFault> {
    if value.is_empty() {
        return Err(DirectoryFault::Empty);
    }
    let text = value.to_str().ok_or(DirectoryFault::NotUtf8)?;
    if !Path::new(text).is_absolute() {
        return Err(DirectoryFault::Relative);
    }
    if text.len() > MAX_DIRECTORY_BYTES {
        return Err(DirectoryFault::TooLong { bytes: text.len() });
    }
    Ok(text.to_owned())
}

/// The host's session id when it is usable. An unusable one is dropped with a
/// note and never fails the call.
fn host_session(value: &OsStr, notes: &mut Vec<String>) -> Option<String> {
    let usable = value
        .to_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_HOST_SESSION_BYTES);
    if usable.is_none() {
        notes.push(format!(
            "CLAUDE_CODE_SESSION_ID is empty, not UTF-8 or over {MAX_HOST_SESSION_BYTES} bytes, so it is not recorded"
        ));
    }
    usable.map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    const MINTED: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";

    fn observed() -> Observation {
        Observation {
            project_dir: Some("/work/project".into()),
            project_is_directory: true,
            working_dir: Some("/work/project".into()),
            host_session: None,
            minted_session: MINTED.into(),
        }
    }

    fn project_of(project_dir: OsString, is_directory: bool) -> ProjectContext {
        judge(Observation {
            project_dir: Some(project_dir),
            project_is_directory: is_directory,
            ..observed()
        })
        .project
    }

    #[test]
    fn a_trailing_slash_or_dot_component_is_recorded_exactly_as_given() {
        for given in ["/work/project/", "/work/./project", "/work/project/."] {
            assert_eq!(
                project_of(given.into(), true),
                ProjectContext::Valid(given.into())
            );
        }
    }

    #[test]
    fn an_unset_project_is_missing_not_invalid() {
        let context = judge(Observation {
            project_dir: None,
            project_is_directory: false,
            ..observed()
        });
        assert_eq!(context.project, ProjectContext::Missing);
    }

    #[test]
    fn an_empty_project_is_invalid_not_missing() {
        assert_eq!(
            project_of("".into(), false),
            ProjectContext::Invalid(DirectoryFault::Empty)
        );
    }

    #[test]
    fn a_relative_project_is_invalid() {
        assert_eq!(
            project_of("work/project".into(), true),
            ProjectContext::Invalid(DirectoryFault::Relative)
        );
    }

    #[test]
    fn a_non_utf8_project_is_invalid() {
        assert_eq!(
            project_of(OsString::from_vec(b"/work/\xff".to_vec()), true),
            ProjectContext::Invalid(DirectoryFault::NotUtf8)
        );
    }

    #[test]
    fn a_project_that_is_not_a_directory_is_invalid() {
        assert_eq!(
            project_of("/work/file.txt".into(), false),
            ProjectContext::Invalid(DirectoryFault::NotADirectory)
        );
    }

    #[test]
    fn the_project_limit_is_4096_bytes_inclusive() {
        let at_limit = format!("/{}", "a".repeat(MAX_DIRECTORY_BYTES - 1));
        assert_eq!(
            project_of(at_limit.clone().into(), true),
            ProjectContext::Valid(at_limit)
        );
        let over = format!("/{}", "a".repeat(MAX_DIRECTORY_BYTES));
        assert_eq!(
            project_of(over.into(), true),
            ProjectContext::Invalid(DirectoryFault::TooLong {
                bytes: MAX_DIRECTORY_BYTES + 1
            })
        );
    }

    #[test]
    fn a_working_directory_that_differs_from_the_project_is_kept_apart() {
        let context = judge(Observation {
            working_dir: Some("/elsewhere".into()),
            ..observed()
        });
        assert_eq!(
            context.project,
            ProjectContext::Valid("/work/project".into())
        );
        assert_eq!(
            context.working_directory,
            WorkingDirectory::Usable("/elsewhere".into())
        );
    }

    #[test]
    fn an_unreadable_or_unusable_working_directory_is_recorded_as_unusable() {
        let unreadable = judge(Observation {
            working_dir: None,
            ..observed()
        });
        assert_eq!(
            unreadable.working_directory,
            WorkingDirectory::Unusable(DirectoryFault::Unreadable)
        );
        let relative = judge(Observation {
            working_dir: Some("here".into()),
            ..observed()
        });
        assert_eq!(
            relative.working_directory,
            WorkingDirectory::Unusable(DirectoryFault::Relative)
        );
        assert_eq!(
            unreadable.project,
            ProjectContext::Valid("/work/project".into())
        );
    }

    #[test]
    fn the_baley_session_is_the_minted_value_even_when_the_host_id_is_a_uuid() {
        let host = "6f1c2d3e-4b5a-4c7d-8e9f-0a1b2c3d4e5f";
        let context = judge(Observation {
            host_session: Some(host.into()),
            ..observed()
        });
        assert_eq!(context.baley_session, MINTED);
        assert_eq!(context.host_session.as_deref(), Some(host));
    }

    #[test]
    fn two_starts_with_the_same_host_id_get_different_baley_sessions() {
        let first = judge(Observation {
            host_session: Some("same".into()),
            ..observed()
        });
        let second = judge(Observation {
            host_session: Some("same".into()),
            minted_session: "1c2d3e4f-5a6b-4c8d-9e0f-1a2b3c4d5e6f".into(),
            ..observed()
        });
        assert_ne!(first.baley_session, second.baley_session);
    }

    #[test]
    fn an_empty_or_oversize_host_id_is_dropped_with_a_note_and_the_context_stands() {
        for value in [String::new(), "x".repeat(MAX_HOST_SESSION_BYTES + 1)] {
            let context = judge(Observation {
                host_session: Some(value.into()),
                ..observed()
            });
            assert_eq!(context.host_session, None);
            assert_eq!(context.notes.len(), 1);
            assert_eq!(
                context.project,
                ProjectContext::Valid("/work/project".into())
            );
        }
    }

    #[test]
    fn a_host_id_at_the_limit_is_kept_without_a_note() {
        let value = "x".repeat(MAX_HOST_SESSION_BYTES);
        let context = judge(Observation {
            host_session: Some(value.clone().into()),
            ..observed()
        });
        assert_eq!(context.host_session, Some(value));
        assert!(context.notes.is_empty());
    }

    #[test]
    fn a_non_utf8_host_id_is_dropped_with_a_note() {
        let context = judge(Observation {
            host_session: Some(OsString::from_vec(vec![0xff, 0xfe])),
            ..observed()
        });
        assert_eq!(context.host_session, None);
        assert_eq!(context.notes.len(), 1);
    }

    #[test]
    fn an_unset_host_id_is_absent_without_a_note() {
        let context = judge(observed());
        assert_eq!(context.host_session, None);
        assert!(context.notes.is_empty());
    }
}
