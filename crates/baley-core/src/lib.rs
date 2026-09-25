//! Baley's domain code: events, views, projectors and the rules that decide
//! what may be recorded. Everything here reaches storage through the port in
//! `baley-store` and nothing here knows which engine is behind it
//! (design 0001, EVD-R12).
//!
//! Build 1, task 2: the event envelope, canonical JSON, the event type
//! registry, the hash chain and its pure verifier.

pub mod canonical;
pub mod chain;
pub mod event;
pub mod registry;

pub use canonical::{CanonicalError, MAX_SAFE_INTEGER, canonical_json};
pub use chain::{
    Anchor, AnchorVerdict, Break, BreakKind, ChainReport, Head, chain_hash, verify_chain,
};
pub use event::{Actor, Event, EventDraft, GitFacts, Hash, ProjectId, RequestId};
pub use registry::{Current, Fence, FenceReason, Registry, RegistryError, UpcastError, Upcaster};
