//! Reference release, payload tombstones and the standalone scrub.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use baley_store::{
    Answer, Command, EXCERPT_EDGE, Hash, NewEvent, Outcome, OutcomeKind, PAYLOAD_PURGED,
    PAYLOAD_PURGED_VERSION, PAYLOAD_REDUCED, PAYLOAD_REDUCED_VERSION, PayloadRef, PayloadReference,
    ProjectId, PurgeReport, PurgedEvent, RETENTION_STREAM, ReducedEvent, Refusal, RetentionClass,
    ScrubReport, StoreError, StreamName, kept_ranges,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::payload::{sql_int, stream_in};
use crate::schema::EPOCH;
use crate::store::{SqliteStore, sql};
use crate::transact::CommandResult;

const SCRUB_ATTEMPTS: u8 = 3;
const REDUCTION_LOOKUP: &str = "SELECT seq, payload_json FROM event INDEXED BY event_type
    WHERE project_id = ?1 AND type = ?2 ORDER BY seq";
const EXCERPT_LOOKUP: &str =
    "SELECT hash FROM payload WHERE state = 'reduced' AND excerpt_hash = ?1";
const NON_PRESENT_LOOKUP: &str =
    "SELECT hash, state, purge_reason FROM payload WHERE state != 'present'";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckpointStep {
    Done,
    Retry,
    Incomplete,
}

/// Judges the busy flag in a checkpoint result row, regardless of its counts.
pub(crate) fn checkpoint_step(row: (i64, i64, i64), attempt: u8) -> CheckpointStep {
    if row.0 == 0 {
        CheckpointStep::Done
    } else if attempt < SCRUB_ATTEMPTS {
        CheckpointStep::Retry
    } else {
        CheckpointStep::Incomplete
    }
}

impl SqliteStore {
    /// Reduces one live output reference to its first and last 64 KiB.
    pub fn reduce(
        &self,
        command: &Command,
        reference: &PayloadReference,
    ) -> Result<PayloadRef, StoreError> {
        let result = self.command_path(
            command,
            |tx, outcome, _| {
                let seq = event_answer(&outcome)?;
                let value = retention_event(tx, &command.project, seq, PAYLOAD_REDUCED)?;
                Ok(ReducedEvent::from_value(&value)
                    .ok_or_else(|| malformed("reduction"))?
                    .excerpt)
            },
            |work| {
                let tx = work.tx;
                let invalid = || StoreError::Refused(Refusal::NotReducible(reference.clone()));
                if reference.project != command.project {
                    return Err(invalid());
                }
                let row = tx
                    .query_row(
                        "SELECT r.class, r.released_seq, p.state, p.bytes, e.payload_json
                 FROM payload_ref r JOIN payload p ON p.hash = r.hash
                 JOIN event e ON e.project_id = r.project_id AND e.seq = r.seq
                 WHERE r.project_id = ?1 AND r.seq = ?2 AND r.hash = ?3",
                        params![
                            command.project.0,
                            sql_int(reference.seq)?,
                            &reference.hash.0[..]
                        ],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(sql)?
                    .ok_or_else(invalid)?;
                let bytes = u64::try_from(row.3).map_err(|_| malformed("payload length"))?;
                let ranges = kept_ranges(bytes).ok_or_else(invalid)?;
                if row.0 != "output" || row.1.is_some() || row.2 != "present" {
                    return Err(invalid());
                }
                let event: Value =
                    serde_json::from_str(&row.4).map_err(|_| malformed("reference event"))?;
                let carried = reference_bytes(&event, &reference.hash)
                    .ok_or_else(|| malformed("reference event"))?;
                let (stored_bytes, mut reader) = stream_in(tx, &reference.hash)?;
                let mut digest = Sha256::new();
                let mut count = 0u64;
                let mut first = Vec::with_capacity(EXCERPT_EDGE as usize);
                let mut last = VecDeque::with_capacity(EXCERPT_EDGE as usize);
                let mut chunk = [0u8; 8192];
                loop {
                    let read = reader
                        .read(&mut chunk)
                        .map_err(|_| body_mismatch(&reference.hash))?;
                    if read == 0 {
                        break;
                    }
                    digest.update(&chunk[..read]);
                    count = count
                        .checked_add(read as u64)
                        .ok_or_else(|| body_mismatch(&reference.hash))?;
                    for &byte in &chunk[..read] {
                        if first.len() < EXCERPT_EDGE as usize {
                            first.push(byte);
                        }
                        if last.len() == EXCERPT_EDGE as usize {
                            last.pop_front();
                        }
                        last.push_back(byte);
                    }
                }
                drop(reader);
                if count != stored_bytes
                    || count != bytes
                    || count != carried
                    || digest.finalize().as_slice() != reference.hash.0
                {
                    return Err(body_mismatch(&reference.hash));
                }
                first.extend(last);
                let excerpt = work.put(&first, RetentionClass::Record)?;
                let reduced = ReducedEvent {
                    seq: reference.seq,
                    original: reference.hash,
                    original_bytes: count,
                    excerpt: excerpt.clone(),
                    kept: ranges.clone(),
                };
                let seq = work.push(NewEvent {
                    stream: StreamName(RETENTION_STREAM.into()),
                    type_name: PAYLOAD_REDUCED.into(),
                    type_version: PAYLOAD_REDUCED_VERSION,
                    git: None,
                    payload: reduced.to_value(),
                    attachments: vec![excerpt.clone()],
                })?;
                tx.execute(
                    "UPDATE payload_ref SET released_seq = ?1
                 WHERE project_id = ?2 AND seq = ?3 AND hash = ?4",
                    params![
                        sql_int(seq)?,
                        command.project.0,
                        sql_int(reference.seq)?,
                        &reference.hash.0[..]
                    ],
                )
                .map_err(sql)?;
                if !required(tx, &reference.hash)? {
                    tx.execute(
                        "UPDATE payload SET state = 'reduced', body = NULL, excerpt_hash = ?1,
                    excerpt_class = 'record', kept = ?2 WHERE hash = ?3",
                        params![
                            &excerpt.hash.0[..],
                            serde_json::to_string(&json!([
                                [ranges[0].start, ranges[0].end],
                                [ranges[1].start, ranges[1].end]
                            ]))
                            .map_err(|_| malformed("kept ranges"))?,
                            &reference.hash.0[..]
                        ],
                    )
                    .map_err(sql)?;
                }
                let outcome = inline_event(seq);
                Ok((excerpt, outcome, None, None))
            },
        )?;
        Ok(match result {
            CommandResult::Replayed(value) | CommandResult::New(value, _) => value,
        })
    }

    /// Releases this project's references to the named hashes and scrubs the database.
    pub fn purge(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
    ) -> Result<PurgeReport, StoreError> {
        self.purge_with(command, hashes, reason, || self.scrub())
    }

    pub(crate) fn purge_with(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
        scrub: impl FnOnce() -> Result<ScrubReport, StoreError>,
    ) -> Result<PurgeReport, StoreError> {
        let (seq, event) = self.logical_purge(command, hashes, reason)?;
        Ok(self.finish_purge(command, seq, event, scrub()))
    }

    fn finish_purge(
        &self,
        command: &Command,
        seq: u64,
        event: PurgedEvent,
        scrub: Result<ScrubReport, StoreError>,
    ) -> PurgeReport {
        let scrub = scrub.unwrap_or_else(|_| {
            let backups = self.home.join("backups");
            ScrubReport {
                scrubbed: false,
                unreachable: if fs::symlink_metadata(&backups).is_ok() {
                    vec![backups]
                } else {
                    Vec::new()
                },
            }
        });
        PurgeReport {
            purged: event.removed,
            shared: event.shared,
            recorded: vec![(command.project.clone(), seq)],
            unreachable: scrub.unreachable,
            scrubbed: scrub.scrubbed,
        }
    }

    /// Commits only the logical removal, leaving its durable marker for the scrub.
    pub(crate) fn logical_purge(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
    ) -> Result<(u64, PurgedEvent), StoreError> {
        let result = self.command_path(
            command,
            |tx, outcome, _| {
                let seq = event_answer(&outcome)?;
                let value = retention_event(tx, &command.project, seq, PAYLOAD_PURGED)?;
                Ok((
                    seq,
                    PurgedEvent::from_value(&value).ok_or_else(|| malformed("purge"))?,
                ))
            },
            |work| {
                let tx = work.tx;
                if hashes.is_empty() {
                    return Err(StoreError::Refused(Refusal::InvalidEvent(
                        "a purge names no hash".into(),
                    )));
                }
                let requested: BTreeSet<Hash> = hashes.iter().copied().collect();
                let mut release = BTreeSet::<(u64, Hash)>::new();
                let mut released_hashes = BTreeSet::<Hash>::new();
                for hash in &requested {
                    let mut own = BTreeSet::new();
                    let mut statement = tx
                        .prepare(
                            "SELECT seq FROM payload_ref WHERE project_id = ?1 AND hash = ?2
                     AND released_seq IS NULL",
                        )
                        .map_err(sql)?;
                    let rows = statement
                        .query_map(params![command.project.0, &hash.0[..]], |row| {
                            row.get::<_, i64>(0)
                        })
                        .map_err(sql)?;
                    for row in rows {
                        own.insert((unsigned(row.map_err(sql)?)?, *hash));
                    }
                    // The index narrows by project and type; JSON identifies this original.
                    let mut statement = tx.prepare(REDUCTION_LOOKUP).map_err(sql)?;
                    let rows = statement
                        .query_map(params![command.project.0, PAYLOAD_REDUCED], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                        })
                        .map_err(sql)?;
                    for row in rows {
                        let (seq, body) = row.map_err(sql)?;
                        let value: Value = serde_json::from_str(&body)
                            .map_err(|_| malformed("reduction event"))?;
                        let reduction = ReducedEvent::from_value(&value)
                            .ok_or_else(|| malformed("reduction event"))?;
                        if reduction.original != *hash {
                            continue;
                        }
                        let exists = tx
                            .query_row(
                                "SELECT 1 FROM payload_ref WHERE project_id = ?1 AND seq = ?2
                         AND hash = ?3 AND released_seq IS NULL",
                                params![command.project.0, seq, &reduction.excerpt.hash.0[..]],
                                |_| Ok(()),
                            )
                            .optional()
                            .map_err(sql)?
                            .is_some();
                        if exists {
                            own.insert((unsigned(seq)?, reduction.excerpt.hash));
                        }
                    }
                    if own.is_empty() {
                        return Err(StoreError::Refused(Refusal::NothingToPurge(*hash)));
                    }
                    for (_, hash) in &own {
                        released_hashes.insert(*hash);
                    }
                    release.extend(own);
                }
                let seq = work.next_seq();
                for (source, hash) in &release {
                    tx.execute(
                        "UPDATE payload_ref SET released_seq = ?1
                     WHERE project_id = ?2 AND seq = ?3 AND hash = ?4
                     AND released_seq IS NULL",
                        params![
                            sql_int(seq)?,
                            command.project.0,
                            sql_int(*source)?,
                            &hash.0[..]
                        ],
                    )
                    .map_err(sql)?;
                }
                let mut removed = BTreeSet::<Hash>::new();
                let mut pending: Vec<Hash> = released_hashes.iter().copied().collect();
                while let Some(hash) = pending.pop() {
                    if removed.contains(&hash) || required(tx, &hash)? {
                        continue;
                    }
                    let row = tx
                        .query_row(
                            "SELECT state, excerpt_hash FROM payload WHERE hash = ?1",
                            [&hash.0[..]],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
                        )
                        .optional()
                        .map_err(sql)?;
                    match row {
                        Some((state, _)) if state == "present" => {
                            tombstone(tx, &hash, reason)?;
                            removed.insert(hash);
                            let mut statement = tx.prepare(EXCERPT_LOOKUP).map_err(sql)?;
                            let rows = statement
                                .query_map([&hash.0[..]], |row| row.get::<_, Vec<u8>>(0))
                                .map_err(sql)?;
                            for row in rows {
                                pending.push(blob_hash(row.map_err(sql)?)?);
                            }
                        }
                        Some((state, Some(excerpt))) if state == "reduced" => {
                            if removed.contains(&blob_hash(excerpt)?) {
                                tombstone(tx, &hash, reason)?;
                                removed.insert(hash);
                            }
                        }
                        _ => {}
                    }
                }
                let mut shared: BTreeSet<_> =
                    released_hashes.difference(&removed).copied().collect();
                for hash in &requested {
                    if !removed.contains(hash) {
                        let state: String = tx
                            .query_row(
                                "SELECT state FROM payload WHERE hash = ?1",
                                [&hash.0[..]],
                                |row| row.get(0),
                            )
                            .map_err(sql)?;
                        if state == "reduced" {
                            shared.insert(*hash);
                        }
                    }
                }
                let trace_candidates = released_hashes.union(&requested).copied().collect();
                remove_derived(tx, &command.project, &trace_candidates, &removed)?;
                let event = PurgedEvent {
                    requested: requested.into_iter().collect(),
                    released: release.into_iter().collect(),
                    removed: removed.into_iter().collect(),
                    shared: shared.into_iter().collect(),
                    reason: reason.into(),
                };
                let written = work.push(NewEvent {
                    stream: StreamName(RETENTION_STREAM.into()),
                    type_name: PAYLOAD_PURGED.into(),
                    type_version: PAYLOAD_PURGED_VERSION,
                    git: None,
                    payload: event.to_value(),
                    attachments: Vec::new(),
                })?;
                debug_assert_eq!(seq, written);
                tx.execute(
                    "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('scrub_pending', ?1)",
                    [&command.recorded_at],
                )
                .map_err(sql)?;
                Ok(((written, event), inline_event(written), None, None))
            },
        )?;
        let (seq, event) = match result {
            CommandResult::Replayed(value) | CommandResult::New(value, _) => value,
        };
        Ok((seq, event))
    }

    /// Repeats the main database and home-backup scrub without a request id.
    pub fn scrub(&self) -> Result<ScrubReport, StoreError> {
        self.maintenance(|conn| {
            check_epoch(conn)?;
            let _ = checkpoint(conn, "PASSIVE");
            let vacuum_ok = conn.execute_batch("VACUUM").is_ok();
            let mut step = CheckpointStep::Incomplete;
            for attempt in 1..=SCRUB_ATTEMPTS {
                step = match checkpoint(conn, "TRUNCATE") {
                    Ok(row) => checkpoint_step(row, attempt),
                    Err(_) if attempt < SCRUB_ATTEMPTS => CheckpointStep::Retry,
                    Err(_) => CheckpointStep::Incomplete,
                };
                if step != CheckpointStep::Retry {
                    break;
                }
            }
            let main_step = if vacuum_ok {
                step
            } else {
                CheckpointStep::Incomplete
            };
            let scrubbed = settle_scrub(conn, main_step)?;
            let tombstones = live_tombstones(conn)?;
            let unreachable = rewrite_backups(&self.home, &tombstones);
            Ok(ScrubReport {
                scrubbed,
                unreachable,
            })
        })
    }
}

fn malformed(what: &str) -> StoreError {
    StoreError::Unavailable(format!("a malformed {what}"))
}

fn body_mismatch(hash: &Hash) -> StoreError {
    StoreError::Unavailable(format!(
        "payload {hash}: its body does not match its hash or length"
    ))
}

fn unsigned(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| malformed("sequence"))
}

fn blob_hash(bytes: Vec<u8>) -> Result<Hash, StoreError> {
    Ok(Hash(bytes.try_into().map_err(|_| malformed("hash"))?))
}

fn reference_bytes(value: &Value, hash: &Hash) -> Option<u64> {
    if let Some(reference) = PayloadRef::from_value(value) {
        return (reference.hash == *hash).then_some(reference.bytes);
    }
    match value {
        Value::Array(items) => items.iter().find_map(|item| reference_bytes(item, hash)),
        Value::Object(fields) => fields.values().find_map(|item| reference_bytes(item, hash)),
        _ => None,
    }
}

fn required(tx: &rusqlite::Transaction<'_>, hash: &Hash) -> Result<bool, StoreError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM payload_ref WHERE hash = ?1 AND released_seq IS NULL LIMIT 1",
            [&hash.0[..]],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?
        .is_some())
}

fn required_by_project(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    hash: &Hash,
) -> Result<bool, StoreError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM payload_ref WHERE project_id = ?1 AND hash = ?2 \
             AND released_seq IS NULL LIMIT 1",
            params![project.0, &hash.0[..]],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?
        .is_some())
}

fn tombstone(tx: &rusqlite::Transaction<'_>, hash: &Hash, reason: &str) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE payload SET state = 'purged', body = NULL, excerpt_hash = NULL,
        excerpt_class = NULL, kept = NULL, purge_reason = ?1 WHERE hash = ?2",
        params![reason, &hash.0[..]],
    )
    .map_err(sql)?;
    Ok(())
}

/// The derived-data seam that the search slice extends.
pub(crate) fn remove_derived(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    candidates: &BTreeSet<Hash>,
    removed: &BTreeSet<Hash>,
) -> Result<(), StoreError> {
    for hash in removed {
        tx.execute("DELETE FROM trace WHERE payload_hash = ?1", [&hash.0[..]])
            .map_err(sql)?;
    }
    for hash in candidates {
        if required_by_project(tx, project, hash)? {
            continue;
        }
        tx.execute(
            "DELETE FROM trace WHERE project_id = ?1 AND payload_hash = ?2",
            params![project.0, &hash.0[..]],
        )
        .map_err(sql)?;
    }
    Ok(())
}

fn inline_event(seq: u64) -> Outcome {
    Outcome {
        kind: OutcomeKind::Done,
        answer: Answer::Inline(json!({"event": seq})),
    }
}

fn event_answer(outcome: &Outcome) -> Result<u64, StoreError> {
    if let Answer::Inline(value) = &outcome.answer
        && let Some(seq) = value.get("event").and_then(Value::as_u64)
    {
        return Ok(seq);
    }
    Err(malformed("retention answer"))
}

fn retention_event(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    seq: u64,
    expected: &str,
) -> Result<Value, StoreError> {
    let (kind, body): (String, String) = tx
        .query_row(
            "SELECT type, payload_json FROM event WHERE project_id = ?1 AND seq = ?2",
            params![project.0, sql_int(seq)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(sql)?;
    if kind != expected {
        return Err(malformed("retention event type"));
    }
    serde_json::from_str(&body).map_err(|_| malformed("retention event"))
}

fn checkpoint(conn: &Connection, mode: &str) -> Result<(i64, i64, i64), StoreError> {
    conn.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
    .map_err(sql)
}

fn check_epoch(conn: &Connection) -> Result<(), StoreError> {
    let epoch: u32 = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'epoch'",
            [],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if epoch > EPOCH {
        return Err(StoreError::ReadOnly {
            needed_epoch: epoch,
        });
    }
    if epoch < EPOCH {
        return Err(malformed("compatibility epoch"));
    }
    Ok(())
}

/// Clears the marker only after a completed main-database checkpoint.
pub(crate) fn settle_scrub(
    conn: &mut Connection,
    step: CheckpointStep,
) -> Result<bool, StoreError> {
    if step != CheckpointStep::Done {
        return Ok(false);
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    check_epoch(&tx)?;
    tx.execute("DELETE FROM schema_meta WHERE key = 'scrub_pending'", [])
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(true)
}

fn live_tombstones(conn: &Connection) -> Result<BTreeMap<Hash, String>, StoreError> {
    let mut statement = conn.prepare(NON_PRESENT_LOOKUP).map_err(sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(sql)?;
    rows.map(|row| {
        let (hash, state, reason) = row.map_err(sql)?;
        let reason = if state == "reduced" {
            "reduced in live store".to_owned()
        } else {
            reason.ok_or_else(|| malformed("purge reason"))?
        };
        Ok((blob_hash(hash)?, reason))
    })
    .collect()
}

fn rewrite_backups(home: &Path, tombstones: &BTreeMap<Hash, String>) -> Vec<PathBuf> {
    let dir = home.join("backups");
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => return vec![dir],
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => return vec![dir],
    }
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => return vec![dir],
    };
    let mut unreachable = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                unreachable.push(dir.clone());
                continue;
            }
        };
        let path = entry.path();
        if !entry.file_name().to_string_lossy().ends_with(".db") {
            continue;
        }
        let regular = fs::symlink_metadata(&path)
            .map(|meta| meta.file_type().is_file())
            .unwrap_or(false);
        if !regular || rewrite_backup(&path, tombstones).is_err() {
            unreachable.push(path);
        }
    }
    unreachable.sort();
    unreachable
}

fn rewrite_backup(path: &Path, tombstones: &BTreeMap<Hash, String>) -> Result<(), StoreError> {
    let mut conn = Connection::open(path).map_err(sql)?;
    conn.execute_batch("PRAGMA secure_delete = ON")
        .map_err(sql)?;
    let secure: i64 = conn
        .query_row("PRAGMA secure_delete", [], |row| row.get(0))
        .map_err(sql)?;
    if secure != 1 {
        return Err(malformed("backup secure_delete"));
    }
    let has_trace_hash: bool = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info('trace') WHERE name = 'payload_hash'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql)?
        .is_some();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    for (hash, reason) in tombstones {
        tx.execute(
            "UPDATE payload SET state = 'purged', body = NULL, excerpt_hash = NULL,
            excerpt_class = NULL, kept = NULL, purge_reason = ?1
            WHERE hash = ?2 AND state IN ('present', 'reduced')",
            params![reason, &hash.0[..]],
        )
        .map_err(sql)?;
        if has_trace_hash {
            tx.execute("DELETE FROM trace WHERE payload_hash = ?1", [&hash.0[..]])
                .map_err(sql)?;
        }
    }
    tx.commit().map_err(sql)?;
    conn.execute_batch("VACUUM").map_err(sql)?;
    let mut step = CheckpointStep::Incomplete;
    for attempt in 1..=SCRUB_ATTEMPTS {
        step = checkpoint_step(checkpoint(&conn, "TRUNCATE")?, attempt);
        if step != CheckpointStep::Retry {
            break;
        }
    }
    if !has_trace_hash || step != CheckpointStep::Done {
        return Err(malformed("backup scrub"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Options, TraceEntry};
    use baley_store::{
        Actor, Answer, CommandKind, Decision, DocKey, Event, EventSchema, KeyValue, Observed,
        PayloadBody, PayloadStatus, Payloads, REQUEST_VIEW, Recorded, RequestId, Views,
        verify_chain,
    };
    use std::io::Read;

    const AT: &str = "2026-09-25T18:00:00Z";
    const RECORDED: &str = "fixture.recorded";

    struct Fixture;
    impl EventSchema for Fixture {
        fn reads(&self, name: &str, version: u32) -> bool {
            name == RECORDED && version == 1
        }
    }

    fn open(home: &Path, projects: &[&str]) -> SqliteStore {
        let store = SqliteStore::open(
            home,
            AT,
            Options {
                schema: Box::new(Fixture),
                ..Options::default()
            },
        )
        .expect("open");
        store
            .write(|tx| {
                for project in projects {
                    tx.execute(
                        "INSERT INTO project (project_id, name, created_at) VALUES (?1, ?1, ?2)",
                        params![project, AT],
                    )
                    .map_err(sql)?;
                }
                Ok(())
            })
            .expect("projects");
        store
    }

    fn command(project: &str, kind: &str, request: &str) -> Command {
        Command {
            project: ProjectId(project.into()),
            kind: CommandKind(kind.into()),
            request_id: RequestId(request.into()),
            digest: Hash([7; 32]),
            policy_version: 1,
            recorded_at: AT.into(),
            actor: Actor::Owner,
        }
    }

    fn done(answer: Value, sensitive: bool) -> Decision {
        Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive,
            observed: Observed::default(),
            git: None,
        }
    }

    fn body(len: usize, middle: u8) -> Vec<u8> {
        let mut body = vec![middle; len];
        body[..65_536].fill(1);
        body[len - 65_536..].fill(2);
        body
    }

    fn attach(
        store: &SqliteStore,
        project: &str,
        request: &str,
        bytes: &[u8],
        class: RetentionClass,
    ) -> (PayloadReference, PayloadRef) {
        let mut attached = None;
        store
            .transact(&command(project, "fixture.attach", request), &mut |tx| {
                let reference = tx.put_payload(bytes, class)?;
                let seq = tx.append(NewEvent {
                    stream: StreamName("fixture".into()),
                    type_name: RECORDED.into(),
                    type_version: 1,
                    git: None,
                    payload: json!({"body": reference.to_value()}),
                    attachments: vec![reference.clone()],
                })?;
                attached = Some((
                    PayloadReference {
                        project: ProjectId(project.into()),
                        seq,
                        hash: reference.hash,
                    },
                    reference,
                ));
                Ok(done(json!("ok"), false))
            })
            .expect("attach");
        attached.expect("attached")
    }

    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw")
    }

    fn head(home: &Path, project: &str) -> i64 {
        raw(home)
            .query_row(
                "SELECT head_seq FROM project WHERE project_id = ?1",
                [project],
                |row| row.get(0),
            )
            .expect("head")
    }

    fn read_body(store: &SqliteStore, hash: &Hash) -> Vec<u8> {
        let PayloadBody::Present(mut reader) = store.open(hash).expect("open payload") else {
            panic!("body absent");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).expect("read");
        bytes
    }

    fn reduce(
        store: &SqliteStore,
        project: &str,
        request: &str,
        reference: &PayloadReference,
    ) -> PayloadRef {
        store
            .reduce(&command(project, "retention.reduce", request), reference)
            .expect("reduce")
    }

    fn purge(store: &SqliteStore, project: &str, request: &str, hash: Hash) -> PurgeReport {
        store
            .purge(
                &command(project, "retention.purge", request),
                &[hash],
                "owner request",
            )
            .expect("purge")
    }

    fn trace(store: &SqliteStore, project: &str, hash: Hash, kind: &str) {
        store
            .record_trace(&TraceEntry {
                at: AT.into(),
                project: Some(ProjectId(project.into())),
                payload: Some(hash),
                kind: kind.into(),
                data: "{}".into(),
            })
            .expect("trace");
    }

    fn kinds(home: &Path) -> Vec<String> {
        let conn = raw(home);
        let mut statement = conn
            .prepare("SELECT kind FROM trace ORDER BY id")
            .expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("kinds")
    }

    fn events(home: &Path, project: &str) -> Vec<Event> {
        let conn = raw(home);
        let mut statement = conn
            .prepare(
                "SELECT seq, stream, stream_version, type, type_version, actor, recorded_at,
                request_id, policy_version, payload_json, prev_hash, hash
             FROM event WHERE project_id = ?1 ORDER BY seq",
            )
            .expect("prepare");
        statement
            .query_map([project], |row| {
                let hash = |bytes: Vec<u8>| Hash(bytes.try_into().expect("hash"));
                Ok(Event {
                    project_id: ProjectId(project.into()),
                    seq: row.get::<_, i64>(0)? as u64,
                    stream: row.get(1)?,
                    stream_version: row.get::<_, i64>(2)? as u64,
                    type_name: row.get(3)?,
                    type_version: row.get(4)?,
                    actor: Actor::parse(&row.get::<_, String>(5)?).expect("actor"),
                    recorded_at: row.get(6)?,
                    request_id: RequestId(row.get(7)?),
                    git: None,
                    policy_version: row.get::<_, i64>(8)? as u64,
                    payload: serde_json::from_str(&row.get::<_, String>(9)?).expect("json"),
                    prev_hash: row.get::<_, Option<Vec<u8>>>(10)?.map(hash),
                    hash: hash(row.get(11)?),
                })
            })
            .expect("events")
            .collect::<rusqlite::Result<_>>()
            .expect("events")
    }

    fn plan(conn: &Connection, query: &str, values: &[&dyn rusqlite::ToSql]) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .expect("plan prepare");
        statement
            .query_map(values, |row| row.get(3))
            .expect("plan query")
            .collect::<rusqlite::Result<_>>()
            .expect("plan rows")
    }

    // Catches the reduction event lookup walking every event of a project.
    #[test]
    fn the_purge_uses_the_event_type_index() {
        let home = tempfile::tempdir().expect("home");
        let _store = open(home.path(), &["a"]);
        let steps = plan(
            &raw(home.path()),
            REDUCTION_LOOKUP,
            &[&"a", &PAYLOAD_REDUCED],
        );
        assert!(
            steps.iter().any(|step| step.contains("event_type")),
            "{steps:?}"
        );
    }

    // Catches the excerpt dependency lookup scanning every payload row.
    #[test]
    fn the_purge_uses_the_excerpt_index() {
        let home = tempfile::tempdir().expect("home");
        let _store = open(home.path(), &["a"]);
        let hash = [0u8; 32];
        let steps = plan(&raw(home.path()), EXCERPT_LOOKUP, &[&&hash[..]]);
        assert!(
            steps.iter().any(|step| step.contains("payload_excerpt")),
            "{steps:?}"
        );
    }

    // Catches the scrub scanning all present payloads to find tombstones.
    #[test]
    fn the_scrub_uses_the_non_present_index() {
        let home = tempfile::tempdir().expect("home");
        let _store = open(home.path(), &["a"]);
        let steps = plan(&raw(home.path()), NON_PRESENT_LOOKUP, &[]);
        assert!(
            steps
                .iter()
                .any(|step| step.contains("payload_non_present")),
            "{steps:?}"
        );
    }

    // Catches a purge that deletes a body another project still requires.
    #[test]
    fn a_shared_body_survives_one_projects_purge() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let bytes = b"shared body";
        let (a, _) = attach(&store, "a", "a1", bytes, RetentionClass::Material);
        attach(&store, "b", "b1", bytes, RetentionClass::Material);
        let b_head = head(home.path(), "b");
        let report = purge(&store, "a", "p1", a.hash);
        assert!(report.purged.is_empty());
        assert_eq!(report.shared, vec![a.hash]);
        assert_eq!(
            store.status(&a.hash),
            Ok(PayloadStatus::Present {
                bytes: bytes.len() as u64
            })
        );
        assert_eq!(head(home.path(), "b"), b_head);
    }

    // Catches a removal that rewrites the event chain or loses its tombstone.
    #[test]
    fn a_purged_chain_still_verifies() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"private", RetentionClass::Material);
        purge(&store, "a", "p1", reference.hash);
        assert!(verify_chain(&events(home.path(), "a"), None).is_intact());
        assert_eq!(
            store.status(&reference.hash),
            Ok(PayloadStatus::Purged {
                reason: "owner request".into()
            })
        );
    }

    // Catches a wrong edge, wrong excerpt hash, or absent reduction tombstone.
    #[test]
    fn reduction_keeps_the_first_and_last_64_kib() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let body = body(300_000, 3);
        let (original, _) = attach(&store, "a", "a1", &body, RetentionClass::Output);
        let excerpt = reduce(&store, "a", "r1", &original);
        let mut expected = body[..65_536].to_vec();
        expected.extend_from_slice(&body[234_464..]);
        assert_eq!(read_body(&store, &excerpt.hash), expected);
        let reduction = events(home.path(), "a")
            .into_iter()
            .find(|event| event.type_name == PAYLOAD_REDUCED)
            .expect("reduction");
        let payload = ReducedEvent::from_value(&reduction.payload).expect("payload");
        assert_eq!(payload.original, original.hash);
        assert_eq!(payload.excerpt, excerpt);
        assert_eq!(payload.kept, [0..65_536, 234_464..300_000]);
        assert_eq!(
            store.status(&original.hash),
            Ok(PayloadStatus::Reduced {
                excerpt,
                kept: vec![0..65_536, 234_464..300_000]
            })
        );
    }

    // Catches a checkpoint judged by a successful pragma call or equal counts.
    #[test]
    fn the_checkpoint_decision_judges_the_busy_flag() {
        assert_eq!(checkpoint_step((1, 12, 4), 1), CheckpointStep::Retry);
        assert_eq!(checkpoint_step((1, 0, 0), 1), CheckpointStep::Retry);
        assert_eq!(checkpoint_step((0, 0, 0), 1), CheckpointStep::Done);
    }

    // Catches retrying forever or treating a blocked last attempt as done.
    #[test]
    fn the_scrub_gives_up_after_its_last_attempt() {
        assert_eq!(checkpoint_step((1, 12, 4), 3), CheckpointStep::Incomplete);
    }

    // Catches an offset time entering the event chain where retention cannot parse it.
    #[test]
    fn a_command_with_an_offset_time_is_refused() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let mut command = command("a", "fixture", "a1");
        command.recorded_at = "2026-09-25T18:00:00+00:00".into();
        assert!(matches!(
            store.transact(&command, &mut |_| Ok(done(json!("ok"), false))),
            Err(StoreError::Refused(Refusal::InvalidEvent(_)))
        ));
        assert_eq!(head(home.path(), "a"), 0);
    }

    // Catches reducing a record or moving the head on that refusal.
    #[test]
    fn only_an_output_reference_is_reduced() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Record);
        let before = head(home.path(), "a");
        assert_eq!(
            store.reduce(&command("a", "retention.reduce", "r1"), &reference),
            Err(StoreError::Refused(Refusal::NotReducible(reference)))
        );
        assert_eq!(head(home.path(), "a"), before);
    }

    // Catches reduction tombstoning a body a second reference needs whole.
    #[test]
    fn a_reduction_leaves_the_body_whole_for_another_reference() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let body = body(300_000, 3);
        let (first, _) = attach(&store, "a", "a1", &body, RetentionClass::Output);
        attach(&store, "a", "a2", &body, RetentionClass::Output);
        let excerpt = reduce(&store, "a", "r1", &first);
        assert!(matches!(
            store.status(&excerpt.hash),
            Ok(PayloadStatus::Present { .. })
        ));
        assert_eq!(
            store.status(&first.hash),
            Ok(PayloadStatus::Present { bytes: 300_000 })
        );
        assert_eq!(read_body(&store, &first.hash), body);
    }

    // Catches corrupt bytes being hidden by a reduction tombstone.
    #[test]
    fn a_reduction_refuses_a_body_that_fails_its_hash() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let corrupt = zstd::bulk::compress(&body(300_000, 4), zstd::DEFAULT_COMPRESSION_LEVEL)
            .expect("compress");
        raw(home.path())
            .execute(
                "UPDATE payload SET body = ?1 WHERE hash = ?2",
                params![corrupt, &reference.hash.0[..]],
            )
            .expect("corrupt");
        let before = head(home.path(), "a");
        assert!(matches!(
            store.reduce(&command("a", "retention.reduce", "r1"), &reference),
            Err(StoreError::Unavailable(_))
        ));
        assert_eq!(head(home.path(), "a"), before);
        assert_eq!(
            store.status(&reference.hash),
            Ok(PayloadStatus::Present { bytes: 300_000 })
        );
    }

    // Catches ranges recorded against a length unlike the decoded body.
    #[test]
    fn a_reduction_refuses_a_body_of_another_length() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        raw(home.path())
            .execute(
                "UPDATE payload SET bytes = bytes - 1 WHERE hash = ?1",
                [&reference.hash.0[..]],
            )
            .expect("shorten");
        let before = head(home.path(), "a");
        assert!(matches!(
            store.reduce(&command("a", "retention.reduce", "r1"), &reference),
            Err(StoreError::Unavailable(_))
        ));
        assert_eq!(head(home.path(), "a"), before);
    }

    // Catches a purged body brought back as an excerpt.
    #[test]
    fn reduction_after_purge_is_refused() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        purge(&store, "a", "p1", reference.hash);
        assert_eq!(
            store.reduce(&command("a", "retention.reduce", "r1"), &reference),
            Err(StoreError::Refused(Refusal::NotReducible(reference)))
        );
    }

    // Catches a reduced original's 128 KiB excerpt left after its purge.
    #[test]
    fn purging_a_reduced_original_removes_its_excerpt() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (original, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let excerpt = reduce(&store, "a", "r1", &original);
        let report = purge(&store, "a", "p1", original.hash);
        assert_eq!(
            report.purged,
            vec![original.hash, excerpt.hash]
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            store.open(&excerpt.hash),
            Ok(PayloadBody::Gone(PayloadStatus::Purged { .. }))
        ));
        assert!(matches!(
            store.status(&original.hash),
            Ok(PayloadStatus::Purged { .. })
        ));
    }

    // Catches releasing every reference to a shared excerpt hash.
    #[test]
    fn an_identical_excerpt_survives_the_other_originals_purge() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (first, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let (second, _) = attach(&store, "a", "a2", &body(300_000, 4), RetentionClass::Output);
        let first_excerpt = reduce(&store, "a", "r1", &first);
        let second_excerpt = reduce(&store, "a", "r2", &second);
        assert_eq!(first_excerpt.hash, second_excerpt.hash);
        let report = purge(&store, "a", "p1", first.hash);
        assert!(report.shared.contains(&first_excerpt.hash));
        assert!(matches!(
            store.status(&first_excerpt.hash),
            Ok(PayloadStatus::Present { .. })
        ));
        let live: i64 = raw(home.path())
            .query_row(
                "SELECT count(*) FROM payload_ref WHERE project_id = 'a'
             AND hash = ?1 AND released_seq IS NULL",
                [&first_excerpt.hash.0[..]],
                |row| row.get(0),
            )
            .expect("live");
        assert_eq!(live, 1);
    }

    // Catches a pre-reduction backup retaining an original whose excerpt is shared.
    #[test]
    fn a_backup_drops_a_reduced_original_with_a_shared_excerpt() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (first, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let (second, _) = attach(&store, "a", "a2", &body(300_000, 4), RetentionClass::Output);
        let saved = backup(home.path(), "before.db");
        let excerpt = reduce(&store, "a", "r1", &first);
        assert_eq!(excerpt.hash, reduce(&store, "a", "r2", &second).hash);
        let report = purge(&store, "a", "p1", first.hash);
        assert!(report.shared.contains(&first.hash));
        assert!(report.shared.contains(&excerpt.hash));
        assert!(!report.unreachable.contains(&saved));
        let (state, body, reason): (String, Option<Vec<u8>>, Option<String>) =
            Connection::open(&saved)
                .expect("backup")
                .query_row(
                    "SELECT state, body, purge_reason FROM payload WHERE hash = ?1",
                    [&first.hash.0[..]],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("original row");
        assert_eq!(
            (state.as_str(), body, reason.as_deref()),
            ("purged", None, Some("reduced in live store"))
        );
    }

    // Catches deleting a project's excerpt trace while its other reduction needs it.
    #[test]
    fn a_shared_excerpt_keeps_its_projects_trace() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (first, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let (second, _) = attach(&store, "a", "a2", &body(300_000, 4), RetentionClass::Output);
        let excerpt = reduce(&store, "a", "r1", &first);
        reduce(&store, "a", "r2", &second);
        trace(&store, "a", excerpt.hash, "still-needed");
        trace(&store, "a", first.hash, "no-longer-needed");
        purge(&store, "a", "p1", first.hash);
        assert_eq!(kinds(home.path()), vec!["still-needed"]);
    }

    // Catches a purge event that cannot rebuild the sequence of released references.
    #[test]
    fn release_sequences_can_be_rebuilt_from_retention_events() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (original, _) = attach(&store, "a", "a1", &body(300_000, 3), RetentionClass::Output);
        let excerpt = reduce(&store, "a", "r1", &original);
        let reduction = events(home.path(), "a")
            .into_iter()
            .find(|event| event.type_name == PAYLOAD_REDUCED)
            .expect("reduction");
        let (purge_seq, purged) = store
            .logical_purge(
                &command("a", "retention.purge", "p1"),
                &[original.hash],
                "owner request",
            )
            .expect("purge");
        let stored = events(home.path(), "a")
            .into_iter()
            .find(|event| event.type_name == PAYLOAD_PURGED)
            .expect("purge event");
        assert_eq!(
            PurgedEvent::from_value(&stored.payload),
            Some(purged.clone())
        );
        assert_eq!(purged.released, vec![(reduction.seq, excerpt.hash)]);
        let conn = raw(home.path());
        let released: (i64, i64) = conn
            .query_row(
                "SELECT (SELECT released_seq FROM payload_ref
                     WHERE project_id = 'a' AND seq = ?1 AND hash = ?2),
                    (SELECT released_seq FROM payload_ref
                     WHERE project_id = 'a' AND seq = ?3 AND hash = ?4)",
                params![
                    sql_int(original.seq).expect("seq"),
                    &original.hash.0[..],
                    sql_int(reduction.seq).expect("seq"),
                    &excerpt.hash.0[..]
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("released sequences");
        assert_eq!(released, (reduction.seq as i64, purge_seq as i64));
    }

    // Catches one project recording the removal of another project's body.
    #[test]
    fn a_purge_in_a_project_that_never_referenced_the_hash_is_refused() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let (reference, _) = attach(&store, "b", "b1", b"secret", RetentionClass::Material);
        assert_eq!(
            store.purge(
                &command("a", "retention.purge", "p1"),
                &[reference.hash],
                "owner"
            ),
            Err(StoreError::Refused(Refusal::NothingToPurge(reference.hash)))
        );
        assert!(matches!(
            store.status(&reference.hash),
            Ok(PayloadStatus::Present { .. })
        ));
    }

    // Catches a decision claiming a removal without releasing anything.
    #[test]
    fn a_decision_cannot_append_a_removal_record() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let result = store.transact(&command("a", "fixture", "a1"), &mut |tx| {
            tx.append(NewEvent {
                stream: StreamName(RETENTION_STREAM.into()),
                type_name: PAYLOAD_PURGED.into(),
                type_version: 1,
                git: None,
                payload: json!({}),
                attachments: Vec::new(),
            })?;
            Ok(done(json!("ok"), false))
        });
        assert!(matches!(
            result,
            Err(StoreError::Refused(Refusal::InvalidEvent(_)))
        ));
        assert_eq!(head(home.path(), "a"), 0);
    }

    fn sensitive_answer(store: &SqliteStore, project: &str, request: &str) -> PayloadRef {
        let recorded = store
            .transact(&command(project, "fixture.answer", request), &mut |_| {
                Ok(done(json!("secret"), true))
            })
            .expect("answer");
        let Recorded::New {
            outcome:
                Outcome {
                    answer: Answer::Stored(reference),
                    ..
                },
            ..
        } = recorded
        else {
            panic!("stored answer");
        };
        reference
    }

    // Catches a request row deleted with its body or a replay that gives out gone bytes.
    #[test]
    fn a_retry_after_purge_gets_the_tombstone() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let reference = sensitive_answer(&store, "a", "a1");
        purge(&store, "a", "p1", reference.hash);
        let before = head(home.path(), "a");
        let replay = store
            .transact(&command("a", "fixture.answer", "a1"), &mut |_| {
                panic!("decision replayed")
            })
            .expect("replay");
        assert_eq!(
            replay,
            Recorded::Replayed {
                outcome: Outcome {
                    kind: OutcomeKind::Done,
                    answer: Answer::Tombstone {
                        reference: reference.clone(),
                        status: PayloadStatus::Purged {
                            reason: "owner request".into()
                        }
                    }
                }
            }
        );
        assert_eq!(head(home.path(), "a"), before);
        let key = DocKey(vec![
            KeyValue::Text("fixture.answer".into()),
            KeyValue::Text("a1".into()),
        ]);
        let document = store
            .get(&ProjectId("a".into()), REQUEST_VIEW, &key)
            .expect("get")
            .expect("request");
        assert_eq!(document.body["answer"]["stored"], reference.to_value());
    }

    // Catches a replay that checks only the hash-wide body, not A's reference.
    #[test]
    fn a_retry_gets_its_tombstone_while_the_body_lives_on_elsewhere() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let reference = sensitive_answer(&store, "a", "a1");
        attach(&store, "b", "b1", b"\"secret\"", RetentionClass::Record);
        purge(&store, "a", "p1", reference.hash);
        let replay = store
            .transact(&command("a", "fixture.answer", "a1"), &mut |_| {
                panic!("decision replayed")
            })
            .expect("replay");
        assert!(matches!(
            replay,
            Recorded::Replayed {
                outcome: Outcome {
                    answer: Answer::Tombstone {
                        status: PayloadStatus::Purged { .. },
                        ..
                    },
                    ..
                }
            }
        ));
        assert!(matches!(
            store.status(&reference.hash),
            Ok(PayloadStatus::Present { .. })
        ));
    }

    // Catches a retried purge recording again or deriving its old report from changed rows.
    #[test]
    fn a_retried_purge_rebuilds_its_report_from_the_event() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        attach(&store, "b", "b1", b"secret", RetentionClass::Material);
        let first = purge(&store, "a", "p1", reference.hash);
        assert_eq!(first.shared, vec![reference.hash]);
        let before = head(home.path(), "a");
        purge(&store, "b", "p2", reference.hash);
        let second = purge(&store, "a", "p1", reference.hash);
        assert_eq!(
            (second.purged, second.shared, second.recorded),
            (first.purged, first.shared, first.recorded)
        );
        assert_eq!(head(home.path(), "a"), before);
    }

    // Catches derived trace text left for a removed body, in any project.
    #[test]
    fn trace_rows_of_a_removed_body_are_deleted_in_every_project() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let other = Hash([8; 32]);
        trace(&store, "a", reference.hash, "a-secret");
        trace(&store, "b", reference.hash, "b-secret");
        trace(&store, "a", other, "unrelated");
        purge(&store, "a", "p1", reference.hash);
        assert_eq!(kinds(home.path()), vec!["unrelated"]);
    }

    // Catches A's purge erasing B's diagnostic for a body B still requires.
    #[test]
    fn trace_rows_of_a_shared_body_go_only_for_the_purging_project() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a", "b"]);
        let (reference, _) = attach(&store, "a", "a1", b"shared", RetentionClass::Material);
        attach(&store, "b", "b1", b"shared", RetentionClass::Material);
        trace(&store, "a", reference.hash, "a-trace");
        trace(&store, "b", reference.hash, "b-trace");
        purge(&store, "a", "p1", reference.hash);
        assert_eq!(kinds(home.path()), vec!["b-trace"]);
    }

    fn backup(home: &Path, name: &str) -> PathBuf {
        let dir = home.join("backups");
        fs::create_dir_all(&dir).expect("backups");
        let path = dir.join(name);
        raw(home)
            .execute("VACUUM INTO ?1", [&path.to_string_lossy().to_string()])
            .expect("vacuum into");
        path
    }

    // Catches a home backup retaining a body that the live store purged.
    #[test]
    fn a_backup_in_the_home_receives_the_purge() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let backup = backup(home.path(), "one.db");
        let report = purge(&store, "a", "p1", reference.hash);
        assert!(!report.unreachable.contains(&backup));
        let row: (String, Option<Vec<u8>>) = Connection::open(&backup)
            .expect("backup")
            .query_row(
                "SELECT state, body FROM payload WHERE hash = ?1",
                [&reference.hash.0[..]],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row");
        assert_eq!(row, ("purged".into(), None));
    }

    // Catches a backup reported clean when its trace rows have no provenance.
    #[test]
    fn a_backup_without_trace_provenance_is_listed() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let backup = backup(home.path(), "old.db");
        Connection::open(&backup)
            .expect("backup")
            .execute_batch("DROP INDEX trace_payload; ALTER TABLE trace DROP COLUMN payload_hash;")
            .expect("older trace");
        let report = purge(&store, "a", "p1", reference.hash);
        assert!(report.unreachable.contains(&backup));
        let state: String = Connection::open(&backup)
            .expect("backup")
            .query_row(
                "SELECT state FROM payload WHERE hash = ?1",
                [&reference.hash.0[..]],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(state, "purged");
    }

    // Catches a backup link followed or silently skipped by the scrub.
    #[cfg(unix)]
    #[test]
    fn a_linked_backup_is_listed_not_followed() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let target = home.path().join("target.db");
        raw(home.path())
            .execute("VACUUM INTO ?1", [&target.to_string_lossy().to_string()])
            .expect("backup");
        fs::create_dir_all(home.path().join("backups")).expect("backups");
        let link = home.path().join("backups/two.db");
        symlink(&target, &link).expect("link");
        let report = purge(&store, "a", "p1", reference.hash);
        assert!(report.unreachable.contains(&link));
        let row: (String, bool) = Connection::open(target)
            .expect("target")
            .query_row(
                "SELECT state, body IS NOT NULL FROM payload WHERE hash = ?1",
                [&reference.hash.0[..]],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row");
        assert_eq!(row, ("present".into(), true));
    }

    // Catches following a linked backups directory outside the home's direct entries.
    #[cfg(unix)]
    #[test]
    fn a_linked_backups_directory_is_listed_not_followed() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let target = home.path().join("elsewhere");
        fs::create_dir(&target).expect("target directory");
        let saved = target.join("one.db");
        raw(home.path())
            .execute("VACUUM INTO ?1", [&saved.to_string_lossy().to_string()])
            .expect("backup");
        let link = home.path().join("backups");
        symlink(&target, &link).expect("link");
        let report = purge(&store, "a", "p1", reference.hash);
        assert!(report.unreachable.contains(&link));
        let row: (String, bool) = Connection::open(saved)
            .expect("saved")
            .query_row(
                "SELECT state, body IS NOT NULL FROM payload WHERE hash = ?1",
                [&reference.hash.0[..]],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row");
        assert_eq!(row, ("present".into(), true));
    }

    // Catches VACUUM running before the compatibility epoch has been checked.
    #[test]
    fn scrub_refuses_a_newer_epoch_before_writing() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let conn = raw(home.path());
        conn.execute_batch(
            "CREATE TABLE spare (body BLOB);
             INSERT INTO spare VALUES (zeroblob(1000000));
             DROP TABLE spare;",
        )
        .expect("free pages");
        conn.execute(
            "UPDATE schema_meta SET value = ?1 WHERE key = 'epoch'",
            [EPOCH + 1],
        )
        .expect("future epoch");
        let before: i64 = conn
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))
            .expect("freelist before");
        assert!(before > 0);
        assert_eq!(
            store.scrub(),
            Err(StoreError::ReadOnly {
                needed_epoch: EPOCH + 1
            })
        );
        let after: i64 = conn
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))
            .expect("freelist after");
        assert_eq!(after, before);
    }

    // Catches reporting an error after logical removal already committed.
    #[test]
    fn a_scrub_error_leaves_a_successful_purge_report_incomplete() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let command = command("a", "retention.purge", "p1");
        let report = store
            .purge_with(&command, &[reference.hash], "owner request", || {
                Err(StoreError::ReadOnly {
                    needed_epoch: EPOCH + 1,
                })
            })
            .expect("purge report");
        assert!(!report.scrubbed);
        assert_eq!(report.purged, vec![reference.hash]);
        assert!(report.unreachable.is_empty());
        assert_eq!(report.recorded.len(), 1);
        let (project, seq) = &report.recorded[0];
        assert_eq!(project, &ProjectId("a".into()));
        let recorded_type: String = raw(home.path())
            .query_row(
                "SELECT type FROM event WHERE project_id = ?1 AND seq = ?2",
                params![&project.0, sql_int(*seq).expect("sequence")],
                |row| row.get(0),
            )
            .expect("recorded event");
        assert_eq!(recorded_type, PAYLOAD_PURGED);
        assert_eq!(
            raw(home.path())
                .query_row(
                    "SELECT count(*) FROM schema_meta WHERE key = 'scrub_pending'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("marker"),
            1
        );
    }

    // Catches hiding an untouched backup when the scrub fails.
    #[test]
    fn a_scrub_error_lists_the_backups_directory() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        let backup = backup(home.path(), "one.db");
        let report = store
            .purge_with(
                &command("a", "retention.purge", "p1"),
                &[reference.hash],
                "owner request",
                || {
                    Err(StoreError::ReadOnly {
                        needed_epoch: EPOCH + 1,
                    })
                },
            )
            .expect("purge report");
        assert_eq!(report.unreachable, vec![home.path().join("backups")]);
        let body: Option<Vec<u8>> = Connection::open(backup)
            .expect("backup")
            .query_row(
                "SELECT body FROM payload WHERE hash = ?1",
                [&reference.hash.0[..]],
                |row| row.get(0),
            )
            .expect("backup body");
        assert!(body.is_some());
    }

    // Catches a scrub clearing its marker after an incomplete checkpoint.
    #[test]
    fn only_a_done_scrub_clears_the_marker() {
        let home = tempfile::tempdir().expect("home");
        let _store = open(home.path(), &["a"]);
        let mut conn = raw(home.path());
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('scrub_pending', ?1)",
            [AT],
        )
        .expect("marker");
        assert!(!settle_scrub(&mut conn, CheckpointStep::Incomplete).expect("incomplete"));
        let marked: i64 = conn
            .query_row(
                "SELECT count(*) FROM schema_meta WHERE key = 'scrub_pending'",
                [],
                |row| row.get(0),
            )
            .expect("marked");
        assert_eq!(marked, 1);
        assert!(settle_scrub(&mut conn, CheckpointStep::Done).expect("done"));
        let marked: i64 = conn
            .query_row(
                "SELECT count(*) FROM schema_meta WHERE key = 'scrub_pending'",
                [],
                |row| row.get(0),
            )
            .expect("cleared");
        assert_eq!(marked, 0);
    }

    // Catches a logical purge whose unsanitized free pages have no durable marker.
    #[test]
    fn the_purge_sets_the_scrub_marker() {
        let home = tempfile::tempdir().expect("home");
        let store = open(home.path(), &["a"]);
        let (reference, _) = attach(&store, "a", "a1", b"secret", RetentionClass::Material);
        store
            .logical_purge(
                &command("a", "retention.purge", "p1"),
                &[reference.hash],
                "owner request",
            )
            .expect("logical purge");
        let at: String = raw(home.path())
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'scrub_pending'",
                [],
                |row| row.get(0),
            )
            .expect("marker");
        assert_eq!(at, AT);
    }
}
