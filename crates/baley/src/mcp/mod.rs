//! The per-session server's pure decisions: what the session is, which client
//! is calling, which operations exist and in what order a call's faults are
//! answered. Nothing here touches stdio, the queue or the store.

/// The session's project, working directory and ids, gathered once and judged.
pub mod context;

/// The two gates around the queue and the one place answers become tool results.
pub mod gate;

/// Decodes the calling client in both protocol eras and selects a supported host.
pub mod client;

/// The literal operation baseline and the build that replaces each retired spelling.
pub mod operations;

/// The three tools, what the server says about itself and what `baley_version` answers.
pub mod tools;
