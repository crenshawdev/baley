//! The SQLite adapter of the Baley storage port (design 0001, ADR 0002).
//! One database per user, write-ahead log, a hash chain per project. The
//! adapter opens behind the writer queue and compatibility epoch, records
//! commands and claims, stores payloads, and rebuilds project views in
//! generations. T10 requires a fresh ledger: earlier epoch-1 files are
//! disposable and a different schema digest is refused at open.

mod claim;
mod payload;
mod queue;
mod rebuild;
mod retention;
mod schema;
mod store;
mod transact;
mod view;

pub use queue::{Monotonic, Timing};
pub use schema::EPOCH;
pub use store::{Options, SqliteStore, TraceEntry};
