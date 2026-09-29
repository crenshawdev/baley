//! The merge of CFG-R6: built-in defaults, then each file's top level and its
//! section for the connected host, later layers winning per setting.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::parse::{
    Diagnostic, FileLayer, ParsedLayer, SettingsFile, Unavailable, Value, parse_layer,
};
use super::schema::{self, Host, Role, Rung, Schema};

/// Where a setting's value came from (design 0003 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// The schema's built-in default.
    Default,
    /// The global file's top level.
    Global,
    /// The global file's section for the connected host.
    GlobalHost,
    /// The project file's top level.
    Project,
    /// The project file's section for the connected host.
    ProjectHost,
}
impl Layer {
    /// The name `policy.effective`'s `sources` use.
    pub fn name(self) -> &'static str {
        match self {
            Layer::Default => "default",
            Layer::Global => "global",
            Layer::GlobalHost => "global-host",
            Layer::Project => "project",
            Layer::ProjectHost => "project-host",
        }
    }
}

/// A settings file as the policy cites it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// Where the file was read from.
    pub path: PathBuf,
    /// The lower-case hex SHA-256 of its bytes.
    pub digest: String,
}

/// The layer, file and position a setting's value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The layer.
    pub layer: Layer,
    /// The file, `None` for a default.
    pub file: Option<FileRef>,
    /// The one-based line it was written on, 0 for a default.
    pub line: u32,
    /// The one-based column it was written at, 0 for a default.
    pub column: u32,
}

/// One setting's value in effect and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The value, `None` only for an absent default.
    pub value: Option<Value>,
    /// Where it came from.
    pub source: Source,
}

/// The merged settings for one project and one host or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePolicy {
    /// The connected host whose sections applied, `None` for the command line.
    pub host: Option<Host>,
    /// One entry per schema entry, in name order.
    pub settings: BTreeMap<String, Effective>,
    /// The global file's diagnostics, then the project file's, each in file order.
    pub diagnostics: Vec<Diagnostic>,
    /// The global file, when present.
    pub global: Option<FileRef>,
    /// The project file, when present.
    pub project: Option<FileRef>,
}
impl EffectivePolicy {
    fn setting(&self, name: &str) -> &Effective {
        self.settings
            .get(name)
            .unwrap_or_else(|| panic!("the policy was merged over a schema without {name}"))
    }

    /// The role's model, `None` for the host session's own, and its source.
    ///
    /// # Panics
    ///
    /// When the policy was merged over a schema without `roles.<role>.model`.
    pub fn model(&self, role: Role) -> (Option<&str>, &Source) {
        let effective = self.setting(&format!("roles.{}.model", role.name()));
        let model = match &effective.value {
            Some(Value::ModelName(name)) => Some(name.as_str()),
            _ => None,
        };
        (model, &effective.source)
    }

    /// The role's starting rung and its source.
    ///
    /// # Panics
    ///
    /// When the policy was merged over a schema without a rung for
    /// `roles.<role>.effort`.
    pub fn effort(&self, role: Role) -> (Rung, &Source) {
        let name = format!("roles.{}.effort", role.name());
        match self.setting(&name) {
            Effective {
                value: Some(Value::Rung(rung)),
                source,
            } => (*rung, source),
            _ => panic!("{name} holds no rung"),
        }
    }

    /// Whether a retry runs one rung up, and its source.
    ///
    /// # Panics
    ///
    /// When the policy was merged over a schema without a boolean for
    /// `escalate_on_failure`.
    pub fn escalate_on_failure(&self) -> (bool, &Source) {
        match self.setting("escalate_on_failure") {
            Effective {
                value: Some(Value::Bool(escalate)),
                source,
            } => (*escalate, source),
            _ => panic!("escalate_on_failure holds no boolean"),
        }
    }
}

/// Merges the parsed files over the schema's defaults for `host`.
///
/// Values from a section for any other host apply to nothing.
pub fn merge(
    schema: &Schema,
    host: Option<Host>,
    global: Option<&ParsedLayer>,
    project: Option<&ParsedLayer>,
) -> EffectivePolicy {
    let mut settings: BTreeMap<String, Effective> = schema
        .entries()
        .iter()
        .map(|entry| {
            let value = match entry.default {
                schema::Default::Absent => None,
                schema::Default::Bool(value) => Some(Value::Bool(value)),
                schema::Default::Rung(rung) => Some(Value::Rung(rung)),
            };
            let source = Source {
                layer: Layer::Default,
                file: None,
                line: 0,
                column: 0,
            };
            (entry.name.clone(), Effective { value, source })
        })
        .collect();
    let files = [
        (global, Layer::Global, Layer::GlobalHost),
        (project, Layer::Project, Layer::ProjectHost),
    ];
    for (parsed, top, section) in files {
        let Some(parsed) = parsed else { continue };
        let mut apply = |layer: Layer, from: Option<Host>| {
            for written in parsed.values.iter().filter(|w| w.host == from) {
                if let Some(effective) = settings.get_mut(&written.name) {
                    *effective = Effective {
                        value: Some(written.value.clone()),
                        source: Source {
                            layer,
                            file: Some(parsed.file.clone()),
                            line: written.line,
                            column: written.column,
                        },
                    };
                }
            }
        };
        apply(top, None);
        if host.is_some() {
            apply(section, host);
        }
    }
    EffectivePolicy {
        host,
        settings,
        diagnostics: [global, project]
            .into_iter()
            .flatten()
            .flat_map(|parsed| parsed.diagnostics.iter().cloned())
            .collect(),
        global: global.map(|parsed| parsed.file.clone()),
        project: project.map(|parsed| parsed.file.clone()),
    }
}

/// Parses the global file, then the project file, and merges them for
/// `host`; the first file that gives no policy refuses.
pub fn effective_policy(
    schema: &Schema,
    host: Option<Host>,
    global: Option<&SettingsFile>,
    project: Option<&SettingsFile>,
) -> Result<EffectivePolicy, Unavailable> {
    let global = global
        .map(|file| parse_layer(file, FileLayer::Global, schema))
        .transpose()?;
    let project = project
        .map(|file| parse_layer(file, FileLayer::Project, schema))
        .transpose()?;
    Ok(merge(schema, host, global.as_ref(), project.as_ref()))
}
