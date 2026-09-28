//! Wall time at the binary edge.
use baley_store::UtcInstant;
use std::time::{SystemTime, UNIX_EPOCH};

/// The wall clock used by CLI gathering.
pub(super) struct SystemClock;
impl SystemClock {
    fn reading() -> (i64, u32) {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => (
                i64::try_from(d.as_secs()).expect("clock range"),
                d.subsec_nanos(),
            ),
            Err(e) => {
                let d = e.duration();
                let seconds = -i64::try_from(d.as_secs()).expect("clock range");
                if d.subsec_nanos() == 0 {
                    (seconds, 0)
                } else {
                    (seconds - 1, 1_000_000_000 - d.subsec_nanos())
                }
            }
        }
    }
    /// Formats the current wall time for the store.
    pub(super) fn now() -> String {
        let (seconds, nanos) = Self::reading();
        UtcInstant::from_unix(seconds, nanos)
            .expect("clock within years 0000 to 9999")
            .to_string()
    }
    /// Reads Unix seconds for the tagger line.
    pub(super) fn seconds() -> i64 {
        Self::reading().0
    }
}
