//! The SQLite adapter of the Baley storage port (design 0001, ADR 0002).
//! One database per user, write-ahead log, a hash chain per project. The
//! adapter opens behind the writer queue and compatibility epoch, records
//! commands and claims, stores payloads, rebuilds project views in
//! generations, and implements the port's `Ledger`, verifying a project's
//! chain and bodies against an anchor the caller fetched. T12 requires a
//! fresh ledger: earlier epoch-1 files are disposable and a different
//! schema digest is refused at open. The adapter checks the real home and
//! store files for ownership, kind, modes and links before every open.
//! A store opened for the guard waits for its locks, its connections and
//! SQLite's busy handler no longer than the storage time it was given, at
//! most `GUARD_STORAGE_CAP`, and then answers `StoreError::Busy`. Its reads
//! and commands never rebuild a project's views inline: views behind this
//! binary's answer `StoreError::NeedsRebuild`. Two paths the guard never
//! calls keep their unbounded waits on such a store: lease renewal still
//! brings views forward under the maintenance lock, and read-only
//! connections (payload bodies, export, chain and view verification) keep
//! `BUSY_TIMEOUT`. A guard caller of either bounds it first, at
//! `SqliteStore::catch_up` and `connect_read_only`.
//! The adapter runs the port's conformance suite as one test per check,
//! and keeps its own tests for SQLite mechanisms.

mod admin;
mod checkpoint;
mod checks;
mod claim;
#[cfg(test)]
mod conformance_tests;
mod doctor;
mod export;
mod health;
mod ledger;
#[cfg(test)]
mod ledger_tests;
mod payload;
mod queue;
mod rebuild;
mod retention;
mod schema;
mod store;
mod transact;
mod view;

pub use checkpoint::{ExitCheckpoint, SkipReason};
pub use health::StartupHealth;
pub use queue::{Monotonic, Timing};
pub use schema::EPOCH;
pub use store::{GUARD_STORAGE_CAP, Options, SqliteStore, TraceEntry};
