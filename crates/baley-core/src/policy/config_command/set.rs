//! The judges of `config set`'s arguments (design 0003 section 5, CFG-R4,
//! CFG-R5, CFG-R7, CFG-R9).

use std::fmt;

use super::{INVALID_VALUE, NOT_A_PROJECT, UNKNOWN_SETTING, WRONG_LAYER};
use crate::policy::{FileLayer, Host, Kind, Rung, Schema, Scope, Value};

/// One pair of a set after its name and value are judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedPair {
    /// The schema name, such as `roles.planner.effort`.
    pub name: String,
    /// The host section it is written in, `None` for the file's top level.
    pub host: Option<Host>,
    /// The value, typed by its setting's kind.
    pub value: Value,
}

/// Why a set is refused before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetRefusal {
    /// `--project` outside a project.
    NotAProject,
    /// The schema has no setting of this name.
    UnknownSetting {
        /// The name as given.
        name: String,
    },
    /// The setting's scope excludes the file asked for.
    WrongLayer {
        /// The name as given.
        name: String,
        /// The setting's scope.
        scope: Scope,
        /// The file asked for.
        layer: FileLayer,
    },
    /// The value is not one its setting's kind takes.
    InvalidValue {
        /// The name as given.
        name: String,
        /// The value as given.
        value: String,
        /// The setting's kind.
        kind: Kind,
    },
}
impl SetRefusal {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            SetRefusal::NotAProject => NOT_A_PROJECT,
            SetRefusal::UnknownSetting { .. } => UNKNOWN_SETTING,
            SetRefusal::WrongLayer { .. } => WRONG_LAYER,
            SetRefusal::InvalidValue { .. } => INVALID_VALUE,
        }
    }
}
impl fmt::Display for SetRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            SetRefusal::NotAProject => f.write_str(
                "a project-file set needs a baley.toml at or below the repository root; \
                 baley init creates one",
            ),
            SetRefusal::UnknownSetting { name } => {
                write!(f, "{} is not a setting Baley reads", name.escape_debug())
            }
            SetRefusal::WrongLayer { name, scope, layer } => {
                let (scope, file, flag) = match (scope, layer) {
                    (Scope::Project, _) => ("project", "global", "--project"),
                    (Scope::Global, _) => ("global", "project", "--global"),
                    (Scope::Both, FileLayer::Global) => ("both", "global", "--global"),
                    (Scope::Both, FileLayer::Project) => ("both", "project", "--project"),
                };
                write!(
                    f,
                    "{} is a {scope} setting and cannot be set in the {file} file; use {flag}",
                    name.escape_debug()
                )
            }
            SetRefusal::InvalidValue { name, value, kind } => {
                let expected = match kind {
                    Kind::Bool => "a boolean (true or false)".to_owned(),
                    Kind::Rung => {
                        let names: Vec<&str> = Rung::ALL.into_iter().map(Rung::name).collect();
                        format!("a rung ({})", names.join(", "))
                    }
                    Kind::ModelName => "a model name (any non-empty text)".to_owned(),
                };
                write!(
                    f,
                    "{} is \"{}\", which is not {expected}",
                    name.escape_debug(),
                    value.escape_debug()
                )
            }
        }
    }
}

/// Whether `scope` keeps a setting out of the file `layer` names.
fn excludes(scope: Scope, layer: FileLayer) -> bool {
    matches!(
        (scope, layer),
        (Scope::Project, FileLayer::Global) | (Scope::Global, FileLayer::Project)
    )
}

/// Converts a command-line value by its setting's kind. The text is never
/// parsed as TOML, so the type written is the type the read expects (D-11).
fn convert(kind: Kind, text: &str) -> Option<Value> {
    match kind {
        Kind::Bool => match text {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        Kind::Rung => Rung::parse(text).map(Value::Rung),
        Kind::ModelName => (!text.is_empty()).then(|| Value::ModelName(text.to_owned())),
    }
}

/// Judges every pair of a set against `schema` before anything is gathered
/// or written, giving the typed pairs in the order given.
///
/// Each check runs over every pair before the next begins, so the refusal is
/// the earliest check any pair fails, naming the first pair that fails it:
/// `not-a-project`, `unknown-setting`, `wrong-layer`, then `invalid-value`.
/// A host comes only from `host`, so a name written with a `host.<name>.`
/// prefix is unknown. A setting given twice comes back twice, so each value
/// given can still be checked against the catalog.
pub fn judge_pairs(
    schema: &Schema,
    layer: FileLayer,
    in_project: bool,
    host: Option<Host>,
    pairs: &[(&str, &str)],
) -> Result<Vec<TypedPair>, SetRefusal> {
    if layer == FileLayer::Project && !in_project {
        return Err(SetRefusal::NotAProject);
    }
    let mut entries = Vec::with_capacity(pairs.len());
    for (name, _) in pairs {
        let Some(entry) = schema.get(name) else {
            let name = (*name).to_owned();
            return Err(SetRefusal::UnknownSetting { name });
        };
        entries.push(entry);
    }
    for ((name, _), entry) in pairs.iter().zip(&entries) {
        if excludes(entry.scope, layer) {
            return Err(SetRefusal::WrongLayer {
                name: (*name).to_owned(),
                scope: entry.scope,
                layer,
            });
        }
    }
    let mut typed = Vec::with_capacity(pairs.len());
    for ((name, text), entry) in pairs.iter().zip(&entries) {
        let Some(value) = convert(entry.kind, text) else {
            return Err(SetRefusal::InvalidValue {
                name: (*name).to_owned(),
                value: (*text).to_owned(),
                kind: entry.kind,
            });
        };
        typed.push(TypedPair {
            name: entry.name.clone(),
            host,
            value,
        });
    }
    Ok(typed)
}
