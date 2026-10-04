//! The two gates around the queue, and the one place answers become tool
//! results.
//!
//! Faults answer in a fixed order: an unknown tool is a protocol error, then
//! an unsupported client, then an unknown or unavailable operation. Those come
//! before the queue, so an unsupported client never takes a slot. After the
//! queue come the project and the caller. Every answer but the protocol error
//! is a successful structured tool result, never `isError`.

use baley_core::policy::Host;
use baley_store::ServerCaller;
use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use super::client::Selection;
use super::context::{CallContext, ProjectContext, form_caller, judge_id};
use super::operations::{Lookup, Tool, lookup, operation_unavailable, unknown_operation};
use crate::envelope::Envelope;

/// The most bytes of an unknown tool's name a protocol error echoes.
const MAX_ECHOED_TOOL_BYTES: usize = 128;

/// Which of the three tools a call named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Called {
    /// `baley_version`.
    Version,
    /// `baley_query` or `baley_apply` with a served spelling.
    Operation(Tool, String),
}

/// A call the first gate lets through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// What to run.
    pub called: Called,
    /// The host the call came from.
    pub host: Host,
    /// The client's reported version.
    pub client_version: String,
    /// Whether the operation needs a project, and so a caller.
    pub needs_project: bool,
}

/// What the gate before the queue decides.
#[derive(Debug, PartialEq)]
pub enum Before {
    /// Take a queue slot and run.
    Proceed(Admitted),
    /// Answer now, as a successful tool result.
    Answer(CallToolResult),
    /// Fail the request as a protocol error.
    ProtocolError(ErrorData),
}

/// What the gate after the queue decides.
#[derive(Debug, PartialEq)]
pub enum After {
    /// Run. A call that needs a project carries the caller it records under.
    Run(Option<ServerCaller>),
    /// Answer instead of running.
    Answer(CallToolResult),
}

/// Encodes an answer once: a successful call carrying it, never `isError`.
pub fn encode(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

/// The gate before the queue, in D-03's order.
pub fn before_queue(tool: &str, selection: &Selection, spelling: Option<&str>) -> Before {
    let which = match tool {
        "baley_version" => None,
        "baley_query" => Some(Tool::Query),
        "baley_apply" => Some(Tool::Apply),
        other => return Before::ProtocolError(unknown_tool(other)),
    };
    let (host, client_version) = match selection {
        Selection::Supported {
            host,
            client_version,
        } => (*host, client_version.clone()),
        Selection::UnknownHost { name, version } => {
            return Before::Answer(encode(unknown_host(name, version)));
        }
    };
    let Some(which) = which else {
        return Before::Proceed(Admitted {
            called: Called::Version,
            host,
            client_version,
            needs_project: false,
        });
    };
    match lookup(which, spelling) {
        Lookup::Unknown => Before::Answer(encode(unknown_operation(which, spelling))),
        Lookup::Unavailable { build } => Before::Answer(encode(operation_unavailable(
            which,
            spelling.unwrap_or_default(),
            build,
        ))),
        Lookup::Available { needs_project } => Before::Proceed(Admitted {
            called: Called::Operation(which, spelling.unwrap_or_default().to_owned()),
            host,
            client_version,
            needs_project,
        }),
    }
}

/// The gate after the queue: the project, then the caller. An operation that
/// needs no project skips the project and working-directory checks but not
/// the id check, since a string id over the limit fails every call.
pub fn after_queue(needs_project: bool, call: &CallContext) -> After {
    if !needs_project {
        return match judge_id(&call.request_id) {
            Ok(()) => After::Run(None),
            Err(fault) => After::Answer(encode(caller_invalid(fault.place(), &fault.to_string()))),
        };
    }
    let project = match &call.project {
        ProjectContext::Valid(text) => text,
        ProjectContext::Missing => {
            return After::Answer(encode(failed(
                "project-context-missing",
                "CLAUDE_PROJECT_DIR is not set, so Baley does not know the project",
                "CLAUDE_PROJECT_DIR",
            )));
        }
        ProjectContext::Invalid(fault) => {
            return After::Answer(encode(failed(
                "project-context-invalid",
                &format!("CLAUDE_PROJECT_DIR cannot be the project: {fault}"),
                "CLAUDE_PROJECT_DIR",
            )));
        }
    };
    match form_caller(call, project) {
        Ok(caller) => After::Run(Some(caller)),
        Err(fault) => After::Answer(encode(caller_invalid(fault.place(), &fault.to_string()))),
    }
}

fn failed(code: &str, reason: &str, place: &str) -> Value {
    serde_json::to_value(Envelope::<Value>::failed(code, reason, place)).expect("failed answer")
}

fn caller_invalid(place: &str, reason: &str) -> Value {
    failed("caller-invalid", reason, place)
}

fn unknown_tool(name: &str) -> ErrorData {
    let mut end = name.len().min(MAX_ECHOED_TOOL_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let cut = if end < name.len() { "..." } else { "" };
    ErrorData::invalid_params(format!("unknown tool `{}{cut}`", &name[..end]), None)
}

fn unknown_host(
    name: &Option<super::client::Reported>,
    version: &Option<super::client::Reported>,
) -> Value {
    let reported = match (name, version) {
        (Some(name), Some(version)) => json!({"name": name.value(), "version": version.value()}),
        _ => Value::Null,
    };
    let supported: Vec<&str> = Host::ALL.iter().map(|host| host.name()).collect();
    serde_json::to_value(Envelope::<Value>::failed_with_details(
        "unknown-host",
        "Baley supports only the hosts listed in `supported`, and the client did not identify as one",
        "client-info",
        json!({"reported": reported, "supported": supported}),
    ))
    .expect("failed answer")
}

#[cfg(test)]
mod tests {
    use super::super::client::Reported;
    use super::super::context::{
        Observation, SessionContext, WorkingDirectory, call_context, judge,
    };
    use super::*;

    fn supported() -> Selection {
        Selection::Supported {
            host: Host::ClaudeCode,
            client_version: "2.1.287".into(),
        }
    }

    fn unsupported() -> Selection {
        Selection::UnknownHost {
            name: Some(Reported::Text("codex".into())),
            version: Some(Reported::Text("1.0".into())),
        }
    }

    fn missing_identity() -> Selection {
        Selection::UnknownHost {
            name: None,
            version: None,
        }
    }

    fn answered(before: Before) -> Value {
        match before {
            Before::Answer(result) => {
                assert_ne!(result.is_error, Some(true), "an answer is never isError");
                result.structured_content.expect("a structured answer")
            }
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn session(project: Option<&str>) -> SessionContext {
        judge(Observation {
            project_dir: project.map(Into::into),
            project_is_directory: project.is_some(),
            working_dir: Some("/work/project".into()),
            host_session: None,
            minted_session: "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f".into(),
        })
    }

    fn call(session: &SessionContext, id: Value) -> CallContext {
        call_context(session, Host::ClaudeCode, "2.1.287", id)
    }

    fn after(needs_project: bool, call: &CallContext) -> Value {
        match after_queue(needs_project, call) {
            After::Answer(result) => {
                assert_ne!(result.is_error, Some(true), "an answer is never isError");
                result.structured_content.expect("a structured answer")
            }
            After::Run(_) => json!({"ran": true}),
        }
    }

    #[test]
    fn an_unknown_tool_from_an_unsupported_client_is_a_protocol_error() {
        let before = before_queue("baley_other", &unsupported(), Some("help"));
        assert!(matches!(before, Before::ProtocolError(_)), "{before:?}");
    }

    #[test]
    fn an_unknown_tool_error_names_the_tool_and_is_bounded() {
        let Before::ProtocolError(error) = before_queue("baley_other", &supported(), None) else {
            panic!("a protocol error");
        };
        assert!(error.message.contains("baley_other"));
        let Before::ProtocolError(long) = before_queue(&"é".repeat(300), &supported(), None)
        else {
            panic!("a protocol error");
        };
        assert!(long.message.len() < 200, "{}", long.message.len());
    }

    #[test]
    fn an_unsupported_client_asking_for_an_unknown_operation_gets_unknown_host() {
        let answer = answered(before_queue("baley_query", &unsupported(), Some("nope")));
        assert_eq!(answer["code"], "unknown-host");
    }

    #[test]
    fn version_from_an_unsupported_client_is_unknown_host_not_ok() {
        for selection in [unsupported(), missing_identity()] {
            let answer = answered(before_queue("baley_version", &selection, None));
            assert_eq!(answer["status"], "failed");
            assert_eq!(answer["code"], "unknown-host");
        }
    }

    #[test]
    fn the_unknown_host_answer_names_client_info_the_supported_host_and_records_nothing() {
        let answer = answered(before_queue("baley_query", &unsupported(), Some("help")));
        assert_eq!(answer["place"], "client-info");
        assert_eq!(answer["recorded"], json!(false));
        assert_eq!(answer["details"]["supported"], json!(["claude-code"]));
        assert_eq!(
            answer["details"]["reported"],
            json!({"name": "codex", "version": "1.0"})
        );
        let missing = answered(before_queue("baley_query", &missing_identity(), None));
        assert_eq!(missing["details"]["reported"], Value::Null);
    }

    #[test]
    fn a_retired_operation_from_a_supported_client_answers_unavailable_before_any_project_check() {
        let answer = answered(before_queue(
            "baley_apply",
            &supported(),
            Some("execution-run"),
        ));
        assert_eq!(answer["status"], "refused");
        assert_eq!(answer["code"], "operation-unavailable");
        assert_eq!(answer["details"], json!({"build": 5}));
    }

    #[test]
    fn an_unrecognized_or_missing_operation_answers_unknown_operation() {
        for spelling in [Some("nope"), None] {
            let answer = answered(before_queue("baley_query", &supported(), spelling));
            assert_eq!(answer["code"], "unknown-operation", "{spelling:?}");
        }
    }

    #[test]
    fn served_calls_proceed_with_their_host_version_and_project_need() {
        let Before::Proceed(version) = before_queue("baley_version", &supported(), None) else {
            panic!("version proceeds");
        };
        assert_eq!(version.called, Called::Version);
        assert_eq!(version.client_version, "2.1.287");
        assert!(!version.needs_project);
        let Before::Proceed(help) = before_queue("baley_query", &supported(), Some("help")) else {
            panic!("help proceeds");
        };
        assert_eq!(help.called, Called::Operation(Tool::Query, "help".into()));
        assert_eq!(help.host, Host::ClaudeCode);
    }

    #[test]
    fn a_call_needing_a_project_with_none_set_answers_project_context_missing() {
        let answer = after(true, &call(&session(None), json!(1)));
        assert_eq!(answer["status"], "failed");
        assert_eq!(answer["code"], "project-context-missing");
        assert_eq!(answer["place"], "CLAUDE_PROJECT_DIR");
        assert_eq!(answer["recorded"], json!(false));
    }

    #[test]
    fn an_invalid_project_answers_invalid_not_missing() {
        let answer = after(true, &call(&session(Some("relative/dir")), json!(1)));
        assert_eq!(answer["code"], "project-context-invalid");
        assert_eq!(answer["place"], "CLAUDE_PROJECT_DIR");
    }

    #[test]
    fn a_valid_project_and_id_run_with_a_caller() {
        let context = call(&session(Some("/work/project")), json!(7));
        let After::Run(Some(caller)) = after_queue(true, &context) else {
            panic!("a call needing a project runs with a caller");
        };
        assert_eq!(caller.project_directory(), "/work/project");
    }

    #[test]
    fn a_caller_fault_on_a_recording_operation_does_not_run() {
        let id = after(
            true,
            &call(&session(Some("/work/project")), json!("x".repeat(300))),
        );
        assert_eq!(id["code"], "caller-invalid");
        assert_eq!(id["place"], "id");
        let mut context = call(&session(Some("/work/project")), json!(1));
        context.working_directory =
            WorkingDirectory::Unusable(super::super::context::DirectoryFault::Unreadable);
        let wd = after(true, &context);
        assert_eq!(wd["code"], "caller-invalid");
        assert_eq!(wd["place"], "working-directory");
    }

    #[test]
    fn version_with_a_300_byte_string_id_is_caller_invalid_at_place_id() {
        let answer = after(
            false,
            &call(&session(Some("/work/project")), json!("x".repeat(300))),
        );
        assert_eq!(answer["status"], "failed");
        assert_eq!(answer["code"], "caller-invalid");
        assert_eq!(answer["place"], "id");
    }

    #[test]
    fn a_call_that_needs_no_project_runs_with_a_missing_project_or_working_directory() {
        let mut context = call(&session(None), json!(1));
        context.working_directory =
            WorkingDirectory::Unusable(super::super::context::DirectoryFault::Unreadable);
        assert_eq!(after(false, &context), json!({"ran": true}));
    }

    #[test]
    fn a_project_fault_wins_over_a_caller_fault() {
        let answer = after(true, &call(&session(None), json!("x".repeat(300))));
        assert_eq!(answer["code"], "project-context-missing");
    }

    #[test]
    fn a_failed_answer_is_never_an_error_result() {
        let result = encode(failed("server-overloaded", "busy", "queue"));
        assert_ne!(result.is_error, Some(true));
        assert_eq!(result.structured_content.unwrap()["retryable"], json!(true));
    }
}
