//! What a host hook call is, judged from its stdin bytes alone (design 0010,
//! section 5). Nothing here runs the scanner or decides an answer: it says
//! which tool sent the call and hands over the fields the guard judges.
//!
//! Malformed input is not treated alike for every tool. A path tool whose
//! input cannot be read is denied, because letting it through would open the
//! read and write barriers. A command tool whose input cannot be read gets no
//! answer, because a declined command passes.

use serde::de::value::MapAccessDeserializer;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::marker::PhantomData;

#[cfg(test)]
mod tests;

/// The most hook input bytes the guard judges. The caller reads up to one byte
/// past it, so an input over the bound is seen as over.
pub const MAX_INPUT_BYTES: u64 = 65_536;

/// What every readable hook event carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The directory the host ran the call from.
    pub cwd: String,
    /// The host's session, when it sent one.
    pub session_id: Option<String>,
    /// The host's id for this call. Absent when the host sent none or an empty
    /// one, and never made up here.
    pub tool_use_id: Option<String>,
}

/// A tool whose input is one shell command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTool {
    /// The Bash tool.
    Bash,
    /// The Monitor tool, in its command form.
    Monitor,
}

/// A tool that reads or writes files, so its input names paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathTool {
    /// Reads one file.
    Read,
    /// Searches file contents.
    Grep,
    /// Lists files by pattern.
    Glob,
    /// Writes one file.
    Write,
    /// Edits one file.
    Edit,
    /// Edits one notebook.
    NotebookEdit,
}

impl PathTool {
    const ALL: [PathTool; 6] = [
        PathTool::Read,
        PathTool::Grep,
        PathTool::Glob,
        PathTool::Write,
        PathTool::Edit,
        PathTool::NotebookEdit,
    ];

    /// The name the host gives the tool.
    pub fn name(self) -> &'static str {
        match self {
            PathTool::Read => "Read",
            PathTool::Grep => "Grep",
            PathTool::Glob => "Glob",
            PathTool::Write => "Write",
            PathTool::Edit => "Edit",
            PathTool::NotebookEdit => "NotebookEdit",
        }
    }
}

/// The paths one path tool call names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTarget {
    /// Which tool sent the call.
    pub tool: PathTool,
    /// The file or folder it names: `file_path` for Read, Write and Edit,
    /// `notebook_path` for NotebookEdit, and `path` for Grep and Glob. Absent
    /// only for a Grep or Glob call with no `path`, which searches the cwd.
    pub path: Option<String>,
    /// A pattern that can reach files outside `path`: Glob's `pattern` or
    /// Grep's `glob`. Grep's own `pattern` is a search expression and is not
    /// carried.
    pub pattern: Option<String>,
}

/// What one hook call turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookInput {
    /// A shell command, with its text exactly as given.
    Command {
        /// What the call carries.
        envelope: Envelope,
        /// Bash, or Monitor in its command form.
        tool: CommandTool,
        /// The command text.
        text: String,
    },
    /// A Monitor watch with no command, such as a WebSocket. It carries no
    /// text and is never given empty or made-up text.
    Watch(Envelope),
    /// A PowerShell call. Baley reads POSIX shell grammar only, so the call
    /// is judged by its tool name and is never offered to the scanner.
    PowerShell(Envelope),
    /// A path tool call.
    Path {
        /// What the call carries.
        envelope: Envelope,
        /// The paths it names.
        target: PathTarget,
    },
    /// A path tool call that cannot be read, with the reason to refuse it.
    Deny(String),
    /// Nothing to judge: another tool, or a command tool call that cannot be
    /// read. The call passes.
    NoAnswer,
}

/// Classifies hook input. `bytes` is stdin read up to one byte past
/// [`MAX_INPUT_BYTES`].
pub fn classify(bytes: &[u8]) -> HookInput {
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return unreadable(
            identify_path_tool(bytes),
            &format!("over the {MAX_INPUT_BYTES}-byte bound"),
        );
    }
    let Ok(Object(wire)) = serde_json::from_slice::<Object<Wire>>(bytes) else {
        return unreadable(
            identify_path_tool(bytes),
            "not one JSON object with readable fields",
        );
    };
    let Some(tool) = Tool::named(&wire.tool_name) else {
        return HookInput::NoAnswer;
    };
    let declined = |why: &str| unreadable(tool.path(), why);
    if wire
        .hook_event_name
        .as_deref()
        .is_some_and(|name| name != "PreToolUse")
    {
        return declined("not a PreToolUse event");
    }
    let Some(cwd) = wire.cwd else {
        return declined("no cwd");
    };
    let envelope = Envelope {
        cwd,
        session_id: wire.session_id,
        tool_use_id: wire.tool_use_id.filter(|id| !id.is_empty()),
    };
    if tool == Tool::PowerShell {
        // Its field names are unmeasured, so nothing in tool_input is read.
        return HookInput::PowerShell(envelope);
    }
    let Ok(Object(Call {
        tool_input: Some(Object(fields)),
    })) = serde_json::from_slice::<Object<Call>>(bytes)
    else {
        return declined("no readable tool_input");
    };
    match tool {
        Tool::Bash => match fields.command {
            Some(text) => HookInput::Command {
                envelope,
                tool: CommandTool::Bash,
                text,
            },
            None => declined("no command"),
        },
        Tool::Monitor => match fields.command {
            Some(text) => HookInput::Command {
                envelope,
                tool: CommandTool::Monitor,
                text,
            },
            None => HookInput::Watch(envelope),
        },
        Tool::PowerShell => HookInput::PowerShell(envelope),
        Tool::Path(tool) => path_input(tool, envelope, fields),
    }
}

/// The path tool's target, or a deny when a field it needs is missing.
fn path_input(tool: PathTool, envelope: Envelope, fields: Fields) -> HookInput {
    let (path, pattern) = match tool {
        PathTool::Read | PathTool::Write | PathTool::Edit => (fields.file_path, None),
        PathTool::NotebookEdit => (fields.notebook_path, None),
        PathTool::Grep => (fields.path, fields.glob),
        PathTool::Glob => (fields.path, fields.pattern),
    };
    let required = match tool {
        PathTool::Glob => pattern.is_some(),
        PathTool::Grep => true,
        _ => path.is_some(),
    };
    if !required {
        return unreadable(Some(tool), "a field it needs is missing");
    }
    HookInput::Path {
        envelope,
        target: PathTarget {
            tool,
            path,
            pattern,
        },
    }
}

/// The answer for input that cannot be read: a deny for a path tool, nothing
/// for anything else.
fn unreadable(tool: Option<PathTool>, why: &str) -> HookInput {
    match tool {
        Some(tool) => HookInput::Deny(format!(
            "{} hook input cannot be read safely ({why}), so the call is refused",
            tool.name()
        )),
        None => HookInput::NoAnswer,
    }
}

/// Finds a path tool's name in bytes that may not parse, through any
/// whitespace. The closing quote is part of the match, so `Reader` is not
/// `Read`.
fn identify_path_tool(bytes: &[u8]) -> Option<PathTool> {
    let compact: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    PathTool::ALL.into_iter().find(|tool| {
        let needle = format!(r#""tool_name":"{}""#, tool.name()).into_bytes();
        compact.windows(needle.len()).any(|window| window == needle)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Bash,
    Monitor,
    PowerShell,
    Path(PathTool),
}

impl Tool {
    fn named(name: &str) -> Option<Tool> {
        match name {
            "Bash" => Some(Tool::Bash),
            "Monitor" => Some(Tool::Monitor),
            "PowerShell" => Some(Tool::PowerShell),
            other => PathTool::ALL
                .into_iter()
                .find(|tool| tool.name() == other)
                .map(Tool::Path),
        }
    }

    fn path(self) -> Option<PathTool> {
        match self {
            Tool::Path(tool) => Some(tool),
            _ => None,
        }
    }
}

/// The envelope fields, with `tool_input` skipped. A wrong type or a repeated
/// key fails the whole read.
#[derive(Deserialize)]
struct Wire {
    tool_name: String,
    #[serde(default, deserialize_with = "present")]
    cwd: Option<String>,
    #[serde(default, deserialize_with = "present")]
    session_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    tool_use_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    hook_event_name: Option<String>,
    #[serde(default, deserialize_with = "present", rename = "tool_input")]
    _tool_input: Option<IgnoredAny>,
}

/// Only `tool_input`, read for the tools that need its fields.
#[derive(Deserialize)]
struct Call {
    #[serde(default, deserialize_with = "present")]
    tool_input: Option<Object<Fields>>,
}

/// The `tool_input` fields the guard reads. Others are ignored.
#[derive(Deserialize)]
struct Fields {
    #[serde(default, deserialize_with = "present")]
    command: Option<String>,
    #[serde(default, deserialize_with = "present")]
    file_path: Option<String>,
    #[serde(default, deserialize_with = "present")]
    notebook_path: Option<String>,
    #[serde(default, deserialize_with = "present")]
    path: Option<String>,
    #[serde(default, deserialize_with = "present")]
    glob: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pattern: Option<String>,
}

/// A key that is there must hold a value of the field's type. Serde reads
/// `null` as an absent `Option`, which would turn `command: null` into a
/// missing command.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// A JSON object and nothing else. Serde reads an array into a struct field
/// by field, which would let `["Write", ...]` pass for an object.
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Only<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Only<T> {
            type Value = T;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map))
            }
        }
        deserializer.deserialize_map(Only(PhantomData)).map(Object)
    }
}
