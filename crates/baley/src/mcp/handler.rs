//! The rmcp handler for one session.
//!
//! It holds the session context, the identity stored from `initialize`, the
//! worker and the ledger the server opened. It decides nothing itself: it reads
//! what a call carries, runs the gate before the queue through the worker's
//! admission, and hands an accepted call to the worker, which runs the gate
//! after the queue and then the operation. A call that needs a project reaches
//! the operation with a [`Preparation`]: the caller the gate formed, the
//! call's host, the session's ledger and the server's time. `capture` sets
//! `needs_project` and prepares its write from it in its [`operate`] arm.
//! rmcp has already validated the request's metadata by the time any
//! of this runs, so a protocol-required metadata error stays a protocol error
//! and runs nothing.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use baley_core::policy::Host;
use baley_store::ServerCaller;
use baley_store_sqlite::SqliteStore;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, InitializeRequestParams, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, RequestMetaObject, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

use super::capture;
use super::client::{ClientIdentity, Selection, decode_2025, decode_2026, select};
use super::context::{SessionContext, call_context};
use super::gate::{Admitted, After, Called, after_queue, encode};
use super::operations::{help_answer, instruction_answer, schema_answer, unknown_operation};
use super::tools::{info, supported_protocol_versions, tool_list, version_answer};
use super::transport::RawFrameBytes;
use super::worker::{Submission, Worker};
use crate::ledger::clock::SystemClock;

/// What `initialize` told the session, if it was used.
#[derive(Default)]
struct Initialized {
    began: bool,
    identity: Option<ClientIdentity>,
}

/// The ledger the server opened at startup and the config folder found with
/// it. The exit checkpoint runs on this same store.
pub struct SessionLedger {
    /// The open per-user store.
    pub store: SqliteStore,
    /// Baley's config folder, where the global settings file is.
    pub config: PathBuf,
}

/// What a project call hands its operation to prepare a read or write. It is
/// built from the gate's caller, the call's host and the session's own state,
/// never from tool arguments. `ledger` is `None` when the server could not open
/// one, so preparation answers `failed` `ledger-unavailable`.
pub struct Preparation {
    /// The store and config folder the server opened at startup.
    pub ledger: Option<Arc<SessionLedger>>,
    /// The host the call came from.
    pub host: Host,
    /// The caller the gate formed for this call.
    pub caller: ServerCaller,
    /// The server's time for this call, read once.
    pub at: String,
}

/// The handler for one session's connection.
pub struct SessionHandler {
    session: Arc<SessionContext>,
    initialized: Mutex<Initialized>,
    worker: Arc<Worker>,
    ledger: Option<Arc<SessionLedger>>,
}

impl SessionHandler {
    /// A handler for `session` that runs accepted calls on `worker`. `ledger`
    /// is the store the server opened, or `None` when it could not.
    pub fn new(
        session: Arc<SessionContext>,
        worker: Arc<Worker>,
        ledger: Option<Arc<SessionLedger>>,
    ) -> Self {
        Self {
            session,
            initialized: Mutex::new(Initialized::default()),
            worker,
            ledger,
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
/// operation. A call the gate runs with a caller gets a [`Preparation`] holding
/// that caller, the call's host, the session's ledger and the server's time,
/// read here once. A call that needs no project gets none and reads no clock.
fn run_decision(
    session: &SessionContext,
    ledger: Option<Arc<SessionLedger>>,
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
        After::Run(caller) => {
            let preparation = caller.map(|caller| Preparation {
                ledger,
                host: admitted.host,
                caller,
                at: SystemClock::now(),
            });
            encode(operate(&admitted.called, arguments, preparation.as_ref()))
        }
    }
}

/// The answer of an admitted operation. It matches the called operation and the
/// project call's [`Preparation`] together, so an arm that needs a project
/// names the input in its pattern and the arms that need none ignore it.
/// `capture` prepares its write from the [`Preparation`].
fn operate(called: &Called, arguments: Option<&Value>, project: Option<&Preparation>) -> Value {
    let arguments_or_null = arguments.unwrap_or(&Value::Null);
    match (called, project) {
        (Called::Version, _) => version_answer(arguments),
        (Called::Operation(_, spelling), _) if spelling == "help" => help_answer(arguments_or_null),
        (Called::Operation(_, spelling), _) if spelling == "schema" => {
            schema_answer(arguments_or_null)
        }
        (Called::Operation(_, spelling), _) if spelling == "instruction" => {
            instruction_answer(arguments_or_null)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "capture" => {
            capture::serve(arguments_or_null, preparation)
        }
        // The gate admits only the spellings above, so a baseline entry marked
        // served with nothing behind it answers as an unknown operation.
        (Called::Operation(tool, spelling), _) => unknown_operation(*tool, Some(spelling)),
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
        let ledger = self.ledger.clone();
        let arguments = request.arguments.map(Value::Object);
        let submission = self.worker.submit(
            &request.name,
            &selection,
            spelling.as_deref(),
            raw_bytes,
            move |admitted| {
                run_decision(&session, ledger, request_id, admitted, arguments.as_ref())
            },
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
        SessionHandler::new(session(), Arc::new(Worker::start().expect("worker")), None)
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
    fn version_help_schema_and_instruction_each_run_their_own_operation() {
        let version = operate(&Called::Version, Some(&json!({})), None);
        assert_eq!(version["status"], "ok");
        assert!(version["version"].is_string());
        let help = operate(
            &Called::Operation(Tool::Query, "help".into()),
            Some(&json!({"operation": "help"})),
            None,
        );
        assert_eq!(help["status"], "ok");
        assert_ne!(help, version);
        // Valid schema arguments, so only the schema operation can answer ok
        // with help's schema.
        let schema = operate(
            &Called::Operation(Tool::Query, "schema".into()),
            Some(&json!({"operation": "schema", "tool": "query", "for": "help"})),
            None,
        );
        assert_eq!(schema["status"], "ok", "{schema}");
        assert_eq!(schema["schema"]["properties"]["operation"]["const"], "help");
        // Valid instruction arguments, which help and schema both refuse, so only
        // the instruction arm can answer ok with the identity.
        let instruction = operate(
            &Called::Operation(Tool::Query, "instruction".into()),
            Some(&json!({"operation": "instruction", "identity": "bal-help"})),
            None,
        );
        assert_eq!(instruction["status"], "ok", "{instruction}");
        assert_eq!(instruction["identity"], "bal-help");
    }

    #[test]
    fn a_capture_argument_refusal_answered_after_preparation_or_recorded_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger, so a preparation that ran first would answer `failed`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-05T09:00:00Z".into(),
        };
        let answer = operate(
            &Called::Operation(Tool::Apply, "capture".into()),
            Some(&json!({"operation": "capture",
                "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
                "kind": "todo", "text": "x"})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "unknown-kind", "{answer}");
    }
}
