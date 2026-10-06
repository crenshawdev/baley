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

use baley_core::policy::Host;
use baley_store::{
    CallerError, MAX_CLIENT_VERSION_BYTES, MAX_DIRECTORY_BYTES, MAX_HOST_SESSION_BYTES,
    ServerCaller,
};
use serde_json::Value;

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
    let project = project_context(
        observation.project_dir.as_deref(),
        observation.project_is_directory,
    );
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

/// Judges `CLAUDE_PROJECT_DIR` as the environment gave it, or `None` when
/// unset, and whether that path is an existing directory. The server judges
/// its startup project with it, and the guard hook judges its project with it
/// too, so both apply one set of rules.
pub fn project_context(value: Option<&OsStr>, is_directory: bool) -> ProjectContext {
    match value {
        None => ProjectContext::Missing,
        Some(value) => match directory(value) {
            Ok(_) if !is_directory => ProjectContext::Invalid(DirectoryFault::NotADirectory),
            Ok(text) => ProjectContext::Valid(text),
            Err(fault) => ProjectContext::Invalid(fault),
        },
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

/// What one call knows: the session's context plus who is calling and under
/// which JSON-RPC id. It has no input from the call's arguments, so no `cwd`,
/// project or root a caller sends can reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    /// The host selected for this call.
    pub host: Host,
    /// The client's version, dropped when empty or over its limit.
    pub client_version: Option<String>,
    /// The request's JSON-RPC id, a string or an integer.
    pub request_id: Value,
    /// The session's project.
    pub project: ProjectContext,
    /// The session's startup working directory.
    pub working_directory: WorkingDirectory,
    /// The session's minted id, the same for the main session and every
    /// subagent. The JSON-RPC id tells their calls apart.
    pub baley_session: String,
    /// The session's host id, when usable.
    pub host_session: Option<String>,
    /// This call's notes for the process to print.
    pub notes: Vec<String>,
}

/// Builds one call's context. Nothing here appends or opens the store.
pub fn call_context(
    session: &SessionContext,
    host: Host,
    client_version: &str,
    request_id: Value,
) -> CallContext {
    let mut notes = Vec::new();
    let kept = !client_version.is_empty() && client_version.len() <= MAX_CLIENT_VERSION_BYTES;
    if !kept {
        notes.push(format!(
            "the client version is empty or over {MAX_CLIENT_VERSION_BYTES} bytes, so it is not recorded"
        ));
    }
    CallContext {
        host,
        client_version: kept.then(|| client_version.to_owned()),
        request_id,
        project: session.project.clone(),
        working_directory: session.working_directory.clone(),
        baley_session: session.baley_session.clone(),
        host_session: session.host_session.clone(),
        notes,
    }
}

/// Why a call cannot form a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerFault {
    /// The JSON-RPC id cannot form a call identity.
    Id(CallerError),
    /// The startup working directory is unusable.
    WorkingDirectory(DirectoryFault),
    /// Another caller field was refused. The session context judges these
    /// fields by the port's own rules, so this is a safety net, not a path.
    Invalid(CallerError),
}

impl CallerFault {
    /// The input at fault, as a `failed` answer names it.
    pub fn place(&self) -> &'static str {
        match self {
            Self::Id(_) => "id",
            Self::WorkingDirectory(_) => "working-directory",
            Self::Invalid(_) => "caller",
        }
    }
}

impl fmt::Display for CallerFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(error) | Self::Invalid(error) => error.fmt(f),
            Self::WorkingDirectory(fault) => {
                write!(
                    f,
                    "the startup working directory cannot be recorded: {fault}"
                )
            }
        }
    }
}

/// Judges the JSON-RPC id of any call, even one that records nothing, by the
/// port's own call-identity construction so the limit is the one the store
/// applies. The other caller fields here are fixed valid values.
pub fn judge_id(id: &Value) -> Result<(), CallerFault> {
    const SESSION: &str = "00000000-0000-4000-8000-000000000000";
    ServerCaller::new("/", "/", Host::ClaudeCode.name(), SESSION, id)
        .map(drop)
        .map_err(CallerFault::Id)
}

/// Forms the caller for a call that records, over the session's valid project.
pub fn form_caller(call: &CallContext, project: &str) -> Result<ServerCaller, CallerFault> {
    judge_id(&call.request_id)?;
    let working_directory = match &call.working_directory {
        WorkingDirectory::Usable(text) => text,
        WorkingDirectory::Unusable(fault) => {
            return Err(CallerFault::WorkingDirectory(fault.clone()));
        }
    };
    let mut caller = ServerCaller::new(
        project,
        working_directory,
        call.host.name(),
        &call.baley_session,
        &call.request_id,
    )
    .map_err(CallerFault::Invalid)?;
    if let Some(version) = &call.client_version {
        caller = caller
            .with_client_version(version)
            .map_err(CallerFault::Invalid)?;
    }
    if let Some(session) = &call.host_session {
        caller = caller
            .with_host_session(session)
            .map_err(CallerFault::Invalid)?;
    }
    Ok(caller)
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStringExt;

    use serde_json::json;

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
    fn session() -> SessionContext {
        judge(Observation {
            host_session: Some("host-abc".into()),
            ..observed()
        })
    }

    fn call(id: Value) -> CallContext {
        call_context(&session(), Host::ClaudeCode, "2.1.287", id)
    }

    #[test]
    fn a_subagents_call_carries_the_main_sessions_baley_session() {
        let main = call(json!(1));
        let subagent = call(json!("sub-2"));
        assert_eq!(main.baley_session, MINTED);
        assert_eq!(subagent.baley_session, main.baley_session);
        assert_ne!(main.request_id, subagent.request_id);
    }

    #[test]
    fn a_client_version_over_128_bytes_is_dropped_with_a_note_and_the_call_goes_on() {
        let over = "9".repeat(MAX_CLIENT_VERSION_BYTES + 1);
        for version in [over.as_str(), ""] {
            let context = call_context(&session(), Host::ClaudeCode, version, json!(1));
            assert_eq!(context.client_version, None);
            assert_eq!(context.notes.len(), 1);
            assert!(form_caller(&context, "/work/project").is_ok());
        }
        let at_limit = "9".repeat(MAX_CLIENT_VERSION_BYTES);
        let kept = call_context(&session(), Host::ClaudeCode, &at_limit, json!(1));
        assert_eq!(kept.client_version, Some(at_limit));
        assert!(kept.notes.is_empty());
    }

    #[test]
    fn an_integer_id_forms_a_caller_and_a_300_byte_string_id_does_not() {
        assert!(form_caller(&call(json!(7)), "/work/project").is_ok());
        let long = call(json!("x".repeat(300)));
        assert!(matches!(
            form_caller(&long, "/work/project"),
            Err(CallerFault::Id(_))
        ));
    }

    #[test]
    fn the_id_judgement_uses_the_ports_limit_which_counts_the_quotes() {
        assert_eq!(judge_id(&json!(7)), Ok(()));
        assert_eq!(judge_id(&json!("sub-2")), Ok(()));
        assert_eq!(judge_id(&json!("x".repeat(254))), Ok(()));
        assert!(matches!(
            judge_id(&json!("x".repeat(255))),
            Err(CallerFault::Id(_))
        ));
        assert!(matches!(
            judge_id(&json!("x".repeat(300))),
            Err(CallerFault::Id(_))
        ));
    }

    #[test]
    fn an_id_that_is_not_a_string_or_integer_is_an_id_fault() {
        for id in [json!(null), json!(1.5), json!(true)] {
            assert_eq!(judge_id(&id).unwrap_err().place(), "id");
        }
    }

    #[test]
    fn a_formed_caller_carries_the_session_contexts_values() {
        let caller = form_caller(&call(json!(7)), "/work/project").unwrap();
        let context = session();
        assert_eq!(caller.project_directory(), "/work/project");
        assert_eq!(caller.working_directory(), "/work/project");
        assert_eq!(caller.host(), "claude-code");
        assert_eq!(caller.baley_session(), context.baley_session);
        assert_eq!(caller.client_version(), Some("2.1.287"));
        assert_eq!(caller.host_session(), Some("host-abc"));
        assert_eq!(caller.call().text(), "7");
    }

    #[test]
    fn the_caller_keeps_a_working_directory_that_differs_from_the_project() {
        let context = judge(Observation {
            working_dir: Some("/elsewhere".into()),
            ..observed()
        });
        let call = call_context(&context, Host::ClaudeCode, "1", json!(1));
        let caller = form_caller(&call, "/work/project").unwrap();
        assert_eq!(caller.project_directory(), "/work/project");
        assert_eq!(caller.working_directory(), "/elsewhere");
    }

    #[test]
    fn an_unusable_working_directory_fails_the_caller_but_not_the_id_judgement() {
        let context = judge(Observation {
            working_dir: None,
            ..observed()
        });
        let call = call_context(&context, Host::ClaudeCode, "1", json!(1));
        let fault = form_caller(&call, "/work/project").unwrap_err();
        assert_eq!(fault.place(), "working-directory");
        assert_eq!(judge_id(&call.request_id), Ok(()));
    }

    #[test]
    fn an_id_fault_wins_over_a_working_directory_fault_and_names_place_id() {
        let context = judge(Observation {
            working_dir: None,
            ..observed()
        });
        let call = call_context(&context, Host::ClaudeCode, "1", json!("x".repeat(300)));
        assert_eq!(
            form_caller(&call, "/work/project").unwrap_err().place(),
            "id"
        );
    }

    #[test]
    fn a_refused_session_or_project_is_a_caller_fault_not_a_panic() {
        let bad_session = judge(Observation {
            minted_session: "not-a-uuid".into(),
            ..observed()
        });
        let call = call_context(&bad_session, Host::ClaudeCode, "1", json!(1));
        assert_eq!(
            form_caller(&call, "/work/project").unwrap_err().place(),
            "caller"
        );
    }
}
