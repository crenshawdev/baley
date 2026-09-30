extern crate self as baley;

/// Metadata admission and bounded acquisition of source and store inputs.
pub mod acquisition;

/// The owner's typed captures: a kind, a phase, and no prose to parse back.
pub mod capture;
/// Two persisted layers; defaults and migration evidence are never a layer.
pub mod config;
pub mod config_service;
pub mod context;
pub mod debug;
pub mod derivation;
pub mod envelope;
pub mod evidence;
pub mod execution;
pub mod git_process;
pub mod help;
pub mod landing;
pub mod milestone;
pub mod next_action;
pub mod pause;
pub mod plan;
/// The one way to start an external program, so a check can hand the code
/// a recorded fake instead of running real git, gpg or `sh`.
pub mod process;
pub mod progress;
pub mod rail;
pub mod read;
pub mod review;
/// The session layer: first-touch store initialization, the config layers and
/// guarded snapshot writes.
pub mod session;
pub mod spike;
pub mod store;
pub mod suggest;
/// The off-roadmap task: explicit identity, shared branch and risk policy, a record whose home the root decides.
pub mod task;
pub mod undo;
pub mod verification;
/// Why a file line is as it is: the git chain joined to the record.
pub mod why;

/// HEAD's copy of the project file, read through git as the project layer.
pub mod committed;
/// Finds the checkout's project: the nearest `baley.toml` at or below the repository root.
pub mod discovery;
/// Runs one command with a provider key in its environment and redacted from its output.
pub mod exec;
/// Platform configuration and ledger folders.
pub mod folders;
/// `baley init`: ties a repository to a ledger project.
pub mod init;
/// The owner's provider keys, read from `keys.env`.
pub mod keys;
/// Owner commands over the evidence ledger.
pub mod ledger;
/// `baley models`: the per-user model catalog and the seeding step before each use.
pub mod models;
/// Replaces a settings file whole, refusing when it changed since it was read.
pub mod replace;
/// The two settings files as bytes: the global file's path and one reader.
pub mod settings;
