//! The task record type and the `task-open` / `task-close` request shapes.
//!
//! The record is defined here for both homes: a treeless run composes it in
//! memory and calls it unrecorded; a rooted run (plan 2) persists it to the
//! store and renders it. Nothing in this module touches the store.
use crate::{
    envelope::Refusal,
    rail::risk_diff::Scan,
    store::{Error, Result},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const RECORD_SCHEMA: &str = "task-1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Inline,
    Planned,
}

/// Whether a planning root exists, decided by the task boundary from the
/// root path itself and never inferred from an unrelated missing file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Root {
    Absent { path: String },
    Present { path: String },
}

impl Root {
    pub fn classify(root: &Path) -> Result<Self> {
        let path = root.to_string_lossy().into_owned();
        match std::fs::symlink_metadata(root) {
            Ok(metadata) if metadata.is_dir() => Ok(Self::Present { path }),
            Ok(_) => Err(Error::Invalid(format!("planning root is not a directory: {path}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::Absent { path }),
            Err(error) => Err(Error::Io(format!("planning root {path}: {error}"))),
        }
    }
    pub fn path(&self) -> &str {
        match self {
            Self::Absent { path } | Self::Present { path } => path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    pub id: String,
    pub subject: String,
    pub files: Vec<String>,
}

/// The risk disposition done states. `blocked` is the one that is not done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Risk {
    /// HEAD did not move: nothing landed, so nothing was scanned.
    Skipped,
    /// The range was scanned against the answered surfaces and held nothing.
    Clear { surfaces: Vec<String>, gate: String, scan: Scan },
    /// The range matched (or was inconclusive) under a gate that blocks done.
    Blocked { surfaces: Vec<String>, gate: String, scan: Scan, matched: Vec<String> },
    /// The range matched under a gate that states the match without blocking.
    Advisory { surfaces: Vec<String>, gate: String, scan: Scan, matched: Vec<String> },
}

/// Whether the record reached durable storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Recording {
    Unrecorded { reason: String },
    Recorded { path: String, revision: String },
}

/// The task record. Under a planning root plan 2 persists and renders it;
/// without one it is composed for the answer and never written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    pub slug: String,
    pub mode: Mode,
    pub description: String,
    pub token: String,
    pub root: Root,
    pub branch: String,
    pub start: String,
    pub head: String,
    pub commits: Vec<Commit>,
    pub files: Vec<String>,
    pub risk: Risk,
    pub report: String,
    pub recording: Recording,
}

/// Open a task: explicit identity, inline or planned; never phase 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    pub request_id: String,
    pub slug: String,
    pub mode: Mode,
    pub description: String,
}

/// What shipped, as text or as a file the caller wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Report {
    Text { text: String },
    File { path: String },
}

/// Close a task: the run token from open, the report, and the per-run risk
/// surface answer a treeless run has nowhere to persist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Close {
    pub request_id: String,
    pub slug: String,
    pub token: String,
    pub report: Report,
    pub surfaces: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum Apply {
    #[serde(rename = "task-open")]
    Open { request: Open },
    #[serde(rename = "task-close")]
    Close { request: Close },
}

impl Apply {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Open { request } => &request.request_id,
            Self::Close { request } => &request.request_id,
        }
    }
    pub fn slug(&self) -> &str {
        match self {
            Self::Open { request } => &request.slug,
            Self::Close { request } => &request.slug,
        }
    }
}

pub fn validate_slug(slug: &str) -> Result<()> {
    crate::debug::model::validate_slug(slug)
}

pub fn validate_description(description: &str) -> Result<()> {
    if description.trim().is_empty() || description.len() > 4096 {
        return Err(Error::Invalid("task description must be nonblank and at most 4096 bytes".into()));
    }
    Ok(())
}

pub fn validate_report(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.len() > 65536 {
        return Err(Error::Invalid("task report must be nonblank and at most 65536 bytes".into()));
    }
    Ok(())
}

/// The reason a treeless record is unrecorded, in words.
pub fn unrecorded(root: &Root) -> Recording {
    Recording::Unrecorded { reason: format!("no planning root at {}: git is the record", root.path()) }
}

/// Compose the disposition from a scan and the configured gate. The rule for
/// what fires is the receipts rail's; only the gate's consequence is decided here.
pub fn disposition(scan: Scan, surfaces: Vec<String>, gate: &str) -> Risk {
    if !crate::rail::receipts::scan_requires_review(&scan) {
        return Risk::Clear { surfaces, gate: gate.into(), scan };
    }
    let matched = scan.matches.iter().map(|m| m.category.clone()).collect();
    match gate {
        "blocking" | "adjudicated" => Risk::Blocked { surfaces, gate: gate.into(), scan, matched },
        _ => Risk::Advisory { surfaces, gate: gate.into(), scan, matched },
    }
}

pub fn done(record: &Record) -> Value {
    json!({"status":"ok","outcome":"done","ephemeral":matches!(record.root, Root::Absent { .. }),"record":record})
}

pub fn blocked(record: &Record, transient: Value) -> Value {
    let Risk::Blocked { matched, scan, gate, .. } = &record.risk else {
        unreachable!("blocked answer needs a blocked disposition")
    };
    let signals = scan.matches.iter().map(|m| format!("{}: {}", m.category, m.signal)).collect::<Vec<_>>();
    let cause = if signals.is_empty() { "inconclusive scan".to_owned() } else { signals.join("; ") };
    Refusal::new("risk-blocked", format!("risk surface {} matched in {}..{} ({cause}); the {gate} gate refuses done",
            matched.join(", "), record.start, record.head))
        .rule("risk-gate").slot("request")
        .details(json!({"record":record,"transient":transient}))
        .value()
}

pub fn missing_file(path: &str, root: &Root) -> Value {
    Refusal::new("missing-file", format!("report file does not exist: {path}"))
        .rule("task-boundary").slot("request.report.path")
        .details(json!({"path":path,"root":root}))
        .value()
}

pub fn unknown_task(slug: &str, token: &str) -> Value {
    Refusal::new("unknown-task", format!("no open task named {slug} with token {token} in this resident"))
        .slot("request.token").details(json!({"slug":slug,"token":token})).value()
}

pub fn invalid(reason: impl Into<String>, slug: &str) -> Value {
    Refusal::new("task-invalid", reason).slot("request").details(json!({"slug":slug})).value()
}

pub fn reused() -> Value {
    Refusal::new("request-reused", "task request identity binds different inputs").slot("request.request_id").value()
}
