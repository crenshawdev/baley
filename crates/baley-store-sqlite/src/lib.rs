//! The SQLite adapter of the Baley storage port (design 0001, ADR 0002).
//! One database per user, write-ahead log, a hash chain per project. The
//! adapter arrives task by task through Build 1: this task opens the store,
//! creates its schema, and puts every write behind the writer queue and the
//! compatibility epoch.

mod payload;
mod queue;
mod schema;
mod store;

pub use schema::EPOCH;
pub use store::{Options, SqliteStore, TraceEntry};
