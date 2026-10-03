//! Portable checks of the storage port, run by each adapter through
//! `conformance_suite!`. Checks own scenarios and assertions. A factory owns
//! one fresh directory and the engine-specific damage and rebuild operations.

use crate::{
    Admin, Caller, DocKey, Event, EventSchema, Hash, Ledger, Payloads, ProjectId, Projector,
    RebuildReport, StoreError, Views,
};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

/// Registrations supplied by the binary opening a store.
pub struct Binary {
    /// Domain projectors, beside the store-owned projectors.
    pub projectors: Vec<Box<dyn Projector>>,
    /// Event types and versions this binary reads.
    pub schema: Box<dyn EventSchema>,
    /// Version of the complete registered view set.
    pub view_set_version: NonZeroU32,
}

/// Engine operations needed by the checks, confined to one fresh directory.
pub trait StoreFactory {
    /// The adapter under test.
    type Store: Ledger + Views + Payloads + Admin;
    /// A consistent copy of a store.
    type Snapshot;
    /// Opens an empty store in a new home.
    fn create(&self, binary: Binary) -> Result<Self::Store, StoreError>;
    /// Opens an independent connection as the supplied binary.
    fn reopen(&self, store: &Self::Store, binary: Binary) -> Result<Self::Store, StoreError>;
    /// Damages one project's rows behind the port.
    fn corrupt(
        &self,
        store: &Self::Store,
        project: &ProjectId,
        damage: Corruption,
    ) -> Result<(), StoreError>;
    /// Stamps and returns the epoch after this adapter's epoch.
    fn stamp_newer_epoch(&self, store: &Self::Store) -> Result<u32, StoreError>;
    /// Copies the database consistently.
    fn snapshot(&self, store: &Self::Store) -> Result<Self::Snapshot, StoreError>;
    /// Opens a snapshot in a new home as the supplied binary.
    fn restore(&self, snapshot: &Self::Snapshot, binary: Binary)
    -> Result<Self::Store, StoreError>;
    /// An absent export target under an existing canonical parent.
    fn export_target(&self) -> PathBuf;
    /// Opens the home created by an export.
    fn open_export(&self, target: &Path, binary: Binary) -> Result<Self::Store, StoreError>;
    /// Abandons a rebuild after this many one-event batches, returning events applied.
    fn crash_rebuild(
        &self,
        store: &Self::Store,
        project: &ProjectId,
        batches: u32,
    ) -> Result<u64, StoreError>;
    /// Calls `between` once after the first batch commits, outside the writer queue.
    fn rebuild_between(
        &self,
        store: &Self::Store,
        project: &ProjectId,
        between: &mut dyn FnMut(),
    ) -> Result<RebuildReport, StoreError>;
}

/// The damage the suite asks for, each an operation on one project's stored rows.
#[derive(Debug, Clone, PartialEq)]
pub enum Corruption {
    /// Replaces a stored event's payload, leaving its hashes as they were.
    AlterPayload {
        /// The stored event sequence.
        seq: u64,
        /// The replacement payload.
        payload: serde_json::Value,
    },
    /// Shifts rows from `at` upward and inserts the event exactly as given.
    Insert {
        /// The sequence at which rows move up.
        at: u64,
        /// The inserted row, with its hashes.
        event: Box<Event>,
    },
    /// Deletes one stored event.
    Delete {
        /// The sequence to delete.
        seq: u64,
    },
    /// Swaps the sequences of two stored events.
    Reorder {
        /// The first sequence to swap.
        first: u64,
        /// The second sequence to swap.
        second: u64,
    },
    /// Replaces a payload and recomputes every hash from that event to the
    /// head, so the local chain is consistent and only an anchor can tell.
    RecomputeAfterEdit {
        /// The stored event sequence.
        seq: u64,
        /// The replacement payload.
        payload: serde_json::Value,
    },
    /// Deletes every event after `keep_through`.
    Truncate {
        /// The last sequence to retain.
        keep_through: u64,
    },
    /// Encodes different bytes of the same length, leaving references untouched.
    CorruptBody(Hash),
    /// Replaces a live view document's body, leaving the events untouched,
    /// so a rebuild that copies live rows is caught.
    AlterDocument {
        /// The fixture view name.
        view: String,
        /// The live document key.
        key: DocKey,
        /// The replacement document body.
        body: serde_json::Value,
    },
    /// Replaces a stored event's caller with the one given, leaving its hashes
    /// as they were. The replacement is a valid caller, so the event still
    /// reads and only its hash can tell.
    ReplaceCaller {
        /// The stored event sequence.
        seq: u64,
        /// The replacement caller.
        caller: Caller,
    },
    /// Sets the building marker to the live generation.
    MarkLiveGenerationBuilding,
}

mod chain;
mod fixture;
pub use chain::{
    a_body_corrupted_in_place_is_corrupt_while_the_chain_holds,
    a_chain_recomputed_after_an_edit_is_a_rewrite_of_its_anchor,
    a_corrupt_excerpt_faults_beside_its_tombstone, a_deleted_event_is_named_at_its_sequence,
    a_local_anchor_row_behind_the_remote_is_reported_behind,
    a_local_anchor_row_that_differs_from_the_remote_is_a_conflict,
    a_purge_leaves_every_recorded_event_as_it_was, a_purged_body_is_a_tombstone_not_a_fault,
    a_reduced_bodys_excerpt_is_hashed, a_regrown_older_copy_is_a_rewrite_of_its_anchor,
    a_replaced_caller_is_named_at_its_sequence,
    a_restored_older_copy_is_a_truncation_of_its_anchor,
    a_truncated_tail_is_a_truncation_of_its_anchor, an_altered_payload_is_named_at_its_sequence,
    an_inserted_event_is_named_at_the_event_after_it,
    an_owner_acknowledged_restore_is_reported_as_acknowledged,
    an_untouched_chain_verifies_against_its_anchor,
    reordered_events_are_named_at_the_first_moved_sequence,
    work_after_the_anchor_is_reported_as_the_unanchored_range,
};
mod commands;
pub use commands::{
    a_command_stamps_its_caller_on_every_event_it_appends,
    a_document_moved_since_it_was_seen_is_stale,
    a_document_that_appeared_since_its_absence_was_seen_is_stale,
    a_git_head_moved_since_it_was_seen_is_stale, a_projector_failure_records_nothing,
    a_replayed_request_returns_its_outcome_and_records_nothing,
    a_reused_request_id_with_another_digest_is_refused,
    an_event_that_appeared_since_its_absence_was_seen_is_stale,
    one_request_id_in_two_projects_is_two_requests,
    one_request_id_under_two_command_kinds_is_two_requests,
};
mod connections;
pub use connections::{
    a_read_on_another_connection_runs_during_a_write,
    writes_through_two_connections_extend_one_chain,
};
mod views;
pub use views::{
    a_cursor_from_another_query_is_refused, a_find_on_an_undeclared_index_is_refused,
    a_key_that_does_not_fit_the_view_is_refused, a_last_full_page_has_no_next_cursor,
    a_read_of_an_unknown_view_is_refused, equality_on_an_index_prefix_keeps_the_rest_of_the_order,
    equality_values_that_do_not_fit_the_index_are_refused, no_page_exceeds_the_views_bound,
    pages_follow_the_declared_order_without_gaps_or_repeats,
    several_keys_read_at_once_come_back_in_the_order_asked,
};
mod rebuild;
pub use rebuild::{
    a_building_marker_on_the_live_generation_stops_a_rebuild,
    a_building_marker_on_the_live_generation_stops_view_verification,
    a_command_during_a_rebuild_reaches_the_new_generation,
    a_cursor_issued_before_a_flip_is_refused,
    a_projector_error_in_view_verification_leaves_no_unfinished_generation,
    a_projector_failing_on_the_tail_leaves_the_old_generation_live,
    a_rebuild_after_a_purge_matches_the_live_views,
    a_rebuild_after_a_reduction_matches_the_live_views,
    a_rebuild_after_an_abandoned_one_matches_the_hand_written_documents,
    a_rebuild_ignores_a_corrupted_live_document, an_abandoned_rebuild_changes_nothing_live,
    an_old_event_replays_upcast_and_stays_as_stored,
    an_unreadable_event_stops_a_rebuild_with_nothing_live_changed,
    live_projection_equals_the_hand_written_documents,
    view_verification_names_a_corrupted_live_document,
    view_verification_refuses_while_a_generation_is_unfinished,
};
mod payloads;
pub use payloads::{
    a_body_is_addressed_by_the_sha256_of_its_bytes,
    a_body_over_several_chunks_streams_back_byte_for_byte,
    a_present_body_reports_its_uncompressed_length,
    a_purged_answer_leaves_its_request_document_unchanged,
    a_reduction_and_a_purge_record_their_callers,
    a_reduction_keeps_the_first_and_last_64_kib_under_their_own_hash,
    a_retry_after_a_purge_gets_the_tombstone, a_shared_body_survives_one_projects_purge,
};
mod export;
pub use export::{
    a_purge_lists_an_earlier_export_of_its_project, a_purge_skips_an_export_made_after_the_release,
    a_replayed_purge_lists_the_same_exports, a_shared_purge_lists_another_projects_export,
    an_export_keeps_every_events_caller, an_export_reports_its_verified_head,
    an_export_tombstones_a_body_its_project_released,
    an_export_verifies_alone_and_holds_no_other_project,
};
mod compatibility;
pub use compatibility::{
    a_newer_epoch_fences_an_open_connection_at_its_next_write,
    a_newer_live_view_refuses_an_older_binarys_commands,
    a_newer_live_view_refuses_an_older_binarys_reads,
    a_newer_view_set_refuses_an_older_binarys_commands,
    a_newer_view_set_refuses_an_older_binarys_reads, a_removed_view_rebuilds_forward_before_use,
    a_store_stamped_with_a_newer_epoch_opens_read_only,
    a_view_set_changed_without_a_new_version_is_refused_at_open,
    an_older_binary_never_rebuilds_a_newer_view, an_older_view_version_rebuilds_forward_before_use,
};
mod claims;
pub use claims::{
    a_claim_and_its_completion_keep_their_own_callers, a_cleanly_failed_effect_completes_the_claim,
    a_command_inside_an_active_claims_scope_is_blocked,
    a_command_outside_an_active_claims_scope_proceeds,
    a_reconciliation_records_no_caller_and_copies_none,
    a_reconciliation_that_carries_a_caller_is_refused, a_retry_during_the_effect_is_in_progress,
    an_interrupted_claim_reconciles_from_a_supplied_finding,
    automatic_reconciliation_leaves_an_awaiting_owner_claim_held,
    owner_reconciliation_resolves_an_awaiting_owner_claim,
};

/// Instantiates each portable check with a fresh factory expression.
#[macro_export]
macro_rules! conformance_suite {
    ($factory:expr) => {
        $crate::conformance_suite!(@checks $factory;
            an_untouched_chain_verifies_against_its_anchor,
            an_altered_payload_is_named_at_its_sequence,
            a_replaced_caller_is_named_at_its_sequence,
            an_inserted_event_is_named_at_the_event_after_it,
            a_deleted_event_is_named_at_its_sequence,
            reordered_events_are_named_at_the_first_moved_sequence,
            a_chain_recomputed_after_an_edit_is_a_rewrite_of_its_anchor,
            work_after_the_anchor_is_reported_as_the_unanchored_range,
            a_truncated_tail_is_a_truncation_of_its_anchor,
            a_restored_older_copy_is_a_truncation_of_its_anchor,
            a_regrown_older_copy_is_a_rewrite_of_its_anchor,
            an_owner_acknowledged_restore_is_reported_as_acknowledged,
            a_body_corrupted_in_place_is_corrupt_while_the_chain_holds,
            a_purged_body_is_a_tombstone_not_a_fault,
            a_reduced_bodys_excerpt_is_hashed,
            a_corrupt_excerpt_faults_beside_its_tombstone,
            a_local_anchor_row_behind_the_remote_is_reported_behind,
            a_local_anchor_row_that_differs_from_the_remote_is_a_conflict,
            a_purge_leaves_every_recorded_event_as_it_was,
            a_projector_failure_records_nothing,
            a_replayed_request_returns_its_outcome_and_records_nothing,
            a_reused_request_id_with_another_digest_is_refused,
            one_request_id_under_two_command_kinds_is_two_requests,
            one_request_id_in_two_projects_is_two_requests,
            a_document_moved_since_it_was_seen_is_stale,
            an_event_that_appeared_since_its_absence_was_seen_is_stale,
            a_document_that_appeared_since_its_absence_was_seen_is_stale,
            a_git_head_moved_since_it_was_seen_is_stale,
            a_read_on_another_connection_runs_during_a_write,
            writes_through_two_connections_extend_one_chain,
            pages_follow_the_declared_order_without_gaps_or_repeats,
            a_last_full_page_has_no_next_cursor,
            no_page_exceeds_the_views_bound,
            equality_on_an_index_prefix_keeps_the_rest_of_the_order,
            a_find_on_an_undeclared_index_is_refused,
            a_read_of_an_unknown_view_is_refused,
            a_key_that_does_not_fit_the_view_is_refused,
            equality_values_that_do_not_fit_the_index_are_refused,
            a_cursor_from_another_query_is_refused,
            several_keys_read_at_once_come_back_in_the_order_asked,
            live_projection_equals_the_hand_written_documents,
            a_rebuild_ignores_a_corrupted_live_document,
            a_command_during_a_rebuild_reaches_the_new_generation,
            a_projector_failing_on_the_tail_leaves_the_old_generation_live,
            an_abandoned_rebuild_changes_nothing_live,
            a_rebuild_after_an_abandoned_one_matches_the_hand_written_documents,
            a_building_marker_on_the_live_generation_stops_a_rebuild,
            a_building_marker_on_the_live_generation_stops_view_verification,
            an_unreadable_event_stops_a_rebuild_with_nothing_live_changed,
            an_old_event_replays_upcast_and_stays_as_stored,
            view_verification_names_a_corrupted_live_document,
            a_projector_error_in_view_verification_leaves_no_unfinished_generation,
            view_verification_refuses_while_a_generation_is_unfinished,
            a_cursor_issued_before_a_flip_is_refused,
            a_rebuild_after_a_reduction_matches_the_live_views,
            a_rebuild_after_a_purge_matches_the_live_views,
            a_body_is_addressed_by_the_sha256_of_its_bytes,
            a_present_body_reports_its_uncompressed_length,
            a_body_over_several_chunks_streams_back_byte_for_byte,
            a_reduction_keeps_the_first_and_last_64_kib_under_their_own_hash,
            a_shared_body_survives_one_projects_purge,
            a_purged_answer_leaves_its_request_document_unchanged,
            a_retry_after_a_purge_gets_the_tombstone,
            an_export_verifies_alone_and_holds_no_other_project,
            an_export_tombstones_a_body_its_project_released,
            an_export_reports_its_verified_head,
            an_export_keeps_every_events_caller,
            a_purge_lists_an_earlier_export_of_its_project,
            a_purge_skips_an_export_made_after_the_release,
            a_shared_purge_lists_another_projects_export,
            a_replayed_purge_lists_the_same_exports,
            a_store_stamped_with_a_newer_epoch_opens_read_only,
            a_newer_epoch_fences_an_open_connection_at_its_next_write,
            a_newer_live_view_refuses_an_older_binarys_reads,
            a_newer_live_view_refuses_an_older_binarys_commands,
            a_newer_view_set_refuses_an_older_binarys_reads,
            a_newer_view_set_refuses_an_older_binarys_commands,
            an_older_binary_never_rebuilds_a_newer_view,
            an_older_view_version_rebuilds_forward_before_use,
            a_removed_view_rebuilds_forward_before_use,
            a_view_set_changed_without_a_new_version_is_refused_at_open,
            a_command_outside_an_active_claims_scope_proceeds,
            a_command_inside_an_active_claims_scope_is_blocked,
            a_retry_during_the_effect_is_in_progress,
            a_cleanly_failed_effect_completes_the_claim,
            an_interrupted_claim_reconciles_from_a_supplied_finding,
            automatic_reconciliation_leaves_an_awaiting_owner_claim_held,
            owner_reconciliation_resolves_an_awaiting_owner_claim,
            a_command_stamps_its_caller_on_every_event_it_appends,
            a_claim_and_its_completion_keep_their_own_callers,
            a_reduction_and_a_purge_record_their_callers,
            a_reconciliation_that_carries_a_caller_is_refused,
            a_reconciliation_records_no_caller_and_copies_none,
        );
    };
    (@checks $factory:expr; $($check:ident),* $(,)?) => {
        $(#[test] fn $check() { $crate::conformance::$check(&$factory); })*
    };
}
