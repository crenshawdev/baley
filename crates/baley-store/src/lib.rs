//! The storage port of the Baley evidence ledger.
//!
//! Domain code speaks to storage through the traits this crate defines:
//! append these events, get this view by key, store this payload. An engine
//! adapter implements them; `baley-core` never sees the engine
//! (design 0001, EVD-R12). The event, its canonical bytes and the hash chain
//! live here too, because the domain writes them and the adapter stores and
//! verifies them.

pub mod anchor;
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

pub use anchor::{
    ANCHOR_ACKNOWLEDGE_RESTORE, ANCHOR_FAILED, ANCHOR_FAILED_VERSION, ANCHOR_PUSH, ANCHOR_PUSHED,
    ANCHOR_PUSHED_VERSION, ANCHOR_RECONCILE, ANCHOR_RESTORE_ACKNOWLEDGED,
    ANCHOR_RESTORE_ACKNOWLEDGED_VERSION, ANCHOR_SCOPE, ANCHOR_STREAM, ANCHOR_TAG_PREFIX,
    AnchorPushedPayload, RestoreAcknowledgedPayload, anchor_command_event, anchor_tag,
};
pub use canonical::{CanonicalError, MAX_SAFE_INTEGER, canonical_json};
pub use chain::{
    AcknowledgedRestore, Anchor, AnchorVerdict, Break, BreakKind, ChainReport, ChainVerifier, Head,
    chain_hash, unanchored_warning, verify_chain,
};
pub use claim::{
    Block, CLAIM_SCOPE_VIEW, COMMAND_CLAIMED, COMMAND_CLAIMED_VERSION, COMMAND_RECONCILED,
    COMMAND_RECONCILED_VERSION, Claim, ClaimDecision, ClaimId, ClaimOwner, ClaimScopeProjector,
    ClaimState, Claimed, ClaimedPayload, LEASE_EXPIRY_SECONDS, LEASE_RENEWAL_SECONDS, LeaseState,
    ReconcileAuthority, ReconciledPayload, ReconciledResolution, Reconciliation, Resolution,
    blocking, claim_scope_spec, claim_state, lease_state,
};
pub use command::{
    Absence, Answer, Command, CommandKind, Decision, EventMatch, GitObservation, NewEvent,
    Observed, ObservedDocument, Outcome, OutcomeKind, Recorded, StreamName,
};
pub use conformance::{Binary, Corruption, StoreFactory};
pub use error::{GitFact, Refusal, StaleInput, StoreError};
pub use event::{
    Actor, ActorError, AgentRole, Event, EventDraft, GitFacts, Hash, ProjectId, RequestId,
    SealError,
};
pub use ledger::{
    Admin, AnchorCheck, Building, ClaimCounts, Decide, DecideClaim, DecideReconcile,
    EVENT_PAGE_BOUND, EventSchema, ExportReport, Health, HistoryFilter, Ledger, PayloadFault,
    ProjectHealth, PurgeReport, RawViewHealth, RebuildReport, ScrubReport, StoredAnchor,
    StoredAnchorComparison, Transaction, UnanchoredAge, VerifyReport, ViewHealth, Views,
    ViewsReport, compare_stored_anchor,
};
pub use payload::{
    PayloadBody, PayloadRef, PayloadReference, PayloadStatus, Payloads, RetentionClass,
};
pub use request::{
    COMMAND_COMPLETED, COMMAND_COMPLETED_VERSION, ClaimDoc, INLINE_ANSWER_LIMIT, REQUEST_VIEW,
    RequestProjector, RequestState, command_stream, completed_payload, completed_payload_for,
    recorded_outcome, request_digest, request_key, request_spec, request_state, store_owned,
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
