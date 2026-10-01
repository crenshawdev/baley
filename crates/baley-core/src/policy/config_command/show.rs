//! `config show`: what it refuses and the report it prints (design 0003
//! section 5, CFG-R4, CFG-R9).

use std::fmt;
use std::path::Path;

use super::file::value_text;
use super::{NOT_A_PROJECT, UNKNOWN_SETTING};
use crate::policy::schema::Default as Builtin;
use crate::policy::{
    CONFIG_UNAVAILABLE, Entry, FileLayer, Host, Kind, ParsedLayer, Schema, Scope, SettingsFile,
    Unavailable, Value, merge, parse_layer,
};

/// The three layers `show` reports from, each parsed and absent when the file
/// is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowLayers {
    /// The global file.
    pub global: Option<ParsedLayer>,
    /// The working-tree `baley.toml`, which holds what `config set --project`
    /// wrote.
    pub working: Option<ParsedLayer>,
    /// `baley.toml` as HEAD holds it, which the policy is built from.
    pub head: Option<ParsedLayer>,
}

/// Why `show` has nothing to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowRefusal {
    /// A name asked for is not in the schema.
    UnknownSetting {
        /// The name as given.
        name: String,
    },
    /// A project-scoped setting was asked for outside a project.
    NotAProject {
        /// The name as given.
        name: String,
    },
    /// A file is unreadable or invalid.
    Unavailable(Unavailable),
}
impl ShowRefusal {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            ShowRefusal::UnknownSetting { .. } => UNKNOWN_SETTING,
            ShowRefusal::NotAProject { .. } => NOT_A_PROJECT,
            ShowRefusal::Unavailable(_) => CONFIG_UNAVAILABLE,
        }
    }
}
impl fmt::Display for ShowRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShowRefusal::UnknownSetting { name } => write!(
                f,
                "{}: {} is not a setting Baley reads",
                self.code(),
                name.escape_debug()
            ),
            ShowRefusal::NotAProject { name } => write!(
                f,
                "{}: {} is a project setting, and this directory is not in a project; \
                 baley init creates a baley.toml",
                self.code(),
                name.escape_debug()
            ),
            ShowRefusal::Unavailable(unavailable) => unavailable.fmt(f),
        }
    }
}

/// Refuses what `show` cannot show, from supplied reads (D-06).
///
/// Names are judged first since they need no file: `unknown-setting` over
/// every name asked, then `not-a-project` over every project-scoped name
/// asked when the directory is not in a project. With no names asked no
/// setting is asked for, so nothing is refused for being project-scoped.
/// Then the three reads are judged in order (global, working tree, HEAD), and
/// only then the parses in the same order, as the policy step does. Each
/// `config-unavailable` is the reader's `Unavailable`, a parse fault in
/// HEAD's copy labelled as HEAD's, since it carries the working-tree path. The global
/// file is a global layer, the working tree and HEAD's copy project layers.
pub fn judge_show(
    schema: &Schema,
    names: &[&str],
    in_project: bool,
    global: Result<Option<SettingsFile>, Unavailable>,
    working: Result<Option<SettingsFile>, Unavailable>,
    head: Result<Option<SettingsFile>, Unavailable>,
) -> Result<ShowLayers, ShowRefusal> {
    for name in names {
        if schema.get(name).is_none() {
            let name = (*name).to_owned();
            return Err(ShowRefusal::UnknownSetting { name });
        }
    }
    if !in_project {
        for name in names {
            if schema
                .get(name)
                .is_some_and(|entry| entry.scope == Scope::Project)
            {
                let name = (*name).to_owned();
                return Err(ShowRefusal::NotAProject { name });
            }
        }
    }
    let global = global.map_err(ShowRefusal::Unavailable)?;
    let working = working.map_err(ShowRefusal::Unavailable)?;
    let head = head.map_err(ShowRefusal::Unavailable)?;
    let parse = |file: Option<SettingsFile>, layer| {
        file.map(|file| parse_layer(&file, layer, schema))
            .transpose()
            .map_err(ShowRefusal::Unavailable)
    };
    Ok(ShowLayers {
        global: parse(global, FileLayer::Global)?,
        working: parse(working, FileLayer::Project)?,
        head: parse(head, FileLayer::Project).map_err(|refusal| match refusal {
            ShowRefusal::Unavailable(unavailable) => {
                ShowRefusal::Unavailable(unavailable.at_head())
            }
            other => other,
        })?,
    })
}

/// Everything `show`'s report is rendered from.
#[derive(Debug, Clone)]
pub struct ShowRequest<'a> {
    /// The schema the settings are listed from.
    pub schema: &'a Schema,
    /// The host whose sections apply, `None` for the command line.
    pub host: Option<Host>,
    /// The settings asked for, every setting in schema order when empty. Each
    /// has passed [`judge_show`].
    pub names: &'a [&'a str],
    /// The layers [`judge_show`] gave.
    pub layers: &'a ShowLayers,
    /// The pending note's text, when the working tree differs from HEAD.
    pub pending: Option<&'a str>,
    /// The global file's path.
    pub global_path: &'a Path,
    /// The working-tree project file's path, `None` outside a project.
    pub project_path: Option<&'a Path>,
}

fn kind_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Bool => "boolean",
        Kind::Rung => "rung",
        Kind::ModelName => "model name",
    }
}

fn scope_text(scope: Scope) -> &'static str {
    match scope {
        Scope::Global => "global",
        Scope::Project => "project",
        Scope::Both => "both",
    }
}

fn default_text(default: Builtin) -> String {
    match default {
        Builtin::Absent => "absent".to_owned(),
        Builtin::Bool(value) => value_text(&Value::Bool(value)),
        Builtin::Rung(rung) => value_text(&Value::Rung(rung)),
    }
}

/// The value a layer writes for a setting at a place, `None` for the top
/// level and a host for its section.
fn stored<'l>(
    layer: Option<&'l ParsedLayer>,
    name: &str,
    place: Option<Host>,
) -> Option<&'l Value> {
    layer?
        .values
        .iter()
        .find(|written| written.name == name && written.host == place)
        .map(|written| &written.value)
}

/// `show`'s report as lines (design 0003 section 5, D-17, D-18).
///
/// Per setting, in the order asked or in schema order: its kind, default and
/// scope; the global file's stored values, the top level and then each host
/// section shown (every known host with none given, that host's only with
/// one); the working-tree project file's stored values the same way, each
/// with HEAD's value beside it and "applies once committed" when HEAD's
/// differs; and the effective value with the layer it came from. The
/// effective values are merged over HEAD's copy, since that is the policy the
/// ledger records. Then the merged policy's diagnostics, the pending note,
/// the global path, and the project path inside a project.
pub fn render_show(request: &ShowRequest<'_>) -> Vec<String> {
    let ShowRequest {
        schema,
        host,
        names,
        layers,
        pending,
        global_path,
        project_path,
    } = *request;
    let policy = merge(schema, host, layers.global.as_ref(), layers.head.as_ref());
    let mut places = vec![None];
    match host {
        Some(host) => places.push(Some(host)),
        None => places.extend(Host::ALL.into_iter().map(Some)),
    }
    let section = |place: Option<Host>| {
        place.map_or_else(String::new, |host| format!(" [host.{}]", host.name()))
    };
    let shown: Vec<&Entry> = if names.is_empty() {
        schema.entries().iter().collect()
    } else {
        names.iter().filter_map(|name| schema.get(name)).collect()
    };

    let mut lines = Vec::new();
    for entry in shown {
        let name = entry.name.as_str();
        lines.push(format!(
            "{name}: kind {}, default {}, scope {}",
            kind_text(entry.kind),
            default_text(entry.default),
            scope_text(entry.scope)
        ));

        let mut global = Vec::new();
        for place in &places {
            if let Some(value) = stored(layers.global.as_ref(), name, *place) {
                global.push(format!(
                    "  global{}: {}",
                    section(*place),
                    value_text(value)
                ));
            }
        }
        if global.is_empty() {
            global.push("  global: not set".to_owned());
        }
        lines.extend(global);

        let mut project = Vec::new();
        for place in &places {
            let working = stored(layers.working.as_ref(), name, *place);
            let head = stored(layers.head.as_ref(), name, *place);
            if working.is_none() && head.is_none() {
                continue;
            }
            let held = working.map_or_else(|| "not set".to_owned(), value_text);
            let mut line = format!("  project{}: {held}", section(*place));
            if working != head {
                let head = head.map_or_else(|| "no value".to_owned(), value_text);
                line.push_str(&format!(" (HEAD has {head}, applies once committed)"));
            }
            project.push(line);
        }
        if project.is_empty() {
            project.push("  project: not set".to_owned());
        }
        lines.extend(project);

        if let Some(effective) = policy.settings.get(name) {
            let value = effective
                .value
                .as_ref()
                .map_or_else(|| "absent".to_owned(), value_text);
            let mut line = format!(
                "  effective: {value} from {}",
                effective.source.layer.name()
            );
            if let Some(file) = &effective.source.file {
                line.push_str(&format!(" ({})", file.path.display()));
            }
            lines.push(line);
        }
    }
    lines.extend(policy.diagnostics.iter().map(ToString::to_string));
    lines.extend(pending.map(str::to_owned));
    lines.push(format!("global file: {}", global_path.display()));
    if let Some(path) = project_path {
        lines.push(format!("project file: {}", path.display()));
    }
    lines
}
