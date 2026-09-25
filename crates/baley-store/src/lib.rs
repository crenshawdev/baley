//! The storage port of the Baley evidence ledger.
//!
//! Domain code speaks to storage through the traits this crate defines:
//! append these events, get this view by key, store this payload. An engine
//! adapter implements them; `baley-core` never sees the engine
//! (design 0001, EVD-R12). The event, its canonical bytes and the hash chain
//! live here too, because the domain writes them and the adapter stores and
//! verifies them.

pub mod canonical;
pub mod chain;
pub mod event;

pub use canonical::{CanonicalError, MAX_SAFE_INTEGER, canonical_json};
pub use chain::{
    Anchor, AnchorVerdict, Break, BreakKind, ChainReport, Head, chain_hash, verify_chain,
};
pub use event::{
    Actor, ActorError, AgentRole, Event, EventDraft, GitFacts, Hash, ProjectId, RequestId,
    SealError,
};
