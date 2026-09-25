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
pub mod claim;
pub mod command;
pub mod conformance;
pub mod error;
pub mod event;
pub mod ledger;
pub mod payload;
pub mod request;
pub mod retention;
pub mod time;
pub mod view;

pub use canonical::{CanonicalError, MAX_SAFE_INTEGER, canonical_json};
pub use chain::{
    Anchor, AnchorVerdict, Break, BreakKind, ChainReport, Head, chain_hash, verify_chain,
};
pub use claim::{Claim, ClaimDecision, ClaimId, ClaimOwner, Claimed, Reconciliation, Resolution};
pub use command::{
    Absence, Answer, Command, CommandKind, Decision, EventMatch, GitObservation, NewEvent,
    Observed, ObservedDocument, Outcome, OutcomeKind, Recorded, StreamName,
};
pub use conformance::{Corruption, StoreFactory};
pub use error::{GitFact, Refusal, StaleInput, StoreError};
pub use event::{
    Actor, ActorError, AgentRole, Event, EventDraft, GitFacts, Hash, ProjectId, RequestId,
    SealError,
};
pub use ledger::{
    Admin, BackupReport, Decide, DecideClaim, DecideReconcile, EventSchema, Health, HistoryFilter,
    Ledger, PayloadFault, ProjectHealth, PurgeReport, RebuildReport, ScrubReport, Transaction,
    VerifyReport, Views, ViewsReport,
};
pub use payload::{
    PayloadBody, PayloadRef, PayloadReference, PayloadStatus, Payloads, RetentionClass,
};
pub use request::{
    COMMAND_COMPLETED, COMMAND_COMPLETED_VERSION, INLINE_ANSWER_LIMIT, REQUEST_VIEW,
    RequestProjector, command_stream, completed_payload, recorded_outcome, request_key,
    request_spec, store_owned,
};
pub use retention::{
    EXCERPT_EDGE, PAYLOAD_PURGED, PAYLOAD_PURGED_VERSION, PAYLOAD_REDUCED, PAYLOAD_REDUCED_VERSION,
    PurgedEvent, RETENTION_STREAM, ReducedEvent, kept_ranges,
};
pub use time::{TimeError, UtcInstant};
pub use view::{
    Change, Cursor, DocKey, Document, FieldKind, FieldSpec, IndexField, IndexQuery, IndexSpec,
    KeyValue, Order, Page, PageRequest, Projector, ProjectorError, ViewSpec,
};
