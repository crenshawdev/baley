//! Baley's domain code: views, projectors and the rules that decide what may
//! be recorded. Everything here reaches storage through the port in
//! `baley-store` and nothing here knows which engine is behind it
//! (design 0001, EVD-R12). The event itself, its canonical bytes and the
//! chain live in the port, because both sides of it speak them.

pub mod anchor;
pub mod capture;
pub mod catalog;
pub mod checkout;
pub mod forge;
pub mod guard;
pub mod policy;
pub mod reconcile;
pub mod registry;
pub mod restore;
pub mod retention;

pub use anchor::{
    AnchorAct, AnchorOutcome, AnchorReport, AnchorRequest, AnchorRow, AnchorSeams, AnchorTarget,
    BlockedAction, ClaimStep, HeldAnchor, PrePushCheck, ReconcileStep, RecordDecision, TickGuard,
    Ticker, TraceRecord, TraceSink, Verification, anchor_check, anchor_claim_decision,
    anchor_command, anchor_status, blocked_action, chain_mismatch, claim_step,
    failed_record_decision, fetch_observation_to_remote_tag, pre_push_check, pre_push_verdict,
    push_observation_to_record_decision, pushed_record_decision, reconcile_from_observation,
    record_refusal, record_step, verify_observed, verify_project, with_heartbeat,
};
pub use forge::{
    FetchObservation, Forge, PushObservation, TagQuery, anchor_annotation, anchor_tag,
    parse_annotation, parse_tag_anchor, tag_sequence,
};
pub use policy::{
    AcceptedNames, CONFIG_UNAVAILABLE, Diagnostic, DiagnosticKind, Effective, EffectivePolicy,
    Entry, Expected, Fault, FileLayer, FileRef, Host, Kind, Layer, OnProtected, ParsedLayer, Role,
    Route, RouteRefusal, RouteRequest, Rung, RungMap, Schema, Scope, SettingSource, SettingsFile,
    Source, UNKNOWN_MODEL, Unavailable, Value, Written, effective_policy, line_and_column, merge,
    parse_layer, resolve_route,
};
pub use reconcile::{AnchorFinding, AnchorIntent, RemoteTag, anchor_reconciliation, judge_anchor};
pub use registry::{
    Current, Fence, FenceReason, PROJECT_INITIALIZED, PROJECT_INITIALIZED_VERSION, Registry,
    RegistryError, UpcastError, Upcaster, register_anchor_events, register_project_events,
};
pub use restore::{
    AcknowledgeRestore, AcknowledgeRestoreError, acknowledge_restore, acknowledgement,
};
pub use retention::{Closure, MATERIAL_SECONDS, Retention, RetentionError, eligibility};
