//! The SQLite adapter of the Baley storage port (design 0001, ADR 0002).
//! One database per user, write-ahead log, a hash chain per project. The
//! adapter arrives task by task through Build 1. So far it opens the store
//! behind the writer queue and the compatibility epoch, stores payloads and
//! view documents, runs a database-only command's transaction, reduces and
//! purges payloads, and rebuilds and verifies a project's views in
//! generations.

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
