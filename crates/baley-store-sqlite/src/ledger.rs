//! The ledger's reads and verification, and the port's `Ledger` for this
//! adapter (design 0001, The storage port, The hash chain and anchors).
//!
//! `stream`, `history` and `head` read stored events as recorded, bounded
//! in SQL, and need no view: they work on a project this binary may not
//! write. `verify` walks the whole chain and every present body the
//! project references on a read-only connection of its own, in one
//! snapshot, so the store's read connection stays free and memory holds no
//! more than one row or one body chunk. The anchor it compares with is
//! fetched by the core; nothing here reaches a forge.

use std::io::Read;
use std::ops::RangeInclusive;

use baley_store::{
    Anchor, ChainVerifier, Claim, ClaimId, ClaimOwner, Claimed, Command, Cursor, Decide,
    DecideClaim, DecideReconcile, EVENT_PAGE_BOUND, Event, Hash, Head, HistoryFilter, KeyValue,
    Ledger, Page, PageRequest, PayloadFault, ProjectId, ReconcileAuthority, Recorded, Refusal,
    StoreError, StoredAnchor, StreamName, VerifyReport, anchor_command_event,
    compare_stored_anchor,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use sha2::{Digest, Sha256};

use crate::payload::stream_in;
use crate::rebuild::{event_columns, stored_event};
use crate::store::{SqliteStore, connect, sql};
use crate::transact::stored_head;
use crate::view::encode;

/// The one position a ledger cursor holds: the last stream version or
/// sequence of its page.
fn event_cursor(identity: &str, position: u64) -> Cursor {
    let mut text = identity.to_owned();
    encode(&mut text, &KeyValue::Integer(clamped(position)));
    Cursor(text)
}

/// The position after which a page starts, or `InvalidCursor` unless this
/// query issued the cursor.
fn read_event_cursor(identity: &str, cursor: &Option<Cursor>) -> Result<i64, StoreError> {
    let Some(cursor) = cursor else {
        return Ok(-1);
    };
    let invalid = || StoreError::Refused(Refusal::InvalidCursor);
    let number = cursor
        .0
        .strip_prefix(identity)
        .and_then(|rest| rest.strip_prefix('i'))
        .and_then(|rest| rest.strip_suffix(';'))
        .ok_or_else(invalid)?;
    let position: i64 = number.parse().map_err(|_| invalid())?;
    if position < 0 || position.to_string() != number {
        return Err(invalid());
    }
    Ok(position)
}

/// A sequence or version as SQLite holds it; past its range is past every row.
fn clamped(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Refuses a project the store does not hold, in the caller's snapshot.
fn known(conn: &Connection, project: &ProjectId) -> Result<(), StoreError> {
    conn.query_row(
        "SELECT 1 FROM project WHERE project_id = ?1",
        [&project.0],
        |_| Ok(()),
    )
    .optional()
    .map_err(sql)?
    .ok_or_else(|| StoreError::Refused(Refusal::UnknownProject(project.clone())))
}

/// Reads one page of events: `sql_text` selects `event_columns!()` in page
/// order, bound to `bound`, and takes the row limit last. `position` says
/// which value of an event the cursor holds.
fn event_page(
    conn: &Connection,
    project: &ProjectId,
    sql_text: &str,
    mut bound: Vec<SqlValue>,
    size: usize,
    identity: &str,
    position: fn(&Event) -> u64,
) -> Result<Page<Event>, StoreError> {
    // One row past the page says whether another follows.
    bound.push(SqlValue::Integer(clamped(size as u64 + 1)));
    let mut statement = conn.prepare(sql_text).map_err(sql)?;
    let mut items = statement
        .query_map(params_from_iter(bound), |row| stored_event(project, row))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<Event>>>()
        .map_err(sql)?;
    let next = if items.len() > size {
        items.truncate(size);
        items
            .last()
            .map(|last| event_cursor(identity, position(last)))
    } else {
        None
    };
    Ok(Page { items, next })
}

/// The cursor identity of a `stream` query.
fn stream_identity(project: &ProjectId, stream: &StreamName, from_version: u64) -> String {
    let mut text = String::from("s1.");
    encode(&mut text, &KeyValue::Text(project.0.clone()));
    encode(&mut text, &KeyValue::Text(stream.0.clone()));
    encode(&mut text, &KeyValue::Integer(clamped(from_version)));
    text.push('|');
    text
}

/// The cursor identity of a `history` query: the range, each type, and the
/// commit if any, each item length-prefixed so two queries never share it.
fn history_identity(
    project: &ProjectId,
    range: &RangeInclusive<u64>,
    filter: &HistoryFilter,
) -> String {
    let mut text = String::from("h1.");
    encode(&mut text, &KeyValue::Text(project.0.clone()));
    encode(&mut text, &KeyValue::Integer(clamped(*range.start())));
    encode(&mut text, &KeyValue::Integer(clamped(*range.end())));
    encode(&mut text, &KeyValue::Integer(filter.types.len() as i64));
    for type_name in &filter.types {
        encode(&mut text, &KeyValue::Text(type_name.clone()));
    }
    match &filter.git_commit {
        Some(commit) => encode(&mut text, &KeyValue::Text(commit.clone())),
        None => encode(&mut text, &KeyValue::Integer(0)),
    }
    text.push('|');
    text
}

impl SqliteStore {
    fn stream_page(
        &self,
        project: &ProjectId,
        stream: &StreamName,
        from_version: u64,
        page: &PageRequest,
    ) -> Result<Page<Event>, StoreError> {
        let identity = stream_identity(project, stream, from_version);
        let after = read_event_cursor(&identity, &page.after)?;
        let size = page.limit.min(EVENT_PAGE_BOUND) as usize;
        self.snapshot(|conn| {
            known(conn, project)?;
            if size == 0 {
                return Ok(Page {
                    items: Vec::new(),
                    next: None,
                });
            }
            event_page(
                conn,
                project,
                concat!(
                    "SELECT ",
                    event_columns!(),
                    " FROM event WHERE project_id = ?1 AND stream = ?2
                       AND stream_version >= ?3 AND stream_version > ?4
                     ORDER BY stream_version LIMIT ?5"
                ),
                vec![
                    SqlValue::Text(project.0.clone()),
                    SqlValue::Text(stream.0.clone()),
                    SqlValue::Integer(clamped(from_version)),
                    SqlValue::Integer(after),
                ],
                size,
                &identity,
                |event| event.stream_version,
            )
        })
    }

    fn history_page(
        &self,
        project: &ProjectId,
        range: &RangeInclusive<u64>,
        filter: &HistoryFilter,
        page: &PageRequest,
    ) -> Result<Page<Event>, StoreError> {
        let identity = history_identity(project, range, filter);
        let after = read_event_cursor(&identity, &page.after)?;
        let size = page.limit.min(EVENT_PAGE_BOUND) as usize;
        self.snapshot(|conn| {
            known(conn, project)?;
            if size == 0 || range.is_empty() {
                return Ok(Page {
                    items: Vec::new(),
                    next: None,
                });
            }
            let mut sql_text = String::from(concat!(
                "SELECT ",
                event_columns!(),
                " FROM event WHERE project_id = ? AND seq >= ? AND seq <= ? AND seq > ?"
            ));
            let mut bound = vec![
                SqlValue::Text(project.0.clone()),
                SqlValue::Integer(clamped(*range.start())),
                SqlValue::Integer(clamped(*range.end())),
                SqlValue::Integer(after),
            ];
            if !filter.types.is_empty() {
                let marks = vec!["?"; filter.types.len()].join(", ");
                sql_text.push_str(&format!(" AND type IN ({marks})"));
                bound.extend(filter.types.iter().cloned().map(SqlValue::Text));
            }
            if let Some(commit) = &filter.git_commit {
                sql_text.push_str(" AND git_commit = ?");
                bound.push(SqlValue::Text(commit.clone()));
            }
            sql_text.push_str(" ORDER BY seq LIMIT ?");
            event_page(conn, project, &sql_text, bound, size, &identity, |event| {
                event.seq
            })
        })
    }

    /// Verifies on a read-only connection of its own, in one snapshot.
    fn verify_project(
        &self,
        project: &ProjectId,
        anchor: Option<&Anchor>,
    ) -> Result<VerifyReport, StoreError> {
        let conn = connect(&self.home.join("baley.db"))?;
        conn.execute_batch("PRAGMA query_only = ON; BEGIN DEFERRED;")
            .map_err(sql)?;
        let report = verify_in(&conn, project, anchor);
        // A read transaction has nothing to keep; ending it cannot lose data.
        let _ = conn.execute_batch("ROLLBACK");
        report
    }
}

/// The whole verification inside the snapshot the first read fixes.
fn verify_in(
    conn: &Connection,
    project: &ProjectId,
    anchor: Option<&Anchor>,
) -> Result<VerifyReport, StoreError> {
    known(conn, project)?;
    let mut verifier = ChainVerifier::new(anchor);
    let unanchored_from = verifier.unanchored_from();
    let mut age_unanchored_since = None;
    {
        let mut statement = conn
            .prepare(concat!(
                "SELECT ",
                event_columns!(),
                " FROM event WHERE project_id = ?1 ORDER BY seq"
            ))
            .map_err(sql)?;
        let mut rows = statement.query([&project.0]).map_err(sql)?;
        // Row by row: the verifier keeps only the head it has reached.
        while let Some(row) = rows.next().map_err(sql)? {
            let event = stored_event(project, row).map_err(sql)?;
            if !verifier.push(&event) {
                break;
            }
            if age_unanchored_since.is_none()
                && event.seq >= unanchored_from
                && !anchor_command_event(&event.type_name, &event.payload)
            {
                age_unanchored_since = Some(event.recorded_at);
            }
        }
    }
    let stored_anchor = latest_anchor_row(conn, project)?;
    let stored_anchor_comparison = compare_stored_anchor(project, anchor, stored_anchor.as_ref());
    let bodies = check_bodies(conn, project)?;
    Ok(VerifyReport {
        chain: verifier.finish(),
        payloads: bodies.faults,
        bodies_checked: bodies.checked,
        tombstones_checked: bodies.tombstones,
        stored_anchor,
        stored_anchor_comparison,
        age_unanchored_since,
    })
}

/// The project's latest local anchor row.
fn latest_anchor_row(
    conn: &Connection,
    project: &ProjectId,
) -> Result<Option<StoredAnchor>, StoreError> {
    let row = conn
        .query_row(
            "SELECT seq, head_hash, tag, pushed_at FROM anchor WHERE project_id = ?1
              ORDER BY seq DESC LIMIT 1",
            [&project.0],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    row.map(|(seq, hash, tag, pushed_at)| {
        let seq = u64::try_from(seq)
            .map_err(|_| StoreError::Unavailable(format!("a stored anchor sequence of {seq}")))?;
        let hash = <[u8; 32]>::try_from(hash).map_err(|_| {
            StoreError::Unavailable("a stored anchor hash that is not 32 bytes".into())
        })?;
        Ok(StoredAnchor {
            anchor: Anchor {
                seq,
                hash: Hash(hash),
            },
            tag,
            pushed_at,
        })
    })
    .transpose()
}

/// What the body walk found.
struct Bodies {
    faults: Vec<PayloadFault>,
    checked: u64,
    tombstones: u64,
}

/// How one referenced body stands.
enum Body {
    Sound,
    Corrupt,
    Missing,
    Tombstone,
}

/// Every distinct body the project references, and every excerpt a
/// reduced one retains, in hash order: each present one streamed and
/// hashed, each reduced or purged one counted as a tombstone.
fn check_bodies(conn: &Connection, project: &ProjectId) -> Result<Bodies, StoreError> {
    let mut bodies = Bodies {
        faults: Vec::new(),
        checked: 0,
        tombstones: 0,
    };
    let mut statement = conn
        .prepare(
            "SELECT hash FROM payload_ref WHERE project_id = ?1
             UNION
             SELECT p.excerpt_hash FROM payload_ref r JOIN payload p ON p.hash = r.hash
              WHERE r.project_id = ?1 AND p.excerpt_hash IS NOT NULL
             ORDER BY 1",
        )
        .map_err(sql)?;
    let mut rows = statement.query([&project.0]).map_err(sql)?;
    while let Some(row) = rows.next().map_err(sql)? {
        let bytes: Vec<u8> = row.get(0).map_err(sql)?;
        let hash = Hash(<[u8; 32]>::try_from(bytes).map_err(|_| {
            StoreError::Unavailable("a stored payload hash that is not 32 bytes".into())
        })?);
        match check_body(conn, &hash)? {
            Body::Sound => bodies.checked += 1,
            Body::Corrupt => {
                bodies.checked += 1;
                bodies.faults.push(PayloadFault::Corrupt(hash));
            }
            Body::Missing => bodies.faults.push(PayloadFault::Missing(hash)),
            Body::Tombstone => bodies.tombstones += 1,
        }
    }
    Ok(bodies)
}

/// Streams one body through the zstd decoder and SHA-256 a chunk at a time,
/// and compares the uncompressed length and hash with what it is stored
/// under. Bytes that do not decompress are corrupt.
fn check_body(conn: &Connection, hash: &Hash) -> Result<Body, StoreError> {
    let state = conn
        .query_row(
            "SELECT state FROM payload WHERE hash = ?1",
            [&hash.0[..]],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?;
    match state.as_deref() {
        None => return Ok(Body::Missing),
        Some("present") => {}
        Some(_) => return Ok(Body::Tombstone),
    }
    let Ok((length, mut reader)) = stream_in(conn, hash) else {
        return Ok(Body::Corrupt);
    };
    let mut hasher = Sha256::new();
    let mut read: u64 = 0;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                hasher.update(&chunk[..count]);
                read += count as u64;
            }
            Err(_) => return Ok(Body::Corrupt),
        }
    }
    Ok(
        if read == length && Hash(hasher.finalize().into()) == *hash {
            Body::Sound
        } else {
            Body::Corrupt
        },
    )
}

/// The port over this adapter. The command methods are the adapter's own,
/// unchanged; the reads and verification are this module's.
impl Ledger for SqliteStore {
    fn transact(&self, command: &Command, decide: &mut Decide<'_>) -> Result<Recorded, StoreError> {
        SqliteStore::transact(self, command, decide)
    }

    fn claim(
        &self,
        command: &Command,
        decide: &mut DecideClaim<'_>,
    ) -> Result<Claimed, StoreError> {
        SqliteStore::claim(self, command, decide)
    }

    fn renew_lease(
        &self,
        project: &ProjectId,
        claim: &ClaimId,
        owner: &ClaimOwner,
        at: &str,
    ) -> Result<(), StoreError> {
        SqliteStore::renew_lease(self, project, claim, owner, at)
    }

    fn complete(
        &self,
        command: &Command,
        owner: &ClaimOwner,
        decide: &mut Decide<'_>,
    ) -> Result<Recorded, StoreError> {
        SqliteStore::complete(self, command, owner, decide)
    }

    fn reconcile(
        &self,
        command: &Command,
        claim: &ClaimId,
        authority: ReconcileAuthority,
        decide: &mut DecideReconcile<'_>,
    ) -> Result<Recorded, StoreError> {
        SqliteStore::reconcile(self, command, claim, authority, decide)
    }

    fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
        SqliteStore::open_claims(self, project)
    }

    fn stream(
        &self,
        project: &ProjectId,
        stream: &StreamName,
        from_version: u64,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError> {
        self.stream_page(project, stream, from_version, &page)
    }

    fn history(
        &self,
        project: &ProjectId,
        range: RangeInclusive<u64>,
        filter: &HistoryFilter,
        page: PageRequest,
    ) -> Result<Page<Event>, StoreError> {
        self.history_page(project, &range, filter, &page)
    }

    fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
        self.snapshot(|conn| stored_head(conn, project))
    }

    fn verify(
        &self,
        project: &ProjectId,
        anchor: Option<&Anchor>,
    ) -> Result<VerifyReport, StoreError> {
        self.verify_project(project, anchor)
    }
}

// The binary shares one store between an anchor command and its heartbeat
// as `Arc<dyn Ledger + Send + Sync>`; this fails to compile if it cannot.
const _: () = {
    const fn shareable<T: Ledger + Send + Sync>() {}
    shareable::<SqliteStore>();
};
