//! Opening the store and the write path every write goes through (design
//! 0001, Opening the store; Processes and concurrency; EVD-R8, R19, R20).

use crate::checks::{self, Judgement};
use crate::health::{StartupHealth, judge_quick_check, runs_quick_check};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use baley_store::{
    COMMAND_CLAIMED, COMMAND_CLAIMED_VERSION, COMMAND_COMPLETED, COMMAND_COMPLETED_VERSION,
    COMMAND_RECONCILED, COMMAND_RECONCILED_VERSION, ClaimScopeProjector, Event, EventSchema, Hash,
    Head, PAYLOAD_PURGED, PAYLOAD_PURGED_VERSION, PAYLOAD_REDUCED, PAYLOAD_REDUCED_VERSION,
    ProjectId, Projector, Refusal, RequestProjector, StoreError, ViewSpec, store_owned,
};
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::queue::{FileLock, Monotonic, Timing, Turn, pause_for};
use crate::schema::{EPOCH, SCHEMA};
use crate::view::ViewSet;

/// The page size every store is created with. It cannot change once the
/// write-ahead log is on.
const PAGE_SIZE: i64 = 8192;

/// How the store is opened.
pub struct Options {
    /// The most trace rows kept; the oldest go first.
    pub trace_cap: u64,
    /// The core's projectors. The store adds `claim_scope` and `request`.
    pub projectors: Vec<Box<dyn Projector>>,
    /// The event types and versions this binary reads, beyond the store's
    /// own `command.*` types. A project holding any other is read-only
    /// here, and a decision may append no other (EVD-R19).
    pub schema: Box<dyn EventSchema>,
    /// The version of the whole registered view set. Raised whenever a view
    /// is added, removed or renamed; version 2 is `claim_scope request`.
    /// Open pins each version to its sorted view names and
    /// refuses a changed set under a version already recorded. Every
    /// generation is stamped with it, so a project whose live set is newer
    /// is read-only here and one whose set is older rebuilds forward.
    pub view_set_version: NonZeroU32,
    /// The clock and pause of rebuilds, cleanups and view verification.
    pub timing: Arc<dyn Timing>,
    /// Optional measurement of time spent waiting for a write's queue turn.
    pub queue_wait: Option<Arc<dyn Fn(Duration) + Send + Sync>>,
    /// Set by the per-session server only. Open then runs `PRAGMA quick_check`
    /// on an existing schema, and a failure fences every write while reads go
    /// on. Command-line and guard opens leave it off.
    pub startup_check: bool,
}

impl Default for Options {
    /// No domain projectors, store view set version 2, a schema that reads none of the
    /// core's types, and the monotonic clock.
    fn default() -> Self {
        Self {
            trace_cap: 10_000,
            projectors: Vec::new(),
            schema: Box::new(ReadsNothing),
            view_set_version: NonZeroU32::new(2).expect("nonzero"),
            timing: Arc::new(Monotonic),
            queue_wait: None,
            startup_check: false,
        }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let views: Vec<&str> = self
            .projectors
            .iter()
            .map(|projector| projector.spec().name.as_str())
            .collect();
        f.debug_struct("Options")
            .field("trace_cap", &self.trace_cap)
            .field("projectors", &views)
            .field("view_set_version", &self.view_set_version)
            .field("startup_check", &self.startup_check)
            .finish_non_exhaustive()
    }
}

/// The schema of a store opened without the core's registry.
struct ReadsNothing;

impl EventSchema for ReadsNothing {
    fn reads(&self, _type_name: &str, _version: u32) -> bool {
        false
    }
}

/// A diagnostic record: timings, retries, busy waits. Outside the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// UTC, RFC 3339, supplied by the caller.
    pub at: String,
    pub project: Option<ProjectId>,
    /// A trace entry derived from a payload body must name that payload.
    /// An entry without one is never removed by a purge.
    pub payload: Option<Hash>,
    pub kind: String,
    pub data: String,
}

/// The user's ledger database: one connection for writes, one for reads,
/// so a read never waits behind this store's own write.
pub struct SqliteStore {
    writer: Mutex<Connection>,
    /// Crate-visible so a test can see whether a stream holds it.
    pub(crate) reader: Mutex<Connection>,
    queue: FileLock,
    /// Held for a whole rebuild or view verification.
    maintenance_lock: FileLock,
    timing: Arc<dyn Timing>,
    queue_wait: Option<Arc<dyn Fn(Duration) + Send + Sync>>,
    trace_cap: u64,
    views: ViewSet,
    /// The store's `request` projector first, then the core's.
    projectors: Vec<Box<dyn Projector>>,
    schema: Box<dyn EventSchema>,
    /// Per project, the head through which every stored event's type and
    /// version was found readable, so a write checks only the events
    /// recorded since. The hash tells a chain rewritten under the mark, as
    /// by a restore, from the one that was checked.
    readable: Mutex<BTreeMap<ProjectId, Head>>,
    /// What the startup `quick_check` found.
    health: StartupHealth,
    pub(crate) home: std::path::PathBuf,
}

impl SqliteStore {
    /// Opens `<home>/baley.db`. Before writing database contents, open reads
    /// the epoch: a store stamped with a newer epoch is opened for reading
    /// and left exactly as it was, one at an older epoch is refused
    /// until migration exists, and a missing schema is created under the
    /// writer queue with `at` as its creation time. An existing file whose
    /// epoch-1 schema digest differs is refused before any write; T12 needs
    /// a fresh ledger at epoch 1. A store at this binary's epoch must be in
    /// write-ahead-log mode with 8 KiB pages.
    /// Declared view specs are validated before any file is opened or
    /// created. At this binary's epoch, stored view and view-set declarations
    /// are checked, and missing tables, indexes and catalog rows are created
    /// through the write path. A read-only store creates none of those.
    /// No project's views are looked at.
    /// With `Options::startup_check` set, an existing schema at this binary's
    /// epoch gets `PRAGMA quick_check` after the epoch, digest and file
    /// setting checks and before any view is declared. A schema this open
    /// creates skips it. Other opens never run it.
    /// The caller creates the home, which must exist. Before anything else,
    /// open checks the real home and each store file present: not a link,
    /// the right kind, owned by the effective user, and no permission bit
    /// beyond 0700 for the home or 0600 for a file. It refuses with
    /// `Refusal::UnsafeHome`, naming each fault. It then creates the lock
    /// files and an empty `baley.db` with mode 0600 when missing.
    pub fn open(home: &Path, at: &str, options: Options) -> Result<Self, StoreError> {
        match checks::judge(&checks::gather(home)?) {
            Judgement::Safe => {}
            Judgement::Unsafe(faults) => {
                return Err(StoreError::Refused(Refusal::UnsafeHome(faults)));
            }
            Judgement::Unreadable { path, error } => {
                return Err(StoreError::Unavailable(format!(
                    "cannot inspect {}: {error}",
                    path.display()
                )));
            }
        }
        let mut projectors: Vec<Box<dyn Projector>> = vec![
            Box::new(ClaimScopeProjector::new()),
            Box::new(RequestProjector::new()),
        ];
        projectors.extend(options.projectors);
        let specs: Vec<ViewSpec> = projectors
            .iter()
            .map(|projector| projector.spec().clone())
            .collect();
        let views = ViewSet::new(&specs, options.view_set_version)?;
        let queue = FileLock::open(&home.join("baley.db.writer")).map_err(io)?;
        let maintenance_lock = FileLock::open(&home.join("baley.db.maintenance")).map_err(io)?;
        let path = home.join("baley.db");
        // Only a missing database is created; an existing one is never opened
        // for writing here, so a private read-only file still reaches SQLite.
        created_or_present(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map(drop),
        )?;
        let writer = connect(&path)?;

        let existing = stored_epoch(&writer)?;
        let schema_existed = existing.is_some();
        let epoch = match existing {
            Some(epoch) if epoch == EPOCH => {
                let digest = writer
                    .query_row(
                        "SELECT value FROM schema_meta WHERE key = 'schema_digest'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(sql)?;
                if digest.as_deref() != Some(schema_digest().as_str()) {
                    return Err(StoreError::Refused(Refusal::SchemaChanged { path }));
                }
                epoch
            }
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
        // Only a store this binary writes is checked, and a schema this open
        // created has nothing to check. It runs before the view declarations
        // so a damaged file is not written to.
        let health = if epoch == EPOCH && runs_quick_check(options.startup_check, schema_existed) {
            judge_quick_check(quick_check(&writer))
        } else {
            StartupHealth::NotChecked
        };
        let store = Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            queue,
            maintenance_lock,
            timing: options.timing,
            queue_wait: options.queue_wait,
            trace_cap: options.trace_cap,
            views,
            projectors,
            schema: options.schema,
            readable: Mutex::new(BTreeMap::new()),
            health,
            home: home.to_path_buf(),
        };
        // Most opens find every view and the view set in place, and ask on
        // the read connection, so they take neither the queue nor a write.
        // No project is looked at: each is brought to this binary's views
        // on its first use.
        if epoch == EPOCH && store.snapshot(|conn| store.views.pending(conn))? {
            // An epoch raised by a newer binary since it was read above
            // leaves this store read-only, and read-only creates nothing.
            match store.write(|tx| store.views.create(tx)) {
                Ok(()) | Err(StoreError::ReadOnly { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(store)
    }

    /// What the startup `quick_check` found: not checked unless the server
    /// option was set and the schema already existed.
    pub fn startup_health(&self) -> &StartupHealth {
        &self.health
    }

    /// The views declared at open.
    pub(crate) fn views(&self) -> &ViewSet {
        &self.views
    }

    /// The projectors, the store's own first.
    pub(crate) fn projectors(&self) -> &[Box<dyn Projector>] {
        &self.projectors
    }

    /// Whether this binary reads events of this type and version: the
    /// store's own, or the core's.
    pub(crate) fn reads(&self, type_name: &str, version: u32) -> bool {
        (type_name == COMMAND_COMPLETED && version == COMMAND_COMPLETED_VERSION)
            || (type_name == COMMAND_CLAIMED && version == COMMAND_CLAIMED_VERSION)
            || (type_name == COMMAND_RECONCILED && version == COMMAND_RECONCILED_VERSION)
            || (type_name == PAYLOAD_REDUCED && version == PAYLOAD_REDUCED_VERSION)
            || (type_name == PAYLOAD_PURGED && version == PAYLOAD_PURGED_VERSION)
            || (!store_owned(type_name) && self.schema.reads(type_name, version))
    }

    /// The copy of `event` projectors see: a core type at its current
    /// version with its payload upcast, a store type exactly as recorded.
    /// The event itself is left as stored. The error says why it cannot be
    /// read.
    pub(crate) fn projection_copy(&self, event: &Event) -> Result<Event, String> {
        if store_owned(&event.type_name) {
            return if self.reads(&event.type_name, event.type_version) {
                Ok(event.clone())
            } else {
                Err(format!(
                    "{} version {} is not readable by this binary",
                    event.type_name, event.type_version
                ))
            };
        }
        let (type_version, payload) = self.schema.projection_payload(event)?;
        Ok(Event {
            type_version,
            payload,
            ..event.clone()
        })
    }

    /// The store's clock and pause.
    pub(crate) fn timing(&self) -> &dyn Timing {
        self.timing.as_ref()
    }

    /// Waits for this thread's turn at a rebuild or view verification, in
    /// this process and then across processes.
    pub(crate) fn hold_maintenance(&self) -> Result<Turn<'_>, StoreError> {
        self.maintenance_lock.wait().map_err(io)
    }

    /// Runs `action` each time a turn of this store's writer queue has
    /// been released, before the releasing code goes on.
    #[cfg(test)]
    pub(crate) fn at_queue_release(&self, action: impl FnMut() + Send + 'static) {
        self.queue.at_release(action);
    }

    /// The per-project readable-through marks.
    pub(crate) fn readable(&self) -> MutexGuard<'_, BTreeMap<ProjectId, Head>> {
        self.readable.lock().unwrap_or_else(PoisonError::into_inner)
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
        let cap = i64::try_from(self.trace_cap).unwrap_or(i64::MAX);
        self.write(|tx| {
            tx.execute(
                "INSERT INTO trace (at, project_id, payload_hash, kind, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entry.at,
                    entry.project.as_ref().map(|project| &project.0),
                    entry.payload.as_ref().map(|hash| &hash.0[..]),
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
        let waiting = self.queue_wait.as_ref().map(|_| self.timing.now());
        let _turn = self.queue.wait().map_err(io)?;
        if let (Some(observer), Some(waiting)) = (&self.queue_wait, waiting) {
            observer(self.timing.now().saturating_sub(waiting));
        }
        self.transaction(f)
    }

    /// One batch of a rebuild, cleanup or view verification: a write like
    /// `write`, given the instant its queue turn began, then `after`,
    /// still holding the queue, then a pause as long as the turn held the
    /// queue. The hold runs from the queue's acquisition, not the wait for
    /// it, through commit and release. The pause follows a failed batch
    /// too, since it held the queue all the same.
    pub(crate) fn batch<T, U>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>, Duration) -> Result<T, StoreError>,
        after: impl FnOnce(T) -> Result<U, StoreError>,
    ) -> Result<U, StoreError> {
        let turn = self.queue.wait().map_err(io)?;
        let acquired = self.timing.now();
        let result = self.transaction(|tx| f(tx, acquired)).and_then(after);
        drop(turn);
        let released = self.timing.now();
        self.timing
            .pause(pause_for(released.saturating_sub(acquired)));
        result
    }

    /// The write connection's `BEGIN IMMEDIATE`, the epoch, `f` and commit,
    /// for a caller that holds the queue.
    fn transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
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

    /// Holds the writer queue and write connection for maintenance outside a transaction.
    pub(crate) fn maintenance<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let _turn = self.queue.wait().map_err(io)?;
        let mut conn = lock(&self.writer);
        f(&mut conn)
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

/// A connection that can only read the existing database file: opened
/// read-only, so a missing file is an error and never created. Only the
/// busy timeout is set; the write settings `connect` applies have nothing
/// to do on it.
pub(crate) fn connect_read_only(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sql)?;
    conn.busy_timeout(Duration::from_millis(5000))
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

fn schema_digest() -> String {
    Hash(Sha256::digest(SCHEMA.as_bytes()).into()).to_hex()
}

/// Creates the file settings and the schema under the writer queue. A
/// second process that was waiting finds the schema and changes nothing.
pub(crate) fn create(conn: &Connection, queue: &FileLock, at: &str) -> Result<(), StoreError> {
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
            "INSERT INTO schema_meta (key, value) VALUES ('epoch', ?1), ('created_at', ?2), ('schema_digest', ?3)",
            params![EPOCH, at, schema_digest()],
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

/// Runs `PRAGMA quick_check` and returns its text rows, or the error text
/// when it could not run.
fn quick_check(conn: &Connection) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("PRAGMA quick_check")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// A panic during a write leaves the connection as the unwinding
/// transaction left it: rolled back. Nothing to repair.
pub(crate) fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn sql(error: rusqlite::Error) -> StoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StoreError::Busy,
        _ => StoreError::Unavailable(error.to_string()),
    }
}

/// The database may already exist; any other failure to create it is an error.
pub(crate) fn created_or_present(created: std::io::Result<()>) -> Result<(), StoreError> {
    match created {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => Err(io(error)),
        _ => Ok(()),
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

    use baley_store::Admin;

    use super::*;

    const AT: &str = "2026-09-25T18:00:00Z";

    #[test]
    fn queue_observer_measures_only_the_wait() {
        let home = crate::checks::private_folder();
        drop(open(home.path()));
        let timing = crate::queue::scripted::Scripted::still();
        timing.script(&[Duration::ZERO, Duration::from_millis(7)]);
        let waits = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&waits);
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                timing,
                queue_wait: Some(Arc::new(move |wait| observed.lock().unwrap().push(wait))),
                ..Options::default()
            },
        )
        .unwrap();
        store.record_trace(&trace("wait")).unwrap();
        assert_eq!(*waits.lock().unwrap(), [Duration::from_millis(7)]);
    }

    #[test]
    fn no_queue_observer_consumes_no_reading() {
        let home = crate::checks::private_folder();
        let timing = crate::queue::scripted::Scripted::still();
        timing.script(&[
            Duration::from_millis(11),
            Duration::from_millis(22),
            Duration::from_millis(33),
        ]);
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                timing: timing.clone(),
                ..Options::default()
            },
        )
        .unwrap();
        store.record_trace(&trace("no wait")).unwrap();
        assert_eq!(timing.now(), Duration::from_millis(11));
    }

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
            payload: None,
            kind: kind.into(),
            data: "{}".into(),
        }
    }

    fn server() -> Options {
        Options {
            startup_check: true,
            ..Options::default()
        }
    }

    /// Breaks a `CHECK` constraint of a table open never reads. The data
    /// changes and the schema does not, so only `quick_check` sees it.
    fn damage(home: &Path) {
        let conn = raw(home);
        conn.execute_batch("PRAGMA ignore_check_constraints = ON;")
            .expect("pragma");
        conn.execute(
            "INSERT INTO project (project_id, name, created_at, head_hash) VALUES ('damaged', 'n', ?1, x'00')",
            params![AT],
        )
        .expect("damage");
    }

    // Catches a server open that never runs the check, or judges a sound
    // file unhealthy.
    #[test]
    fn a_server_open_of_a_healthy_existing_store_does_not_report_healthy() {
        let home = crate::checks::private_folder();
        drop(open(home.path()));
        let store = SqliteStore::open(home.path(), AT, server()).expect("open");
        assert_eq!(store.startup_health(), &StartupHealth::Healthy);
    }

    // Catches a check whose failure is dropped or reported without its text.
    #[test]
    fn a_server_open_of_a_damaged_store_does_not_report_the_fault() {
        let home = crate::checks::private_folder();
        drop(open(home.path()));
        damage(home.path());
        let store = SqliteStore::open(home.path(), AT, server()).expect("open");
        assert!(
            matches!(store.startup_health(), StartupHealth::Unhealthy { report } if report.contains("project")),
            "{:?}",
            store.startup_health()
        );
    }

    // Catches the check running for a command-line or guard open.
    #[test]
    fn an_open_without_the_option_runs_the_check_on_a_damaged_store() {
        let home = crate::checks::private_folder();
        drop(open(home.path()));
        damage(home.path());
        let store = open(home.path());
        assert_eq!(store.startup_health(), &StartupHealth::NotChecked);
    }

    // Catches the check running on a schema this open just created.
    #[test]
    fn a_server_open_that_creates_the_schema_runs_the_check() {
        let home = crate::checks::private_folder();
        let store = SqliteStore::open(home.path(), AT, server()).expect("open");
        assert_eq!(store.startup_health(), &StartupHealth::NotChecked);
    }

    // A second open finds the schema and keeps it: one epoch row, and the
    // creation time of the first open, not the second. Catches a schema
    // created or stamped again on every open.
    #[test]
    fn opening_twice_keeps_one_schema_and_one_epoch() {
        let home = crate::checks::private_folder();
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
        assert_eq!(rows, 3);
        assert_eq!(created_at, AT);
    }

    // Catches trace writes bypassing the epoch fence.
    #[test]
    fn a_trace_write_after_a_newer_epoch_records_nothing() {
        let home = crate::checks::private_folder();
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                timing: crate::queue::scripted::Scripted::still(),
                ..Options::default()
            },
        )
        .expect("open");
        raw(home.path())
            .execute("UPDATE schema_meta SET value = 2 WHERE key = 'epoch'", [])
            .expect("stamp");
        assert_eq!(
            store.record_trace(&trace("after")),
            Err(StoreError::ReadOnly { needed_epoch: 2 })
        );
        let rows: i64 = raw(home.path())
            .query_row("SELECT count(*) FROM trace", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 0);
    }

    // Catches a reopened store misreporting its epoch or accepting a trace write.
    #[test]
    fn a_reopened_newer_epoch_is_reported_and_fences_trace() {
        let home = crate::checks::private_folder();
        drop(
            SqliteStore::open(
                home.path(),
                AT,
                Options {
                    timing: crate::queue::scripted::Scripted::still(),
                    ..Options::default()
                },
            )
            .expect("open"),
        );
        raw(home.path())
            .execute("UPDATE schema_meta SET value = 3 WHERE key = 'epoch'", [])
            .expect("stamp");
        let store = SqliteStore::open(
            home.path(),
            AT,
            Options {
                timing: crate::queue::scripted::Scripted::still(),
                ..Options::default()
            },
        )
        .expect("open");
        assert_eq!(store.epoch(), Ok(3));
        assert_eq!(
            store.record_trace(&trace("refused")),
            Err(StoreError::ReadOnly { needed_epoch: 3 })
        );
    }

    // With every connection closed, the write-ahead log checkpointed and
    // its -wal and -shm files gone, a read-only connection still opens the
    // database and reads what was written. Catches a verification that
    // cannot open a store no other connection holds open.
    #[test]
    fn a_read_only_connection_opens_a_closed_store() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        store.record_trace(&trace("written")).expect("write");
        drop(store);
        assert!(!home.path().join("baley.db-wal").exists());
        assert!(!home.path().join("baley.db-shm").exists());
        let conn = connect_read_only(&home.path().join("baley.db")).expect("open");
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM trace", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 1);
    }

    // The settings the design names read back from both of the store's
    // connections. Catches a pragma misspelt, which SQLite ignores without
    // an error, or one set on the write connection only.
    #[test]
    fn the_connection_settings_read_back_as_set() {
        let home = crate::checks::private_folder();
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

    // Past the cap, each new diagnostic drops the oldest. Catches a trace
    // that grows without bound or drops the newest.
    #[test]
    fn the_trace_keeps_its_cap_and_drops_the_oldest() {
        let home = crate::checks::private_folder();
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
        crate::checks::private_file(&home.join("baley.db"));
        let conn = raw(home);
        conn.execute_batch(&format!("PRAGMA page_size = {page_size};"))
            .expect("page size");
        conn.pragma_update(None, "journal_mode", journal_mode)
            .expect("journal mode");
        conn.execute_batch(SCHEMA).expect("schema");
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('epoch', ?1), ('created_at', ?2), ('schema_digest', ?3)",
            params![EPOCH, AT, schema_digest()],
        )
        .expect("epoch");
    }

    // Opening a store a newer binary stamped, and left in another journal
    // mode, changes nothing in it. Catches an open that switches the log
    // or takes a write transaction before reading the epoch.
    #[test]
    fn opening_a_newer_store_leaves_it_as_it_was() {
        let home = crate::checks::private_folder();
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

    /// The digest of epoch 1's schema text before `event.caller` was added.
    /// A file stamped with it must be refused now that the text has changed.
    const DIGEST_BEFORE_EVENT_CALLER: &str =
        "505ad005e25c990f5ee90fcc17ad927fbb35cf073c0326e02a70d06d8c816141";

    fn stamp_digest(home: &Path, digest: &str) {
        raw(home)
            .execute(
                "UPDATE schema_meta SET value = ?1 WHERE key = 'schema_digest'",
                [digest],
            )
            .expect("stamp");
    }

    // A store created before `event.caller` carries the old schema digest and
    // is refused. Catches an open that skips the digest comparison, or a
    // schema edit that leaves the digest where it was.
    #[test]
    fn a_store_stamped_before_the_caller_column_is_refused() {
        let home = crate::checks::private_folder();
        drop(open(home.path()));
        stamp_digest(home.path(), DIGEST_BEFORE_EVENT_CALLER);
        let path = home.path().join("baley.db");
        let result = SqliteStore::open(home.path(), AT, Options::default());
        assert!(
            matches!(result, Err(StoreError::Refused(Refusal::SchemaChanged { path: named })) if named == path)
        );
    }

    // An export home stamped before `event.caller` is refused the same way.
    // Catches an export that is allowed to open under another digest.
    #[test]
    fn an_export_home_stamped_before_the_caller_column_is_refused() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let project = ProjectId("7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70".into());
        store.create_project(&project, "one", AT).expect("project");
        let target = home.path().join("export");
        store.export(&project, &target, AT).expect("export");
        drop(store);
        stamp_digest(&target, DIGEST_BEFORE_EVENT_CALLER);
        let result = SqliteStore::open(&target, AT, Options::default());
        assert!(
            matches!(result, Err(StoreError::Refused(Refusal::SchemaChanged { path: named })) if named == target.join("baley.db"))
        );
    }

    // A store whose epoch goes back, as when an older copy is restored
    // under a running binary, is not written to. Catches a fence that
    // refuses only newer epochs.
    #[test]
    fn an_older_epoch_refuses_the_next_write() {
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
        let home = crate::checks::private_folder();
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
