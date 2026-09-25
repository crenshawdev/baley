//! Payloads and their references (design 0001, Payloads, retention and
//! purge; EVD-R11).
//!
//! A body is stored once, compressed with zstd, keyed by the SHA-256 of its
//! uncompressed bytes. Each event's use of it is a `payload_ref` row with its
//! own retention class. A reduced or purged body leaves its tombstone in the
//! `payload` row; writing one is the retention task's work, reading it is
//! this module's.

use std::io::{self, Read};
use std::ops::Range;
use std::path::Path;

use baley_store::{
    Hash, PayloadBody, PayloadRef, PayloadStatus, Payloads, ProjectId, Refusal, RetentionClass,
    StoreError,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::store::{SqliteStore, connect, sql};

/// The only encoding this binary writes or reads.
const ZSTD: &str = "zstd";

/// Stores `bytes` once under the SHA-256 of the uncompressed bytes and
/// returns the reference an event carries. A present body is not stored
/// again. A reduced or purged one is refused: Figure 7 has no way back from
/// a tombstone, and a new reference must not point at a body that is gone.
pub(crate) fn put_payload(
    tx: &rusqlite::Transaction<'_>,
    bytes: &[u8],
    class: RetentionClass,
) -> Result<PayloadRef, StoreError> {
    let hash = Hash(Sha256::digest(bytes).into());
    let length = u64::try_from(bytes.len())
        .map_err(|_| StoreError::Unavailable("a payload longer than 2^64 bytes".into()))?;
    let state = tx
        .query_row(
            "SELECT state FROM payload WHERE hash = ?1",
            [&hash.0[..]],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?;
    match state.as_deref() {
        Some("present") => {}
        Some(_) => return Err(StoreError::Refused(Refusal::PayloadTombstoned(hash))),
        None => {
            let body =
                zstd::bulk::compress(bytes, zstd::DEFAULT_COMPRESSION_LEVEL).map_err(|error| {
                    StoreError::Unavailable(format!("compressing a payload: {error}"))
                })?;
            tx.execute(
                "INSERT INTO payload (hash, bytes, encoding, body, state)
                 VALUES (?1, ?2, ?3, ?4, 'present')",
                params![&hash.0[..], sql_int(length)?, ZSTD, body],
            )
            .map_err(sql)?;
        }
    }
    Ok(PayloadRef {
        hash,
        bytes: length,
        class,
    })
}

/// Records that event `seq` of `project` uses `payload` under its class.
/// The event and the payload must already be stored; the foreign keys
/// refuse otherwise.
pub(crate) fn put_reference(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    seq: u64,
    payload: &PayloadRef,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO payload_ref (project_id, seq, hash, class) VALUES (?1, ?2, ?3, ?4)",
        params![
            project.0,
            sql_int(seq)?,
            &payload.hash.0[..],
            payload.class.as_str()
        ],
    )
    .map_err(sql)?;
    Ok(())
}

impl Payloads for SqliteStore {
    /// The body as a stream, read from a connection of its own that holds
    /// one read transaction for the stream's life. The store's read
    /// connection is free again when this returns, and the stream sees the
    /// body as it was when opened even if a purge commits meanwhile.
    fn open(&self, hash: &Hash) -> Result<PayloadBody<'_>, StoreError> {
        let path = self
            .read(|conn| Ok(conn.path().map(str::to_owned)))?
            .filter(|path| !path.is_empty())
            .ok_or_else(|| StoreError::Unavailable("the store has no database file".into()))?;
        let conn = connect(Path::new(&path))?;
        // The first read below starts the snapshot the stream keeps.
        conn.execute_batch("PRAGMA query_only = ON; BEGIN DEFERRED;")
            .map_err(sql)?;
        let found = stored(&conn, hash)?;
        if !matches!(found.status, PayloadStatus::Present { .. }) {
            return Ok(PayloadBody::Gone(found.status));
        }
        if found.encoding != ZSTD {
            return Err(StoreError::Unavailable(format!(
                "payload {hash} has encoding {}, which this binary cannot read",
                found.encoding
            )));
        }
        let source = BlobSource {
            conn,
            rowid: found.rowid,
        };
        let stream =
            body_stream(source).map_err(|error| StoreError::Unavailable(error.to_string()))?;
        Ok(PayloadBody::Present(stream))
    }

    fn status(&self, hash: &Hash) -> Result<PayloadStatus, StoreError> {
        self.read(|conn| {
            // One snapshot for the row and its kept ranges.
            let tx = conn.unchecked_transaction()?;
            Ok(stored(&tx, hash))
        })?
        .map(|found| found.status)
    }
}

/// A stored payload row as the read side needs it.
struct Stored {
    rowid: i64,
    encoding: String,
    status: PayloadStatus,
}

/// The payload row for `hash`, with its tombstone read into a status.
fn stored(conn: &Connection, hash: &Hash) -> Result<Stored, StoreError> {
    let row = conn
        .query_row(
            "SELECT p.rowid, p.encoding, p.state, p.bytes, p.purge_reason,
                    p.excerpt_hash, p.excerpt_class, e.bytes
             FROM payload p LEFT JOIN payload e ON e.hash = p.excerpt_hash
             WHERE p.hash = ?1",
            [&hash.0[..]],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?
        .ok_or(StoreError::Refused(Refusal::UnknownPayload(*hash)))?;
    let (rowid, encoding, state, bytes, reason, excerpt_hash, excerpt_class, excerpt_bytes) = row;
    let corrupt = |what: &str| StoreError::Unavailable(format!("payload {hash}: {what}"));

    let status = match state.as_str() {
        "present" => PayloadStatus::Present {
            bytes: u64::try_from(bytes).map_err(|_| corrupt("negative length"))?,
        },
        "reduced" => {
            let excerpt_hash = excerpt_hash
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or_else(|| corrupt("reduced without a 32-byte excerpt hash"))?;
            let excerpt_bytes = excerpt_bytes
                .and_then(|bytes| u64::try_from(bytes).ok())
                .ok_or_else(|| corrupt("reduced without a stored excerpt"))?;
            let class = excerpt_class
                .as_deref()
                .and_then(RetentionClass::parse)
                .ok_or_else(|| corrupt("reduced without an excerpt class"))?;
            PayloadStatus::Reduced {
                excerpt: PayloadRef {
                    hash: Hash(excerpt_hash),
                    bytes: excerpt_bytes,
                    class,
                },
                kept: kept(conn, hash)?,
            }
        }
        "purged" => PayloadStatus::Purged {
            reason: reason.ok_or_else(|| corrupt("purged without a reason"))?,
        },
        other => return Err(corrupt(&format!("unknown state {other}"))),
    };
    Ok(Stored {
        rowid,
        encoding,
        status,
    })
}

/// The ranges of the original a reduced payload's excerpt keeps, in their
/// stored order. SQLite's JSON functions read `kept`, so the text stays
/// readable with standard tools.
fn kept(conn: &Connection, hash: &Hash) -> Result<Vec<Range<u64>>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT r.value ->> 0, r.value ->> 1
             FROM payload p, json_each(p.kept) r
             WHERE p.hash = ?1 ORDER BY r.key",
        )
        .map_err(sql)?;
    let pairs = statement
        .query_map([&hash.0[..]], |row| {
            Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    pairs
        .into_iter()
        .map(|pair| match pair {
            (Some(start), Some(end)) if 0 <= start && start <= end => {
                Ok(start.unsigned_abs()..end.unsigned_abs())
            }
            _ => Err(StoreError::Unavailable(format!(
                "payload {hash}: a kept range is not [start, end]"
            ))),
        })
        .collect()
}

/// Where a stream's compressed bytes come from: the bytes at an offset,
/// as many as fit in `buf`, and 0 at the end. The seam between the stream
/// and SQLite's blob I/O.
trait ChunkSource {
    fn read_at(&mut self, buf: &mut [u8], offset: usize) -> io::Result<usize>;
}

/// The compressed body through incremental blob I/O on the stream's own
/// connection. Each read opens the blob afresh because a blob handle
/// borrows its connection, which this struct owns; the held read
/// transaction keeps the rowid and the bytes fixed between reads.
struct BlobSource {
    conn: Connection,
    rowid: i64,
}

impl ChunkSource for BlobSource {
    fn read_at(&mut self, buf: &mut [u8], offset: usize) -> io::Result<usize> {
        let blob = self
            .conn
            .blob_open("main", "payload", "body", self.rowid, true)
            .map_err(io::Error::other)?;
        blob.read_at(buf, offset).map_err(io::Error::other)
    }
}

/// A source read in order, one chunk per call.
struct Chunks<S> {
    source: S,
    offset: usize,
}

impl<S: ChunkSource> Read for Chunks<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.source.read_at(buf, self.offset)?;
        self.offset += read;
        Ok(read)
    }
}

/// The uncompressed body as a stream: the zstd decoder pulls the compressed
/// bytes through its own buffer, one chunk at a time, and never holds the
/// whole body.
fn body_stream<'a, S: ChunkSource + 'a>(source: S) -> io::Result<Box<dyn Read + 'a>> {
    let decoder = zstd::stream::read::Decoder::new(Chunks { source, offset: 0 })?;
    Ok(Box::new(decoder))
}

/// SQLite integers are signed.
pub(crate) fn sql_int(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Unavailable(format!("{value} does not fit a SQLite integer")))
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::store::Options;

    const AT: &str = "2026-09-25T18:00:00Z";
    const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";

    /// SHA-256("abc"), FIPS 180-2 appendix B.1.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn open(home: &Path) -> SqliteStore {
        SqliteStore::open(home, AT, Options::default()).expect("open")
    }

    fn project() -> ProjectId {
        ProjectId(PROJECT.into())
    }

    /// A connection of the test's own, beside the store's.
    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    /// The project and event rows the references' foreign keys need.
    fn events(store: &SqliteStore, count: u64) {
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)",
                    params![PROJECT, AT],
                )
                .map_err(sql)?;
                for seq in 1..=sql_int(count)? {
                    tx.execute(
                        "INSERT INTO event (project_id, seq, stream, stream_version, type,
                           type_version, actor, recorded_at, request_id, policy_version,
                           payload_json, hash)
                         VALUES (?1, ?2, 'project', ?2, 'fixture.recorded', 1, 'baley', ?3,
                           'fixture', 1, '{}', zeroblob(32))",
                        params![PROJECT, seq, AT],
                    )
                    .map_err(sql)?;
                }
                Ok(())
            })
            .expect("fixture events");
    }

    fn put(store: &SqliteStore, bytes: &[u8], class: RetentionClass) -> PayloadRef {
        store
            .write(|tx| put_payload(tx, bytes, class))
            .expect("put payload")
    }

    /// Deterministic bytes zstd cannot shrink, so the compressed body spans
    /// many chunks.
    fn noise(length: usize) -> Vec<u8> {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect()
    }

    fn read_all(body: PayloadBody<'_>) -> Vec<u8> {
        let PayloadBody::Present(mut stream) = body else {
            panic!("the body is gone");
        };
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).expect("read the stream");
        bytes
    }

    // The same bytes stored for two events make one payload row and two
    // references, each naming the one hash. Catches a body stored again per
    // use, or a reference that carries its own copy.
    #[test]
    fn the_same_bytes_stored_twice_make_one_row_and_two_references() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        events(&store, 2);
        let refs = store
            .write(|tx| {
                let first = put_payload(tx, b"test output", RetentionClass::Output)?;
                put_reference(tx, &project(), 1, &first)?;
                let second = put_payload(tx, b"test output", RetentionClass::Record)?;
                put_reference(tx, &project(), 2, &second)?;
                Ok((first, second))
            })
            .expect("store twice");
        assert_eq!(refs.0.hash, refs.1.hash);

        let conn = raw(home.path());
        let payloads: i64 = conn
            .query_row("SELECT count(*) FROM payload", [], |row| row.get(0))
            .expect("count payloads");
        assert_eq!(payloads, 1);
        let mut statement = conn
            .prepare("SELECT seq, hash, class FROM payload_ref ORDER BY seq")
            .expect("prepare");
        let references = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("references");
        let hash = refs.0.hash.0.to_vec();
        assert_eq!(
            references,
            [
                (1, hash.clone(), "output".to_string()),
                (2, hash, "record".to_string())
            ]
        );
    }

    // The hash is SHA-256 of the bytes as given, and the length is theirs.
    // Catches hashing or measuring the compressed body instead.
    #[test]
    fn the_hash_and_length_are_those_of_the_uncompressed_bytes() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let payload = put(&store, b"abc", RetentionClass::Record);
        assert_eq!(payload.hash.to_hex(), ABC_SHA256);
        assert_eq!(payload.bytes, 3);
        assert_eq!(payload.class, RetentionClass::Record);
    }

    // A body larger than one chunk is stored compressed and streams back
    // byte for byte. Catches a body stored raw, and a chunked read that
    // loses or repeats its place between chunks.
    #[test]
    fn the_body_streams_back_as_the_original_bytes() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = [noise(400_000), vec![b'x'; 400_000]].concat();
        let payload = put(&store, &original, RetentionClass::Output);

        let (encoding, stored): (String, i64) = raw(home.path())
            .query_row(
                "SELECT encoding, length(body) FROM payload WHERE hash = ?1",
                [&payload.hash.0[..]],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stored row");
        assert_eq!(encoding, "zstd");
        assert!(stored < 500_000, "stored {stored} bytes of 800000");

        let streamed = read_all(store.open(&payload.hash).expect("open"));
        assert!(streamed == original, "the streamed body differs");
    }

    // A present payload reports its uncompressed length. Catches the
    // compressed length reported, or a present body read as a tombstone.
    #[test]
    fn a_present_payload_reports_its_length() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let payload = put(&store, &vec![b'a'; 10_000], RetentionClass::Output);
        assert_eq!(
            store.status(&payload.hash),
            Ok(PayloadStatus::Present { bytes: 10_000 })
        );
    }

    // A hash the store never saw is a refusal naming it. Catches a panic or
    // an engine error in place of the port's refusal.
    #[test]
    fn the_status_of_an_unknown_hash_is_a_refusal() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let unknown = Hash([9; 32]);
        assert_eq!(
            store.status(&unknown),
            Err(StoreError::Refused(Refusal::UnknownPayload(unknown)))
        );
    }

    // Opening a hash the store never saw is the same refusal.
    // Catches an open that fails differently from `status`.
    #[test]
    fn opening_an_unknown_hash_is_a_refusal() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let unknown = Hash([9; 32]);
        assert!(matches!(
            store.open(&unknown),
            Err(StoreError::Refused(Refusal::UnknownPayload(hash))) if hash == unknown
        ));
    }

    /// Tombstones a payload by hand, as the retention task will.
    fn reduce_by_hand(home: &Path, original: &Hash, excerpt: &Hash, kept: &str) {
        raw(home)
            .execute(
                "UPDATE payload SET state = 'reduced', body = NULL, excerpt_hash = ?2,
                   excerpt_class = 'output', kept = ?3
                 WHERE hash = ?1",
                params![&original.0[..], &excerpt.0[..], kept],
            )
            .expect("reduce by hand");
    }

    // A reduced payload reads back its excerpt, with the excerpt's own
    // length and class, and its kept ranges in order, from `status` and
    // from `open`. Catches a tombstone read as an error, the original's
    // length given for the excerpt, or ranges dropped or reordered.
    #[test]
    fn a_reduced_payload_reads_back_its_tombstone() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = put(&store, &noise(1_000_000), RetentionClass::Output);
        let excerpt = put(&store, &noise(131_072), RetentionClass::Output);
        reduce_by_hand(
            home.path(),
            &original.hash,
            &excerpt.hash,
            "[[0,65536],[934464,1000000]]",
        );

        let expected = PayloadStatus::Reduced {
            excerpt: PayloadRef {
                hash: excerpt.hash,
                bytes: 131_072,
                class: RetentionClass::Output,
            },
            kept: vec![0..65_536, 934_464..1_000_000],
        };
        assert_eq!(store.status(&original.hash), Ok(expected.clone()));
        assert!(matches!(
            store.open(&original.hash),
            Ok(PayloadBody::Gone(status)) if status == expected
        ));
    }

    // A purged payload reads back its reason, from `status` and from
    // `open`. Catches a purged row read as unknown or as an error.
    #[test]
    fn a_purged_payload_reads_back_its_reason() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let payload = put(&store, b"a leaked token", RetentionClass::Output);
        raw(home.path())
            .execute(
                "UPDATE payload SET state = 'purged', body = NULL, purge_reason = 'owner purge'
                 WHERE hash = ?1",
                [&payload.hash.0[..]],
            )
            .expect("purge by hand");

        let expected = PayloadStatus::Purged {
            reason: "owner purge".into(),
        };
        assert_eq!(store.status(&payload.hash), Ok(expected.clone()));
        assert!(matches!(
            store.open(&payload.hash),
            Ok(PayloadBody::Gone(status)) if status == expected
        ));
    }

    // A stream opened before a purge commits reads the whole body as it
    // was. Catches a stream that does not hold its read transaction, whose
    // next chunk would find the body gone.
    #[test]
    fn a_stream_keeps_its_snapshot_across_a_purge() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = noise(600_000);
        let payload = put(&store, &original, RetentionClass::Output);

        let PayloadBody::Present(mut stream) = store.open(&payload.hash).expect("open") else {
            panic!("the body is gone");
        };
        // Purged after the open and before any byte is read, so the
        // snapshot must have been taken by `open` itself.
        raw(home.path())
            .execute(
                "UPDATE payload SET state = 'purged', body = NULL, purge_reason = 'owner purge'
                 WHERE hash = ?1",
                [&payload.hash.0[..]],
            )
            .expect("purge by hand");
        let mut streamed = Vec::new();
        stream.read_to_end(&mut streamed).expect("the body");
        assert!(streamed == original, "the streamed body differs");
    }

    // While a stream is open, the store's read connection is free: its lock
    // can be taken at once. Catches a stream that holds that lock for its
    // life, blocking every other read on the store.
    #[test]
    fn an_open_stream_leaves_the_read_connection_free() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let payload = put(&store, &noise(300_000), RetentionClass::Output);
        let stream = store.open(&payload.hash).expect("open");
        assert!(
            store.reader.try_lock().is_ok(),
            "an open stream holds the store's read connection"
        );
        drop(stream);
    }

    /// Runs `sql` against a raw connection with the original's and the
    /// excerpt's hashes bound, and says whether a CHECK refused it.
    fn check_refuses(home: &Path, sql: &str, original: &Hash, excerpt: &Hash) -> bool {
        let result = raw(home).execute(sql, [&original.0[..], &excerpt.0[..]]);
        matches!(result, Err(error) if error.to_string().contains("CHECK constraint failed"))
    }

    // A reduced row without its excerpt class and kept ranges is refused.
    // Catches a reduction the read side would have to guess at.
    #[test]
    fn the_schema_refuses_a_reduction_without_its_ranges() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = put(&store, b"original", RetentionClass::Output);
        let excerpt = put(&store, b"excerpt", RetentionClass::Output);
        assert!(check_refuses(
            home.path(),
            "UPDATE payload SET state = 'reduced', body = NULL, excerpt_hash = ?2,
               excerpt_class = 'output' WHERE hash = ?1",
            &original.hash,
            &excerpt.hash,
        ));
    }

    // A purged row without its reason is refused. Catches a purge that
    // records no cause.
    #[test]
    fn the_schema_refuses_a_purge_without_a_reason() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = put(&store, b"original", RetentionClass::Output);
        let excerpt = put(&store, b"excerpt", RetentionClass::Output);
        assert!(check_refuses(
            home.path(),
            "UPDATE payload SET state = 'purged', body = NULL WHERE hash = ?1 AND ?2 IS NOT NULL",
            &original.hash,
            &excerpt.hash,
        ));
    }

    // A present row without a body is refused. Catches a body lost while
    // the row still claims it.
    #[test]
    fn the_schema_refuses_a_present_row_without_a_body() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = put(&store, b"original", RetentionClass::Output);
        let excerpt = put(&store, b"excerpt", RetentionClass::Output);
        assert!(check_refuses(
            home.path(),
            "UPDATE payload SET body = NULL WHERE hash = ?1 AND ?2 IS NOT NULL",
            &original.hash,
            &excerpt.hash,
        ));
    }

    // A payload cannot be reduced to an excerpt of itself. Catches a
    // reduction whose excerpt is the body it replaced.
    #[test]
    fn the_schema_refuses_an_excerpt_of_itself() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let original = put(&store, b"original", RetentionClass::Output);
        assert!(check_refuses(
            home.path(),
            "UPDATE payload SET state = 'reduced', body = NULL, excerpt_hash = ?2,
               excerpt_class = 'output', kept = '[[0, 8]]' WHERE hash = ?1",
            &original.hash,
            &original.hash,
        ));
    }

    // Bytes whose hash was reduced or purged are refused, and nothing is
    // stored or referenced for them. Catches a new reference pointing at a
    // body that is gone, or a purged body brought back.
    #[test]
    fn storing_tombstoned_bytes_again_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let purged = put(&store, b"a secret", RetentionClass::Material);
        let reduced = put(&store, b"long output", RetentionClass::Output);
        let excerpt = put(&store, b"excerpt", RetentionClass::Output);
        let conn = raw(home.path());
        conn.execute(
            "UPDATE payload SET state = 'purged', body = NULL, purge_reason = 'owner purge'
             WHERE hash = ?1",
            [&purged.hash.0[..]],
        )
        .expect("purge by hand");
        conn.execute(
            "UPDATE payload SET state = 'reduced', body = NULL, excerpt_hash = ?2,
               excerpt_class = 'output', kept = '[[0, 4]]' WHERE hash = ?1",
            [&reduced.hash.0[..], &excerpt.hash.0[..]],
        )
        .expect("reduce by hand");

        for (bytes, hash) in [
            (&b"a secret"[..], purged.hash),
            (&b"long output"[..], reduced.hash),
        ] {
            assert_eq!(
                store.write(|tx| put_payload(tx, bytes, RetentionClass::Record)),
                Err(StoreError::Refused(Refusal::PayloadTombstoned(hash)))
            );
        }
        assert!(matches!(
            store.status(&purged.hash),
            Ok(PayloadStatus::Purged { .. })
        ));
    }

    /// Compressed bytes held in memory, with a record of how far the
    /// stream has asked into them.
    struct RecordingSource {
        bytes: Vec<u8>,
        furthest: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl ChunkSource for RecordingSource {
        fn read_at(&mut self, buf: &mut [u8], offset: usize) -> io::Result<usize> {
            let available = self.bytes.get(offset..).unwrap_or_default();
            let read = available.len().min(buf.len());
            buf[..read].copy_from_slice(&available[..read]);
            self.furthest.set(self.furthest.get().max(offset + read));
            Ok(read)
        }
    }

    // Reading the first bytes of a 2 MB body fetches only the start of its
    // compressed form. Catches a stream that fetches or decompresses the
    // whole body before handing back the first byte.
    #[test]
    fn reading_the_start_of_a_body_fetches_only_the_start() {
        let compressed = zstd::bulk::compress(&noise(2_000_000), zstd::DEFAULT_COMPRESSION_LEVEL)
            .expect("compress");
        let furthest = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut stream = body_stream(RecordingSource {
            bytes: compressed.clone(),
            furthest: std::rc::Rc::clone(&furthest),
        })
        .expect("stream");
        let mut first = [0u8; 16];
        stream.read_exact(&mut first).expect("read the start");
        assert!(
            furthest.get() <= compressed.len() / 4,
            "fetched {} of {} compressed bytes to read 16",
            furthest.get(),
            compressed.len()
        );
    }
}
