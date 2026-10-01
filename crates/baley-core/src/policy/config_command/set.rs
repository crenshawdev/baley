//! The judges of `config set`'s arguments (design 0003 section 5, CFG-R4,
//! CFG-R5, CFG-R7, CFG-R9).

use std::collections::BTreeMap;
use std::fmt;

use super::{INVALID_VALUE, NOT_A_PROJECT, UNKNOWN_SETTING, WRONG_LAYER};
use crate::policy::{
    AcceptedNames, FileLayer, Host, Kind, Rung, Schema, Scope, UNKNOWN_MODEL, Value,
};

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
    /// No catalog checked accepts the model name (CFG-R14).
    UnknownModel {
        /// The setting that holds it.
        setting: String,
        /// The model name as given.
        model: String,
        /// The host whose catalog was asked, `None` when any host's would do.
        host: Option<Host>,
        /// Each host checked and the names it accepts, sorted.
        checked: Vec<(Host, Vec<String>)>,
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
            SetRefusal::UnknownModel { .. } => UNKNOWN_MODEL,
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
            SetRefusal::UnknownModel {
                setting,
                model,
                host,
                checked,
            } => {
                let who = match host {
                    Some(host) => format!("the {} catalog does not accept", host.name()),
                    None => "no host's catalog accepts".to_owned(),
                };
                write!(
                    f,
                    "{} is \"{}\", which {who}",
                    setting.escape_debug(),
                    model.escape_debug()
                )?;
                for (host, names) in checked {
                    let names = if names.is_empty() {
                        "none".to_owned()
                    } else {
                        names.join(", ")
                    };
                    write!(f, "; {} accepts: {names}", host.name())?;
                }
                Ok(())
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

/// Whether any pair sets a model name, which is the only case that needs the
/// hosts' catalogs: the binary opens and seeds the store only then.
pub fn needs_catalog(pairs: &[TypedPair]) -> bool {
    pairs
        .iter()
        .any(|pair| matches!(pair.value, Value::ModelName(_)))
}

/// Checks every model name given against the hosts' accepted names (CFG-R14).
///
/// With a host, that host's names must hold the model. With none, at least
/// one host's names must, and a provider's catalog never counts. A host
/// missing from `accepted` accepts nothing. The first pair to fail is
/// refused, so a setting given twice has each of its values checked: collapse
/// repeats only after this.
pub fn judge_models(
    pairs: &[TypedPair],
    host: Option<Host>,
    accepted: &BTreeMap<Host, AcceptedNames>,
) -> Result<(), SetRefusal> {
    let checked: Vec<Host> = match host {
        Some(host) => vec![host],
        None => Host::ALL.to_vec(),
    };
    let holds = |host: &Host, model: &str| {
        accepted
            .get(host)
            .is_some_and(|catalog| catalog.names.contains(model))
    };
    for pair in pairs {
        let Value::ModelName(model) = &pair.value else {
            continue;
        };
        if checked.iter().any(|host| holds(host, model)) {
            continue;
        }
        return Err(SetRefusal::UnknownModel {
            setting: pair.name.clone(),
            model: model.clone(),
            host,
            checked: checked
                .iter()
                .map(|host| {
                    let names = accepted
                        .get(host)
                        .map(|catalog| catalog.names.iter().cloned().collect())
                        .unwrap_or_default();
                    (*host, names)
                })
                .collect(),
        });
    }
    Ok(())
}

/// Holds a setting given twice once, at its first position, with its last
/// value, so the renderer and the no-op judge never see a repeat.
pub fn collapse_repeats(pairs: Vec<TypedPair>) -> Vec<TypedPair> {
    let mut held: Vec<TypedPair> = Vec::with_capacity(pairs.len());
    for pair in pairs {
        match held
            .iter_mut()
            .find(|earlier| earlier.name == pair.name && earlier.host == pair.host)
        {
            Some(earlier) => earlier.value = pair.value,
            None => held.push(pair),
        }
    }
    held
}
