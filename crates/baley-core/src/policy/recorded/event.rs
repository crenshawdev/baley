//! The `policy.effective` event: its type, its registration, the policy as
//! text and the payload built from it (design 0003 section 6).

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json, json};

use crate::policy::merge::{EffectivePolicy, Source};
use crate::policy::parse::{CONFIG_UNAVAILABLE, Diagnostic, DiagnosticKind, FileLayer, Value};
use crate::policy::schema::{Host, Scope};
use crate::registry::{Registry, RegistryError};

/// The effective policy one checkout and host ran under:
/// `{project, checkout, host, values, sources, diagnostics, catalog_version}`.
/// `host` is `null` for the command line. Built by [`effective_payload`].
pub const POLICY_EFFECTIVE: &str = "policy.effective";
/// The current `policy.effective` payload version.
pub const POLICY_EFFECTIVE_VERSION: u32 = 1;

/// Registers `policy.effective` at version 1, with no upcasters. The event
/// goes on a project's `project` stream.
pub fn register_policy_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(POLICY_EFFECTIVE, POLICY_EFFECTIVE_VERSION, [])
}

/// A checkout or settings file whose path is not UTF-8. The record holds
/// paths as JSON text, so it is refused rather than converted lossily.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathNotUtf8 {
    /// The path at fault.
    pub path: PathBuf,
}

impl PathNotUtf8 {
    /// Always `config-unavailable`.
    pub fn code(&self) -> &'static str {
        CONFIG_UNAVAILABLE
    }
}

impl fmt::Display for PathNotUtf8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} is not UTF-8, and the recorded policy holds its paths as text",
            self.code(),
            self.path.display()
        )
    }
}

impl std::error::Error for PathNotUtf8 {}

/// A checkout and the policy it ran under, with every path as text: what a
/// `policy.effective` holds apart from the project and catalog version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPolicy {
    /// The checkout's root.
    pub checkout: String,
    /// The connected host, `None` for the command line.
    pub host: Option<Host>,
    values: Json,
    sources: Json,
    diagnostics: Json,
}

fn text(path: &Path) -> Result<String, PathNotUtf8> {
    path.to_str().map(str::to_owned).ok_or_else(|| PathNotUtf8 {
        path: path.to_owned(),
    })
}

/// The checkout and `policy` as the record holds them. Refuses the first
/// path that is not UTF-8: the checkout, then the global file, then the
/// project file. Every source and diagnostic cites one of those two files.
pub fn recorded_policy(
    checkout: &Path,
    policy: &EffectivePolicy,
) -> Result<RecordedPolicy, PathNotUtf8> {
    let checkout = text(checkout)?;
    for file in [&policy.global, &policy.project].into_iter().flatten() {
        text(&file.path)?;
    }
    let mut values = Map::new();
    let mut sources = Map::new();
    for (name, effective) in &policy.settings {
        values.insert(name.clone(), value_json(effective.value.as_ref()));
        sources.insert(name.clone(), source_json(&effective.source)?);
    }
    let diagnostics = policy
        .diagnostics
        .iter()
        .map(diagnostic_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RecordedPolicy {
        checkout,
        host: policy.host,
        values: Json::Object(values),
        sources: Json::Object(sources),
        diagnostics: Json::Array(diagnostics),
    })
}

fn value_json(value: Option<&Value>) -> Json {
    match value {
        None => Json::Null,
        Some(Value::Bool(value)) => (*value).into(),
        Some(Value::Rung(rung)) => rung.name().into(),
        Some(Value::ModelName(name)) => name.clone().into(),
        Some(Value::RemoteName(name)) => name.clone().into(),
        Some(Value::OnProtected(on)) => on.name().into(),
        Some(Value::BranchList(names)) => names.clone().into(),
    }
}

fn source_json(source: &Source) -> Result<Json, PathNotUtf8> {
    Ok(match &source.file {
        None => json!({"layer": source.layer.name()}),
        Some(file) => json!({
            "layer": source.layer.name(),
            "path": text(&file.path)?,
            "digest": file.digest,
            "line": source.line,
            "column": source.column,
        }),
    })
}

fn diagnostic_json(diagnostic: &Diagnostic) -> Result<Json, PathNotUtf8> {
    let layer = match diagnostic.layer {
        FileLayer::Global => "global",
        FileLayer::Project => "project",
    };
    let mut entry = json!({
        "layer": layer,
        "path": text(&diagnostic.path)?,
        "name": diagnostic.name,
        "line": diagnostic.line,
        "column": diagnostic.column,
    });
    let kind = match diagnostic.kind {
        DiagnosticKind::UnknownName => "unknown-name",
        DiagnosticKind::UnknownHost => "unknown-host",
        DiagnosticKind::WrongScope { scope } => {
            let scope = match scope {
                Scope::Global => "global",
                Scope::Project => "project",
                Scope::Both => "both",
            };
            entry["scope"] = scope.into();
            "wrong-scope"
        }
    };
    entry["kind"] = kind.into();
    Ok(entry)
}

/// The `policy.effective` payload for `recorded` in `project`, citing
/// `catalog_version`. The whole-file refs are not in it, so a file that
/// supplies no value leaves no trace.
pub fn effective_payload(recorded: &RecordedPolicy, project: &str, catalog_version: u64) -> Json {
    json!({
        "project": project,
        "checkout": recorded.checkout,
        "host": recorded.host.map(Host::name),
        "values": recorded.values,
        "sources": recorded.sources,
        "diagnostics": recorded.diagnostics,
        "catalog_version": catalog_version,
    })
}
