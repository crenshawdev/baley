//! The `[project]` table of the project file: the project's id and name
//! (design 0003, CFG-R3, ADR 0004).

use toml::Spanned;
use toml::de::DeValue;

use super::parse::{Fault, SettingsFile, Unavailable, document, line_and_column};

/// The project as its project file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    /// The project id, a lower-case UUID version 4.
    pub id: String,
    /// The project's name, any string, empty included.
    pub name: String,
}

/// What is wrong with a key of the `[project]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectProblem {
    /// The file does not hold it.
    Missing,
    /// It holds a value of another type: a table for `project`, a string
    /// for `id` and `name`.
    WrongType {
        /// What the file holds, as `toml` names its type.
        found: &'static str,
    },
    /// The id is a string but not a lower-case UUID version 4.
    NotAnId {
        /// The string the file holds.
        written: String,
    },
}

/// Whether `id` is a project id: a UUID version 4 written as 36 bytes of
/// lower-case hex in 8-4-4-4-12 groups, with version nibble `4` and variant
/// nibble `8`, `9`, `a` or `b`. Nothing else is accepted, so every clone
/// reads the one id byte for byte.
pub fn is_project_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

/// Reads the project's id and name from the project file's `[project]`
/// table.
///
/// Bytes that are not UTF-8 or text that is not TOML give the same fault
/// `parse_layer` gives. A missing table, a missing or non-string `id` or
/// `name`, or an id that is not a project id make the file unavailable,
/// naming the id before the name. Every other key and table is left to
/// `parse_layer`.
pub fn read_project(file: &SettingsFile) -> Result<ProjectIdentity, Unavailable> {
    let (text, table) = document(file)?;
    let refuse = |name, problem, position| Unavailable {
        path: file.path.clone(),
        fault: Fault::Project {
            name,
            problem,
            position,
        },
    };
    let at = |item: &Spanned<DeValue<'_>>| Some(line_and_column(text, item.span().start));
    let Some(project) = table.get_ref().get("project") else {
        return Err(refuse("project", ProjectProblem::Missing, None));
    };
    let DeValue::Table(project) = project.get_ref() else {
        let found = project.get_ref().type_str();
        return Err(refuse(
            "project",
            ProjectProblem::WrongType { found },
            at(project),
        ));
    };
    let string = |key, name| match project.get(key) {
        None => Err(refuse(name, ProjectProblem::Missing, None)),
        Some(item) => match item.get_ref().as_str() {
            Some(value) => Ok((value, item)),
            None => {
                let found = item.get_ref().type_str();
                Err(refuse(name, ProjectProblem::WrongType { found }, at(item)))
            }
        },
    };
    let (id, item) = string("id", "project.id")?;
    if !is_project_id(id) {
        let written = id.to_owned();
        let problem = ProjectProblem::NotAnId { written };
        return Err(refuse("project.id", problem, at(item)));
    }
    let (name, _) = string("name", "project.name")?;
    Ok(ProjectIdentity {
        id: id.to_owned(),
        name: name.to_owned(),
    })
}
