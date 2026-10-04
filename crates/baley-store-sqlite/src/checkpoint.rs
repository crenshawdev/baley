//! The one write-ahead-log checkpoint attempt a per-session server makes at
//! exit (design 0001, Checkpoints). The decisions are pure and sit first,
//! the attempt itself follows.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the exit method on the store calls these")
)]

use baley_store::StoreError;

use crate::schema::EPOCH;

/// Why the exit checkpoint did not try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The startup `quick_check` failed, so the store is fenced.
    Fenced,
    /// The stored epoch is not this binary's, so the store is read-only here.
    EpochNotThisBinary,
}

/// What the exit checkpoint did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitCheckpoint {
    /// No attempt was made.
    Skipped(SkipReason),
    /// Every frame in the log was folded into the database file.
    Complete,
    /// The attempt ran, and some frames stayed in the log.
    Incomplete,
    /// No connection was free, or the database was busy or locked.
    Unavailable,
    /// The attempt failed for another reason, or answered nonsense.
    Error(String),
}

/// What the attempt observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// Neither of the store's connections was free.
    NoConnection,
    /// A statement failed.
    Failed(StoreError),
    /// The `(busy, log, checkpointed)` row of `PRAGMA wal_checkpoint`.
    Row {
        busy: i64,
        log: i64,
        checkpointed: i64,
    },
}

/// Whether the attempt is skipped. `fenced` is known before any connection
/// is touched. The stored epoch is known only after it was read, so it is
/// `None` until then.
pub(crate) fn skip_reason(fenced: bool, stored_epoch: Option<u32>) -> Option<SkipReason> {
    if fenced {
        return Some(SkipReason::Fenced);
    }
    match stored_epoch {
        Some(epoch) if epoch != EPOCH => Some(SkipReason::EpochNotThisBinary),
        _ => None,
    }
}

/// Judges what the attempt observed. SQLite answers busy 1 when another
/// connection holds the checkpoint lock, which is "got there first" and not
/// a fault. A negative count means the checkpoint could not run, as when
/// the file is not in write-ahead-log mode.
pub(crate) fn judge_exit_checkpoint(attempt: Attempt) -> ExitCheckpoint {
    match attempt {
        Attempt::NoConnection | Attempt::Failed(StoreError::Busy) => ExitCheckpoint::Unavailable,
        Attempt::Failed(error) => ExitCheckpoint::Error(error.to_string()),
        Attempt::Row { busy: 1, .. } => ExitCheckpoint::Incomplete,
        Attempt::Row {
            busy,
            log,
            checkpointed,
        } => {
            if busy != 0 || log < 0 || checkpointed < 0 || checkpointed > log {
                ExitCheckpoint::Error(format!(
                    "unexpected checkpoint result (busy {busy}, log {log}, checkpointed {checkpointed})"
                ))
            } else if checkpointed == log {
                ExitCheckpoint::Complete
            } else {
                ExitCheckpoint::Incomplete
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(busy: i64, log: i64, checkpointed: i64) -> ExitCheckpoint {
        judge_exit_checkpoint(Attempt::Row {
            busy,
            log,
            checkpointed,
        })
    }

    // Catches a fully folded log being reported as anything but done.
    #[test]
    fn a_fully_checkpointed_log_is_not_complete() {
        assert_eq!(row(0, 7, 7), ExitCheckpoint::Complete);
    }

    // Catches busy or a short count being taken for success.
    #[test]
    fn a_busy_or_short_checkpoint_is_complete() {
        assert_eq!(row(1, 7, 3), ExitCheckpoint::Incomplete);
        assert_eq!(row(0, 7, 3), ExitCheckpoint::Incomplete);
    }

    // Catches a held checkpoint lock, which SQLite reports with no counts,
    // being read as a fault.
    #[test]
    fn a_checkpoint_lock_held_elsewhere_is_an_error() {
        assert_eq!(row(1, -1, -1), ExitCheckpoint::Incomplete);
    }

    // Catches a checkpoint that could not run being read as done or partial.
    #[test]
    fn a_negative_count_is_complete_or_incomplete() {
        assert!(matches!(row(0, -1, -1), ExitCheckpoint::Error(_)));
    }

    // Catches more frames folded than the log holds being read as done.
    #[test]
    fn more_checkpointed_than_logged_is_complete() {
        assert!(matches!(row(0, 3, 7), ExitCheckpoint::Error(_)));
    }

    // Catches a busy statement being reported as a fault.
    #[test]
    fn a_busy_statement_is_an_error() {
        assert_eq!(
            judge_exit_checkpoint(Attempt::Failed(StoreError::Busy)),
            ExitCheckpoint::Unavailable
        );
    }

    // Catches a failure that is not contention being reported as contention.
    #[test]
    fn a_disk_full_message_is_unavailable() {
        let judged = judge_exit_checkpoint(Attempt::Failed(StoreError::Unavailable(
            "database or disk is full".into(),
        )));
        assert!(
            matches!(&judged, ExitCheckpoint::Error(text) if text.contains("disk is full")),
            "{judged:?}"
        );
    }

    // Catches a store with no free connection being reported as a fault.
    #[test]
    fn no_free_connection_is_an_error() {
        assert_eq!(
            judge_exit_checkpoint(Attempt::NoConnection),
            ExitCheckpoint::Unavailable
        );
    }

    // Catches the exit checkpointing a store it must not write.
    #[test]
    fn a_fenced_store_is_not_skipped() {
        assert_eq!(skip_reason(true, None), Some(SkipReason::Fenced));
        assert_eq!(skip_reason(true, Some(EPOCH)), Some(SkipReason::Fenced));
    }

    // Catches a newer epoch, which this binary may not write, being checkpointed.
    #[test]
    fn a_newer_stored_epoch_is_not_skipped() {
        assert_eq!(
            skip_reason(false, Some(EPOCH + 1)),
            Some(SkipReason::EpochNotThisBinary)
        );
    }

    // Catches an older epoch being checkpointed too.
    #[test]
    fn an_older_stored_epoch_is_not_skipped() {
        assert_eq!(
            skip_reason(false, Some(EPOCH - 1)),
            Some(SkipReason::EpochNotThisBinary)
        );
    }

    // Catches a healthy store at this binary's epoch being skipped.
    #[test]
    fn a_healthy_store_at_this_epoch_is_skipped() {
        assert_eq!(skip_reason(false, Some(EPOCH)), None);
        assert_eq!(skip_reason(false, None), None);
    }
}
