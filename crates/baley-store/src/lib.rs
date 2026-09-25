//! The storage port of the Baley evidence ledger.
//!
//! Domain code speaks to storage through the traits this crate will define:
//! append these events, get this view by key, store this payload. An engine
//! adapter implements them; `baley-core` never sees the engine
//! (design 0001, EVD-R12). The traits and the conformance suite land in
//! Build 1's third task; this is the crate boundary they land inside.
