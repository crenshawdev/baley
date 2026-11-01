//! The rmcp handler for one session.
//!
//! It holds the session context, the identity stored from `initialize`, the
//! worker and the ledger the server opened. It decides nothing itself: it reads
//! what a call carries, runs the gate before the queue through the worker's
//! admission, and hands an accepted call to the worker, which runs the gate
//! after the queue and then the operation. A call that needs a project reaches
//! the operation with a [`Preparation`]: the caller the gate formed, the
//! call's host, the session's ledger and the server's time. `capture`,
//! `scope-submit`, the ten change operations from `project-describe` to
//! `backlog-reorder`, `project-start`, `work-order-fail`, `work-order-retry`,
//! `questions-answer`, `story-refine`, `story-questions-submit`,
//! `phase-plan`, `plan-submit`, `plan-replace`, `context-submit`, `document`,
//! `context-intake`, `plan-read` and `route` set `needs_project`, and in
//! their [`operate`] arms the writes prepare a write from it, `plan-submit`
//! and `plan-replace` after a settings read that records nothing, `document`,
//! `context-intake` and `plan-read` a read and `route` a read that also
//! reads the settings.
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
use super::document;
use super::gate::{Admitted, After, Called, after_queue, encode};
use super::operations::{help_answer, instruction_answer, schema_answer, unknown_operation};
use super::planning;
use super::questions;
use super::refine;
use super::scope;
use super::tools::{info, supported_protocol_versions, tool_list, version_answer};
use super::transport::RawFrameBytes;
use super::work_order;
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
/// `capture`, `scope-submit` and the ten change operations prepare a write
/// from the [`Preparation`], and `document`, `context-intake` and
/// `plan-read` a read.
pub(crate) fn operate(
    called: &Called,
    arguments: Option<&Value>,
    project: Option<&Preparation>,
) -> Value {
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
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "scope-submit" => {
            scope::serve(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "document" => {
            document::serve(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "project-start" => {
            work_order::serve_start(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "work-order-fail" => {
            work_order::serve_fail(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "work-order-retry" => {
            work_order::serve_retry(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "questions-answer" => {
            questions::serve(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "story-refine" => {
            refine::serve(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation))
            if spelling == "story-questions-submit" =>
        {
            refine::serve_questions(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "phase-plan" => {
            planning::serve(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation))
            if spelling == "plan-submit" || spelling == "plan-replace" =>
        {
            planning::serve_submit(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "context-submit" => {
            refine::serve_context(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "context-intake" => {
            refine::serve_intake(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "plan-read" => {
            planning::serve_read(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation)) if spelling == "route" => {
            work_order::serve_route(arguments_or_null, preparation)
        }
        (Called::Operation(_, spelling), Some(preparation))
            if scope::change_kind(spelling).is_some() =>
        {
            scope::serve_change(arguments_or_null, preparation)
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
        // A real store holding one project, so a refusal written to the ledger
        // shows in its head. A preparation that ran first would answer
        // `failed`, since the caller's directory holds no project.
        let at = "2026-10-05T09:00:00Z";
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            at,
            crate::ledger::open::options(),
        )
        .unwrap();
        let project = baley_store::ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        baley_store::Admin::create_project(&store, &project, "a", at).unwrap();
        let head = |store: &SqliteStore| {
            baley_store::Ledger::head(store, &project)
                .unwrap()
                .map(|head| head.seq)
        };
        let before = head(&store);
        let preparation = Preparation {
            ledger: Some(Arc::new(SessionLedger {
                store,
                config: dir.path().join("config"),
            })),
            host: Host::ClaudeCode,
            caller,
            at: at.into(),
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
        let ledger = preparation.ledger.as_deref().expect("a ledger");
        assert_eq!(head(&ledger.store), before);
    }

    #[test]
    fn a_refused_start_submission_answered_after_preparation_or_recorded_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // A real store holding one project, so anything recorded shows in its
        // head. A preparation that ran first would answer `failed`, since the
        // caller's directory holds no project.
        let at = "2026-10-13T09:00:00Z";
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            at,
            crate::ledger::open::options(),
        )
        .unwrap();
        let project = baley_store::ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        baley_store::Admin::create_project(&store, &project, "a", at).unwrap();
        let head = |store: &SqliteStore| {
            baley_store::Ledger::head(store, &project)
                .unwrap()
                .map(|head| head.seq)
        };
        let before = head(&store);
        let preparation = Preparation {
            ledger: Some(Arc::new(SessionLedger {
                store,
                config: dir.path().join("config"),
            })),
            host: Host::ClaudeCode,
            caller,
            at: at.into(),
        };
        let s1 = json!({"description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]},
            "stories": [
                {"label": "record", "sentence": "Every step is recorded", "status": "active"},
                {"label": "approve", "sentence": "The owner approves scope", "status": "active"},
                {"label": "later", "sentence": "Teammates see the backlog", "status": "deferred"}],
            "roadmap": [
                {"name": "Records", "goal": "Steps are recorded",
                    "detail": "The ledger and its views", "depends_on": [],
                    "stories": ["record"]},
                {"name": "Approval", "goal": "The owner approves",
                    "detail": "Approval by digest", "depends_on": [1],
                    "stories": ["approve"]}]});
        let mut no_goal = s1.clone();
        no_goal["roadmap"][0]
            .as_object_mut()
            .unwrap()
            .remove("goal");
        let mut double = s1.clone();
        double["roadmap"][1]["stories"] = json!(["approve", "record"]);
        let mut cycle = s1;
        cycle["roadmap"][0]["depends_on"] = json!([2]);
        for (request_id, submission, code, details) in [
            (
                "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
                no_goal,
                "malformed-submission",
                json!({"section": "roadmap", "field": "roadmap[0].goal"}),
            ),
            (
                "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f",
                double,
                "story-double-assigned",
                json!({"story": "record", "phases": [1, 2]}),
            ),
            (
                "5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b",
                cycle,
                "dependency-cycle",
                json!({"phases": [1, 2]}),
            ),
        ] {
            let answer = operate(
                &Called::Operation(Tool::Apply, "scope-submit".into()),
                Some(
                    &json!({"operation": "scope-submit", "request_id": request_id,
                    "work_order": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
                    "submission": submission}),
                ),
                Some(&preparation),
            );
            assert_eq!(answer["status"], "refused", "{answer}");
            assert_eq!(answer["code"], code, "{answer}");
            assert_eq!(answer["slot"], "submission", "{answer}");
            assert_eq!(answer["details"], details, "{answer}");
        }
        let ledger = preparation.ledger.as_deref().expect("a ledger");
        assert_eq!(head(&ledger.store), before);
    }

    #[test]
    fn an_owner_approval_accepted_over_mcp_is_caught() {
        // The owner approves on the command line only: nothing a session sends
        // proves a person answered, so `scope-approve` is no apply spelling.
        let expected = json!({
            "status": "refused",
            "code": "unknown-operation",
            "slot": "operation",
            "reason": format!(
                "no baley_apply operation is named `scope-approve`; the operations are {}",
                super::super::operations::expected::APPLY.join(", ")
            ),
        });
        let arguments = json!({"operation": "scope-approve",
            "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
            "digest": crate::scope::fixtures::M_DIGEST});
        let selection = Selection::Supported {
            host: Host::ClaudeCode,
            client_version: "2.1.287".into(),
        };
        let spelling = super::super::operations::spelling(arguments.as_object());
        let super::super::gate::Before::Answer(gate) =
            super::super::gate::before_queue("baley_apply", &selection, spelling)
        else {
            panic!("the gate must answer scope-approve before the queue");
        };
        assert_eq!(gate.structured_content.as_ref(), Some(&expected));

        // Past the gate too, a real store holding one project shows that
        // nothing is prepared or recorded.
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        let at = "2026-10-14T09:00:00Z";
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            at,
            crate::ledger::open::options(),
        )
        .unwrap();
        let project = baley_store::ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        baley_store::Admin::create_project(&store, &project, "a", at).unwrap();
        let head = |store: &SqliteStore| {
            baley_store::Ledger::head(store, &project)
                .unwrap()
                .map(|head| head.seq)
        };
        let before = head(&store);
        let preparation = Preparation {
            ledger: Some(Arc::new(SessionLedger {
                store,
                config: dir.path().join("config"),
            })),
            host: Host::ClaudeCode,
            caller,
            at: at.into(),
        };
        let answer = operate(
            &Called::Operation(Tool::Apply, "scope-approve".into()),
            Some(&arguments),
            Some(&preparation),
        );
        assert_eq!(answer, expected);
        let ledger = preparation.ledger.as_deref().expect("a ledger");
        assert_eq!(head(&ledger.store), before);
    }

    #[test]
    fn a_valid_submission_answered_as_an_unknown_operation_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger and no project at the caller's directory, so a submission
        // that reaches preparation is answered `failed` there.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-13T09:00:00Z".into(),
        };
        let answer = operate(
            &Called::Operation(Tool::Apply, "scope-submit".into()),
            Some(&json!({"operation": "scope-submit",
                "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
                "work_order": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
                "submission": crate::scope::fixtures::m()})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }

    #[test]
    fn a_valid_change_answered_as_an_unknown_operation_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger and no project at the caller's directory, so a change
        // that reaches preparation is answered `failed` there.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-13T09:00:00Z".into(),
        };
        let changes = [
            (
                "project-describe",
                json!({"what": "A ledger for agent work",
                    "core_value": "Nothing moves on an unproven claim",
                    "constraints": ["One binary", "No server"]}),
            ),
            (
                "phase-declare",
                json!({"name": "Reports", "goal": "The owner reads progress",
                    "detail": "Progress from the ledger", "depends_on": [1], "stories": []}),
            ),
            (
                "phase-edit",
                json!({"phase": 2, "goal": "The owner approves every change"}),
            ),
            ("phase-reorder", json!({"order": [2, 1]})),
            (
                "phase-withdraw",
                json!({"phase": 2, "decisions": [{"story": 2, "to": 1}]}),
            ),
            (
                "story-declare",
                json!({"sentence": "s", "status": "active", "phase": null}),
            ),
            (
                "story-edit",
                json!({"story": 3, "sentence": "Teammates read the backlog"}),
            ),
            ("story-reassign", json!({"story": 3, "phase": 2})),
            ("story-drop", json!({"story": 3})),
            ("backlog-reorder", json!({"order": [2, 1, 3]})),
        ];
        for (operation, submission) in changes {
            let answer = operate(
                &Called::Operation(Tool::Apply, operation.into()),
                Some(&json!({"operation": operation,
                    "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
                    "submission": submission})),
                Some(&preparation),
            );
            assert_eq!(answer["status"], "failed", "{operation}: {answer}");
            assert_eq!(answer["recorded"], false, "{operation}: {answer}");
            assert!(
                ["project-context-invalid", "not-a-project"]
                    .contains(&answer["code"].as_str().unwrap()),
                "{operation}: {answer}"
            );
        }
    }

    #[test]
    fn a_malformed_change_answered_after_preparation_or_recorded_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // A real store holding one project, so anything recorded shows in its
        // head. A preparation that ran first would answer `failed`, since the
        // caller's directory holds no project.
        let at = "2026-10-13T09:00:00Z";
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            at,
            crate::ledger::open::options(),
        )
        .unwrap();
        let project = baley_store::ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        baley_store::Admin::create_project(&store, &project, "a", at).unwrap();
        let head = |store: &SqliteStore| {
            baley_store::Ledger::head(store, &project)
                .unwrap()
                .map(|head| head.seq)
        };
        let before = head(&store);
        let preparation = Preparation {
            ledger: Some(Arc::new(SessionLedger {
                store,
                config: dir.path().join("config"),
            })),
            host: Host::ClaudeCode,
            caller,
            at: at.into(),
        };
        for (operation, request_id, submission, details) in [
            (
                "phase-reorder",
                "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
                json!({"order": []}),
                json!({"section": "order", "field": "order"}),
            ),
            (
                "story-declare",
                "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f",
                json!({"sentence": "s", "status": "active", "phase": null, "capture": "c-1"}),
                json!({"section": "capture", "field": "capture"}),
            ),
        ] {
            let answer = operate(
                &Called::Operation(Tool::Apply, operation.into()),
                Some(&json!({"operation": operation, "request_id": request_id,
                    "submission": submission})),
                Some(&preparation),
            );
            assert_eq!(answer["status"], "refused", "{operation}: {answer}");
            assert_eq!(
                answer["code"], "malformed-submission",
                "{operation}: {answer}"
            );
            assert_eq!(answer["slot"], "submission", "{operation}: {answer}");
            assert_eq!(answer["details"], details, "{operation}: {answer}");
        }
        let ledger = preparation.ledger.as_deref().expect("a ledger");
        assert_eq!(head(&ledger.store), before);
    }

    #[test]
    fn a_document_argument_refusal_answered_after_preparation_is_caught() {
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
            &Called::Operation(Tool::Query, "document".into()),
            Some(&json!({"operation": "document",
                "identity": {"kind": "capture", "id": "c1"},
                "instruction": "bal-capture"})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "invalid-arguments", "{answer}");
    }

    #[test]
    fn a_start_prepared_before_its_brief_is_judged_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger and no project at the caller's directory, so a start that
        // reaches preparation is answered `failed` there.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-13T09:00:00Z".into(),
        };
        let start = |brief: &str| {
            operate(
                &Called::Operation(Tool::Apply, "project-start".into()),
                Some(&json!({"operation": "project-start",
                    "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
                    "brief": brief})),
                Some(&preparation),
            )
        };
        let blank = start("");
        assert_eq!(blank["status"], "refused", "{blank}");
        assert_eq!(blank["code"], "brief-blank", "{blank}");

        let answer = start("Build a ledger for agent work");
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
        assert!(
            ["project-context-invalid", "not-a-project"]
                .contains(&answer["code"].as_str().unwrap()),
            "{answer}"
        );
    }

    #[test]
    fn the_work_order_operations_left_unserved_by_operate_are_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger, so a call that reached preparation would answer `failed`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-13T09:00:00Z".into(),
        };
        let order = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
        let request_id = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
        let fail = operate(
            &Called::Operation(Tool::Apply, "work-order-fail".into()),
            Some(
                &json!({"operation": "work-order-fail", "request_id": request_id,
                "work_order": order, "reason": " "}),
            ),
            Some(&preparation),
        );
        let retry = operate(
            &Called::Operation(Tool::Apply, "work-order-retry".into()),
            Some(
                &json!({"operation": "work-order-retry", "request_id": request_id,
                "work_order": order.to_uppercase()}),
            ),
            Some(&preparation),
        );
        for (answer, slot) in [(fail, "reason"), (retry, "work_order")] {
            assert_eq!(answer["status"], "refused", "{answer}");
            assert_eq!(answer["code"], "invalid-arguments", "{answer}");
            assert_eq!(answer["slot"], slot, "{answer}");
        }
    }

    #[test]
    fn a_route_query_left_unserved_by_operate_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger, so a call that reached preparation would answer `failed`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-13T09:00:00Z".into(),
        };
        let answer = operate(
            &Called::Operation(Tool::Query, "route".into()),
            Some(&json!({"operation": "route"})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "invalid-arguments", "{answer}");
        assert_eq!(answer["slot"], "arguments", "{answer}");
    }

    #[test]
    fn a_questions_answer_left_unserved_by_operate_is_caught() {
        let caller = ServerCaller::new(
            "/real/r",
            "/real/r",
            "claude-code",
            "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
            &json!(7),
        )
        .unwrap();
        // No ledger, so a call that reached preparation would answer `failed`.
        let preparation = Preparation {
            ledger: None,
            host: Host::ClaudeCode,
            caller,
            at: "2026-10-22T09:00:00Z".into(),
        };
        let answer = operate(
            &Called::Operation(Tool::Apply, "questions-answer".into()),
            Some(&json!({"operation": "questions-answer",
                "request_id": "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f",
                "set": "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d", "items": []})),
            Some(&preparation),
        );
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "typed-content", "{answer}");
    }
}
