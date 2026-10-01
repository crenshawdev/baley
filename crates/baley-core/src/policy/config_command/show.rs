//! `config show`: what it refuses and the report it prints (design 0003
//! section 5, CFG-R4, CFG-R9).

use std::fmt;

use super::{NOT_A_PROJECT, UNKNOWN_SETTING};
use crate::policy::{
    CONFIG_UNAVAILABLE, FileLayer, ParsedLayer, Schema, Scope, SettingsFile, Unavailable,
    parse_layer,
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
/// `config-unavailable` is the reader's `Unavailable` unchanged. The global
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
        head: parse(head, FileLayer::Project)?,
    })
}
