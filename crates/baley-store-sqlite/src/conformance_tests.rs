//! SQLite operations for the port's portable checks, confined to one directory.
use crate::payload::sql_int;
use crate::queue::scripted::Scripted;
use crate::rebuild::{event_columns, stored_event};
use crate::store::sql;

use crate::{EPOCH, Options, SqliteStore};
use baley_store::*;
use rusqlite::{Connection, params};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const AT: &str = "2026-09-25T18:00:00Z";
struct SqliteFactory {
    _directory: TempDir,
    root: PathBuf,
    next: Cell<u64>,
    timing: RefCell<Vec<Arc<Scripted>>>,
}
impl SqliteFactory {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        Self {
            _directory: directory,
            root,
            next: Cell::new(0),
            timing: RefCell::new(Vec::new()),
        }
    }
    fn inside(&self, path: &Path) {
        assert!(path.starts_with(&self.root));
        assert_ne!(path, self.root);
    }
    fn path(&self, kind: &str) -> PathBuf {
        let n = self.next.get();
        self.next.set(n + 1);
        let path = self.root.join(format!("{kind}-{n}"));
        self.inside(&path);
        path
    }
    fn open(&self, home: &Path, binary: Binary) -> Result<SqliteStore, StoreError> {
        self.inside(home);
        let timing = Scripted::still();
        let store = SqliteStore::open(
            home,
            AT,
            Options {
                projectors: binary.projectors,
                schema: binary.schema,
                view_set_version: binary.view_set_version,
                timing: timing.clone(),
                ..Options::default()
            },
        )?;
        self.timing.borrow_mut().push(timing);
        Ok(store)
    }
    fn raw(&self, store: &SqliteStore) -> Result<Connection, StoreError> {
        self.inside(&store.home);
        Connection::open(store.home.join("baley.db")).map_err(sql)
    }
}
impl StoreFactory for SqliteFactory {
    type Store = SqliteStore;
    type Snapshot = PathBuf;
    fn create(&self, binary: Binary) -> Result<SqliteStore, StoreError> {
        let home = self.path("home");
        std::fs::create_dir(&home).map_err(io)?;
        self.open(&home, binary)
    }
    fn reopen(&self, store: &SqliteStore, binary: Binary) -> Result<SqliteStore, StoreError> {
        self.inside(&store.home);
        self.open(&store.home, binary)
    }
    fn stamp_newer_epoch(&self, store: &SqliteStore) -> Result<u32, StoreError> {
        self.raw(store)?
            .execute(
                "UPDATE schema_meta SET value=?1 WHERE key='epoch'",
                [EPOCH + 1],
            )
            .map_err(sql)?;
        Ok(EPOCH + 1)
    }
    fn snapshot(&self, store: &SqliteStore) -> Result<PathBuf, StoreError> {
        let conn = self.raw(store)?;
        let busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .map_err(sql)?;
        assert_eq!(busy, 0);
        let path = self.path("snapshot");
        std::fs::copy(store.home.join("baley.db"), &path).map_err(io)?;
        Ok(path)
    }
    fn restore(&self, snapshot: &PathBuf, binary: Binary) -> Result<SqliteStore, StoreError> {
        self.inside(snapshot);
        let home = self.path("home");
        std::fs::create_dir(&home).map_err(io)?;
        std::fs::copy(snapshot, home.join("baley.db")).map_err(io)?;
        self.open(&home, binary)
    }
    fn export_target(&self) -> PathBuf {
        self.path("export")
    }
    fn open_export(&self, target: &Path, binary: Binary) -> Result<SqliteStore, StoreError> {
        self.inside(target);
        self.open(target, binary)
    }
    fn crash_rebuild(
        &self,
        store: &SqliteStore,
        project: &ProjectId,
        batches: u32,
    ) -> Result<u64, StoreError> {
        self.inside(&store.home);
        let timing = self
            .timing
            .borrow()
            .iter()
            .find(|timing| std::ptr::addr_eq(store.timing(), timing.as_ref()))
            .expect("store timing")
            .clone();
        let mut session = store.start_rebuild(project)?;
        let mut applied = 0;
        for _ in 0..batches {
            timing.script(&[
                Duration::ZERO,
                Duration::from_millis(15),
                Duration::from_millis(15),
            ]);
            applied += session.apply_one_batch()?.applied;
        }
        Ok(applied)
    }
    fn rebuild_between(
        &self,
        store: &SqliteStore,
        project: &ProjectId,
        between: &mut dyn FnMut(),
    ) -> Result<RebuildReport, StoreError> {
        self.inside(&store.home);
        let mut session = store.start_rebuild(project)?;
        session.apply_one_batch()?;
        between();
        session.finish()?.into_report(project)
    }
    fn corrupt(
        &self,
        store: &SqliteStore,
        project: &ProjectId,
        damage: Corruption,
    ) -> Result<(), StoreError> {
        let mut conn = self.raw(store)?;
        match damage {
            Corruption::CorruptBody(hash) => {
                let (body, length): (Vec<u8>, i64) = conn
                    .query_row(
                        "SELECT body,bytes FROM payload WHERE hash=?1",
                        [&hash.0[..]],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(sql)?;
                let mut bytes =
                    zstd::bulk::decompress(&body, usize::try_from(length).expect("body length"))
                        .map_err(io)?;
                assert!(!bytes.is_empty());
                bytes[0] ^= 1;
                let encoded =
                    zstd::bulk::compress(&bytes, zstd::DEFAULT_COMPRESSION_LEVEL).map_err(io)?;
                conn.execute(
                    "UPDATE payload SET body=?1 WHERE hash=?2",
                    params![encoded, &hash.0[..]],
                )
                .map_err(sql)?;
            }
            Corruption::AlterDocument { view, key, body } => {
                // Only fixture views have an integer id key.
                assert!(matches!(view.as_str(), "item" | "tally"));
                let [KeyValue::Integer(id)] = key.0.as_slice() else {
                    panic!("fixture key")
                };
                let version = if view == "item" { 2 } else { 1 };
                let query = format!(
                    "UPDATE v_{view}_{version} SET doc_json=?1 WHERE project_id=?2 AND k_id=?3 AND generation=(SELECT live_gen FROM project_gen WHERE project_id=?2)"
                );
                assert_eq!(
                    conn.execute(&query, params![body.to_string(), project.0, id])
                        .map_err(sql)?,
                    1
                );
            }
            Corruption::MarkLiveGenerationBuilding => {
                conn.execute("UPDATE project_gen SET building_gen=live_gen,building_applied_seq=0 WHERE project_id=?1",[&project.0]).map_err(sql)?;
            }
            damage => {
                let mut events = {
                    let mut statement = conn
                        .prepare(concat!(
                            "SELECT ",
                            event_columns!(),
                            " FROM event WHERE project_id=?1 ORDER BY seq"
                        ))
                        .map_err(sql)?;
                    statement
                        .query_map([&project.0], |row| stored_event(project, row))
                        .map_err(sql)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(sql)?
                };
                let mut change_head = false;
                match damage {
                    Corruption::AlterPayload { seq, payload } => {
                        events.iter_mut().find(|e| e.seq == seq).unwrap().payload = payload
                    }
                    Corruption::Insert { at, event } => {
                        for e in &mut events {
                            if e.seq >= at {
                                e.seq += 1;
                            }
                        }
                        events.push(*event);
                        events.sort_by_key(|e| e.seq);
                    }
                    Corruption::Delete { seq } => events.retain(|e| e.seq != seq),
                    Corruption::Reorder { first, second } => {
                        for e in &mut events {
                            if e.seq == first {
                                e.seq = second;
                            } else if e.seq == second {
                                e.seq = first;
                            }
                        }
                        events.sort_by_key(|e| e.seq);
                    }
                    Corruption::Truncate { keep_through } => {
                        events.retain(|e| e.seq <= keep_through);
                        change_head = true;
                    }
                    Corruption::RecomputeAfterEdit { seq, payload } => {
                        events.iter_mut().find(|e| e.seq == seq).unwrap().payload = payload;
                        let mut prev = None;
                        for e in &mut events {
                            if e.seq >= seq {
                                e.prev_hash = prev;
                                e.hash = e.compute_hash().unwrap();
                            }
                            prev = Some(e.hash);
                        }
                        change_head = true;
                    }
                    _ => unreachable!(),
                }
                let tx = conn.transaction().map_err(sql)?;
                tx.execute("DELETE FROM event WHERE project_id=?1", [&project.0])
                    .map_err(sql)?;
                for e in &events {
                    tx.execute("INSERT INTO event (project_id,seq,stream,stream_version,type,type_version,actor,recorded_at,request_id,git_commit,git_tree,git_checkout,policy_version,payload_json,prev_hash,hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![e.project_id.0,sql_int(e.seq)?,e.stream,sql_int(e.stream_version)?,e.type_name,e.type_version,e.actor.as_str(),e.recorded_at,e.request_id.0,e.git.as_ref().map(|g|&g.commit),e.git.as_ref().map(|g|&g.tree),e.git.as_ref().map(|g|&g.checkout),sql_int(e.policy_version)?,e.payload.to_string(),e.prev_hash.as_ref().map(|h|&h.0[..]),&e.hash.0[..]]).map_err(sql)?;
                }
                if change_head {
                    let head = events.last();
                    tx.execute(
                        "UPDATE project SET head_seq=?1,head_hash=?2 WHERE project_id=?3",
                        params![
                            sql_int(head.map_or(0, |e| e.seq))?,
                            head.map(|e| &e.hash.0[..]),
                            project.0
                        ],
                    )
                    .map_err(sql)?;
                }
                tx.commit().map_err(sql)?;
            }
        }
        Ok(())
    }
}

fn io(error: std::io::Error) -> StoreError {
    StoreError::Unavailable(error.to_string())
}

baley_store::conformance_suite!(SqliteFactory::new());
