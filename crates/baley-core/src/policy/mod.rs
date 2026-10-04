//! The owner's policy as a pure computation (design 0003): the settings
//! schema, one settings file judged against it, the merge of the layers into
//! the effective policy, the route of one dispatch resolved from it, the
//! project file's `[project]` table that identifies the project, the policy as
//! the ledger records it, and the pure half of the settings commands.
//!
//! Nothing here reads a file, the environment or a clock: the binary supplies
//! each file's path, bytes and digest, the connected host, the attempt, the
//! host's accepted model names and its rung map.

pub mod config_command;
pub mod merge;
pub mod parse;
pub mod project;
pub mod recorded;
pub mod route;
pub mod schema;

#[cfg(test)]
mod tests;

pub use merge::{Effective, EffectivePolicy, FileRef, Layer, Source, effective_policy, merge};
pub use parse::{
    CONFIG_UNAVAILABLE, Diagnostic, DiagnosticKind, Expected, Fault, FileLayer, ParsedLayer,
    SettingsFile, Unavailable, Value, Written, line_and_column, parse_layer,
};
pub use project::{ProjectIdentity, ProjectProblem, is_project_id, read_project, render_project};
pub use route::{
    AcceptedNames, Route, RouteRefusal, RouteRequest, RungMap, SettingSource, UNKNOWN_MODEL,
    resolve_route,
};
pub use schema::{Default, Entry, Host, Kind, OnProtected, Role, Rung, Schema, Scope};
