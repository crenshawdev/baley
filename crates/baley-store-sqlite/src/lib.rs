//! The SQLite adapter of the Baley storage port (design 0001, ADR 0002).
//! One database per user, write-ahead log, a hash chain per project. The
//! adapter arrives task by task through Build 1; this is its crate.
