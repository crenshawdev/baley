//! The content Claude Code loads for Baley, rendered from the compiled tables
//! and supplied values (design 0012, sections 9 and 10). Nothing here reads
//! the environment, reads or writes a file, or asks a clock, so the same
//! binary with the same inputs always renders the same bytes.
//!
//! After Build 3 T11 the module holds:
//! - `stubs`: one skill stub per served front door and the manifest that
//!   lists them with host, identity, bytes and digest;
//! - `executable`: the supplied absolute executable the hook and the
//!   registration run, judged once;
//! - `registration`: the `mcpServers` entry for `baley serve`;
//! - `hook`: the `PreToolUse` hook for `baley guard`, with GRD-R1's
//!   nine-tool matcher and the guard's host timeout;
//! - `placement`: an explicit placement map that turns supplied paths into
//!   the expected files and the protected-path list the guard and the doctor
//!   take;
//! - `security`: the sandbox and deny-rule proposal that keeps agents out of
//!   the home and config folder and off the placed files;
//! - `coverage`: the judge of what each guarded tool can read and write in
//!   the folders and the placed files, from a settings document;
//! - `compose`: Baley's entries put into an owner's existing settings
//!   document, keeping every unrelated key and reporting every bypass;
//! - `command`: `baley artifact`, which prints the stubs, the manifest, the
//!   registration, the hook and the settings to standard output and writes
//!   no file.
//!
//! Delivery is `baley install`'s (Build 3 T15): where each artifact goes, who
//! owns the file and how it is written or removed are decided there, with
//! this module's content as the input.
//!
//! Build 4 adds the agent and rung definition renderer here. Its definitions
//! reference the session's `baley` MCP entry by the key `registration`
//! exports and never define an inline server, so every subagent shares the
//! session's connection (ADR 0034).
//!
//! Host facts the shapes depend on, confirmed on 2026-10-06 against Claude
//! Code 2.1.292's bundled configuration schema and hooks reference:
//! - A skill is a `SKILL.md` whose YAML frontmatter carries `name` and
//!   `description`; the body below the frontmatter is the instruction text
//!   the session reads.
//! - An `mcpServers` entry of any server type takes an optional boolean
//!   `alwaysLoad` at the entry's own level. When true, every tool of that
//!   server stays in the prompt instead of behind tool search, and startup
//!   waits for the server's connection, capped at the host's connect
//!   timeout. The key is read in whichever scope the entry is registered in.
//! - A command hook whose item has no `args` is the shell form: its `command`
//!   string is passed to a shell (`bash` unless the item's `shell` says
//!   `powershell`), so an executable path in it is quoted for POSIX `sh`. A
//!   hook `matcher` is a regular expression over tool names.

pub mod command;
pub mod compose;
pub mod coverage;
pub mod executable;
pub mod hook;
pub mod placement;
pub mod registration;
pub mod security;
pub mod stubs;
