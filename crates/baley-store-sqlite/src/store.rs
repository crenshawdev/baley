//! Opening the store and the write path every write goes through (design
//! 0001, Opening the store; Processes and concurrency; EVD-R8, R19, R20).

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use baley_store::{ProjectId, StoreError, ViewSpec};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use crate::queue::WriterQueue;
use crate::schema::{EPOCH, SCHEMA};
use crate::view::ViewSet;

/// The page size every store is created with. It cannot change once the
/// write-ahead log is on.
const PAGE_SIZE: i64 = 8192;

/// How the store is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The most trace rows kept; the oldest go first.
    pub trace_cap: u64,
    /// The views this binary declares. Their missing tables and indexes are
    /// created at open. Projectors will carry these specs once they exist;
    /// until then they are handed in here.
    pub views: Vec<ViewSpec>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            trace_cap: 10_000,
            views: Vec::new(),
        }
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

/// The user's ledger database: one connection for writes, one for reads,
/// so a read never waits behind this store's own write.
pub struct SqliteStore {
    writer: Mutex<Connection>,
    /// Crate-visible so a test can see whether a stream holds it.
    pub(crate) reader: Mutex<Connection>,
    queue: WriterQueue,
    options: Options,
    views: ViewSet,
}

impl SqliteStore {
    /// Opens `<home>/baley.db`. The epoch is read before anything that
    /// could write: a store stamped with a newer epoch is opened for
    /// reading and left exactly as it was, one at an older epoch is refused
    /// until migration exists, and a missing schema is created under the
    /// writer queue with `at` as its creation time. A store at this
    /// binary's epoch must be in write-ahead-log mode with 8 KiB pages.
    /// The declared views are checked before anything is touched, and at
    /// this binary's epoch their missing tables and indexes are created
    /// through the write path; a read-only store creates none.
    /// The home must already exist; locating it and checking its safety is
    /// slice 2's work.
    pub fn open(home: &Path, at: &str, options: Options) -> Result<Self, StoreError> {
        if !home.is_dir() {
            return Err(StoreError::Unavailable(format!(
                "{} is not a directory",
                home.display()
            )));
        }
        let views = ViewSet::new(&options.views)?;
        let queue = WriterQueue::open(&home.join("baley.db.writer")).map_err(io)?;
        let path = home.join("baley.db");
        let writer = connect(&path)?;

        let epoch = match stored_epoch(&writer)? {
            Some(epoch) => epoch,
            None => {
                create(&writer, &queue, at)?;
                stored_epoch(&writer)?
                    .ok_or_else(|| StoreError::Unavailable("the schema was not created".into()))?
            }
        };
        if epoch < EPOCH {
            return Err(StoreError::Unavailable(format!(
                "the store is at epoch {epoch}; migrating to {EPOCH} is not built yet"
            )));
        }
        // Opened after creation, so it reads the file as created.
        let reader = connect(&path)?;
        if epoch == EPOCH {
            check_file_settings(&writer)?;
        }
        let store = Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            queue,
            options,
            views,
        };
        // Most opens find every view in place, and ask on the read
        // connection, so they take neither the queue nor a write.
        if epoch == EPOCH
            && !store.views.is_empty()
            && store.snapshot(|conn| store.views.pending(conn))?
        {
            // An epoch raised by a newer binary since it was read above
            // leaves this store read-only, and read-only creates nothing.
            match store.write(|tx| store.views.create(tx)) {
                Ok(()) | Err(StoreError::ReadOnly { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(store)
    }

    /// The views declared at open.
    pub(crate) fn views(&self) -> &ViewSet {
        &self.views
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

    /// Records a diagnostic and keeps only the newest `trace_cap` rows.
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
            // Counted by rows, not by id arithmetic: a purge leaves gaps.
            tx.execute(
                "DELETE FROM trace WHERE id <= (SELECT id FROM trace ORDER BY id DESC LIMIT 1 OFFSET ?1)",
                params![cap],
            )
            .map_err(sql)?;
            Ok(())
        })
    }

    /// Runs `f` on the read connection, outside any write transaction.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let conn = lock(&self.reader);
        f(&conn).map_err(sql)
    }

    /// Runs `f` on the read connection inside one read transaction, so
    /// every statement in it sees the same snapshot.
    pub(crate) fn snapshot<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let conn = lock(&self.reader);
        // Dropped unfinished, which ends the read; there is nothing to keep.
        let tx = conn.unchecked_transaction().map_err(sql)?;
        f(&tx)
    }

    /// The write path: the writer queue, then the write connection, then
    /// `BEGIN IMMEDIATE`, then the epoch, then `f`, then commit. Anything
    /// that fails, or panics, rolls back. `synchronous=FULL` makes the
    /// commit survive power loss (EVD-R20); that rests on SQLite's
    /// documented behaviour and is not tested.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let _turn = self.queue.wait().map_err(io)?;
        let mut conn = lock(&self.writer);
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
        if stored < EPOCH {
            return Err(StoreError::Unavailable(format!(
                "the store is at epoch {stored}; this binary writes only epoch {EPOCH}"
            )));
        }
        let value = f(&tx)?;
        tx.commit().map_err(sql)?;
        Ok(value)
    }
}

/// A connection with the design's per-connection settings. None of them
/// writes to the database file.
pub(crate) fn connect(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(path).map_err(sql)?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(sql)?;
    conn.execute_batch(
        "PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;",
    )
    .map_err(sql)?;
    Ok(conn)
}

/// The stored epoch, or `None` when there is no schema yet. Reads only.
fn stored_epoch(conn: &Connection) -> Result<Option<u32>, StoreError> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'schema_meta'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?
        .is_some();
    if !exists {
        return Ok(None);
    }
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'epoch'",
        [],
        |row| row.get(0),
    )
    .map(Some)
    .map_err(sql)
}

/// Creates the file settings and the schema under the writer queue. A
/// second process that was waiting finds the schema and changes nothing.
fn create(conn: &Connection, queue: &WriterQueue, at: &str) -> Result<(), StoreError> {
    let _turn = queue.wait().map_err(io)?;
    if stored_epoch(conn)?.is_some() {
        return Ok(());
    }
    // The page size takes only before the first table and before the log
    // is switched on.
    conn.execute_batch(&format!("PRAGMA page_size = {PAGE_SIZE};"))
        .map_err(sql)?;
    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        .map_err(sql)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Unavailable(format!(
            "this filesystem refused the write-ahead log (journal mode {mode})"
        )));
    }
    conn.execute_batch("BEGIN IMMEDIATE;").map_err(sql)?;
    let created = (|| {
        if stored_epoch(conn)?.is_some() {
            return Ok(());
        }
        conn.execute_batch(SCHEMA).map_err(sql)?;
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('epoch', ?1), ('created_at', ?2)",
            params![EPOCH, at],
        )
        .map_err(sql)?;
        Ok(())
    })();
    match created {
        Ok(()) => conn.execute_batch("COMMIT;").map_err(sql),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

/// A store at this binary's epoch that is not in write-ahead-log mode with
/// 8 KiB pages was not made by Baley, or was changed behind its back.
fn check_file_settings(conn: &Connection) -> Result<(), StoreError> {
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(sql)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Unavailable(format!(
            "the store's journal mode is {mode}, not the write-ahead log"
        )));
    }
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(sql)?;
    if page_size != PAGE_SIZE {
        return Err(StoreError::Unavailable(format!(
            "the store's page size is {page_size}, not {PAGE_SIZE}"
        )));
    }
    Ok(())
}

/// A panic during a write leaves the connection as the unwinding
/// transaction left it: rolled back. Nothing to repair.
fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn sql(error: rusqlite::Error) -> StoreError {
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
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

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

    // The settings the design names read back from both of the store's
    // connections. Catches a pragma misspelt, which SQLite ignores without
    // an error, or one set on the write connection only.
    #[test]
    fn the_connection_settings_read_back_as_set() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        for connection in [&store.writer, &store.reader] {
            let conn = lock(connection);
            let text = |name: &str| {
                conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, String>(0))
                    .expect("pragma")
            };
            let number = |name: &str| {
                conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
                    .expect("pragma")
            };
            let settings = (
                text("journal_mode"),
                number("synchronous"),
                number("foreign_keys"),
                number("secure_delete"),
                number("busy_timeout"),
                number("page_size"),
            );
            // synchronous 2 is FULL.
            assert_eq!(settings, ("wal".into(), 2, 1, 1, 5000, 8192));
        }
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
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                trace_cap: 3,
                ..Options::default()
            },
        )
        .expect("open");
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

    /// A database the test builds by hand with Baley's schema at this
    /// epoch, but with the journal mode and page size it is given.
    fn foreign_store(home: &Path, journal_mode: &str, page_size: i64) {
        let conn = raw(home);
        conn.execute_batch(&format!("PRAGMA page_size = {page_size};"))
            .expect("page size");
        conn.pragma_update(None, "journal_mode", journal_mode)
            .expect("journal mode");
        conn.execute_batch(SCHEMA).expect("schema");
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('epoch', ?1), ('created_at', ?2)",
            params![EPOCH, AT],
        )
        .expect("epoch");
    }

    // Opening a store a newer binary stamped, and left in another journal
    // mode, changes nothing in it. Catches an open that switches the log
    // or takes a write transaction before reading the epoch.
    #[test]
    fn opening_a_newer_store_leaves_it_as_it_was() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open(home.path()));
        let conn = raw(home.path());
        conn.execute("UPDATE schema_meta SET value = 2 WHERE key = 'epoch'", [])
            .expect("stamp");
        conn.pragma_update(None, "journal_mode", "DELETE")
            .expect("journal mode");
        drop(conn);
        let store = open(home.path());
        assert_eq!(store.epoch(), Ok(2));
        drop(store);
        let mode: String = raw(home.path())
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("mode");
        assert_eq!(mode, "delete");
    }

    // A store whose epoch goes back, as when an older copy is restored
    // under a running binary, is not written to. Catches a fence that
    // refuses only newer epochs.
    #[test]
    fn an_older_epoch_refuses_the_next_write() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        raw(home.path())
            .execute("UPDATE schema_meta SET value = 0 WHERE key = 'epoch'", [])
            .expect("stamp");
        assert!(matches!(
            store.record_trace(&trace("refused")),
            Err(StoreError::Unavailable(_))
        ));
        let rows: i64 = raw(home.path())
            .query_row("SELECT count(*) FROM trace", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0);
    }

    // A store at this epoch outside the write-ahead log is refused, not
    // switched. Catches a journal mode that is never checked.
    #[test]
    fn a_store_outside_the_write_ahead_log_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        foreign_store(home.path(), "DELETE", PAGE_SIZE);
        let refusal = SqliteStore::open(home.path(), AT, Options::default()).err();
        assert!(
            matches!(&refusal, Some(StoreError::Unavailable(reason)) if reason.contains("journal mode")),
            "{refusal:?}"
        );
    }

    // A store at this epoch with another page size is refused. Catches a
    // page size set but never read back, which SQLite ignores once tables
    // exist.
    #[test]
    fn a_store_with_another_page_size_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        foreign_store(home.path(), "WAL", 4096);
        let refusal = SqliteStore::open(home.path(), AT, Options::default()).err();
        assert!(
            matches!(&refusal, Some(StoreError::Unavailable(reason)) if reason.contains("page size")),
            "{refusal:?}"
        );
    }

    // A read through the same store runs while that store's own write is
    // open, and sees the state before it. Catches reads that share the
    // write connection and wait behind it. The read runs on its own thread
    // so a regression fails on the timeout instead of hanging the suite.
    #[test]
    fn a_read_through_the_same_store_runs_during_its_write() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open(home.path()));
        let (in_write, wait_for_write) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel::<()>();
        let writer = Arc::clone(&store);
        let handle = thread::spawn(move || {
            writer.write(|tx| {
                tx.execute(
                    "INSERT INTO trace (at, kind, data) VALUES (?1, 'write', '{}')",
                    params![AT],
                )
                .map_err(sql)?;
                in_write.send(()).expect("signal");
                wait_for_release.recv().expect("wait");
                Ok(())
            })
        });
        wait_for_write
            .recv()
            .expect("writer inside its transaction");

        let (counted, count) = mpsc::channel();
        let reader = Arc::clone(&store);
        thread::spawn(move || {
            let rows = reader.read(|conn| {
                conn.query_row("SELECT count(*) FROM trace", [], |row| row.get::<_, i64>(0))
            });
            counted.send(rows).expect("send");
        });
        let rows = count.recv_timeout(Duration::from_secs(2));
        release.send(()).expect("release");
        handle.join().expect("writer thread").expect("write");
        assert_eq!(rows, Ok(Ok(0)));
    }

    // With a gap in the ids, as a purge leaves, the trace still keeps its
    // newest rows up to the cap. Catches rotation by id arithmetic, which
    // would drop a row while fewer than the cap remain.
    #[test]
    fn the_trace_keeps_its_cap_across_a_gap() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                trace_cap: 3,
                ..Options::default()
            },
        )
        .expect("open");
        for kind in ["one", "two", "three", "four"] {
            store.record_trace(&trace(kind)).expect("trace");
        }
        raw(home.path())
            .execute("DELETE FROM trace WHERE kind = 'three'", [])
            .expect("purge");
        store.record_trace(&trace("five")).expect("trace");
        let kinds = store
            .read(|conn| {
                let mut statement = conn.prepare("SELECT kind FROM trace ORDER BY id")?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("kinds");
        assert_eq!(kinds, ["two", "four", "five"]);
    }

    // A panic inside a write rolls it back, and the store goes on reading
    // and writing. Catches a poisoned lock that makes every later call
    // fail, or a panic that commits half a write.
    #[test]
    fn a_panic_inside_a_write_rolls_back_and_the_store_goes_on() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            store.write(|tx| {
                tx.execute(
                    "INSERT INTO trace (at, kind, data) VALUES (?1, 'lost', '{}')",
                    params![AT],
                )
                .map_err(sql)?;
                panic!("a bug inside a write");
                #[allow(unreachable_code)]
                Ok(())
            })
        }));
        assert!(unwound.is_err());
        store
            .record_trace(&trace("after"))
            .expect("write after the panic");
        let kinds = store
            .read(|conn| {
                let mut statement = conn.prepare("SELECT kind FROM trace ORDER BY id")?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("kinds");
        assert_eq!(kinds, ["after"]);
    }

    // The schema refuses an epoch that is not an integer and a hash that
    // is not 32 bytes. Catches type checks left to the code alone.
    #[test]
    fn the_schema_refuses_values_of_the_wrong_type() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open(home.path()));
        let conn = raw(home.path());
        assert!(
            conn.execute(
                "UPDATE schema_meta SET value = 'one' WHERE key = 'epoch'",
                []
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO payload (hash, bytes, encoding, state) VALUES (x'00', 0, 'zstd', 'present')",
                [],
            )
            .is_err()
        );
    }
}
