//! Diagnostics remain outside the chain.
use baley_core::{TraceRecord, TraceSink};
use baley_store::StoreError;
use baley_store_sqlite::{SqliteStore, TraceEntry};
use std::sync::Arc;

/// Bridges core diagnostics to the adapter.
pub(super) struct StoreTrace(pub Arc<SqliteStore>);
/// Preserves every diagnostic field at the adapter boundary.
pub(super) fn trace_entry(record: TraceRecord) -> TraceEntry {
    TraceEntry {
        at: record.at,
        project: record.project,
        payload: record.payload,
        kind: record.kind,
        data: record.data,
    }
}
impl TraceSink for StoreTrace {
    fn record(&self, record: TraceRecord) -> Result<(), StoreError> {
        self.0.record_trace(&trace_entry(record))
    }
}
