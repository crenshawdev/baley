//! Prototype of design 0001's SQLite adapter, complete enough that every cost
//! the design puts inside a transaction is paid here.

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const EPOCH: i64 = 1;
pub const PROJECTOR_VERSION: i64 = 1;
/// The design's views, one table each.
pub const VIEWS: [&str; 15] = [
    "phase", "plan", "evidence_map", "admission", "dispatch", "run", "verification", "review", "review_queue", "risk", "milestone",
    "pause", "policy", "guard_policy", "capture",
];
pub const RECORD_HISTORY: usize = 50;
pub const PHASE_HISTORY: usize = 20;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Attachment {
    pub class: &'static str,
    pub hash: [u8; 32],
    pub bytes: usize,
    pub body: Vec<u8>, // compressed before the transaction: the caller's slow work
    pub searchable: Option<String>,
}

pub struct NewEvent {
    pub stream: String,
    pub etype: String,
    pub phase: i64,
    pub payload: Value,
    pub attachments: Vec<Attachment>,
    pub git: bool,
}

pub struct Command {
    pub project: String,
    pub kind: String,
    pub request_id: String,
    pub phase: i64,
    /// Commands with an external effect run claim, act, record.
    pub external: bool,
    /// Decisions that grant authority confirm a fact against events.
    pub authority: Option<(String, String)>,
    pub events: Vec<NewEvent>,
}

#[derive(Default)]
pub struct Outcome {
    pub lock_wait: Duration,
    pub commit: Duration,
    pub claim_commit: Duration,
    pub replay: bool,
    pub answer: String,
    pub events: usize,
    pub payload_bytes: usize,
}

pub fn prepare(class: &'static str, bytes: &[u8], searchable: bool) -> Attachment {
    let hash: [u8; 32] = Sha256::digest(bytes).into();
    let body = zstd::encode_all(bytes, 3).expect("zstd");
    let searchable = if searchable { Some(String::from_utf8_lossy(bytes).into_owned()) } else { None };
    Attachment { class, hash, bytes: bytes.len(), body, searchable }
}

/// Canonical JSON (RFC 8785 for the values used here: strings, integers,
/// booleans, null, nesting): sorted keys, no whitespace.
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

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Every open checks what EVD-R22 and EVD-R23 require on the real path.
pub fn check_path(db: &Path) -> Result<()> {
    let real = std::fs::canonicalize(db.parent().ok_or("database has no parent")?)?;
    for p in [real.clone(), real.join("baley.db")] {
        let meta = match std::fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && p != real => continue,
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
    let c = std::ffi::CString::new(real.as_os_str().as_encoded_bytes())?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if [0x6969_i64, 0x517B, 0xFF534D42, 0xFE534D42].contains(&(s.f_type as i64)) {
        return Err("the store is on a network filesystem".into());
    }
    Ok(())
}

/// The cheap git facts re-checked inside the transaction: HEAD's commit and the
/// index's identity, read from git's own files rather than by spawning git.
pub fn git_facts(repo: &Path) -> Result<(String, u64)> {
    let head = std::fs::read_to_string(repo.join(".git/HEAD"))?;
    let commit = if let Some(r) = head.trim().strip_prefix("ref: ") {
        match std::fs::read_to_string(repo.join(".git").join(r)) {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                let packed = std::fs::read_to_string(repo.join(".git/packed-refs"))?;
                packed.lines().find(|l| l.ends_with(r)).and_then(|l| l.split(' ').next()).unwrap_or("").to_string()
            }
        }
    } else {
        head.trim().to_string()
    };
    use std::os::unix::fs::MetadataExt;
    let index = std::fs::metadata(repo.join(".git/index")).map(|m| m.mtime_nsec() as u64 ^ m.size()).unwrap_or(0);
    Ok((commit, index))
}

pub struct Store {
    pub conn: Connection,
    gate: Option<std::fs::File>,
    pub repo: PathBuf,
    pub owner: String,
}

/// Held while writing: the writer queue's lock, released on drop.
pub struct Gate(Option<i32>);
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(fd) = self.0 {
            unsafe { libc::flock(fd, libc::LOCK_UN) };
        }
    }
}

impl Store {
    pub fn create(db: &Path, repo: &Path) -> Result<Store> {
        let home = db.parent().unwrap();
        std::fs::create_dir_all(home)?;
        std::fs::set_permissions(home, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        let conn = Connection::open(db)?;
        std::fs::set_permissions(db, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        conn.execute_batch(&format!("PRAGMA page_size=8192; PRAGMA journal_mode=WAL; {SCHEMA}"))?;
        for v in VIEWS {
            conn.execute_batch(&view_ddl(&format!("view_{v}")))?;
        }
        conn.execute("INSERT INTO schema_meta VALUES ('epoch', ?1)", params![EPOCH])?;
        let s = Store { conn, gate: gate_for(db)?, repo: repo.into(), owner: format!("{}", std::process::id()) };
        s.pragmas()?;
        Ok(s)
    }

    pub fn open(db: &Path, checks: bool, repo: &Path) -> Result<Store> {
        if checks {
            check_path(db)?;
        }
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let s = Store { conn, gate: gate_for(db)?, repo: repo.into(), owner: format!("{}", std::process::id()) };
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

    /// The writer queue: a blocking exclusive lock the kernel grants in turn.
    pub fn gate(&self) -> Result<Gate> {
        if let Some(g) = &self.gate {
            if unsafe { libc::flock(g.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            return Ok(Gate(Some(g.as_raw_fd())));
        }
        Ok(Gate(None))
    }

    pub fn init_project(&mut self, project: &str) -> Result<()> {
        self.conn.execute("INSERT INTO project VALUES (?1, ?1, '2026-09-25T00:00:00Z', 0, x'')", params![project])?;
        self.conn.execute("INSERT INTO project_gen VALUES (?1, 0, NULL, 1, 0)", params![project])?;
        Ok(())
    }

    /// Runs one command. Database-only commands take one transaction; commands
    /// with an external effect take a claim transaction and a record transaction.
    pub fn transact(&mut self, cmd: &Command) -> Result<Outcome> {
        let digest = request_digest(cmd);
        if cmd.external {
            let mut o = self.claim(cmd, &digest)?;
            if o.replay {
                return Ok(o);
            }
            // Act: the external work would run here, outside any transaction.
            let r = self.record(cmd, &digest, true)?;
            o.commit = r.commit;
            o.lock_wait += r.lock_wait;
            o.answer = r.answer;
            o.events += r.events;
            o.payload_bytes = r.payload_bytes;
            return Ok(o);
        }
        self.record(cmd, &digest, false)
    }

    fn claim(&mut self, cmd: &Command, digest: &str) -> Result<Outcome> {
        let started = Instant::now();
        let _g = self.gate()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lock_wait = started.elapsed();
        let locked = Instant::now();
        check_epoch(&tx)?;
        if let Some(answer) = replay(&tx, cmd, digest)? {
            tx.rollback()?;
            return Ok(Outcome { lock_wait, commit: locked.elapsed(), replay: true, answer, ..Default::default() });
        }
        let claim = NewEvent {
            stream: format!("command/{}", cmd.kind),
            etype: "command.claimed".into(),
            phase: cmd.phase,
            payload: json!({"phase": cmd.phase, "request_id": cmd.request_id, "digest": digest, "effect": cmd.kind, "owner": self.owner}),
            attachments: vec![],
            git: false,
        };
        let (first, last) = append(&tx, &self.repo, &cmd.project, &cmd.request_id, &[&claim])?;
        tx.execute(
            "INSERT INTO claim_lease (project_id, kind, request_id, owner, scope, renewed_at, claim_seq) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![cmd.project, cmd.kind, cmd.request_id, self.owner, format!("{}/{}", cmd.kind, cmd.phase), now_ms(), first],
        )?;
        let _ = last;
        tx.commit()?;
        Ok(Outcome { lock_wait, claim_commit: locked.elapsed(), events: 1, ..Default::default() })
    }

    fn record(&mut self, cmd: &Command, digest: &str, claimed: bool) -> Result<Outcome> {
        let started = Instant::now();
        let _g = self.gate()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lock_wait = started.elapsed();
        let locked = Instant::now();
        check_epoch(&tx)?;
        if claimed {
            let owner: Option<String> = tx
                .query_row("SELECT owner FROM claim_lease WHERE project_id=?1 AND kind=?2 AND request_id=?3", params![cmd.project, cmd.kind, cmd.request_id], |r| r.get(0))
                .optional()?;
            if owner.as_deref() != Some(self.owner.as_str()) {
                return Err("claim is no longer current".into());
            }
        } else if let Some(answer) = replay(&tx, cmd, digest)? {
            tx.rollback()?;
            return Ok(Outcome { lock_wait, commit: locked.elapsed(), replay: true, answer, ..Default::default() });
        }
        decide(&tx, &self.repo, cmd)?;
        let refs: Vec<&NewEvent> = cmd.events.iter().collect();
        let (first, mut last) = append(&tx, &self.repo, &cmd.project, &cmd.request_id, &refs)?;
        let answer = json!({"request_id": cmd.request_id, "first_seq": first, "last_seq": last,
            "events": cmd.events.iter().map(|e| json!({"type": e.etype, "stream": e.stream})).collect::<Vec<_>>(),
            "answer": format!("recorded {} events for {} in phase {}", cmd.events.len(), cmd.kind, cmd.phase)});
        let done = NewEvent {
            stream: format!("command/{}", cmd.kind),
            etype: "command.completed".into(),
            phase: cmd.phase,
            payload: json!({"phase": cmd.phase, "request_id": cmd.request_id, "kind": cmd.kind, "digest": digest, "outcome": answer}),
            attachments: vec![],
            git: false,
        };
        let (_, l) = append(&tx, &self.repo, &cmd.project, &cmd.request_id, &[&done])?;
        last = l;
        if claimed {
            tx.execute("DELETE FROM claim_lease WHERE project_id=?1 AND kind=?2 AND request_id=?3", params![cmd.project, cmd.kind, cmd.request_id])?;
        }
        tx.execute("INSERT INTO trace (at, project_id, kind, data) VALUES (?1, ?2, 'commit', ?3)", params![now_ms(), cmd.project, format!("{} {}..{}", cmd.request_id, first, last)])?;
        tx.commit()?;
        let payload_bytes = cmd.events.iter().flat_map(|e| &e.attachments).map(|a| a.body.len()).sum();
        Ok(Outcome { lock_wait, commit: locked.elapsed(), answer: answer.to_string(), events: cmd.events.len() + 1, payload_bytes, ..Default::default() })
    }

    /// Full verification of one project's chain against its head, including
    /// every present payload body against its hash.
    pub fn verify(&self, project: &str) -> Result<(i64, i64)> {
        // One read snapshot for the chain, the head and the payloads.
        self.conn.execute_batch("BEGIN DEFERRED")?;
        let result = self.verify_in_snapshot(project);
        self.conn.execute_batch("COMMIT")?;
        result
    }

    fn verify_in_snapshot(&self, project: &str) -> Result<(i64, i64)> {
        let mut head: Vec<u8> = vec![];
        let mut stmt = self.conn.prepare(&format!("SELECT {ENVELOPE_COLUMNS}, payload_json, prev_hash, hash FROM event WHERE project_id=?1 ORDER BY seq"))?;
        let mut rows = stmt.query(params![project])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let env = envelope_from_row(r)?;
            let seq = env["seq"].as_i64().unwrap();
            if seq != n + 1 {
                return Err(format!("sequence gap at {seq}").into());
            }
            let payload: String = r.get(13)?;
            let prev: Vec<u8> = r.get(14)?;
            let hash: Vec<u8> = r.get(15)?;
            let want = chain_hash(project, &head, &env, &payload);
            if prev != head || want.to_vec() != hash {
                return Err(format!("chain breaks at seq {seq}").into());
            }
            head = hash;
            n += 1;
        }
        let (hs, hh): (i64, Vec<u8>) = self.conn.query_row("SELECT head_seq, head_hash FROM project WHERE project_id=?1", params![project], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if hs != n || hh != head {
            return Err("project head does not match the chain".into());
        }
        let mut checked = 0;
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT p.hash, p.bytes, p.body FROM payload_ref r JOIN payload p ON p.hash=r.hash WHERE r.project_id=?1 AND p.body IS NOT NULL",
        )?;
        let mut rows = stmt.query(params![project])?;
        while let Some(r) = rows.next()? {
            let (h, bytes, body): (Vec<u8>, i64, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
            let raw = zstd::decode_all(&body[..])?;
            let got: [u8; 32] = Sha256::digest(&raw).into();
            if got.to_vec() != h || raw.len() as i64 != bytes {
                return Err("payload body does not match its hash".into());
            }
            checked += 1;
        }
        Ok((n, checked))
    }
}

fn gate_for(db: &Path) -> Result<Option<std::fs::File>> {
    if std::env::var("BENCH_GATE").as_deref() == Ok("0") {
        return Ok(None);
    }
    Ok(Some(std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(db.with_extension("db.writer"))?))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn check_epoch(tx: &Transaction) -> Result<()> {
    let epoch: i64 = tx.query_row("SELECT value FROM schema_meta WHERE key='epoch'", [], |r| r.get(0))?;
    if epoch > EPOCH {
        return Err("fenced by a newer epoch".into());
    }
    Ok(())
}

/// The digest covers the command kind and every event's stream, type, payload
/// and attachment identities.
pub fn request_digest(cmd: &Command) -> String {
    let events: Vec<Value> = cmd
        .events
        .iter()
        .map(|e| json!({"stream": e.stream, "type": e.etype, "payload": e.payload, "attachments": e.attachments.iter().map(|a| hex(&a.hash)).collect::<Vec<_>>()}))
        .collect();
    hex(&Sha256::digest(canonical(&json!({"kind": cmd.kind, "events": events}))))
}

fn replay(tx: &Transaction, cmd: &Command, digest: &str) -> Result<Option<String>> {
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT request_digest, outcome_json FROM view_request WHERE project_id=?1 AND gen=(SELECT live_gen FROM project_gen WHERE project_id=?1) AND kind=?2 AND request_id=?3",
            params![cmd.project, cmd.kind, cmd.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match prior {
        Some((d, _)) if d != digest => Err("request id reused with different content".into()),
        Some((_, answer)) => Ok(Some(answer)),
        None => Ok(None),
    }
}

/// The decision's re-reads inside the transaction: its input documents, the
/// authority fact confirmed against events, and the cheap git facts.
fn decide(tx: &Transaction, repo: &Path, cmd: &Command) -> Result<()> {
    let phase_key = format!("phase/{}", cmd.phase);
    let generation = live_gen(tx, &cmd.project)?;
    let _: Option<String> = tx.query_row("SELECT doc_json FROM view_phase WHERE project_id=?1 AND gen=?2 AND doc_key=?3", params![cmd.project, generation, phase_key], |r| r.get(0)).optional()?;
    if let Some(e) = cmd.events.first() {
        let view = view_for(&e.etype);
        let _: Option<String> = tx
            .query_row(&format!("SELECT doc_json FROM view_{view} WHERE project_id=?1 AND gen=?2 AND doc_key=?3"), params![cmd.project, generation, e.stream], |r| r.get(0))
            .optional()?;
    }
    if let Some((etype, prefix)) = &cmd.authority {
        let _: Option<i64> = tx
            .query_row(
                "SELECT seq FROM event WHERE project_id=?1 AND stream>=?2 AND stream<?3 AND type=?4 LIMIT 1",
                params![cmd.project, prefix, format!("{prefix}\u{10FFFF}"), etype],
                |r| r.get(0),
            )
            .optional()?;
    }
    if cmd.events.iter().any(|e| e.git) {
        git_facts(repo)?;
    }
    Ok(())
}

const ENVELOPE_COLUMNS: &str = "seq, stream, stream_version, type, type_version, actor, recorded_at, request_id, git_commit, git_tree, git_checkout, policy_version, project_id";

fn envelope_from_row(r: &rusqlite::Row) -> Result<Value> {
    let git_commit: Option<String> = r.get(8)?;
    let git = match git_commit {
        Some(c) => json!({"commit": c, "tree": r.get::<_, Option<String>>(9)?, "checkout": r.get::<_, Option<String>>(10)?}),
        None => Value::Null,
    };
    Ok(json!({"project_id": r.get::<_, String>(12)?, "seq": r.get::<_, i64>(0)?, "stream": r.get::<_, String>(1)?, "stream_version": r.get::<_, i64>(2)?,
        "type": r.get::<_, String>(3)?, "type_version": r.get::<_, i64>(4)?, "actor": r.get::<_, String>(5)?, "recorded_at": r.get::<_, String>(6)?,
        "request_id": r.get::<_, String>(7)?, "git": git, "policy_version": r.get::<_, i64>(11)?}))
}

/// hash(1) = SHA-256("baley-ledger/1" || project_id || JCS(envelope) || JCS(payload));
/// hash(n) = SHA-256(hash(n-1) || JCS(envelope) || JCS(payload)).
fn chain_hash(project: &str, prev: &[u8], env: &Value, payload_json: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    if prev.is_empty() {
        h.update(b"baley-ledger/1");
        h.update(project.as_bytes());
    } else {
        h.update(prev);
    }
    h.update(canonical(env));
    h.update(payload_json);
    h.finalize().into()
}

/// Appends events with the hash chain, payloads, references, projectors and
/// search entries. Returns the first and last sequence written.
pub fn append(tx: &Transaction, repo: &Path, project: &str, request: &str, events: &[&NewEvent]) -> Result<(i64, i64)> {
    let (mut seq, mut head): (i64, Vec<u8>) = tx.query_row("SELECT head_seq, head_hash FROM project WHERE project_id=?1", params![project], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let first = seq + 1;
    let git = if events.iter().any(|e| e.git) { Some(git_facts(repo)?) } else { None };
    let mut docs = Docs::new(live_gen(tx, project)?);
    for e in events {
        let version: i64 = tx.query_row("SELECT coalesce(max(stream_version),0) FROM event WHERE project_id=?1 AND stream=?2", params![project, e.stream], |r| r.get(0))?;
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
        let (gc, gt, gk) = match (&git, e.git) {
            (Some((c, i)), true) => (Some(c.clone()), Some(format!("{i:x}")), Some("/code/baley".to_string())),
            _ => (None, None, None),
        };
        let gitv = match &gc {
            Some(c) => json!({"commit": c, "tree": gt, "checkout": gk}),
            None => Value::Null,
        };
        let env = json!({"project_id": project, "seq": seq, "stream": e.stream, "stream_version": version + 1, "type": e.etype, "type_version": 1,
            "actor": "daneel:executor", "recorded_at": "2026-09-25T00:00:00Z", "request_id": request, "git": gitv, "policy_version": 1});
        let payload_json = canonical(&payload);
        let hash = chain_hash(project, &head, &env, &payload_json);
        tx.execute(
            "INSERT INTO event (project_id, seq, stream, stream_version, type, type_version, actor, recorded_at, request_id, git_commit, git_tree, git_checkout, policy_version, payload_json, prev_hash, hash)
             VALUES (?1,?2,?3,?4,?5,1,'daneel:executor','2026-09-25T00:00:00Z',?6,?7,?8,?9,1,?10,?11,?12)",
            params![project, seq, e.stream, version + 1, e.etype, request, gc, gt, gk, payload_json, head, hash.to_vec()],
        )?;
        for a in &e.attachments {
            tx.execute("INSERT OR IGNORE INTO payload_ref VALUES (?1, ?2, ?3, ?4, NULL)", params![project, seq, a.hash.to_vec(), a.class])?;
            if let Some(text) = &a.searchable {
                tx.execute("INSERT INTO search (body, project_id, phase, seq, hash) VALUES (?1, ?2, ?3, ?4, ?5)", params![text, project, e.phase, seq, hex(&a.hash)])?;
                tx.execute("INSERT INTO search_ref VALUES (?1, ?2, ?3)", params![project, a.hash.to_vec(), tx.last_insert_rowid()])?;
            }
        }
        if let Some(text) = e.payload.get("text").and_then(Value::as_str) {
            tx.execute("INSERT INTO search (body, project_id, phase, seq, hash) VALUES (?1, ?2, ?3, ?4, NULL)", params![text, project, e.phase, seq])?;
        }
        project_event(tx, &mut docs, project, seq, e, &payload)?;
        head = hash.to_vec();
    }
    docs.flush(tx, project)?;
    tx.execute("UPDATE project SET head_seq=?2, head_hash=?3 WHERE project_id=?1", params![project, seq, head])?;
    Ok((first, seq))
}

pub fn view_for(etype: &str) -> &'static str {
    match etype.split('.').next().unwrap_or("") {
        "context" | "phase" => "phase",
        "plan" => "plan",
        "evidence_map" => "evidence_map",
        "execution" => "admission",
        "dispatch" | "task" | "suite" | "evidence" | "worker" => "dispatch",
        "verification" | "verdict" | "truth" | "human" => "verification",
        "review" => "review",
        "risk" => "risk",
        "milestone" | "release" | "landing" | "payload" => "milestone",
        "pause" => "pause",
        "policy" => "policy",
        "guard" => "guard_policy",
        "item" => "capture",
        _ => "phase",
    }
}

/// The documents a transaction's events change, folded in memory and written
/// once per transaction.
pub struct Docs {
    generation: i64,
    map: std::collections::HashMap<(String, String), (Map<String, Value>, i64, String, i64)>,
    order: Vec<(String, String)>,
}
impl Docs {
    pub fn new(generation: i64) -> Docs {
        Docs { generation, map: Default::default(), order: vec![] }
    }
    fn get(&mut self, tx: &Connection, table: &str, project: &str, key: &str) -> Result<&mut (Map<String, Value>, i64, String, i64)> {
        let k = (table.to_string(), key.to_string());
        if !self.map.contains_key(&k) {
            let doc = load_doc(tx, table, project, self.generation, key)?;
            self.order.push(k.clone());
            self.map.insert(k.clone(), (doc, 0, String::new(), 0));
        }
        Ok(self.map.get_mut(&k).unwrap())
    }
    pub fn flush(self, tx: &Connection, project: &str) -> Result<()> {
        let mut map = self.map;
        for k in self.order {
            let (doc, phase, state, seq) = map.remove(&k).unwrap();
            upsert(tx, &k.0, project, self.generation, &k.1, phase, &state, seq, &Value::Object(doc))?;
        }
        Ok(())
    }
}

fn upsert(tx: &Connection, table: &str, project: &str, generation: i64, key: &str, phase: i64, state: &str, seq: i64, doc: &Value) -> Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO {table} (project_id, gen, doc_key, phase, state, produced_seq, projector_version, doc_json) VALUES (?1,?8,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(project_id, gen, doc_key) DO UPDATE SET phase=excluded.phase, state=excluded.state, produced_seq=excluded.produced_seq, projector_version=excluded.projector_version, doc_json=excluded.doc_json"
        ),
        params![project, key, phase, state, seq, PROJECTOR_VERSION, doc.to_string(), generation],
    )?;
    Ok(())
}

fn load_doc(tx: &Connection, table: &str, project: &str, generation: i64, key: &str) -> Result<Map<String, Value>> {
    let doc: Option<String> = tx.query_row(&format!("SELECT doc_json FROM {table} WHERE project_id=?1 AND gen=?2 AND doc_key=?3"), params![project, generation, key], |r| r.get(0)).optional()?;
    Ok(doc.and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default())
}

fn push_capped(doc: &mut Map<String, Value>, field: &str, entry: Value, cap: usize) {
    let list = doc.entry(field).or_insert_with(|| Value::Array(vec![]));
    let a = list.as_array_mut().unwrap();
    a.push(entry);
    if a.len() > cap {
        a.remove(0);
    }
}

/// The projector. Every event updates its record's document and its phase's
/// document; task and suite events also write a run document; commands write
/// the request view. `suffix` targets shadow tables during a rebuild.
pub fn project_event(tx: &Connection, docs: &mut Docs, project: &str, seq: i64, e: &NewEvent, stored_payload: &Value) -> Result<()> {
    let suffix = "";
    let view = view_for(&e.etype);
    let attachments: Vec<Value> = stored_payload.get("attachments").cloned().and_then(|a| a.as_array().cloned()).unwrap_or_default();
    let summary: String = e.payload.get("text").and_then(Value::as_str).unwrap_or(&e.etype).chars().take(60).collect();
    let entry = json!({"seq": seq, "type": e.etype, "summary": summary, "attachments": attachments.iter().map(|a| a["payload"].clone()).collect::<Vec<_>>()});

    let table = format!("view_{view}{suffix}");
    let d = docs.get(tx, &table, project, &e.stream)?;
    d.1 = e.phase;
    d.2 = e.etype.clone();
    d.3 = seq;
    let doc = &mut d.0;
    doc.insert("stream".into(), json!(e.stream));
    doc.insert("status".into(), json!(e.etype));
    doc.insert("events".into(), json!(doc.get("events").and_then(Value::as_i64).unwrap_or(0) + 1));
    if let Some(Value::Object(m)) = e.payload.get("facts") {
        for (k, v) in m {
            doc.insert(k.clone(), v.clone());
        }
    }
    push_capped(doc, "history", entry.clone(), RECORD_HISTORY);

    let ptable = format!("view_phase{suffix}");
    let pkey = format!("phase/{}", e.phase);
    let p = docs.get(tx, &ptable, project, &pkey)?;
    p.1 = e.phase;
    p.2 = e.etype.clone();
    p.3 = seq;
    let pdoc = &mut p.0;
    let family = e.etype.split('.').next().unwrap_or("").to_string();
    let counts = pdoc.entry("counts").or_insert_with(|| json!({}));
    let c = counts.get(&family).and_then(Value::as_i64).unwrap_or(0) + 1;
    counts.as_object_mut().unwrap().insert(family, json!(c));
    push_capped(pdoc, "recent", entry.clone(), PHASE_HISTORY);

    if e.etype.starts_with("task.") || e.etype.starts_with("suite.") {
        let rtable = format!("view_run{suffix}");
        let r = docs.get(tx, &rtable, project, &format!("run/{seq}"))?;
        r.1 = e.phase;
        r.2 = e.etype.clone();
        r.3 = seq;
        r.0 = json!({"seq": seq, "stream": e.stream, "type": e.etype, "outcome": summary, "attachments": attachments, "phase": e.phase}).as_object().unwrap().clone();
    }
    if e.etype == "command.completed" {
        let o = &e.payload;
        tx.execute(
            &format!("INSERT OR REPLACE INTO view_request{suffix} VALUES (?1, ?7, ?2, ?3, ?4, ?5, ?6)"),
            params![project, o["kind"].as_str().unwrap_or(""), o["request_id"].as_str().unwrap_or(""), o["digest"].as_str().unwrap_or(""), seq, o["outcome"].to_string(), docs.generation],
        )?;
    }
    Ok(())
}

/// Every view row carries a generation. Readers and live projectors use the
/// project's live generation; a rebuild writes the next one beside it, and the
/// swap is one update of `project_gen`.
pub fn view_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (project_id TEXT NOT NULL, gen INTEGER NOT NULL, doc_key TEXT NOT NULL, phase INTEGER, state TEXT,
         produced_seq INTEGER NOT NULL, projector_version INTEGER NOT NULL, doc_json TEXT NOT NULL,
         PRIMARY KEY (project_id, gen, doc_key)) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS {table}_phase ON {table}(project_id, gen, phase, state);"
    )
}
pub const VIEW_COLUMNS: &str = "project_id, doc_key, phase, state, produced_seq, projector_version, doc_json";
pub const REQUEST_COLUMNS: &str = "project_id, kind, request_id, request_digest, seq, outcome_json";

pub fn live_gen(tx: &Connection, project: &str) -> Result<i64> {
    Ok(tx.query_row("SELECT live_gen FROM project_gen WHERE project_id=?1", [project], |r| r.get(0))?)
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
  request_id TEXT NOT NULL, git_commit TEXT, git_tree TEXT, git_checkout TEXT, policy_version INTEGER NOT NULL,
  payload_json TEXT NOT NULL, prev_hash BLOB NOT NULL, hash BLOB NOT NULL,
  PRIMARY KEY (project_id, seq)) WITHOUT ROWID;
CREATE UNIQUE INDEX event_stream ON event(project_id, stream, stream_version);
CREATE INDEX event_type ON event(project_id, type, seq);
CREATE INDEX event_commit ON event(project_id, git_commit);
CREATE TABLE payload (hash BLOB PRIMARY KEY, bytes INTEGER NOT NULL, encoding TEXT NOT NULL, body BLOB, state TEXT NOT NULL);
CREATE TABLE payload_ref (project_id TEXT NOT NULL, seq INTEGER NOT NULL, hash BLOB NOT NULL, class TEXT NOT NULL, expires_at TEXT,
  PRIMARY KEY (project_id, seq, hash)) WITHOUT ROWID;
CREATE INDEX payload_ref_hash ON payload_ref(hash);
CREATE TABLE view_request (project_id TEXT NOT NULL, gen INTEGER NOT NULL, kind TEXT NOT NULL, request_id TEXT NOT NULL, request_digest TEXT NOT NULL, seq INTEGER NOT NULL,
  outcome_json TEXT NOT NULL, PRIMARY KEY (project_id, gen, kind, request_id)) WITHOUT ROWID;
CREATE TABLE project_gen (project_id TEXT PRIMARY KEY, live_gen INTEGER NOT NULL, building_gen INTEGER, projector_version INTEGER NOT NULL, applied_seq INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE claim_lease (project_id TEXT NOT NULL, kind TEXT NOT NULL, request_id TEXT NOT NULL, owner TEXT NOT NULL, scope TEXT NOT NULL,
  renewed_at INTEGER NOT NULL, claim_seq INTEGER NOT NULL, PRIMARY KEY (project_id, kind, request_id)) WITHOUT ROWID;
CREATE TABLE trace (id INTEGER PRIMARY KEY, at INTEGER NOT NULL, project_id TEXT, kind TEXT NOT NULL, data TEXT);
CREATE TABLE search_ref (project_id TEXT NOT NULL, hash BLOB NOT NULL, row INTEGER NOT NULL, PRIMARY KEY (project_id, hash, row)) WITHOUT ROWID;
CREATE VIRTUAL TABLE search USING fts5(body, project_id UNINDEXED, phase UNINDEXED, seq UNINDEXED, hash UNINDEXED);
";
