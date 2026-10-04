//! The rmcp handler for one session.
//!
//! It holds the session context, the identity stored from `initialize` and the
//! worker. It decides nothing itself: it reads what a call carries, runs the
//! gate before the queue through the worker's admission, and hands an accepted
//! call to the worker, which runs the gate after the queue and then the
//! operation. rmcp has already validated the request's metadata by the time
//! any of this runs, so a protocol-required metadata error stays a protocol
//! error and runs nothing.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, PoisonError};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, InitializeRequestParams, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, RequestMetaObject, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

use super::client::{ClientIdentity, Selection, decode_2025, decode_2026, select};
use super::context::{SessionContext, call_context};
use super::gate::{Admitted, After, Called, after_queue, encode};
use super::operations::{help_answer, schema_answer, unknown_operation};
use super::tools::{info, supported_protocol_versions, tool_list, version_answer};
use super::transport::RawFrameBytes;
use super::worker::{Submission, Worker};

/// What `initialize` told the session, if it was used.
#[derive(Default)]
struct Initialized {
    began: bool,
    identity: Option<ClientIdentity>,
}

/// The handler for one session's connection.
pub struct SessionHandler {
    session: Arc<SessionContext>,
    initialized: Mutex<Initialized>,
    worker: Arc<Worker>,
}

impl SessionHandler {
    /// A handler for `session` that runs accepted calls on `worker`.
    pub fn new(session: Arc<SessionContext>, worker: Arc<Worker>) -> Self {
        Self {
            session,
            initialized: Mutex::new(Initialized::default()),
            worker,
        }
    }

    /// Stores the identity a 2025 `initialize` carried and marks that the
    /// session began with it. It never refuses a client: support is decided
    /// for each `tools/call`, so an unsupported client still sees the tools.
    fn remember(&self, request: &InitializeRequestParams) {
        let mut initialized = self
            .initialized
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        initialized.began = true;
        initialized.identity = Some(decode_2025(request));
    }

    /// Selects the host for one call from the stored identity or the call's
    /// own `_meta`, as the session began.
    fn selection(&self, meta: Option<&RequestMetaObject>) -> Selection {
        let current = decode_2026(meta);
        let initialized = self
            .initialized
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        select(
            initialized.began,
            initialized.identity.as_ref(),
            current.as_ref(),
        )
    }
}

/// Runs one admitted call on the worker: the gate after the queue, then the
/// operation. T5 attaches the caller this gate forms to what a call records.
fn run_decision(
    session: &SessionContext,
    request_id: Value,
    admitted: Admitted,
    arguments: Option<&Value>,
) -> rmcp::model::CallToolResult {
    let call = call_context(session, admitted.host, &admitted.client_version, request_id);
    for note in &call.notes {
        eprintln!("baley: {note}");
    }
    match after_queue(admitted.needs_project, &call) {
        After::Answer(result) => result,
        After::Run(_caller) => encode(operate(&admitted.called, arguments)),
    }
}

/// The answer of an admitted operation.
fn operate(called: &Called, arguments: Option<&Value>) -> Value {
    let arguments_or_null = arguments.unwrap_or(&Value::Null);
    match called {
        Called::Version => version_answer(arguments),
        Called::Operation(_, spelling) if spelling == "help" => help_answer(arguments_or_null),
        Called::Operation(_, spelling) if spelling == "schema" => schema_answer(arguments_or_null),
        // The gate admits only the spellings above, so a baseline entry marked
        // served with nothing behind it answers as an unknown operation.
        Called::Operation(tool, spelling) => unknown_operation(*tool, Some(spelling)),
    }
}

impl ServerHandler for SessionHandler {
    fn get_info(&self) -> ServerConfig {
        info()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(supported_protocol_versions())
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.remember(&request);
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(tool_list())
    }

    // No tool lookup and no typed parameters here: every declared tool's
    // arguments must reach this handler before validation.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Everything up to the submission is synchronous, so admission sees
        // calls in the order rmcp spawned their tasks, which is arrival order
        // on the current-thread runtime the server runs on. An `.await` before
        // the submission would let a later call overtake an earlier one.
        let Some(RawFrameBytes(raw_bytes)) = context.extensions.get::<RawFrameBytes>().copied()
        else {
            return Err(ErrorData::internal_error(
                "the call's size was not recorded",
                None,
            ));
        };
        let spelling = super::operations::spelling(request.arguments.as_ref()).map(str::to_owned);
        let selection = self.selection(Some(&context.meta));
        let request_id = serde_json::to_value(&context.id).unwrap_or(Value::Null);
        let session = Arc::clone(&self.session);
        let arguments = request.arguments.map(Value::Object);
        let submission = self.worker.submit(
            &request.name,
            &selection,
            spelling.as_deref(),
            raw_bytes,
            move |admitted| run_decision(&session, request_id, admitted, arguments.as_ref()),
        );
        match submission {
            Submission::Answer(result) => Ok(result.into()),
            Submission::ProtocolError(error) => Err(error),
            Submission::Accepted(receiver) => match receiver.await {
                Ok(answer) => answer.map(Into::into),
                // Dropped unanswered: the call was abandoned unstarted at shutdown.
                Err(_) => Err(ErrorData::internal_error(
                    "Baley is shutting down, so the call was not run",
                    None,
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::{Observation, judge};
    use super::super::operations::Tool;
    use super::*;
    use rmcp::model::{ClientCapabilities, Implementation};
    use serde_json::json;
    use std::ffi::OsString;

    fn session() -> Arc<SessionContext> {
        Arc::new(judge(Observation {
            project_dir: None,
            project_is_directory: false,
            working_dir: Some(OsString::from("/work")),
            host_session: None,
            minted_session: "00000000-0000-4000-8000-000000000000".into(),
        }))
    }

    fn handler() -> SessionHandler {
        SessionHandler::new(session(), Arc::new(Worker::start().expect("worker")))
    }

    fn initialize_as(name: &str) -> InitializeRequestParams {
        InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(name, "2.1.287"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }

    fn meta_of(name: &str) -> RequestMetaObject {
        serde_json::from_value(json!({"io.modelcontextprotocol/clientInfo":
            {"name": name, "version": "2.1.287"}}))
        .unwrap()
    }

    #[test]
    fn an_initialize_from_another_host_is_negotiated_not_refused() {
        let handler = handler();
        let negotiated = handler.negotiate_initialize(&initialize_as("other-host"));
        assert_eq!(
            negotiated
                .expect("initialize must succeed")
                .protocol_version,
            ProtocolVersion::V_2025_11_25
        );
    }

    #[test]
    fn the_server_supports_only_the_two_tested_protocol_revisions() {
        let versions = ServerHandler::supported_protocol_versions(&handler());
        assert_eq!(
            versions.as_ref(),
            [ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28]
        );
    }

    #[test]
    fn an_identity_from_initialize_is_used_by_a_call_that_carries_no_meta() {
        let handler = handler();
        handler.remember(&initialize_as("claude-code"));
        assert!(matches!(
            handler.selection(None),
            Selection::Supported { .. }
        ));
    }

    #[test]
    fn an_initialize_identity_that_is_not_claude_code_selects_no_host() {
        let handler = handler();
        handler.remember(&initialize_as("other-host"));
        // Its own meta must not rescue it: the session began with initialize.
        assert!(matches!(
            handler.selection(Some(&meta_of("claude-code"))),
            Selection::UnknownHost { .. }
        ));
    }

    #[test]
    fn a_2026_identity_from_an_earlier_call_is_reused_by_a_later_call_without_meta() {
        let handler = handler();
        assert!(matches!(
            handler.selection(Some(&meta_of("claude-code"))),
            Selection::Supported { .. }
        ));
        assert!(matches!(
            handler.selection(None),
            Selection::UnknownHost { .. }
        ));
    }

    #[test]
    fn version_help_and_schema_each_run_their_own_operation() {
        let version = operate(&Called::Version, Some(&json!({})));
        assert_eq!(version["status"], "ok");
        assert!(version["version"].is_string());
        let help = operate(
            &Called::Operation(Tool::Query, "help".into()),
            Some(&json!({"operation": "help"})),
        );
        assert_eq!(help["status"], "ok");
        assert_ne!(help, version);
        let schema = operate(
            &Called::Operation(Tool::Query, "schema".into()),
            Some(&json!({"operation": "schema", "tool": "query", "name": "help"})),
        );
        assert_ne!(schema, help);
    }
}
