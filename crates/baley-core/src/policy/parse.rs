//! One settings file judged against the schema: typed values, diagnostics for
//! what is ignored, and `config-unavailable` for a file that is not valid
//! (design 0003, CFG-R5, CFG-R7, CFG-R9).

use std::fmt;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

use super::merge::FileRef;
use super::project::ProjectProblem;
use super::schema::{Entry, Host, Kind, OnProtected, Rung, Schema, Scope};

/// The code of every refusal that leaves the policy unbuilt.
pub const CONFIG_UNAVAILABLE: &str = "config-unavailable";

/// One settings file as read: where it is, its exact bytes and their digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFile {
    /// The path the file was read from.
    pub path: PathBuf,
    /// The file's exact bytes.
    pub bytes: Vec<u8>,
    /// The lower-case hex SHA-256 of the bytes, computed by the reader.
    pub digest: String,
}

/// Which of the two settings files a layer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileLayer {
    /// The global file, `config.toml` in the config folder.
    Global,
    /// The project file, `baley.toml` at the repository root.
    Project,
}

/// A setting's value, typed by its entry's kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A boolean.
    Bool(bool),
    /// A rung.
    Rung(Rung),
    /// A non-empty model name.
    ModelName(String),
    /// A non-empty git remote name.
    RemoteName(String),
    /// What to do with a commit on a protected branch.
    OnProtected(OnProtected),
}

/// One setting a file writes, typed and within its scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The schema name, without any host prefix.
    pub name: String,
    /// The host section it was written in, `None` for the file's top level.
    pub host: Option<Host>,
    /// The typed value.
    pub value: Value,
    /// The one-based line of its key.
    pub line: u32,
    /// The one-based column of its key.
    pub column: u32,
}

/// One file's settings and diagnostics, ready to merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLayer {
    /// The file's path and digest.
    pub file: FileRef,
    /// Which file this is.
    pub layer: FileLayer,
    /// Every setting written, top level and host sections alike.
    pub values: Vec<Written>,
    /// What was ignored, in file order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Something a file holds that Baley ignored; it refuses nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The file it was found in.
    pub layer: FileLayer,
    /// That file's path.
    pub path: PathBuf,
    /// The path as written, host prefix included.
    pub name: String,
    /// The one-based line of the key.
    pub line: u32,
    /// The one-based column of the key.
    pub column: u32,
    /// Why it was ignored.
    pub kind: DiagnosticKind,
}

/// Why a written name was ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// The schema has no such setting.
    UnknownName,
    /// The setting's scope excludes this file.
    WrongScope {
        /// The setting's scope.
        scope: Scope,
    },
    /// `[host.<name>]` names a host Baley does not know.
    UnknownHost,
}

/// A settings file that exists but gives no policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    /// The file at fault.
    pub path: PathBuf,
    /// What is wrong with it.
    pub fault: Fault,
}

/// Why a settings file gives no policy. Positions are one-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The path is not a regular file.
    NotRegular,
    /// The file could not be opened or read.
    Unreadable {
        /// The operating system's error text.
        cause: String,
    },
    /// The bytes stop being UTF-8 at this position.
    NotUtf8 {
        /// The line.
        line: u32,
        /// The column.
        column: u32,
    },
    /// The text is not TOML.
    Parse {
        /// Where `toml` found the error, when it gave a span.
        position: Option<(u32, u32)>,
        /// `toml`'s own message.
        message: String,
    },
    /// A value's type is not the one its place requires.
    WrongType {
        /// The path as written, host prefix included.
        name: String,
        /// What the place requires.
        expected: Expected,
        /// What the file holds, as `toml` names its type.
        found: &'static str,
        /// The line of the value.
        line: u32,
        /// The column of the value.
        column: u32,
    },
    /// A value of the right type that its grammar refuses.
    OutsideGrammar {
        /// The path as written, host prefix included.
        name: String,
        /// The setting's kind.
        kind: Kind,
        /// The string the file holds.
        written: String,
        /// The line of the value.
        line: u32,
        /// The column of the value.
        column: u32,
    },
    /// The project file's `[project]` table does not give an id and a name.
    Project {
        /// The key as written: `project`, `project.id` or `project.name`.
        name: &'static str,
        /// What is wrong with it.
        problem: ProjectProblem,
        /// The value's line and column, when the file holds one.
        position: Option<(u32, u32)>,
    },
    /// A fault in HEAD's copy of the project file. HEAD's copy carries the
    /// working-tree file's path, and the file on disk may not share the fault.
    AtHead {
        /// The fault as parsing found it.
        fault: Box<Fault>,
    },
}
impl Fault {
    fn position(&self) -> Option<(u32, u32)> {
        match self {
            Fault::NotUtf8 { line, column }
            | Fault::WrongType { line, column, .. }
            | Fault::OutsideGrammar { line, column, .. } => Some((*line, *column)),
            Fault::Parse { position, .. } | Fault::Project { position, .. } => *position,
            Fault::NotRegular | Fault::Unreadable { .. } => None,
            Fault::AtHead { fault } => fault.position(),
        }
    }
}

/// What a place in the file requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    /// A table, such as `roles` or `host.claude-code`.
    Table,
    /// A boolean.
    Bool,
    /// A rung name.
    Rung,
    /// A model name.
    ModelName,
    /// A git remote name.
    RemoteName,
    /// One of the `git.on_protected` names.
    OnProtected,
}
impl From<Kind> for Expected {
    fn from(kind: Kind) -> Expected {
        match kind {
            Kind::Bool => Expected::Bool,
            Kind::Rung => Expected::Rung,
            Kind::ModelName => Expected::ModelName,
            Kind::RemoteName => Expected::RemoteName,
            Kind::OnProtected => Expected::OnProtected,
        }
    }
}

impl Unavailable {
    /// Always `config-unavailable`.
    pub fn code(&self) -> &'static str {
        CONFIG_UNAVAILABLE
    }

    /// The same refusal labelled as HEAD's copy, so the owner does not look
    /// for the fault in the working-tree file at the same path.
    pub fn at_head(self) -> Unavailable {
        if matches!(self.fault, Fault::AtHead { .. }) {
            return self;
        }
        Unavailable {
            path: self.path,
            fault: Fault::AtHead {
                fault: Box::new(self.fault),
            },
        }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        describe(f, &self.path.display().to_string(), &self.fault)
    }
}

fn describe(f: &mut fmt::Formatter<'_>, path: &str, fault: &Fault) -> fmt::Result {
    match fault {
        Fault::AtHead { fault } => describe(f, &format!("HEAD's copy of {path}"), fault),
        Fault::NotRegular => write!(f, "{path} is not a regular file"),
        Fault::Unreadable { cause } => write!(f, "cannot read {path}: {cause}"),
        Fault::NotUtf8 { line, column } => write!(
            f,
            "{path}:{line}:{column}: the file is not UTF-8 from this point"
        ),
        Fault::Parse {
            position: Some((line, column)),
            message,
        } => write!(f, "{path}:{line}:{column}: {message}"),
        Fault::Parse {
            position: None,
            message,
        } => write!(f, "{path}: {message}"),
        Fault::WrongType {
            name,
            expected,
            found,
            line,
            column,
        } => {
            let article = article(found);
            let expected = match expected {
                Expected::Table => "a table".to_owned(),
                Expected::Bool => "a boolean".to_owned(),
                Expected::Rung => format!("a rung ({})", rungs()),
                Expected::ModelName => "a model name".to_owned(),
                Expected::RemoteName => "a remote name".to_owned(),
                Expected::OnProtected => format!("one of {}", on_protected_names()),
            };
            write!(
                f,
                "{path}:{line}:{column}: {name} is {article} {found}, not {expected}"
            )
        }
        Fault::OutsideGrammar {
            name,
            kind,
            written,
            line,
            column,
        } => {
            write!(f, "{path}:{line}:{column}: {name} is ")?;
            // Escaped so a value holding a line break stays on one line.
            let written = written.escape_debug();
            match kind {
                Kind::Rung => write!(f, "\"{written}\", which is not a rung ({})", rungs()),
                Kind::ModelName => f.write_str("empty; write a model name or remove the line"),
                Kind::RemoteName => f.write_str("empty; write a remote name or remove the line"),
                Kind::Bool => write!(f, "\"{written}\", which is not a boolean"),
                Kind::OnProtected => write!(
                    f,
                    "\"{written}\", which is not one of {}",
                    on_protected_names()
                ),
            }
        }
        Fault::Project {
            name,
            problem,
            position,
        } => {
            match position {
                Some((line, column)) => write!(f, "{path}:{line}:{column}: ")?,
                None => write!(f, "{path}: ")?,
            }
            match problem {
                    ProjectProblem::Missing if *name == "project" => f.write_str(
                        "there is no [project] table; the project file needs one with an id and a string name",
                    ),
                    ProjectProblem::Missing => write!(
                        f,
                        "{name} is missing; the [project] table needs an id and a string name"
                    ),
                    ProjectProblem::WrongType { found } => {
                        let expected = if *name == "project" {
                            "a table"
                        } else {
                            "a string"
                        };
                        write!(f, "{name} is {} {found}, not {expected}", article(found))
                    }
                    ProjectProblem::NotAnId { written } => write!(
                        f,
                        "{name} is \"{}\", which is not a lower-case UUID version 4",
                        written.escape_debug()
                    ),
                }
        }
    }
}

fn article(found: &str) -> &'static str {
    if found.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {} ",
            self.path.display(),
            self.line,
            self.column,
            self.name
        )?;
        match self.kind {
            DiagnosticKind::UnknownName => {
                f.write_str("is not a setting Baley reads and was ignored")
            }
            DiagnosticKind::WrongScope { .. } => {
                // Only a project setting can be out of scope in the global file.
                let (setting, file) = match self.layer {
                    FileLayer::Global => ("project", "global"),
                    FileLayer::Project => ("global", "project"),
                };
                write!(
                    f,
                    "is a {setting} setting and was ignored in the {file} file"
                )
            }
            DiagnosticKind::UnknownHost => {
                let hosts: Vec<&str> = Host::ALL.into_iter().map(Host::name).collect();
                write!(
                    f,
                    "is not a host Baley knows ({}) and its section was ignored",
                    hosts.join(", ")
                )
            }
        }
    }
}

fn rungs() -> String {
    let names: Vec<&str> = Rung::ALL.into_iter().map(Rung::name).collect();
    names.join(", ")
}

fn on_protected_names() -> String {
    let names: Vec<&str> = OnProtected::ALL
        .into_iter()
        .map(OnProtected::name)
        .collect();
    names.join(", ")
}

/// Parses one settings file and judges it against `schema`.
///
/// Unknown names, names outside their scope and unknown host sections become
/// diagnostics; bytes that are not UTF-8, text that is not TOML, a wrong type
/// or a value outside its grammar make the file unavailable. When a file holds
/// several such values, the first in the file is named.
pub fn parse_layer(
    file: &SettingsFile,
    layer: FileLayer,
    schema: &Schema,
) -> Result<ParsedLayer, Unavailable> {
    let (text, table) = document(file)?;
    let mut walk = Walk {
        text,
        path: &file.path,
        layer,
        entries: schema
            .entries()
            .iter()
            .map(|entry| (entry.name.split('.').collect(), entry))
            .collect(),
        values: Vec::new(),
        diagnostics: Vec::new(),
        faults: Vec::new(),
    };
    walk.table(table.get_ref(), &[], None);
    if let Some(fault) = walk.faults.into_iter().min_by_key(Fault::position) {
        return Err(Unavailable {
            path: file.path.clone(),
            fault,
        });
    }
    // Dotted keys interleave subtrees, so the walk alone is not file order.
    walk.diagnostics.sort_by_key(|d| (d.line, d.column));
    Ok(ParsedLayer {
        file: FileRef {
            path: file.path.clone(),
            digest: file.digest.clone(),
        },
        layer,
        values: walk.values,
        diagnostics: walk.diagnostics,
    })
}

/// The file's text and its TOML document, or the `NotUtf8` or `Parse` fault
/// that stops either. Every reader of a settings file goes through here, so
/// they name the same position and message for the same bytes.
pub(super) fn document(file: &SettingsFile) -> Result<(&str, Spanned<DeTable<'_>>), Unavailable> {
    let unavailable = |fault| Unavailable {
        path: file.path.clone(),
        fault,
    };
    let text = std::str::from_utf8(&file.bytes).map_err(|error| {
        let valid = &file.bytes[..error.valid_up_to()];
        let prefix = std::str::from_utf8(valid).unwrap_or_default();
        let (line, column) = line_and_column(prefix, prefix.len());
        unavailable(Fault::NotUtf8 { line, column })
    })?;
    let table = DeTable::parse(text).map_err(|error| {
        unavailable(Fault::Parse {
            position: error.span().map(|span| line_and_column(text, span.start)),
            message: error.message().to_owned(),
        })
    })?;
    Ok((text, table))
}

/// The one-based line and column of a byte offset, by the rule `toml` 1.1.6
/// applies when it prints an error: an offset at or past the end is clamped
/// to the last byte and the excess added to the column, and the column counts
/// characters up to and including the byte at the offset, or bytes when that
/// slice is not UTF-8. Baley's own positions follow the same rule.
pub fn line_and_column(text: &str, offset: usize) -> (u32, u32) {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return (1, count(offset.saturating_add(1)));
    }
    let index = offset.min(bytes.len() - 1);
    let carry = offset - index;
    let line_start = bytes[..index]
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |newline| newline + 1);
    let line = 1 + bytes[..line_start].iter().filter(|b| **b == b'\n').count();
    let column = match std::str::from_utf8(&bytes[line_start..=index]) {
        Ok(slice) => slice.chars().count() - 1,
        Err(_) => index - line_start,
    };
    (count(line), count(column + carry + 1))
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Joins a path for display, quoting a segment that holds a dot as TOML does.
fn display(segments: &[&str]) -> String {
    let quoted: Vec<String> = segments
        .iter()
        .map(|segment| {
            if segment.contains('.') {
                format!("\"{}\"", segment.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                (*segment).to_owned()
            }
        })
        .collect();
    quoted.join(".")
}

enum Mismatch {
    Type(&'static str),
    Grammar(String),
}

fn typed(kind: Kind, value: &DeValue<'_>) -> Result<Value, Mismatch> {
    let wrong = || Mismatch::Type(value.type_str());
    match kind {
        Kind::Bool => value.as_bool().map(Value::Bool).ok_or_else(wrong),
        Kind::Rung => {
            let written = value.as_str().ok_or_else(wrong)?;
            Rung::parse(written)
                .map(Value::Rung)
                .ok_or_else(|| Mismatch::Grammar(written.to_owned()))
        }
        Kind::ModelName => match value.as_str().ok_or_else(wrong)? {
            "" => Err(Mismatch::Grammar(String::new())),
            name => Ok(Value::ModelName(name.to_owned())),
        },
        Kind::RemoteName => match value.as_str().ok_or_else(wrong)? {
            "" => Err(Mismatch::Grammar(String::new())),
            name => Ok(Value::RemoteName(name.to_owned())),
        },
        Kind::OnProtected => {
            let written = value.as_str().ok_or_else(wrong)?;
            OnProtected::parse(written)
                .map(Value::OnProtected)
                .ok_or_else(|| Mismatch::Grammar(written.to_owned()))
        }
    }
}

type Key<'i> = Spanned<DeString<'i>>;
type Item<'i> = Spanned<DeValue<'i>>;

struct Walk<'a> {
    text: &'a str,
    path: &'a Path,
    layer: FileLayer,
    entries: Vec<(Vec<&'a str>, &'a Entry)>,
    values: Vec<Written>,
    diagnostics: Vec<Diagnostic>,
    faults: Vec<Fault>,
}

impl<'a> Walk<'a> {
    fn at(&self, offset: usize) -> (u32, u32) {
        line_and_column(self.text, offset)
    }

    /// The path as written: the host section's prefix, then the segments.
    fn name(path: &[&str], host: Option<Host>) -> String {
        match host {
            Some(host) => {
                let mut full = vec!["host", host.name()];
                full.extend_from_slice(path);
                display(&full)
            }
            None => display(path),
        }
    }

    fn table<'t>(&mut self, table: &'t DeTable<'_>, prefix: &[&'t str], host: Option<Host>) {
        let top = prefix.is_empty() && host.is_none();
        for (key, item) in table.iter() {
            let segment: &'t str = key.get_ref();
            let path: Vec<&'t str> = prefix.iter().copied().chain([segment]).collect();
            if top && segment == "project" && self.layer == FileLayer::Project {
                // The project's identity, read by `project::read_project`, not a setting.
                continue;
            }
            if top && segment == "host" {
                self.hosts(item);
                continue;
            }
            let exact = self
                .entries
                .iter()
                .find(|(segments, _)| *segments == path)
                .map(|(_, entry)| *entry);
            let below = self
                .entries
                .iter()
                .any(|(segments, _)| segments.len() > path.len() && segments.starts_with(&path));
            if let Some(entry) = exact {
                self.setting(entry, &path, host, key, item);
            } else if below {
                match item.get_ref() {
                    DeValue::Table(inner) => self.table(inner, &path, host),
                    _ => self.wrong_type(&path, host, Expected::Table, item),
                }
            } else {
                self.unknown(&path, host, key, item);
            }
        }
    }

    fn hosts(&mut self, item: &Item<'_>) {
        let DeValue::Table(sections) = item.get_ref() else {
            self.wrong_type(&["host"], None, Expected::Table, item);
            return;
        };
        for (key, section) in sections.iter() {
            let name: &str = key.get_ref();
            match (Host::parse(name), section.get_ref()) {
                (None, _) => self.diagnose(
                    display(&["host", name]),
                    key.span().start,
                    DiagnosticKind::UnknownHost,
                ),
                (Some(host), DeValue::Table(inner)) => self.table(inner, &[], Some(host)),
                (Some(_), _) => self.wrong_type(&["host", name], None, Expected::Table, section),
            }
        }
    }

    fn setting(
        &mut self,
        entry: &Entry,
        path: &[&str],
        host: Option<Host>,
        key: &Key<'_>,
        item: &Item<'_>,
    ) {
        // Typed before the scope is judged: a wrong value is a fault in any file.
        let value = match typed(entry.kind, item.get_ref()) {
            Ok(value) => value,
            Err(mismatch) => {
                let name = Self::name(path, host);
                let (line, column) = self.at(item.span().start);
                self.faults.push(match mismatch {
                    Mismatch::Type(found) => Fault::WrongType {
                        name,
                        expected: entry.kind.into(),
                        found,
                        line,
                        column,
                    },
                    Mismatch::Grammar(written) => Fault::OutsideGrammar {
                        name,
                        kind: entry.kind,
                        written,
                        line,
                        column,
                    },
                });
                return;
            }
        };
        let excluded = matches!(
            (entry.scope, self.layer),
            (Scope::Project, FileLayer::Global) | (Scope::Global, FileLayer::Project)
        );
        if excluded {
            let kind = DiagnosticKind::WrongScope { scope: entry.scope };
            self.diagnose(Self::name(path, host), key.span().start, kind);
            return;
        }
        let (line, column) = self.at(key.span().start);
        self.values.push(Written {
            name: entry.name.clone(),
            host,
            value,
            line,
            column,
        });
    }

    /// One diagnostic per leaf under an unknown name, so the owner sees what
    /// was written; an empty table reports itself.
    fn unknown(&mut self, path: &[&str], host: Option<Host>, key: &Key<'_>, item: &Item<'_>) {
        match item.get_ref() {
            DeValue::Table(inner) if !inner.is_empty() => {
                for (key, item) in inner.iter() {
                    let mut below = path.to_vec();
                    below.push(key.get_ref());
                    self.unknown(&below, host, key, item);
                }
            }
            _ => self.diagnose(
                Self::name(path, host),
                key.span().start,
                DiagnosticKind::UnknownName,
            ),
        }
    }

    fn wrong_type(
        &mut self,
        path: &[&str],
        host: Option<Host>,
        expected: Expected,
        item: &Item<'_>,
    ) {
        let (line, column) = self.at(item.span().start);
        self.faults.push(Fault::WrongType {
            name: Self::name(path, host),
            expected,
            found: item.get_ref().type_str(),
            line,
            column,
        });
    }

    fn diagnose(&mut self, name: String, offset: usize, kind: DiagnosticKind) {
        let (line, column) = self.at(offset);
        self.diagnostics.push(Diagnostic {
            layer: self.layer,
            path: self.path.to_path_buf(),
            name,
            line,
            column,
            kind,
        });
    }
}
