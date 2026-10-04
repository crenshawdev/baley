//! The one write-ahead-log checkpoint attempt a per-session server makes at
//! exit (design 0001, Checkpoints). The decisions are pure and sit first,
//! the attempt itself follows.

use std::time::Duration;

use baley_store::StoreError;
use rusqlite::Connection;

use crate::retention::checkpoint;
use crate::schema::EPOCH;
use crate::store::{BUSY_TIMEOUT, SqliteStore, sql};

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

impl SqliteStore {
    /// Makes one `PRAGMA wal_checkpoint(PASSIVE)` attempt for the server to
    /// call once at exit, and says what happened. It never waits: it takes a
    /// free connection with `try_lock`, leaves the writer queue and the
    /// maintenance lock alone, and sets the connection's busy timeout to zero
    /// for the epoch read and the pragma, so neither can enter the busy
    /// handler. It reads the stored epoch fresh, so a newer binary's raise
    /// since open is seen. A fenced store, or one at another epoch, is
    /// skipped. Nothing is written to stderr.
    pub fn checkpoint_at_exit(&self) -> ExitCheckpoint {
        if let Some(reason) = skip_reason(self.fenced(), None) {
            return ExitCheckpoint::Skipped(reason);
        }
        let Some(conn) = self.try_connection() else {
            return judge_exit_checkpoint(Attempt::NoConnection);
        };
        without_waiting(&conn, attempt)
    }
}

/// Runs `run` with the connection's busy timeout at zero, so no statement in
/// it can enter the busy handler, and puts the timeout back afterwards so a
/// store used after the call waits as it did before.
fn without_waiting(
    conn: &Connection,
    run: impl FnOnce(&Connection) -> ExitCheckpoint,
) -> ExitCheckpoint {
    let outcome = match conn.busy_timeout(Duration::ZERO) {
        Ok(()) => run(conn),
        Err(error) => judge_exit_checkpoint(Attempt::Failed(sql(error))),
    };
    // A failure here has no better home than the outcome already in hand.
    let _ = conn.busy_timeout(BUSY_TIMEOUT);
    outcome
}

/// Reads the stored epoch and, unless that skips the attempt, runs the
/// pragma once on a connection whose busy timeout is zero.
fn attempt(conn: &Connection) -> ExitCheckpoint {
    let epoch = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'epoch'",
            [],
            |row| row.get::<_, u32>(0),
        )
        .map_err(sql);
    let epoch = match epoch {
        Ok(epoch) => epoch,
        Err(error) => return judge_exit_checkpoint(Attempt::Failed(error)),
    };
    if let Some(reason) = skip_reason(false, Some(epoch)) {
        return ExitCheckpoint::Skipped(reason);
    }
    judge_exit_checkpoint(match checkpoint(conn, "PASSIVE") {
        Ok((busy, log, checkpointed)) => Attempt::Row {
            busy,
            log,
            checkpointed,
        },
        Err(error) => Attempt::Failed(error),
    })
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

    use std::path::Path;
    use std::sync::{Arc, mpsc};
    use std::thread;

    use rusqlite::params;

    use crate::checks::private_folder;
    use crate::store::{Options, TraceEntry};

    const AT: &str = "2026-10-03T09:00:00Z";

    fn open(home: &Path, startup_check: bool) -> SqliteStore {
        SqliteStore::open(
            home,
            AT,
            Options {
                startup_check,
                ..Options::default()
            },
        )
        .expect("open")
    }

    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    fn write_traces(store: &SqliteStore, count: usize) {
        for n in 0..count {
            store
                .record_trace(&TraceEntry {
                    at: AT.into(),
                    project: None,
                    payload: None,
                    kind: format!("trace {n}"),
                    data: "{}".into(),
                })
                .expect("trace");
        }
    }

    // Catches an exit attempt that cannot fold an idle store's log.
    #[test]
    fn an_idle_store_after_trace_writes_does_not_report_complete() {
        let home = private_folder();
        let store = open(home.path(), false);
        write_traces(&store, 3);
        assert_eq!(store.checkpoint_at_exit(), ExitCheckpoint::Complete);
    }

    // Catches an attempt that reports done while a reader still pins frames
    // the checkpoint may not fold.
    #[test]
    fn a_log_pinned_by_a_reader_reports_complete() {
        let home = private_folder();
        let store = open(home.path(), false);
        write_traces(&store, 2);
        let reader = raw(home.path());
        reader.execute_batch("BEGIN").expect("begin");
        reader
            .query_row("SELECT count(*) FROM trace", [], |row| row.get::<_, i64>(0))
            .expect("read");
        write_traces(&store, 3);
        assert_eq!(store.checkpoint_at_exit(), ExitCheckpoint::Incomplete);
        drop(reader);
    }

    // Catches an attempt that waits for a connection. It runs on its own
    // thread so a regression fails on the timeout and does not hang.
    #[test]
    fn with_both_connections_held_the_call_blocks_or_reports_anything_but_unavailable() {
        let home = private_folder();
        let store = Arc::new(open(home.path(), false));
        let writer = store.writer.lock().expect("writer");
        let reader = store.reader.lock().expect("reader");
        let (done, answer) = mpsc::channel();
        let caller = Arc::clone(&store);
        thread::spawn(move || done.send(caller.checkpoint_at_exit()).expect("send"));
        let outcome = answer.recv_timeout(std::time::Duration::from_secs(2));
        drop((writer, reader));
        assert_eq!(outcome, Ok(ExitCheckpoint::Unavailable));
    }

    /// A fenced store with both connections held, so any use of a
    /// connection shows as unavailable and not as the skip.
    #[test]
    fn a_fenced_store_is_checkpointed_rather_than_skipped() {
        let home = private_folder();
        drop(open(home.path(), false));
        let conn = raw(home.path());
        conn.execute_batch("PRAGMA ignore_check_constraints = ON;")
            .expect("pragma");
        conn.execute(
            "INSERT INTO project (project_id, name, created_at, head_hash) VALUES ('damaged', 'n', ?1, x'00')",
            params![AT],
        )
        .expect("damage");
        drop(conn);
        let store = open(home.path(), true);
        let _writer = store.writer.lock().expect("writer");
        let _reader = store.reader.lock().expect("reader");
        assert_eq!(
            store.checkpoint_at_exit(),
            ExitCheckpoint::Skipped(SkipReason::Fenced)
        );
    }

    // Catches an attempt that trusts the epoch read at open, and so folds
    // the log of a store a newer binary has since taken over.
    #[test]
    fn a_store_raised_past_this_epoch_is_checkpointed_rather_than_skipped() {
        let home = private_folder();
        let store = open(home.path(), false);
        write_traces(&store, 2);
        raw(home.path())
            .execute(
                "UPDATE schema_meta SET value = ?1 WHERE key = 'epoch'",
                [EPOCH + 1],
            )
            .expect("raise");
        assert_eq!(
            store.checkpoint_at_exit(),
            ExitCheckpoint::Skipped(SkipReason::EpochNotThisBinary)
        );
    }

    // Catches statements that run with a busy timeout above zero, which could
    // wait in SQLite's busy handler. The raw-connection route to a held
    // database is closed: a connection cannot take exclusive locking while
    // the store's own connections are attached, so the timeout is read from
    // inside the wrapper the attempt runs in.
    #[test]
    fn the_attempt_runs_with_a_busy_timeout_that_can_wait() {
        let home = private_folder();
        let store = open(home.path(), false);
        let conn = store.writer.lock().expect("writer");
        let inside = without_waiting(&conn, |conn| {
            let timeout: i64 = conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .expect("timeout");
            ExitCheckpoint::Error(timeout.to_string())
        });
        assert_eq!(inside, ExitCheckpoint::Error("0".into()));
    }

    // Catches a connection left with no wait after the call.
    #[test]
    fn the_busy_timeout_is_not_back_to_five_seconds_after_the_call() {
        let home = private_folder();
        let store = open(home.path(), false);
        assert_eq!(store.checkpoint_at_exit(), ExitCheckpoint::Complete);
        let timeout: i64 = store
            .writer
            .lock()
            .expect("writer")
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("timeout");
        assert_eq!(timeout, 5000);
    }
}
