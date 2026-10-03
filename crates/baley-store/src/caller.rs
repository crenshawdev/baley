//! The caller of a ledger event: who asked for the append, in a bounded,
//! checked value that is part of the hashed record (design 0001, Events;
//! design 0012, HST-R7 and HST-R11).
//!
//! A caller has two forms. The server form is what the Baley server records
//! for a request: it names a Baley session. The hook form is what the guard
//! hook records for a tool call: it has no Baley session field at all, so a
//! hook cannot claim a server session. Both are built through constructors
//! that refuse bad text, and both have one strict canonical JSON encoding,
//! so the envelope can hash a caller and an adapter can store it and read it
//! back to an equal value.
//!
//! This module reads neither the environment nor the process: whoever builds
//! a caller gathers its fields.

use std::fmt;
use std::path::Path;

use serde_json::{Map, Value};

use crate::canonical::canonical_json;

/// Bytes allowed in a project or working directory. Documentation leaves the
/// length open (the hook input's `cwd` has none), so this is a choice: Linux's
/// `PATH_MAX`, the larger of the two platforms', so any absolute path either
/// accepts whole fits.
pub const MAX_DIRECTORY_BYTES: usize = 4096;
/// Bytes allowed in a host name. A choice: `claude-code` is 11 bytes.
pub const MAX_HOST_BYTES: usize = 64;
/// Bytes allowed in a client version. MCP `Implementation.version` is a
/// string with no stated length, so this is a choice with headroom over a
/// version such as `2.1.278`.
pub const MAX_CLIENT_VERSION_BYTES: usize = 128;
/// Bytes in a Baley session: a lower-case UUID version 4, 36 bytes by its
/// own rule. Baley mints it.
pub const BALEY_SESSION_BYTES: usize = 36;
/// Bytes allowed in a host-native session id. Claude Code documents
/// `session_id` as a string with no length and no format, so this is a
/// choice.
pub const MAX_HOST_SESSION_BYTES: usize = 128;
/// Bytes allowed in a call identity's text. `tool_use_id` is a string with
/// no stated length and a JSON-RPC id is a string or integer with none, so
/// this is a choice. It counts the quotes and escapes of a string id.
pub const MAX_CALL_IDENTITY_BYTES: usize = 256;
/// Bytes allowed in a work order id. Work orders are defined by a later
/// build, so this is a choice.
pub const MAX_WORK_ORDER_BYTES: usize = 256;
/// Instruction evidence entries a caller may carry. A choice.
pub const MAX_INSTRUCTIONS: usize = 32;
/// Bytes allowed in an instruction identity. A choice.
pub const MAX_INSTRUCTION_IDENTITY_BYTES: usize = 256;
/// Bytes allowed in an instruction version. A choice.
pub const MAX_INSTRUCTION_VERSION_BYTES: usize = 64;
/// Bytes allowed in an instruction hash. A choice: a SHA-256 hex digest is
/// 64 bytes, so 128 leaves room for a prefix.
pub const MAX_INSTRUCTION_HASH_BYTES: usize = 128;

const FORM_SERVER: &str = "server";
const FORM_HOOK: &str = "hook";

const SERVER_KEYS: &[&str] = &[
    "form",
    "project_directory",
    "working_directory",
    "host",
    "baley_session",
    "call",
    "client_version",
    "host_session",
    "work_order",
    "instructions",
];
const HOOK_KEYS: &[&str] = &[
    "form",
    "project_directory",
    "working_directory",
    "host",
    "call",
    "host_session",
    "work_order",
    "instructions",
];
const CALL_KEYS: &[&str] = &["text", "source"];
const INSTRUCTION_KEYS: &[&str] = &["identity", "version", "hash"];

/// Why a caller cannot be built or read. Each variant that concerns a field
/// names it by its key in the encoded form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerError {
    /// A text field is empty. An empty optional field is an error, never
    /// read as absent.
    Empty { field: &'static str },
    /// A text field is longer than its limit.
    TooLong {
        field: &'static str,
        limit: usize,
        bytes: usize,
    },
    /// A directory is not absolute.
    NotAbsolute { field: &'static str },
    /// The Baley session is not a lower-case UUID version 4.
    NotSessionId,
    /// The host is not 1 to 64 bytes of lower-case letters, digits and `-`
    /// starting with a letter.
    NotHostName,
    /// More than 32 instruction evidence entries.
    TooManyInstructions { count: usize },
    /// A JSON-RPC id that is not a string or an integer in the `i64` range.
    NotJsonRpcId,
    /// An encoded caller, call or instruction entry is not an object.
    NotAnObject { at: &'static str },
    /// The `form` key is missing or is neither `server` nor `hook`.
    BadForm,
    /// A key the form does not define.
    UnknownKey { at: &'static str, key: String },
    /// A required key is missing.
    MissingField { field: &'static str },
    /// A text field holds `null` or another value that is not a string.
    NotText { field: &'static str },
    /// The instruction list is not an array.
    NotAList,
    /// The instruction list is empty: an absent list has no key.
    EmptyInstructionList,
    /// A hook object holds the Baley session key.
    HookHoldsSession,
    /// The call source is neither `jsonrpc_id` nor `tool_use_id`.
    UnknownSource { found: String },
    /// The call source belongs to the other form.
    SourceMismatch,
    /// The server call text is neither an integer's digits nor a canonical
    /// JSON string literal.
    NotCanonicalJsonRpcId,
}

impl fmt::Display for CallerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { field } => write!(f, "caller {field} is empty"),
            Self::TooLong {
                field,
                limit,
                bytes,
            } => write!(f, "caller {field} is {bytes} bytes, over its limit of {limit}"),
            Self::NotAbsolute { field } => write!(f, "caller {field} is not an absolute path"),
            Self::NotSessionId => f.write_str("caller baley_session is not a lower-case UUID v4"),
            Self::NotHostName => f.write_str(
                "caller host is not name text: lower-case letters, digits and `-`, starting with a letter",
            ),
            Self::TooManyInstructions { count } => write!(
                f,
                "caller carries {count} instruction entries, over the limit of {MAX_INSTRUCTIONS}"
            ),
            Self::NotJsonRpcId => {
                f.write_str("a JSON-RPC id is a string or an integer in the 64-bit range")
            }
            Self::NotAnObject { at } => write!(f, "caller {at} is not an object"),
            Self::BadForm => f.write_str("caller form is missing or is neither server nor hook"),
            Self::UnknownKey { at, key } => write!(f, "caller {at} has an unknown key {key}"),
            Self::MissingField { field } => write!(f, "caller {field} is missing"),
            Self::NotText { field } => write!(f, "caller {field} is not a string"),
            Self::NotAList => f.write_str("caller instructions is not a list"),
            Self::EmptyInstructionList => {
                f.write_str("caller instructions is empty, so it must have no key")
            }
            Self::HookHoldsSession => f.write_str("a hook caller cannot hold a baley_session"),
            Self::UnknownSource { found } => write!(f, "caller call source {found} is unknown"),
            Self::SourceMismatch => f.write_str("caller call source does not belong to its form"),
            Self::NotCanonicalJsonRpcId => {
                f.write_str("caller call text is not a canonical JSON-RPC id")
            }
        }
    }
}

impl std::error::Error for CallerError {}

/// Where a call identity came from. The set is closed: each form accepts
/// exactly one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSource {
    /// The request's JSON-RPC id, scoped to the Baley session. Server form
    /// only.
    JsonRpcId,
    /// Claude Code's `tool_use_id`. Hook form only.
    ToolUseId,
}

impl CallSource {
    /// The value the encoded form carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonRpcId => "jsonrpc_id",
            Self::ToolUseId => "tool_use_id",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "jsonrpc_id" => Some(Self::JsonRpcId),
            "tool_use_id" => Some(Self::ToolUseId),
            _ => None,
        }
    }
}

/// Which call a caller made: a text and the source that text came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallIdentity {
    text: String,
    source: CallSource,
}

impl CallIdentity {
    /// The text. For a JSON-RPC id it is the id written as JSON, so the
    /// integer `1` is `1` and the string `"1"` is `"1"` with its quotes.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the text came from.
    pub fn source(&self) -> CallSource {
        self.source
    }

    fn json_rpc(id: &Value) -> Result<Self, CallerError> {
        let text = json_rpc_text(id)?;
        Ok(Self {
            text: text_field("call_identity", &text, MAX_CALL_IDENTITY_BYTES)?,
            source: CallSource::JsonRpcId,
        })
    }

    fn tool_use(id: &str) -> Result<Self, CallerError> {
        Ok(Self {
            text: text_field("call_identity", id, MAX_CALL_IDENTITY_BYTES)?,
            source: CallSource::ToolUseId,
        })
    }
}

/// An id written as JSON: an integer as its digits, a string as the literal
/// `canonical_json` writes. No id has two texts.
fn json_rpc_text(id: &Value) -> Result<String, CallerError> {
    match id {
        Value::Number(number) => number
            .as_i64()
            .map(|integer| integer.to_string())
            .ok_or(CallerError::NotJsonRpcId),
        Value::String(_) => canonical_json(id)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(CallerError::NotJsonRpcId),
        _ => Err(CallerError::NotJsonRpcId),
    }
}

/// The id a call text names, if the text is exactly what `json_rpc_text`
/// writes for it.
fn json_rpc_id(text: &str) -> Result<Value, CallerError> {
    let refused = CallerError::NotCanonicalJsonRpcId;
    let id = if text.starts_with('"') {
        let string = serde_json::from_str::<String>(text).map_err(|_| refused.clone())?;
        Value::String(string)
    } else {
        Value::from(text.parse::<i64>().map_err(|_| refused.clone())?)
    };
    match json_rpc_text(&id) {
        Ok(written) if written == text => Ok(id),
        _ => Err(CallerError::NotCanonicalJsonRpcId),
    }
}

/// One instruction a caller was working under: its identity, version and
/// hash, all text. Later builds say what each names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionEvidence {
    identity: String,
    version: String,
    hash: String,
}

impl InstructionEvidence {
    /// An entry, refused if any part is empty or over its limit.
    pub fn new(identity: &str, version: &str, hash: &str) -> Result<Self, CallerError> {
        Ok(Self {
            identity: text_field(
                "instruction_identity",
                identity,
                MAX_INSTRUCTION_IDENTITY_BYTES,
            )?,
            version: text_field(
                "instruction_version",
                version,
                MAX_INSTRUCTION_VERSION_BYTES,
            )?,
            hash: text_field("instruction_hash", hash, MAX_INSTRUCTION_HASH_BYTES)?,
        })
    }

    /// The instruction's identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The instruction's version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The instruction's hash.
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

/// The fields both forms share.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Common {
    working_directory: String,
    host: String,
    call: CallIdentity,
    host_session: Option<String>,
    work_order: Option<String>,
    instructions: Vec<InstructionEvidence>,
}

impl Common {
    fn new(working_directory: &str, host: &str, call: CallIdentity) -> Result<Self, CallerError> {
        Ok(Self {
            working_directory: directory("working_directory", working_directory)?,
            host: host_name(host)?,
            call,
            host_session: None,
            work_order: None,
            instructions: Vec::new(),
        })
    }

    fn set_instructions(&mut self, entries: Vec<InstructionEvidence>) -> Result<(), CallerError> {
        if entries.len() > MAX_INSTRUCTIONS {
            return Err(CallerError::TooManyInstructions {
                count: entries.len(),
            });
        }
        self.instructions = entries;
        Ok(())
    }

    /// The keys both forms write, into `object`.
    fn write(&self, object: &mut Map<String, Value>) {
        object.insert(
            "working_directory".into(),
            self.working_directory.clone().into(),
        );
        object.insert("host".into(), self.host.clone().into());
        let mut call = Map::new();
        call.insert("text".into(), self.call.text.clone().into());
        call.insert("source".into(), self.call.source.as_str().into());
        object.insert("call".into(), Value::Object(call));
        if let Some(session) = &self.host_session {
            object.insert("host_session".into(), session.clone().into());
        }
        if let Some(order) = &self.work_order {
            object.insert("work_order".into(), order.clone().into());
        }
        if !self.instructions.is_empty() {
            let entries = self.instructions.iter().map(|entry| {
                let mut item = Map::new();
                item.insert("identity".into(), entry.identity.clone().into());
                item.insert("version".into(), entry.version.clone().into());
                item.insert("hash".into(), entry.hash.clone().into());
                Value::Object(item)
            });
            object.insert("instructions".into(), Value::Array(entries.collect()));
        }
    }
}

/// A caller the Baley server recorded for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCaller {
    project_directory: String,
    baley_session: String,
    client_version: Option<String>,
    common: Common,
}

impl ServerCaller {
    /// A server caller with its required fields. `request_id` is the JSON-RPC
    /// id the request carried, a string or an integer.
    pub fn new(
        project_directory: &str,
        working_directory: &str,
        host: &str,
        baley_session: &str,
        request_id: &Value,
    ) -> Result<Self, CallerError> {
        let common = Common::new(working_directory, host, CallIdentity::json_rpc(request_id)?)?;
        Ok(Self {
            project_directory: directory("project_directory", project_directory)?,
            baley_session: session_id(baley_session)?,
            client_version: None,
            common,
        })
    }

    /// With the client's self-reported version.
    pub fn with_client_version(mut self, version: &str) -> Result<Self, CallerError> {
        let limit = MAX_CLIENT_VERSION_BYTES;
        self.client_version = Some(text_field("client_version", version, limit)?);
        Ok(self)
    }

    /// With the host's own session id.
    pub fn with_host_session(mut self, session: &str) -> Result<Self, CallerError> {
        self.common.host_session = Some(host_session(session)?);
        Ok(self)
    }

    /// With the work order the call acts under.
    pub fn with_work_order(mut self, order: &str) -> Result<Self, CallerError> {
        self.common.work_order = Some(work_order(order)?);
        Ok(self)
    }

    /// With the instructions the caller worked under.
    pub fn with_instructions(
        mut self,
        entries: Vec<InstructionEvidence>,
    ) -> Result<Self, CallerError> {
        self.common.set_instructions(entries)?;
        Ok(self)
    }

    /// The project directory.
    pub fn project_directory(&self) -> &str {
        &self.project_directory
    }

    /// The working directory.
    pub fn working_directory(&self) -> &str {
        &self.common.working_directory
    }

    /// The host's name.
    pub fn host(&self) -> &str {
        &self.common.host
    }

    /// The Baley session.
    pub fn baley_session(&self) -> &str {
        &self.baley_session
    }

    /// The call, derived from the request's JSON-RPC id.
    pub fn call(&self) -> &CallIdentity {
        &self.common.call
    }

    /// The client's self-reported version, if given.
    pub fn client_version(&self) -> Option<&str> {
        self.client_version.as_deref()
    }

    /// The host's own session id, if given.
    pub fn host_session(&self) -> Option<&str> {
        self.common.host_session.as_deref()
    }

    /// The work order, if given.
    pub fn work_order(&self) -> Option<&str> {
        self.common.work_order.as_deref()
    }

    /// The instruction evidence, possibly none.
    pub fn instructions(&self) -> &[InstructionEvidence] {
        &self.common.instructions
    }
}

/// A caller the guard hook recorded for a tool call. It has no Baley session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCaller {
    project_directory: Option<String>,
    common: Common,
}

impl HookCaller {
    /// A hook caller with its required fields. `tool_use_id` is Claude Code's
    /// own id for the tool call.
    pub fn new(
        host: &str,
        working_directory: &str,
        tool_use_id: &str,
    ) -> Result<Self, CallerError> {
        Ok(Self {
            project_directory: None,
            common: Common::new(
                working_directory,
                host,
                CallIdentity::tool_use(tool_use_id)?,
            )?,
        })
    }

    /// With the project directory, when the hook found a project.
    pub fn with_project_directory(mut self, directory_text: &str) -> Result<Self, CallerError> {
        self.project_directory = Some(directory("project_directory", directory_text)?);
        Ok(self)
    }

    /// With the host's own session id.
    pub fn with_host_session(mut self, session: &str) -> Result<Self, CallerError> {
        self.common.host_session = Some(host_session(session)?);
        Ok(self)
    }

    /// With the work order the call acts under.
    pub fn with_work_order(mut self, order: &str) -> Result<Self, CallerError> {
        self.common.work_order = Some(work_order(order)?);
        Ok(self)
    }

    /// With the instructions the caller worked under.
    pub fn with_instructions(
        mut self,
        entries: Vec<InstructionEvidence>,
    ) -> Result<Self, CallerError> {
        self.common.set_instructions(entries)?;
        Ok(self)
    }

    /// The project directory, if given.
    pub fn project_directory(&self) -> Option<&str> {
        self.project_directory.as_deref()
    }

    /// The working directory.
    pub fn working_directory(&self) -> &str {
        &self.common.working_directory
    }

    /// The host's name.
    pub fn host(&self) -> &str {
        &self.common.host
    }

    /// The call, Claude Code's `tool_use_id`.
    pub fn call(&self) -> &CallIdentity {
        &self.common.call
    }

    /// The host's own session id, if given.
    pub fn host_session(&self) -> Option<&str> {
        self.common.host_session.as_deref()
    }

    /// The work order, if given.
    pub fn work_order(&self) -> Option<&str> {
        self.common.work_order.as_deref()
    }

    /// The instruction evidence, possibly none.
    pub fn instructions(&self) -> &[InstructionEvidence] {
        &self.common.instructions
    }
}

/// The caller of a ledger event, in one of two forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// Recorded by the Baley server for a request.
    Server(ServerCaller),
    /// Recorded by the guard hook for a tool call.
    Hook(HookCaller),
}

impl Caller {
    /// The one JSON object that encodes this caller. An absent optional field
    /// and an empty instruction list have no key, so a value has one
    /// encoding. It holds only strings, arrays and objects, so
    /// `canonical_json` always succeeds on it.
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        match self {
            Self::Server(server) => {
                object.insert("form".into(), FORM_SERVER.into());
                object.insert(
                    "project_directory".into(),
                    server.project_directory.clone().into(),
                );
                object.insert("baley_session".into(), server.baley_session.clone().into());
                if let Some(version) = &server.client_version {
                    object.insert("client_version".into(), version.clone().into());
                }
                server.common.write(&mut object);
            }
            Self::Hook(hook) => {
                object.insert("form".into(), FORM_HOOK.into());
                if let Some(directory) = &hook.project_directory {
                    object.insert("project_directory".into(), directory.clone().into());
                }
                hook.common.write(&mut object);
            }
        }
        Value::Object(object)
    }

    /// The caller an encoded object holds. Anything construction refuses is
    /// refused here, and so is anything the encoder would not write.
    pub fn from_value(value: &Value) -> Result<Self, CallerError> {
        let object = value
            .as_object()
            .ok_or(CallerError::NotAnObject { at: "caller" })?;
        match object.get("form").and_then(Value::as_str) {
            Some(FORM_SERVER) => decode_server(object).map(Self::Server),
            Some(FORM_HOOK) => decode_hook(object).map(Self::Hook),
            _ => Err(CallerError::BadForm),
        }
    }
}

fn decode_server(object: &Map<String, Value>) -> Result<ServerCaller, CallerError> {
    only_keys("caller", object, SERVER_KEYS)?;
    let id = decode_call(object, CallSource::JsonRpcId)?;
    let mut caller = ServerCaller::new(
        required(object, "project_directory")?,
        required(object, "working_directory")?,
        required(object, "host")?,
        required(object, "baley_session")?,
        &json_rpc_id(&id)?,
    )?;
    if let Some(version) = optional(object, "client_version")? {
        caller = caller.with_client_version(version)?;
    }
    if let Some(session) = optional(object, "host_session")? {
        caller = caller.with_host_session(session)?;
    }
    if let Some(order) = optional(object, "work_order")? {
        caller = caller.with_work_order(order)?;
    }
    if let Some(entries) = decode_instructions(object)? {
        caller = caller.with_instructions(entries)?;
    }
    Ok(caller)
}

fn decode_hook(object: &Map<String, Value>) -> Result<HookCaller, CallerError> {
    // Checked before the key set so the refusal says what was claimed.
    if object.contains_key("baley_session") {
        return Err(CallerError::HookHoldsSession);
    }
    only_keys("caller", object, HOOK_KEYS)?;
    let id = decode_call(object, CallSource::ToolUseId)?;
    let mut caller = HookCaller::new(
        required(object, "host")?,
        required(object, "working_directory")?,
        &id,
    )?;
    if let Some(directory_text) = optional(object, "project_directory")? {
        caller = caller.with_project_directory(directory_text)?;
    }
    if let Some(session) = optional(object, "host_session")? {
        caller = caller.with_host_session(session)?;
    }
    if let Some(order) = optional(object, "work_order")? {
        caller = caller.with_work_order(order)?;
    }
    if let Some(entries) = decode_instructions(object)? {
        caller = caller.with_instructions(entries)?;
    }
    Ok(caller)
}

/// The call's text, once its keys are known and its source is the one the
/// form accepts.
fn decode_call(object: &Map<String, Value>, expected: CallSource) -> Result<String, CallerError> {
    let call = object
        .get("call")
        .ok_or(CallerError::MissingField { field: "call" })?
        .as_object()
        .ok_or(CallerError::NotAnObject { at: "call" })?;
    only_keys("call", call, CALL_KEYS)?;
    let source = required(call, "source")?;
    match CallSource::parse(source) {
        None => {
            return Err(CallerError::UnknownSource {
                found: source.to_owned(),
            });
        }
        Some(found) if found != expected => return Err(CallerError::SourceMismatch),
        Some(_) => {}
    }
    required(call, "text").map(str::to_owned)
}

fn decode_instructions(
    object: &Map<String, Value>,
) -> Result<Option<Vec<InstructionEvidence>>, CallerError> {
    let Some(list) = object.get("instructions") else {
        return Ok(None);
    };
    let items = list.as_array().ok_or(CallerError::NotAList)?;
    if items.is_empty() {
        return Err(CallerError::EmptyInstructionList);
    }
    // Counted before the entries are read, so a huge list costs nothing.
    if items.len() > MAX_INSTRUCTIONS {
        return Err(CallerError::TooManyInstructions { count: items.len() });
    }
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let entry = item
            .as_object()
            .ok_or(CallerError::NotAnObject { at: "instruction" })?;
        only_keys("instruction", entry, INSTRUCTION_KEYS)?;
        entries.push(InstructionEvidence::new(
            required(entry, "identity")?,
            required(entry, "version")?,
            required(entry, "hash")?,
        )?);
    }
    Ok(Some(entries))
}

fn only_keys(
    at: &'static str,
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), CallerError> {
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(CallerError::UnknownKey {
            at,
            key: key.clone(),
        }),
        None => Ok(()),
    }
}

/// A present key's string. `null` or any other value is an error, never
/// absence.
fn optional<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<Option<&'a str>, CallerError> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(CallerError::NotText { field }),
    }
}

fn required<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, CallerError> {
    optional(object, field)?.ok_or(CallerError::MissingField { field })
}

/// `text` if it is 1 to `limit` bytes.
fn text_field(field: &'static str, text: &str, limit: usize) -> Result<String, CallerError> {
    if text.is_empty() {
        return Err(CallerError::Empty { field });
    }
    if text.len() > limit {
        return Err(CallerError::TooLong {
            field,
            limit,
            bytes: text.len(),
        });
    }
    Ok(text.to_owned())
}

fn directory(field: &'static str, text: &str) -> Result<String, CallerError> {
    let text = text_field(field, text, MAX_DIRECTORY_BYTES)?;
    if !Path::new(&text).is_absolute() {
        return Err(CallerError::NotAbsolute { field });
    }
    Ok(text)
}

fn host_name(text: &str) -> Result<String, CallerError> {
    let text = text_field("host", text, MAX_HOST_BYTES)?;
    let mut bytes = text.bytes();
    let name = bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !name {
        return Err(CallerError::NotHostName);
    }
    Ok(text)
}

/// The rule the core crate's `is_project_id` states, written out here
/// because this crate never depends on that crate: 36 bytes of lower-case
/// hex in 8-4-4-4-12 groups, version nibble `4`, variant nibble `8`, `9`,
/// `a` or `b`.
fn session_id(text: &str) -> Result<String, CallerError> {
    let text = text_field("baley_session", text, BALEY_SESSION_BYTES)?;
    let bytes = text.as_bytes();
    let uuid = bytes.len() == BALEY_SESSION_BYTES
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b');
    if !uuid {
        return Err(CallerError::NotSessionId);
    }
    Ok(text)
}

fn host_session(text: &str) -> Result<String, CallerError> {
    text_field("host_session", text, MAX_HOST_SESSION_BYTES)
}

fn work_order(text: &str) -> Result<String, CallerError> {
    text_field("work_order", text, MAX_WORK_ORDER_BYTES)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";

    fn server() -> ServerCaller {
        ServerCaller::new(
            "/work/proj",
            "/work/proj/src",
            "claude-code",
            SESSION,
            &json!(1),
        )
        .unwrap()
    }

    fn hook() -> HookCaller {
        HookCaller::new("claude-code", "/work/proj", "toolu_01ABC").unwrap()
    }

    fn entry() -> InstructionEvidence {
        InstructionEvidence::new("stub:claude", "3", "ab12").unwrap()
    }

    fn full_server() -> ServerCaller {
        server()
            .with_client_version("2.1.278")
            .unwrap()
            .with_host_session("abc123")
            .unwrap()
            .with_work_order("wo-7")
            .unwrap()
            .with_instructions(vec![entry()])
            .unwrap()
    }

    fn full_hook() -> HookCaller {
        hook()
            .with_project_directory("/work/proj")
            .unwrap()
            .with_host_session("abc123")
            .unwrap()
            .with_work_order("wo-7")
            .unwrap()
            .with_instructions(vec![entry()])
            .unwrap()
    }

    fn path_of(len: usize) -> String {
        format!("/{}", "a".repeat(len - 1))
    }

    #[test]
    fn server_form_keeps_every_field_it_was_given() {
        let caller = full_server();
        assert_eq!(caller.project_directory(), "/work/proj");
        assert_eq!(caller.working_directory(), "/work/proj/src");
        assert_eq!(caller.host(), "claude-code");
        assert_eq!(caller.baley_session(), SESSION);
        assert_eq!(caller.call().text(), "1");
        assert_eq!(caller.call().source(), CallSource::JsonRpcId);
        assert_eq!(caller.client_version(), Some("2.1.278"));
        assert_eq!(caller.host_session(), Some("abc123"));
        assert_eq!(caller.work_order(), Some("wo-7"));
        assert_eq!(caller.instructions(), &[entry()]);
        assert_eq!(entry().identity(), "stub:claude");
        assert_eq!(entry().version(), "3");
        assert_eq!(entry().hash(), "ab12");
    }

    #[test]
    fn server_form_with_only_required_fields_has_no_optional_ones() {
        let caller = server();
        assert_eq!(caller.client_version(), None);
        assert_eq!(caller.host_session(), None);
        assert_eq!(caller.work_order(), None);
        assert!(caller.instructions().is_empty());
    }

    #[test]
    fn hook_form_keeps_every_field_it_was_given() {
        let caller = full_hook();
        assert_eq!(caller.project_directory(), Some("/work/proj"));
        assert_eq!(caller.working_directory(), "/work/proj");
        assert_eq!(caller.host(), "claude-code");
        assert_eq!(caller.call().text(), "toolu_01ABC");
        assert_eq!(caller.call().source(), CallSource::ToolUseId);
        assert_eq!(caller.host_session(), Some("abc123"));
        assert_eq!(caller.work_order(), Some("wo-7"));
        assert_eq!(caller.instructions(), &[entry()]);
    }

    #[test]
    fn hook_form_builds_without_a_project_directory() {
        let caller = hook();
        assert_eq!(caller.project_directory(), None);
        assert_eq!(caller.host_session(), None);
        assert_eq!(caller.work_order(), None);
        assert!(caller.instructions().is_empty());
    }

    #[test]
    fn an_empty_required_field_is_refused_in_both_forms() {
        let new = |dir: &str, cwd: &str, host: &str, session: &str| {
            ServerCaller::new(dir, cwd, host, session, &json!(1))
        };
        let empty = |field| CallerError::Empty { field };
        assert_eq!(
            new("", "/w", "h", SESSION).unwrap_err(),
            empty("project_directory")
        );
        assert_eq!(
            new("/p", "", "h", SESSION).unwrap_err(),
            empty("working_directory")
        );
        assert_eq!(new("/p", "/w", "", SESSION).unwrap_err(), empty("host"));
        assert_eq!(
            new("/p", "/w", "h", "").unwrap_err(),
            empty("baley_session")
        );
        assert_eq!(
            HookCaller::new("claude-code", "/w", "").unwrap_err(),
            CallerError::Empty {
                field: "call_identity"
            }
        );
        assert_eq!(
            ServerCaller::new("/p", "/w", "h", SESSION, &json!(""))
                .map(|c| c.call().text().to_owned()),
            Ok("\"\"".to_owned()),
            "the empty string id has the text of two quotes, so it is not an empty text"
        );
    }

    #[test]
    fn an_empty_optional_field_is_refused_not_dropped() {
        let empty = |field| CallerError::Empty { field };
        assert_eq!(
            server().with_client_version("").unwrap_err(),
            empty("client_version")
        );
        assert_eq!(
            server().with_host_session("").unwrap_err(),
            empty("host_session")
        );
        assert_eq!(
            server().with_work_order("").unwrap_err(),
            empty("work_order")
        );
        assert_eq!(
            hook().with_project_directory("").unwrap_err(),
            empty("project_directory")
        );
        assert_eq!(
            hook().with_host_session("").unwrap_err(),
            empty("host_session")
        );
        assert_eq!(hook().with_work_order("").unwrap_err(), empty("work_order"));
        assert_eq!(
            InstructionEvidence::new("", "1", "h").unwrap_err(),
            empty("instruction_identity")
        );
        assert_eq!(
            InstructionEvidence::new("i", "", "h").unwrap_err(),
            empty("instruction_version")
        );
        assert_eq!(
            InstructionEvidence::new("i", "1", "").unwrap_err(),
            empty("instruction_hash")
        );
    }

    type Build = Box<dyn Fn(usize) -> Result<(), CallerError>>;

    #[test]
    fn a_limit_that_is_wrong_on_any_field_fails_the_boundary_table() {
        let session = |len: usize| match len {
            36 => SESSION.to_owned(),
            len => format!("{SESSION}{}", "a".repeat(len - 36)),
        };
        // Literal limits, written from the plan's table, not from the constants.
        let cases: Vec<(&str, usize, Build)> = vec![
            (
                "project_directory",
                4096,
                Box::new(|n| {
                    ServerCaller::new(&path_of(n), "/w", "h", SESSION, &json!(1)).map(drop)
                }),
            ),
            (
                "working_directory",
                4096,
                Box::new(|n| {
                    ServerCaller::new("/p", &path_of(n), "h", SESSION, &json!(1)).map(drop)
                }),
            ),
            (
                "host",
                64,
                Box::new(|n| {
                    ServerCaller::new("/p", "/w", &"a".repeat(n), SESSION, &json!(1)).map(drop)
                }),
            ),
            (
                "baley_session",
                36,
                Box::new(move |n| {
                    ServerCaller::new("/p", "/w", "h", &session(n), &json!(1)).map(drop)
                }),
            ),
            (
                "client_version",
                128,
                Box::new(|n| server().with_client_version(&"1".repeat(n)).map(drop)),
            ),
            (
                "host_session",
                128,
                Box::new(|n| server().with_host_session(&"s".repeat(n)).map(drop)),
            ),
            (
                "call_identity",
                256,
                Box::new(|n| HookCaller::new("h", "/w", &"t".repeat(n)).map(drop)),
            ),
            (
                "work_order",
                256,
                Box::new(|n| hook().with_work_order(&"w".repeat(n)).map(drop)),
            ),
            (
                "instruction_identity",
                256,
                Box::new(|n| InstructionEvidence::new(&"i".repeat(n), "1", "h").map(drop)),
            ),
            (
                "instruction_version",
                64,
                Box::new(|n| InstructionEvidence::new("i", &"v".repeat(n), "h").map(drop)),
            ),
            (
                "instruction_hash",
                128,
                Box::new(|n| InstructionEvidence::new("i", "1", &"h".repeat(n)).map(drop)),
            ),
        ];
        for (field, limit, build) in cases {
            assert_eq!(build(limit), Ok(()), "{field} at its limit");
            assert_eq!(
                build(limit + 1),
                Err(CallerError::TooLong {
                    field,
                    limit,
                    bytes: limit + 1
                }),
                "{field} one byte over"
            );
        }
    }

    #[test]
    fn a_json_rpc_string_id_counts_its_quotes_toward_the_call_limit() {
        // 254 characters plus two quotes is 256 bytes, 255 plus two is 257.
        let at = |n: usize| ServerCaller::new("/p", "/w", "h", SESSION, &json!("x".repeat(n)));
        assert!(at(254).is_ok());
        assert!(matches!(
            at(255),
            Err(CallerError::TooLong {
                field: "call_identity",
                ..
            })
        ));
    }

    #[test]
    fn a_relative_directory_is_refused_in_both_forms() {
        let relative = |field| CallerError::NotAbsolute { field };
        assert_eq!(
            ServerCaller::new("proj", "/w", "h", SESSION, &json!(1)).unwrap_err(),
            relative("project_directory")
        );
        assert_eq!(
            ServerCaller::new("/p", "w", "h", SESSION, &json!(1)).unwrap_err(),
            relative("working_directory")
        );
        assert_eq!(
            HookCaller::new("h", "./w", "t").unwrap_err(),
            relative("working_directory")
        );
        assert_eq!(
            hook().with_project_directory("proj").unwrap_err(),
            relative("project_directory")
        );
    }

    #[test]
    fn a_session_that_is_not_a_lower_case_uuid_v4_is_refused() {
        let build = |session: &str| ServerCaller::new("/p", "/w", "h", session, &json!(1));
        let upper = SESSION.to_uppercase();
        assert_eq!(build(&upper).unwrap_err(), CallerError::NotSessionId);
        let version_one = "0b7e4a52-3c1d-1f6a-8e9b-1a2b3c4d5e6f";
        assert_eq!(build(version_one).unwrap_err(), CallerError::NotSessionId);
        let variant_c = "0b7e4a52-3c1d-4f6a-ce9b-1a2b3c4d5e6f";
        assert_eq!(build(variant_c).unwrap_err(), CallerError::NotSessionId);
        let short = &SESSION[..35];
        assert_eq!(build(short).unwrap_err(), CallerError::NotSessionId);
        let no_dashes = SESSION.replace('-', "x");
        assert_eq!(build(&no_dashes).unwrap_err(), CallerError::NotSessionId);
        assert!(build(SESSION).is_ok());
    }

    #[test]
    fn a_host_that_is_not_name_text_is_refused() {
        let build = |host: &str| HookCaller::new(host, "/w", "t");
        assert!(build("claude-code").is_ok());
        assert!(build("codex2").is_ok());
        assert_eq!(build("").unwrap_err(), CallerError::Empty { field: "host" });
        for bad in [
            "Claude-Code",
            "claude code",
            "-claude",
            "1claude",
            "claude_code",
            "claudé",
        ] {
            assert_eq!(build(bad).unwrap_err(), CallerError::NotHostName, "{bad}");
        }
    }

    #[test]
    fn instruction_entries_stop_at_thirty_two() {
        let many = |n: usize| vec![entry(); n];
        assert!(hook().with_instructions(many(32)).is_ok());
        assert_eq!(
            hook().with_instructions(many(33)).unwrap_err(),
            CallerError::TooManyInstructions { count: 33 }
        );
        assert!(server().with_instructions(many(32)).is_ok());
        assert_eq!(
            server().with_instructions(many(33)).unwrap_err(),
            CallerError::TooManyInstructions { count: 33 }
        );
    }

    #[test]
    fn json_rpc_integer_one_and_string_one_have_different_texts() {
        let text = |id: Value| {
            ServerCaller::new("/p", "/w", "h", SESSION, &id)
                .map(|caller| caller.call().text().to_owned())
        };
        assert_eq!(text(json!(1)), Ok("1".to_owned()));
        assert_eq!(text(json!("1")), Ok("\"1\"".to_owned()));
        assert_eq!(text(json!(-12)), Ok("-12".to_owned()));
        assert_eq!(text(json!("a\"b")), Ok("\"a\\\"b\"".to_owned()));
        assert_eq!(text(json!(0)), Ok("0".to_owned()));
    }

    #[test]
    fn json_rpc_ids_that_are_not_a_string_or_integer_are_refused() {
        let refused = |id: Value| ServerCaller::new("/p", "/w", "h", SESSION, &id).unwrap_err();
        assert_eq!(refused(Value::Null), CallerError::NotJsonRpcId);
        assert_eq!(refused(json!(true)), CallerError::NotJsonRpcId);
        assert_eq!(refused(json!(1.5)), CallerError::NotJsonRpcId);
        assert_eq!(refused(json!([1])), CallerError::NotJsonRpcId);
        assert_eq!(refused(json!({})), CallerError::NotJsonRpcId);
        let big: Value = serde_json::from_str("9223372036854775808").unwrap();
        assert_eq!(refused(big), CallerError::NotJsonRpcId);
        let exponent: Value = serde_json::from_str("1e2").unwrap();
        assert_eq!(refused(exponent), CallerError::NotJsonRpcId);
        let fraction: Value = serde_json::from_str("1.0").unwrap();
        assert_eq!(refused(fraction), CallerError::NotJsonRpcId);
    }

    fn round_trip(caller: Caller) {
        let value = caller.to_value();
        let back = Caller::from_value(&value).unwrap();
        assert_eq!(back, caller);
        assert_eq!(back.to_value(), value);
    }

    #[test]
    fn full_and_minimal_values_of_both_forms_round_trip() {
        round_trip(Caller::Server(full_server()));
        round_trip(Caller::Server(server()));
        round_trip(Caller::Hook(full_hook()));
        round_trip(Caller::Hook(hook()));
    }

    #[test]
    fn json_rpc_string_id_and_integer_id_round_trip_to_different_callers() {
        let with = |id: Value| ServerCaller::new("/p", "/w", "h", SESSION, &id).unwrap();
        let string = Caller::Server(with(json!("1")));
        let integer = Caller::Server(with(json!(1)));
        round_trip(string.clone());
        round_trip(integer.clone());
        assert_ne!(string, integer);
        assert_ne!(string.to_value(), integer.to_value());
        round_trip(Caller::Server(with(json!("a\"\\\n\u{1}é"))));
        round_trip(Caller::Server(with(json!(-9_000_000_000_000_000_000_i64))));
    }

    #[test]
    fn canonical_bytes_of_both_forms_match_the_hand_written_rules() {
        let bytes = |caller: Caller| canonical_json(&caller.to_value()).unwrap();
        let server = bytes(Caller::Server(full_server()));
        let expected = concat!(
            r#"{"baley_session":"0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f","#,
            r#""call":{"source":"jsonrpc_id","text":"1"},"#,
            r#""client_version":"2.1.278","form":"server","host":"claude-code","#,
            r#""host_session":"abc123","#,
            r#""instructions":[{"hash":"ab12","identity":"stub:claude","version":"3"}],"#,
            r#""project_directory":"/work/proj","work_order":"wo-7","#,
            r#""working_directory":"/work/proj/src"}"#,
        );
        assert_eq!(String::from_utf8(server).unwrap(), expected);

        let hook = bytes(Caller::Hook(hook()));
        let expected = concat!(
            r#"{"call":{"source":"tool_use_id","text":"toolu_01ABC"},"form":"hook","#,
            r#""host":"claude-code","working_directory":"/work/proj"}"#,
        );
        assert_eq!(String::from_utf8(hook).unwrap(), expected);

        let string_id = ServerCaller::new("/p", "/w", "h", SESSION, &json!("1")).unwrap();
        let text = String::from_utf8(bytes(Caller::Server(string_id))).unwrap();
        assert!(
            text.contains(r#""call":{"source":"jsonrpc_id","text":"\"1\""}"#),
            "{text}"
        );
    }

    fn server_value() -> Value {
        Caller::Server(full_server()).to_value()
    }

    fn hook_value() -> Value {
        Caller::Hook(full_hook()).to_value()
    }

    fn with_key(mut value: Value, path: &[&str], key: &str, to: Value) -> Value {
        let mut at = &mut value;
        for step in path {
            at = if let Ok(index) = step.parse::<usize>() {
                &mut at[index]
            } else {
                &mut at[*step]
            };
        }
        at.as_object_mut().unwrap().insert(key.into(), to);
        value
    }

    #[test]
    fn an_unknown_key_is_refused_at_every_level() {
        for base in [server_value(), hook_value()] {
            let top = with_key(base.clone(), &[], "extra", json!("x"));
            assert_eq!(
                Caller::from_value(&top).unwrap_err(),
                CallerError::UnknownKey {
                    at: "caller",
                    key: "extra".into()
                }
            );
            let call = with_key(base.clone(), &["call"], "extra", json!("x"));
            assert_eq!(
                Caller::from_value(&call).unwrap_err(),
                CallerError::UnknownKey {
                    at: "call",
                    key: "extra".into()
                }
            );
            let entry = with_key(base, &["instructions", "0"], "extra", json!("x"));
            assert_eq!(
                Caller::from_value(&entry).unwrap_err(),
                CallerError::UnknownKey {
                    at: "instruction",
                    key: "extra".into()
                }
            );
        }
    }

    #[test]
    fn a_server_key_on_a_hook_other_than_the_session_is_an_unknown_key() {
        let value = with_key(hook_value(), &[], "client_version", json!("1"));
        assert_eq!(
            Caller::from_value(&value).unwrap_err(),
            CallerError::UnknownKey {
                at: "caller",
                key: "client_version".into()
            }
        );
    }

    #[test]
    fn null_for_a_present_field_is_refused_not_read_as_absent() {
        for field in ["client_version", "host_session", "work_order"] {
            let value = with_key(server_value(), &[], field, Value::Null);
            assert_eq!(
                Caller::from_value(&value).unwrap_err(),
                CallerError::NotText { field }
            );
        }
        let hook = with_key(hook_value(), &[], "project_directory", Value::Null);
        assert_eq!(
            Caller::from_value(&hook).unwrap_err(),
            CallerError::NotText {
                field: "project_directory"
            }
        );
        let call = with_key(server_value(), &["call"], "text", Value::Null);
        assert_eq!(
            Caller::from_value(&call).unwrap_err(),
            CallerError::NotText { field: "text" }
        );
        let number = with_key(server_value(), &[], "host", json!(5));
        assert_eq!(
            Caller::from_value(&number).unwrap_err(),
            CallerError::NotText { field: "host" }
        );
    }

    #[test]
    fn an_empty_host_session_is_refused_when_read_not_read_as_absent() {
        for base in [server_value(), hook_value()] {
            let value = with_key(base, &[], "host_session", json!(""));
            assert_eq!(
                Caller::from_value(&value).unwrap_err(),
                CallerError::Empty {
                    field: "host_session"
                }
            );
        }
    }

    #[test]
    fn a_hook_object_holding_the_baley_session_gets_its_own_error() {
        let value = with_key(hook_value(), &[], "baley_session", json!(SESSION));
        assert_eq!(
            Caller::from_value(&value).unwrap_err(),
            CallerError::HookHoldsSession
        );
    }

    #[test]
    fn an_empty_instruction_list_is_refused_when_read() {
        for base in [server_value(), hook_value()] {
            let value = with_key(base.clone(), &[], "instructions", json!([]));
            assert_eq!(
                Caller::from_value(&value).unwrap_err(),
                CallerError::EmptyInstructionList
            );
            let not_list = with_key(base, &[], "instructions", json!({}));
            assert_eq!(
                Caller::from_value(&not_list).unwrap_err(),
                CallerError::NotAList
            );
        }
    }

    #[test]
    fn more_than_thirty_two_instruction_entries_are_refused_when_read() {
        let one = json!({"identity": "i", "version": "1", "hash": "h"});
        let at = |n: usize| {
            with_key(
                hook_value(),
                &[],
                "instructions",
                json!(vec![one.clone(); n]),
            )
        };
        assert!(Caller::from_value(&at(32)).is_ok());
        assert_eq!(
            Caller::from_value(&at(33)).unwrap_err(),
            CallerError::TooManyInstructions { count: 33 }
        );
    }

    #[test]
    fn a_call_source_of_the_other_form_is_refused() {
        let server = with_key(server_value(), &["call"], "source", json!("tool_use_id"));
        assert_eq!(
            Caller::from_value(&server).unwrap_err(),
            CallerError::SourceMismatch
        );
        let hook = with_key(hook_value(), &["call"], "source", json!("jsonrpc_id"));
        assert_eq!(
            Caller::from_value(&hook).unwrap_err(),
            CallerError::SourceMismatch
        );
        let unknown = with_key(hook_value(), &["call"], "source", json!("cli"));
        assert_eq!(
            Caller::from_value(&unknown).unwrap_err(),
            CallerError::UnknownSource {
                found: "cli".into()
            }
        );
    }

    #[test]
    fn a_json_rpc_call_text_that_is_not_canonical_is_refused() {
        for text in [
            "01",
            "1.0",
            "\"\\u0031\"",
            "{}",
            "+1",
            "-0",
            "\"1",
            "\"1\" ",
            " 1",
            "1e2",
            "null",
            "9223372036854775808",
        ] {
            let value = with_key(server_value(), &["call"], "text", json!(text));
            assert_eq!(
                Caller::from_value(&value).unwrap_err(),
                CallerError::NotCanonicalJsonRpcId,
                "{text}"
            );
        }
        for text in ["1", "-1", "\"1\"", "\"\"", "9223372036854775807"] {
            let value = with_key(server_value(), &["call"], "text", json!(text));
            assert!(Caller::from_value(&value).is_ok(), "{text}");
        }
    }

    #[test]
    fn a_tool_use_id_that_looks_like_a_json_rpc_id_is_kept_as_written() {
        let value = with_key(hook_value(), &["call"], "text", json!("01"));
        let Ok(Caller::Hook(caller)) = Caller::from_value(&value) else {
            panic!("a tool use id is free text");
        };
        assert_eq!(caller.call().text(), "01");
    }

    #[test]
    fn decoding_refuses_what_construction_refuses() {
        let relative = with_key(server_value(), &[], "working_directory", json!("w"));
        assert_eq!(
            Caller::from_value(&relative).unwrap_err(),
            CallerError::NotAbsolute {
                field: "working_directory"
            }
        );
        let session = with_key(
            server_value(),
            &[],
            "baley_session",
            json!(SESSION.to_uppercase()),
        );
        assert_eq!(
            Caller::from_value(&session).unwrap_err(),
            CallerError::NotSessionId
        );
        let host = with_key(hook_value(), &[], "host", json!("Claude"));
        assert_eq!(
            Caller::from_value(&host).unwrap_err(),
            CallerError::NotHostName
        );
        let long = with_key(hook_value(), &[], "work_order", json!("w".repeat(257)));
        assert_eq!(
            Caller::from_value(&long).unwrap_err(),
            CallerError::TooLong {
                field: "work_order",
                limit: 256,
                bytes: 257
            }
        );
    }

    #[test]
    fn decoding_refuses_a_missing_required_key_and_a_bad_form() {
        for key in [
            "project_directory",
            "baley_session",
            "host",
            "working_directory",
            "call",
        ] {
            let mut value = server_value();
            value.as_object_mut().unwrap().remove(key);
            assert!(
                matches!(
                    Caller::from_value(&value),
                    Err(CallerError::MissingField { .. })
                ),
                "{key}"
            );
        }
        let mut no_project = hook_value();
        no_project
            .as_object_mut()
            .unwrap()
            .remove("project_directory");
        assert!(Caller::from_value(&no_project).is_ok());
        for form in [json!("cli"), json!(null), json!(1)] {
            let value = with_key(hook_value(), &[], "form", form);
            assert_eq!(
                Caller::from_value(&value).unwrap_err(),
                CallerError::BadForm
            );
        }
        assert_eq!(
            Caller::from_value(&json!({"host": "h"})).unwrap_err(),
            CallerError::BadForm
        );
        assert_eq!(
            Caller::from_value(&json!([])).unwrap_err(),
            CallerError::NotAnObject { at: "caller" }
        );
        let call = with_key(hook_value(), &[], "call", json!("x"));
        assert_eq!(
            Caller::from_value(&call).unwrap_err(),
            CallerError::NotAnObject { at: "call" }
        );
    }
}
