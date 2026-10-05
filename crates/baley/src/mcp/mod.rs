//! The per-session server: what the session is, which client is calling, which
//! operations exist and in what order a call's faults are answered, then the
//! queue, the worker and the transports that carry the calls. The judging is
//! pure and tested with supplied values. The stdio, the worker thread and the
//! store are reached only by the gathering in `serve`, `transport`, `worker`,
//! `prepare` and `capture`.

/// The session's project, working directory and ids, gathered once and judged.
pub mod context;

/// The two gates around the queue and the one place answers become tool results.
pub mod gate;

/// Decodes the calling client in both protocol eras and selects a supported host.
pub mod client;

/// The literal operation baseline and the build that replaces each retired spelling.
pub mod operations;

/// Cuts a long read answer into numbered parts at one shared bound.
pub mod parts;

/// The three tools, what the server says about itself and what `baley_version` answers.
pub mod tools;

/// Bounded line framing: a frame over 4 MiB or 128 levels deep is discarded and answered, never ending the input.
pub mod frame;

/// The session queue: one running decision, four waiting, a 16 MiB cap, no caller identity.
pub mod queue;

/// Admits a call in the order faults are answered: the gate first, then the queue.
pub mod admission;

/// Shutdown as transitions: stop admission, drain for at most ten seconds, one checkpoint attempt.
pub mod lifecycle;

/// The thread Baley owns that runs accepted decisions one at a time in queue order.
pub mod worker;

/// Carries frames between stdio and rmcp through the bounded decoder.
pub mod transport;

/// The rmcp handler for one session: gates, queue and worker behind rmcp's validation.
pub mod handler;

/// One session's start and end: gather, serve, drain and make the one checkpoint attempt.
pub mod serve;

/// Prepares a session's project read or write from its own project directory.
pub mod prepare;

/// Records a note or story as one `capture.recorded` in its own domain transaction.
pub mod capture;

/// Reads a capture back by identity: its text, or a tombstone once its body is purged.
pub mod document;
