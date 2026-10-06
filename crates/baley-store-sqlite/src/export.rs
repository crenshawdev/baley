//! One project exported from one source snapshot into a private new home.

use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use baley_store::{ExportReport, ProjectId, Refusal, StoreError};
use rusqlite::{Connection, OpenFlags, params};

use crate::ledger::verify_in;
use crate::payload::sql_int;
use crate::queue::FileLock;
use crate::store::{SqliteStore, connect_read_only, create, sql};
use crate::view::Fence;

fn io(error: std::io::Error) -> StoreError {
    StoreError::Unavailable(error.to_string())
}

fn canonical_target(target: &Path) -> Result<PathBuf, StoreError> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let final_part = target
        .file_name()
        .ok_or_else(|| StoreError::Unavailable("export target has no final component".into()))?;
    Ok(parent.canonicalize().map_err(io)?.join(final_part))
}

fn uri(path: &Path) -> Result<String, StoreError> {
    let text = path
        .to_str()
        .ok_or_else(|| StoreError::Unavailable("database path is not UTF-8".into()))?;
    let mut uri = String::from("file:");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?mode=ro");
    Ok(uri)
}

impl SqliteStore {
    /// Exports with a test seam after the first source read fixes its snapshot.
    pub(crate) fn export_with(
        &self,
        project: &ProjectId,
        target: &Path,
        at: &str,
        after_first_read: impl FnOnce(),
    ) -> Result<ExportReport, StoreError> {
        self.snapshot(|conn| {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM project WHERE project_id = ?1)",
                    [&project.0],
                    |row| row.get(0),
                )
                .map_err(sql)?;
            if exists {
                Ok(())
            } else {
                Err(StoreError::Refused(Refusal::UnknownProject(
                    project.clone(),
                )))
            }
        })?;
        match fs::symlink_metadata(target) {
            Ok(_) => return Err(StoreError::Refused(Refusal::TargetExists(target.into()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io(error)),
        }
        self.read_current(project, |_, _| Ok(()))?;
        let canonical = canonical_target(target)?;
        let record: i64 = self.write(|tx| {
            tx.execute("INSERT INTO export_record (project_id, target, exported_at, head_seq) VALUES (?1, ?2, ?3, NULL)",
                params![project.0, canonical.to_string_lossy(), at]).map_err(sql)?;
            Ok(tx.last_insert_rowid())
        })?;
        let mut created = false;
        let result = self
            .make_export(project, target, at, after_first_read, &mut created)
            .and_then(|report| {
                self.write(|tx| {
                    tx.execute(
                        "UPDATE export_record SET head_seq = ?1 WHERE id = ?2",
                        params![
                            sql_int(report.head.as_ref().map_or(0, |head| head.seq))?,
                            record
                        ],
                    )
                    .map_err(sql)?;
                    Ok(())
                })?;
                Ok(report)
            });
        if result.is_err() {
            // Keep intent if either cleanup fails, so purge continues listing it.
            let removed = if !created {
                true
            } else {
                match fs::symlink_metadata(target) {
                    Ok(metadata) if metadata.file_type().is_dir() => {
                        fs::remove_dir_all(target).is_ok()
                    }
                    Ok(_) => fs::remove_file(target).is_ok(),
                    Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                }
            };
            if removed {
                let _ = self.write(|tx| {
                    tx.execute(
                        "DELETE FROM export_record WHERE id = ?1 AND head_seq IS NULL",
                        [record],
                    )
                    .map_err(sql)?;
                    Ok(())
                });
            }
        }
        result
    }

    fn make_export(
        &self,
        project: &ProjectId,
        target: &Path,
        at: &str,
        after_first_read: impl FnOnce(),
        created: &mut bool,
    ) -> Result<ExportReport, StoreError> {
        fs::create_dir(target).map_err(io)?;
        *created = true;
        fs::set_permissions(target, fs::Permissions::from_mode(0o700)).map_err(io)?;
        let path = target.join("baley.db");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(io)?;
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql)?;
        conn.execute_batch(
            "PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;",
        )
        .map_err(sql)?;
        let queue = FileLock::open(&target.join("baley.db.writer")).map_err(io)?;
        create(&conn, &queue, None, self.timing(), at)?;
        conn.execute(
            "ATTACH DATABASE ?1 AS source",
            [uri(&self.home.join("baley.db"))?],
        )
        .map_err(sql)?;
        let copy = (|| {
            conn.execute_batch("BEGIN IMMEDIATE; PRAGMA defer_foreign_keys = ON;")
                .map_err(sql)?;
            let first: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM source.project WHERE project_id = ?1",
                    [&project.0],
                    |row| row.get(0),
                )
                .map_err(sql)?;
            if first != 1 {
                return Err(StoreError::Refused(Refusal::UnknownProject(
                    project.clone(),
                )));
            }
            after_first_read();
            let live = live_views_source(&conn, project)?;
            match self.views().judge_set(&live) {
                Fence::Current(_) => {}
                Fence::Newer(reason) => {
                    return Err(StoreError::Refused(Refusal::ProjectReadOnly {
                        project: project.clone(),
                        reason,
                    }));
                }
                _ => {
                    return Err(StoreError::Unavailable(
                        "project views moved during export".into(),
                    ));
                }
            }
            conn.execute(
                "INSERT INTO main.project SELECT * FROM source.project WHERE project_id = ?1",
                [&project.0],
            )
            .map_err(sql)?;
            conn.execute(
                "INSERT INTO main.event SELECT * FROM source.event WHERE project_id = ?1",
                [&project.0],
            )
            .map_err(sql)?;
            conn.execute_batch("CREATE TEMP TABLE export_hash (hash BLOB PRIMARY KEY)")
                .map_err(sql)?;
            conn.execute("INSERT INTO export_hash SELECT hash FROM source.payload_ref WHERE project_id = ?1 UNION SELECT p.excerpt_hash FROM source.payload_ref r JOIN source.payload p ON p.hash = r.hash WHERE r.project_id = ?1 AND p.excerpt_hash IS NOT NULL", [&project.0]).map_err(sql)?;
            conn.execute("INSERT INTO main.payload (hash, bytes, encoding, body, state, excerpt_hash, excerpt_class, kept, purge_reason)
                SELECT p.hash, p.bytes, p.encoding,
                  CASE WHEN p.state = 'present' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.hash AND r.released_seq IS NULL) THEN p.body ELSE NULL END,
                  CASE WHEN p.state = 'purged' THEN 'purged'
                    WHEN p.state = 'present' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.hash AND r.released_seq IS NULL) THEN 'present'
                    WHEN p.state = 'reduced' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.excerpt_hash AND r.released_seq IS NULL) THEN 'reduced'
                    ELSE 'purged' END,
                  CASE WHEN p.state = 'reduced' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.excerpt_hash AND r.released_seq IS NULL) THEN p.excerpt_hash ELSE NULL END,
                  CASE WHEN p.state = 'reduced' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.excerpt_hash AND r.released_seq IS NULL) THEN p.excerpt_class ELSE NULL END,
                  CASE WHEN p.state = 'reduced' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.excerpt_hash AND r.released_seq IS NULL) THEN p.kept ELSE NULL END,
                  CASE WHEN p.state = 'purged' THEN p.purge_reason
                    WHEN p.state = 'present' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.hash AND r.released_seq IS NULL) THEN NULL
                    WHEN p.state = 'reduced' AND EXISTS(SELECT 1 FROM source.payload_ref r WHERE r.project_id = ?1 AND r.hash = p.excerpt_hash AND r.released_seq IS NULL) THEN NULL
                    ELSE 'released by this project' END
                FROM source.payload p JOIN export_hash h ON h.hash = p.hash", [&project.0]).map_err(sql)?;
            conn.execute("INSERT INTO main.payload_ref SELECT * FROM source.payload_ref WHERE project_id = ?1", [&project.0]).map_err(sql)?;
            conn.execute(
                "INSERT INTO main.anchor SELECT * FROM source.anchor WHERE project_id = ?1",
                [&project.0],
            )
            .map_err(sql)?;
            conn.execute("INSERT INTO main.project_gen (project_id, live_gen) SELECT project_id, live_gen FROM source.project_gen WHERE project_id = ?1", [&project.0]).map_err(sql)?;
            conn.execute("INSERT INTO main.view_gen SELECT * FROM source.view_gen WHERE project_id = ?1 AND gen = ?2", params![project.0, live.generation]).map_err(sql)?;
            self.views().create(&conn)?;
            for table in self.views().tables() {
                let name = table.physical_name();
                conn.execute(&format!("INSERT INTO main.{name} SELECT * FROM source.{name} WHERE project_id = ?1 AND generation = ?2"),
                    params![project.0, live.generation]).map_err(sql)?;
            }
            conn.execute_batch("COMMIT").map_err(sql)?;
            Ok(())
        })();
        if copy.is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        let detach = conn.execute_batch("DETACH DATABASE source").map_err(sql);
        copy?;
        detach?;
        drop(conn);
        let read = connect_read_only(&path)?;
        let report = verify_in(&read, project, None)?;
        if report.chain.first_break.is_some() || !report.payloads.is_empty() {
            return Err(StoreError::Refused(Refusal::ExportUnverified {
                project: project.clone(),
                report: Box::new(report),
            }));
        }
        Ok(ExportReport {
            target: target.into(),
            head: report.chain.head,
        })
    }
}

fn live_views_source(
    conn: &Connection,
    project: &ProjectId,
) -> Result<crate::view::LiveViews, StoreError> {
    // The attached source is fixed at the first read above.
    crate::view::live_views_attached(conn, project, "source")
}
