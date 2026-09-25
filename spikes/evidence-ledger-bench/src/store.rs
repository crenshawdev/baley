//! Prototype of design 0001's SQLite adapter: just enough to measure it honestly.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const EPOCH: i64 = 1;
pub const PROJECTOR_VERSION: i64 = 1;
/// One table per view, as the design lays out.
pub const VIEWS: [&str; 8] = ["phase", "plan", "dispatch", "run", "verification", "review", "capture", "misc"];

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Store {
    pub conn: Connection,
    /// Writer queue: a blocking file lock taken before `BEGIN IMMEDIATE`, so
    /// waiting writers are parked by the kernel and woken in turn instead of
    /// polling. Off when `BENCH_GATE=0`, to measure SQLite's busy handler alone.
    gate: Option<std::fs::File>,
}

fn gate_for(db: &Path) -> Result<Option<std::fs::File>> {
    if std::env::var("BENCH_GATE").as_deref() == Ok("0") {
        return Ok(None);
    }
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(db.with_extension("db.writer"))?;
    Ok(Some(f))
}

pub struct Attachment {
    pub class: &'static str,
    pub hash: [u8; 32],
    pub bytes: usize,
    pub body: Vec<u8>, // compressed before the transaction: the caller's slow work
}

pub struct NewEvent {
    pub stream: String,
    pub etype: String,
    pub phase: i64,
    pub payload: Value,
    pub attachments: Vec<Attachment>,
}

pub struct Command {
    pub project: String,
    pub kind: String,
    pub request_id: String,
    pub events: Vec<NewEvent>,
}

pub struct Outcome {
    pub lock_wait: Duration,
    pub commit: Duration,
    pub replay: bool,
}

pub fn prepare(class: &'static str, bytes: &[u8]) -> Attachment {
    let hash: [u8; 32] = Sha256::digest(bytes).into();
    let body = zstd::encode_all(bytes, 3).expect("zstd");
    Attachment { class, hash, bytes: bytes.len(), body }
}

/// Canonical JSON: sorted keys, no whitespace. Enough for the benchmark's own
/// values, which hold only strings, integers and nesting.
pub fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let body: Vec<String> = keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&m[*k]))).collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// Every open checks what EVD-R22 and EVD-R23 require on the real path.
pub fn check_path(db: &Path) -> Result<()> {
    let home = db.parent().ok_or("database has no parent")?;
    for p in [home, db] {
        let meta = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && p == db => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.file_type().is_symlink() {
            return Err(format!("{} is a symbolic link", p.display()).into());
        }
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(format!("{} is not owned by this user", p.display()).into());
        }
        if meta.mode() & 0o077 != 0 {
            return Err(format!("{} is readable by others", p.display()).into());
        }
    }
    let c = std::ffi::CString::new(home.as_os_str().as_encoded_bytes())?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // NFS, SMB, CIFS, SMB2
    if [0x6969_i64, 0x517B, 0xFF534D42, 0xFE534D42].contains(&(s.f_type as i64)) {
        return Err("the store is on a network filesystem".into());
    }
    Ok(())
}

impl Store {
    pub fn create(db: &Path) -> Result<Store> {
        let home = db.parent().unwrap();
        std::fs::create_dir_all(home)?;
        std::fs::set_permissions(home, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        let conn = Connection::open(db)?;
        std::fs::set_permissions(db, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        conn.execute_batch(&format!("PRAGMA page_size=8192; PRAGMA journal_mode=WAL; {}", SCHEMA))?;
        for v in VIEWS {
            conn.execute_batch(&format!(
                "CREATE TABLE view_{v} (project_id TEXT NOT NULL, doc_key TEXT NOT NULL, phase INTEGER, state TEXT,
                 produced_seq INTEGER NOT NULL, projector_version INTEGER NOT NULL, doc_json TEXT NOT NULL,
                 PRIMARY KEY (project_id, doc_key)) WITHOUT ROWID;
                 CREATE INDEX view_{v}_phase ON view_{v}(project_id, phase, state);"
            ))?;
        }
        conn.execute("INSERT INTO schema_meta VALUES ('epoch', ?1)", params![EPOCH])?;
        let s = Store { conn, gate: gate_for(db)? };
        s.pragmas()?;
        Ok(s)
    }

    pub fn open(db: &Path, checks: bool) -> Result<Store> {
        if checks {
            check_path(db)?;
        }
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let s = Store { conn, gate: gate_for(db)? };
        s.pragmas()?;
        let epoch: i64 = s.conn.query_row("SELECT value FROM schema_meta WHERE key='epoch'", [], |r| r.get(0))?;
        if epoch > EPOCH {
            return Err("newer epoch: read-only".into());
        }
        Ok(s)
    }

    fn pragmas(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA busy_timeout=5000;")?;
        Ok(())
    }

    pub fn init_project(&self, project: &str) -> Result<()> {
        let genesis: [u8; 32] = Sha256::digest(format!("baley-ledger/1{project}")).into();
        self.conn.execute("INSERT INTO project VALUES (?1, ?1, '2026-09-25T00:00:00Z', 0, ?2)", params![project, genesis.to_vec()])?;
        Ok(())
    }

    /// One database-only command: request check first, then the decision's reads,
    /// the appends, the projectors and the commit, all under the single write lock.
    pub fn transact(&mut self, cmd: &Command) -> Result<Outcome> {
        let started = Instant::now();
        use std::os::fd::AsRawFd;
        if let Some(g) = &self.gate {
            if unsafe { libc::flock(g.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let _unlock = Unlock(self.gate.as_ref().map(|g| g.as_raw_fd()));
        let tx = self.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let lock_wait = started.elapsed();
        let locked = Instant::now();
        let epoch: i64 = tx.query_row("SELECT value FROM schema_meta WHERE key='epoch'", [], |r| r.get(0))?;
        if epoch > EPOCH {
            return Err("fenced by a newer epoch".into());
        }
        let digest = hex(&Sha256::digest(canonical(&json!({"kind": cmd.kind, "events": cmd.events.iter().map(|e| &e.payload).collect::<Vec<_>>()}))));
        let prior: Option<String> = tx
            .query_row("SELECT request_digest FROM view_request WHERE project_id=?1 AND kind=?2 AND request_id=?3", params![cmd.project, cmd.kind, cmd.request_id], |r| r.get(0))
            .optional()?;
        if let Some(p) = prior {
            if p != digest {
                return Err("request id reused with different content".into());
            }
            tx.rollback()?;
            return Ok(Outcome { lock_wait, commit: locked.elapsed(), replay: true });
        }
        let (mut seq, mut head): (i64, Vec<u8>) = tx.query_row("SELECT head_seq, head_hash FROM project WHERE project_id=?1", params![cmd.project], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut all = Vec::with_capacity(cmd.events.len() + 1);
        for e in &cmd.events {
            all.push(e);
        }
        let done = NewEvent {
            stream: format!("command/{}", cmd.kind),
            etype: "command.completed".into(),
            phase: cmd.events.first().map(|e| e.phase).unwrap_or(0),
            payload: json!({"request_id": cmd.request_id, "digest": digest, "answer": {"ok": true}}),
            attachments: vec![],
        };
        all.push(&done);
        for e in all {
            // The decision's re-read: the stream's version and the view document it changes.
            let version: i64 = tx.query_row("SELECT coalesce(max(stream_version),0) FROM event WHERE project_id=?1 AND stream=?2", params![cmd.project, e.stream], |r| r.get(0))?;
            let mut payload = e.payload.clone();
            let mut refs = vec![];
            for a in &e.attachments {
                tx.execute("INSERT OR IGNORE INTO payload (hash, bytes, encoding, body, state) VALUES (?1, ?2, 'zstd', ?3, 'stored')", params![a.hash.to_vec(), a.bytes as i64, a.body])?;
                refs.push(json!({"payload": hex(&a.hash), "bytes": a.bytes, "class": a.class}));
            }
            if !refs.is_empty() {
                payload.as_object_mut().unwrap().insert("attachments".into(), Value::Array(refs));
            }
            seq += 1;
            let envelope = json!({"project_id": cmd.project, "seq": seq, "stream": e.stream, "stream_version": version + 1, "type": e.etype,
                "type_version": 1, "actor": "daneel:executor", "recorded_at": "2026-09-25T00:00:00Z", "request_id": cmd.request_id});
            let payload_json = canonical(&payload);
            let mut h = Sha256::new();
            h.update(&head);
            h.update(canonical(&envelope));
            h.update(&payload_json);
            let hash: [u8; 32] = h.finalize().into();
            tx.execute(
                "INSERT INTO event (project_id, seq, stream, stream_version, type, type_version, actor, recorded_at, request_id, git_commit, payload_json, prev_hash, hash)
                 VALUES (?1,?2,?3,?4,?5,1,'daneel:executor','2026-09-25T00:00:00Z',?6,?7,?8,?9,?10)",
                params![cmd.project, seq, e.stream, version + 1, e.etype, cmd.request_id, format!("{:040x}", seq), payload_json, head, hash.to_vec()],
            )?;
            for a in &e.attachments {
                tx.execute("INSERT OR IGNORE INTO payload_ref VALUES (?1, ?2, ?3, ?4, NULL)", params![cmd.project, seq, a.hash.to_vec(), a.class])?;
            }
            project(&tx, &cmd.project, seq, e, "")?;
            if let Some(text) = e.payload.get("text").and_then(Value::as_str) {
                tx.execute("INSERT INTO search (project_id, phase, seq, body) VALUES (?1, ?2, ?3, ?4)", params![cmd.project, e.phase, seq, text])?;
            }
            head = hash.to_vec();
        }
        tx.execute("INSERT INTO view_request VALUES (?1, ?2, ?3, ?4, ?5)", params![cmd.project, cmd.kind, cmd.request_id, digest, seq])?;
        tx.execute("UPDATE project SET head_seq=?2, head_hash=?3 WHERE project_id=?1", params![cmd.project, seq, head])?;
        tx.commit()?;
        Ok(Outcome { lock_wait, commit: locked.elapsed(), replay: false })
    }

    pub fn verify(&self, project: &str) -> Result<i64> {
        let genesis: [u8; 32] = Sha256::digest(format!("baley-ledger/1{project}")).into();
        let mut head = genesis.to_vec();
        let mut stmt = self.conn.prepare(
            "SELECT seq, stream, stream_version, type, request_id, payload_json, prev_hash, hash FROM event WHERE project_id=?1 ORDER BY seq",
        )?;
        let mut rows = stmt.query(params![project])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let (seq, stream, version, etype, request, payload, prev, hash): (i64, String, i64, String, String, String, Vec<u8>, Vec<u8>) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?);
            let envelope = json!({"project_id": project, "seq": seq, "stream": stream, "stream_version": version, "type": etype,
                "type_version": 1, "actor": "daneel:executor", "recorded_at": "2026-09-25T00:00:00Z", "request_id": request});
            let mut h = Sha256::new();
            h.update(&head);
            h.update(canonical(&envelope));
            h.update(&payload);
            let want: [u8; 32] = h.finalize().into();
            if prev != head || want.to_vec() != hash {
                return Err(format!("chain breaks at seq {seq}").into());
            }
            head = hash;
            n += 1;
        }
        Ok(n)
    }
}

struct Unlock(Option<i32>);
impl Drop for Unlock {
    fn drop(&mut self) {
        if let Some(fd) = self.0 {
            unsafe { libc::flock(fd, libc::LOCK_UN) };
        }
    }
}

pub fn view_for(etype: &str) -> &'static str {
    match etype.split('.').next().unwrap_or("") {
        "context" | "phase" | "execution" => "phase",
        "plan" | "evidence_map" => "plan",
        "dispatch" | "task" => "dispatch",
        "suite" => "run",
        "verification" => "verification",
        "review" => "review",
        "item" => "capture",
        _ => "misc",
    }
}

/// The projector: loads the one document the event changes, folds the event in,
/// writes it back with its provenance. `suffix` targets a shadow table.
pub fn project(tx: &Connection, project: &str, seq: i64, e: &NewEvent, suffix: &str) -> Result<()> {
    let view = view_for(&e.etype);
    let key = &e.stream;
    let table = format!("view_{view}{suffix}");
    let doc: Option<String> = tx
        .query_row(&format!("SELECT doc_json FROM {table} WHERE project_id=?1 AND doc_key=?2"), params![project, key], |r| r.get(0))
        .optional()?;
    let mut doc: Map<String, Value> = doc.and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default();
    let count = doc.get("events").and_then(Value::as_i64).unwrap_or(0) + 1;
    doc.insert("events".into(), json!(count));
    doc.insert("last_type".into(), json!(e.etype));
    doc.insert("last_seq".into(), json!(seq));
    if let Some(Value::Object(m)) = e.payload.get("facts") {
        for (k, v) in m {
            doc.insert(k.clone(), v.clone());
        }
    }
    tx.execute(
        &format!(
            "INSERT INTO {table} (project_id, doc_key, phase, state, produced_seq, projector_version, doc_json) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(project_id, doc_key) DO UPDATE SET state=excluded.state, produced_seq=excluded.produced_seq, doc_json=excluded.doc_json"
        ),
        params![project, key, e.phase, e.etype, seq, PROJECTOR_VERSION, Value::Object(doc).to_string()],
    )?;
    Ok(())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn db_path(home: &Path) -> PathBuf {
    home.join("baley.db")
}

const SCHEMA: &str = "
CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value) WITHOUT ROWID;
CREATE TABLE project (project_id TEXT PRIMARY KEY, name TEXT, created_at TEXT, head_seq INTEGER NOT NULL, head_hash BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE event (
  project_id TEXT NOT NULL, seq INTEGER NOT NULL, stream TEXT NOT NULL, stream_version INTEGER NOT NULL,
  type TEXT NOT NULL, type_version INTEGER NOT NULL, actor TEXT NOT NULL, recorded_at TEXT NOT NULL,
  request_id TEXT NOT NULL, git_commit TEXT, payload_json TEXT NOT NULL, prev_hash BLOB NOT NULL, hash BLOB NOT NULL,
  PRIMARY KEY (project_id, seq)) WITHOUT ROWID;
CREATE UNIQUE INDEX event_stream ON event(project_id, stream, stream_version);
CREATE INDEX event_type ON event(project_id, type, seq);
CREATE INDEX event_commit ON event(project_id, git_commit);
CREATE TABLE payload (hash BLOB PRIMARY KEY, bytes INTEGER NOT NULL, encoding TEXT NOT NULL, body BLOB, state TEXT NOT NULL);
CREATE TABLE payload_ref (project_id TEXT NOT NULL, seq INTEGER NOT NULL, hash BLOB NOT NULL, class TEXT NOT NULL, expires_at TEXT,
  PRIMARY KEY (project_id, seq, hash)) WITHOUT ROWID;
CREATE INDEX payload_ref_hash ON payload_ref(hash);
CREATE TABLE view_request (project_id TEXT NOT NULL, kind TEXT NOT NULL, request_id TEXT NOT NULL, request_digest TEXT NOT NULL, seq INTEGER NOT NULL,
  PRIMARY KEY (project_id, kind, request_id)) WITHOUT ROWID;
CREATE TABLE view_meta (view TEXT NOT NULL, project_id TEXT NOT NULL, projector_version INTEGER NOT NULL, applied_seq INTEGER NOT NULL,
  PRIMARY KEY (view, project_id)) WITHOUT ROWID;
CREATE VIRTUAL TABLE search USING fts5(body, project_id UNINDEXED, phase UNINDEXED, seq UNINDEXED);
";
