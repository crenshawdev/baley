//! Opening the store and the write path every write goes through (design
//! 0001, Opening the store; Processes and concurrency; EVD-R8, R19, R20).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use baley_store::{ProjectId, StoreError};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use crate::queue::WriterQueue;
use crate::schema::{EPOCH, SCHEMA};

/// How the store is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The most trace rows kept; the oldest go first.
    pub trace_cap: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { trace_cap: 10_000 }
    }
}

/// A diagnostic record: timings, retries, busy waits. Outside the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// UTC, RFC 3339, supplied by the caller.
    pub at: String,
    pub project: Option<ProjectId>,
    pub kind: String,
    pub data: String,
}

/// One connection to the user's ledger database.
pub struct SqliteStore {
    conn: Mutex<Connection>,
    queue: WriterQueue,
    options: Options,
}

impl SqliteStore {
    /// Opens `<home>/baley.db`, creating it and its schema if absent with
    /// `at` as its creation time. The home must already exist; locating it
    /// and checking its safety is slice 2's work. A store stamped with a
    /// newer epoch opens for reading and refuses every write.
    pub fn open(home: &Path, at: &str, options: Options) -> Result<Self, StoreError> {
        if !home.is_dir() {
            return Err(StoreError::Unavailable(format!(
                "{} is not a directory",
                home.display()
            )));
        }
        let queue = WriterQueue::open(&home.join("baley.db.writer")).map_err(io)?;
        let conn = Connection::open(home.join("baley.db")).map_err(sql)?;
        let store = Self {
            conn: Mutex::new(conn),
            queue,
            options,
        };
        store.create_if_absent(at)?;
        let epoch = store.epoch()?;
        if epoch < EPOCH {
            return Err(StoreError::Unavailable(format!(
                "the store is at epoch {epoch}; migrating to {EPOCH} is not built yet"
            )));
        }
        Ok(store)
    }

    /// The compatibility epoch stamped in the store.
    pub fn epoch(&self) -> Result<u32, StoreError> {
        self.read(|conn| {
            conn.query_row(
                "SELECT value FROM schema_meta WHERE key = 'epoch'",
                [],
                |row| row.get(0),
            )
        })
    }

    /// Records a diagnostic and drops the oldest rows beyond the cap.
    pub fn record_trace(&self, entry: &TraceEntry) -> Result<(), StoreError> {
        // SQLite integers are signed; a cap past i64 keeps everything.
        let cap = i64::try_from(self.options.trace_cap).unwrap_or(i64::MAX);
        self.write(|tx| {
            tx.execute(
                "INSERT INTO trace (at, project_id, kind, data) VALUES (?1, ?2, ?3, ?4)",
                params![
                    entry.at,
                    entry.project.as_ref().map(|project| &project.0),
                    entry.kind,
                    entry.data
                ],
            )
            .map_err(sql)?;
            tx.execute(
                "DELETE FROM trace WHERE id <= (SELECT MAX(id) FROM trace) - ?1",
                params![cap],
            )
            .map_err(sql)?;
            Ok(())
        })
    }

    /// Runs `f` on this store's connection, outside any write transaction.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let conn = self.connection()?;
        f(&conn).map_err(sql)
    }

    /// The write path: the writer queue, then `BEGIN IMMEDIATE`, then the
    /// epoch, then `f`, then commit. Anything that fails rolls back.
    /// `synchronous=FULL` makes the commit survive power loss (EVD-R20);
    /// that rests on SQLite's documented behaviour and is not tested.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.connection()?;
        let _turn = self.queue.wait().map_err(io)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let stored: u32 = tx
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'epoch'",
                [],
                |row| row.get(0),
            )
            .map_err(sql)?;
        if stored > EPOCH {
            return Err(StoreError::ReadOnly {
                needed_epoch: stored,
            });
        }
        let value = f(&tx)?;
        tx.commit().map_err(sql)?;
        Ok(value)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|_| StoreError::Unavailable("the connection lock is poisoned".into()))
    }

    /// Sets the connection's pragmas and, under the writer queue so two
    /// processes opening a fresh home cannot both create it, the schema.
    fn create_if_absent(&self, at: &str) -> Result<(), StoreError> {
        let mut conn = self.connection()?;
        let _turn = self.queue.wait().map_err(io)?;
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(sql)?;
        // The page size only takes on a database with no tables yet, and
        // before the log is switched on; on an existing one it is a no-op.
        conn.execute_batch("PRAGMA page_size = 8192;")
            .map_err(sql)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
            .map_err(sql)?;
        conn.execute_batch(
            "PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;",
        )
        .map_err(sql)?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'schema_meta'",
                [],
                |_| Ok(()),
            )
            .optional()
            .map_err(sql)?
            .is_some();
        if !exists {
            tx.execute_batch(SCHEMA).map_err(sql)?;
            tx.execute(
                "INSERT INTO schema_meta (key, value) VALUES ('epoch', ?1), ('created_at', ?2)",
                params![EPOCH, at],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)
    }
}

fn sql(error: rusqlite::Error) -> StoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StoreError::Busy,
        _ => StoreError::Unavailable(error.to_string()),
    }
}

fn io(error: std::io::Error) -> StoreError {
    StoreError::Unavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    const AT: &str = "2026-09-25T18:00:00Z";

    fn open(home: &Path) -> SqliteStore {
        SqliteStore::open(home, AT, Options::default()).expect("open")
    }

    /// A connection of the test's own, beside the store's, for stamping and
    /// counting behind the store's back.
    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    fn trace(kind: &str) -> TraceEntry {
        TraceEntry {
            at: AT.into(),
            project: None,
            kind: kind.into(),
            data: "{}".into(),
        }
    }

    // A second open finds the schema and keeps it: one epoch row, and the
    // creation time of the first open, not the second. Catches a schema
    // created or stamped again on every open.
    #[test]
    fn opening_twice_keeps_one_schema_and_one_epoch() {
        let home = tempfile::tempdir().expect("temp dir");
        let first = open(home.path());
        let second = SqliteStore::open(home.path(), "2026-09-26T09:00:00Z", Options::default())
            .expect("second open");
        assert_eq!(first.epoch(), Ok(EPOCH));
        assert_eq!(second.epoch(), Ok(EPOCH));
        let (rows, created_at): (i64, String) = raw(home.path())
            .query_row(
                "SELECT (SELECT count(*) FROM schema_meta),
                        (SELECT value FROM schema_meta WHERE key = 'created_at')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("meta");
        assert_eq!(rows, 2);
        assert_eq!(created_at, AT);
    }

    // A newer binary raising the epoch while this store is open fences
    // this store at its next write, which records nothing, while reads go
    // on. Catches a write that skips the epoch read and trusts the value
    // seen at open.
    #[test]
    fn a_newer_epoch_fences_an_open_store_at_its_next_write() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        raw(home.path())
            .execute("UPDATE schema_meta SET value = 2 WHERE key = 'epoch'", [])
            .expect("stamp");
        assert_eq!(
            store.record_trace(&trace("after")),
            Err(StoreError::ReadOnly { needed_epoch: 2 })
        );
        assert_eq!(store.epoch(), Ok(2));
        let rows: i64 = raw(home.path())
            .query_row("SELECT count(*) FROM trace", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0);
    }

    // A store already stamped newer opens for reading and refuses writes
    // with the epoch it needs. Catches an open that fails outright, or
    // one that writes to a schema it does not understand.
    #[test]
    fn a_store_stamped_newer_opens_read_only() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open(home.path()));
        raw(home.path())
            .execute("UPDATE schema_meta SET value = 3 WHERE key = 'epoch'", [])
            .expect("stamp");
        let store = open(home.path());
        assert_eq!(store.epoch(), Ok(3));
        assert_eq!(
            store.record_trace(&trace("refused")),
            Err(StoreError::ReadOnly { needed_epoch: 3 })
        );
    }

    // The settings the design names read back from the store's own
    // connection. Catches a pragma misspelt, which SQLite ignores without
    // an error.
    #[test]
    fn the_connection_settings_read_back_as_set() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let settings = store
            .read(|conn| {
                let text = |name: &str| {
                    conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, String>(0))
                };
                let number = |name: &str| {
                    conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
                };
                Ok((
                    text("journal_mode")?,
                    number("synchronous")?,
                    number("foreign_keys")?,
                    number("secure_delete")?,
                    number("busy_timeout")?,
                    number("page_size")?,
                ))
            })
            .expect("pragmas");
        // synchronous 2 is FULL.
        assert_eq!(settings, ("wal".into(), 2, 1, 1, 5000, 8192));
    }

    // While one connection holds a write transaction open, a second
    // connection reads without error and sees the state before that write;
    // after the commit it sees the write. Catches a reader that waits for
    // the writer, as under an exclusive locking mode.
    #[test]
    fn a_reader_runs_during_another_connections_write() {
        let home = tempfile::tempdir().expect("temp dir");
        let writer = open(home.path());
        let reader = open(home.path());
        let count = |store: &SqliteStore| {
            store.read(|conn| {
                conn.query_row("SELECT count(*) FROM trace", [], |row| row.get::<_, i64>(0))
            })
        };

        let (in_write, wait_for_write) = mpsc::channel();
        let (read_done, wait_for_read) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            writer.write(|tx| {
                tx.execute(
                    "INSERT INTO trace (at, kind, data) VALUES (?1, 'write', '{}')",
                    params![AT],
                )
                .map_err(sql)?;
                in_write.send(()).expect("signal");
                wait_for_read.recv().expect("wait");
                Ok(())
            })
        });

        wait_for_write
            .recv()
            .expect("writer inside its transaction");
        assert_eq!(count(&reader), Ok(0));
        read_done.send(()).expect("release");
        handle.join().expect("writer thread").expect("write");
        assert_eq!(count(&reader), Ok(1));
    }

    // Past the cap, each new diagnostic drops the oldest. Catches a trace
    // that grows without bound or drops the newest.
    #[test]
    fn the_trace_keeps_its_cap_and_drops_the_oldest() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = SqliteStore::open(home.path(), AT, Options { trace_cap: 3 }).expect("open");
        for kind in ["one", "two", "three", "four", "five"] {
            store.record_trace(&trace(kind)).expect("trace");
        }
        let kinds = store
            .read(|conn| {
                let mut statement = conn.prepare("SELECT kind FROM trace ORDER BY id")?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("kinds");
        assert_eq!(kinds, ["three", "four", "five"]);
    }
}
