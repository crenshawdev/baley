//! Benchmark harness for design 0001. See README.md for method and how to run.

mod store;

use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::{Duration, Instant};
use store::{Attachment, Command, NewEvent, Result, Store, prepare};

#[derive(Deserialize)]
struct Profile {
    compression_ratio_zlib6: HashMap<String, f64>,
    commands: Vec<ProfileCommand>,
}
#[derive(Deserialize)]
struct ProfileCommand {
    phase: i64,
    stream: String,
    events: Vec<ProfileEvent>,
}
#[derive(Deserialize)]
struct ProfileEvent {
    #[serde(rename = "type")]
    etype: String,
    inline: usize,
    attachments: Vec<ProfileAttachment>,
}
#[derive(Deserialize)]
struct ProfileAttachment {
    class: String,
    bytes: usize,
    content: u64,
}

/// Deterministic: every run of the generator produces the same bytes.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Text whose zstd ratio is close to the class's measured ratio: new lines of
/// random words, mixed with repeats of earlier lines at probability `repeat`.
struct Text {
    words: Vec<String>,
}
impl Text {
    fn new() -> Text {
        let mut r = Rng(7);
        let words = (0..4000)
            .map(|_| (0..(3 + r.below(7))).map(|_| (b'a' + r.below(26) as u8) as char).collect())
            .collect();
        Text { words }
    }
    fn make(&self, seed: u64, len: usize, repeat: f64) -> Vec<u8> {
        let mut r = Rng(seed);
        let mut out = Vec::with_capacity(len + 128);
        let mut lines: Vec<(usize, usize)> = vec![];
        while out.len() < len {
            if !lines.is_empty() && (r.below(10_000) as f64) < repeat * 10_000.0 {
                let (a, b) = lines[r.below(lines.len() as u64) as usize];
                let copy = out[a..b].to_vec();
                out.extend_from_slice(&copy);
            } else {
                let start = out.len();
                for _ in 0..(4 + r.below(12)) {
                    out.extend_from_slice(self.words[r.below(self.words.len() as u64) as usize].as_bytes());
                    out.push(b' ');
                }
                out.push(b'\n');
                lines.push((start, out.len()));
            }
        }
        out.truncate(len);
        out
    }
    /// Picks the repeat probability whose zstd ratio is nearest the target.
    fn calibrate(&self, target: f64) -> f64 {
        let mut best = (0.0, f64::MAX);
        for i in 0..=98 {
            let q = i as f64 / 100.0;
            let s = self.make(99, 256 * 1024, q);
            let ratio = s.len() as f64 / zstd::encode_all(&s[..], 3).unwrap().len() as f64;
            let d = (ratio - target).abs();
            if d < best.1 {
                best = (q, d);
            }
        }
        best.0
    }
}

struct Generator {
    profile: Profile,
    text: Text,
    repeat: HashMap<String, f64>,
    cache: HashMap<(u64, String, u64), (Vec<u8>, [u8; 32])>,
}
impl Generator {
    fn new(path: &Path) -> Result<Generator> {
        let profile: Profile = serde_json::from_slice(&std::fs::read(path)?)?;
        let text = Text::new();
        let repeat = profile.compression_ratio_zlib6.iter().map(|(k, v)| (k.clone(), text.calibrate(*v))).collect();
        Ok(Generator { profile, text, repeat, cache: HashMap::new() })
    }
    fn class(c: &str) -> &'static str {
        match c {
            "output" => "output",
            "material" => "material",
            _ => "record",
        }
    }
    fn attachment(&mut self, project_seed: u64, a: &ProfileAttachment) -> Attachment {
        let q = *self.repeat.get(&a.class).unwrap_or(&0.5);
        let key = (project_seed, a.class.clone(), a.content);
        let bytes = self.cache.entry(key).or_insert_with(|| {
            let b = self.text.make(project_seed.wrapping_mul(1_000_003) ^ a.content, a.bytes, q);
            let h = [0u8; 32];
            (b, h)
        });
        prepare(Self::class(&a.class), &bytes.0)
    }
    fn inline(&self, seed: u64, phase: i64, etype: &str, size: usize) -> Value {
        let text = String::from_utf8(self.text.make(seed, size.saturating_sub(160).max(16), 0.2)).unwrap();
        json!({"phase": phase, "facts": {"status": etype, "phase": phase}, "text": text})
    }
    /// Commands for one project: the profile replayed, plus synthetic guard events.
    fn commands(&mut self, project: &str, project_seed: u64) -> Vec<Command> {
        let mut out = vec![];
        let mut n = 0u64;
        let cmds: Vec<(i64, String, Vec<(String, usize, Vec<ProfileAttachment>)>)> = self
            .profile
            .commands
            .iter()
            .map(|c| (c.phase, c.stream.clone(), c.events.iter().map(|e| (e.etype.clone(), e.inline, e.attachments.iter().map(|a| ProfileAttachment { class: a.class.clone(), bytes: a.bytes, content: a.content }).collect())).collect()))
            .collect();
        for (phase, stream, events) in cmds {
            n += 1;
            let mut evs = vec![];
            for (i, (etype, inline, atts)) in events.iter().enumerate() {
                let attachments = atts.iter().map(|a| self.attachment(project_seed, a)).collect();
                evs.push(NewEvent {
                    stream: format!("{stream}/{}", n % 7),
                    etype: etype.clone(),
                    phase,
                    payload: self.inline(project_seed ^ (n << 8) ^ i as u64, phase, etype, *inline),
                    attachments,
                });
            }
            let guarded = evs.iter().any(|e| e.etype.starts_with("task."));
            out.push(Command { project: project.into(), kind: stream.split('/').next().unwrap().into(), request_id: format!("{project_seed}-{n}"), events: evs });
            if guarded && n % 3 == 0 {
                out.push(guard_command(project, &format!("{project_seed}-g{n}"), phase));
            }
        }
        out
    }
}

fn guard_command(project: &str, request: &str, phase: i64) -> Command {
    Command {
        project: project.into(),
        kind: "guard".into(),
        request_id: request.into(),
        events: vec![NewEvent {
            stream: "guard".into(),
            etype: "guard.allowed".into(),
            phase,
            payload: json!({"phase": phase, "facts": {"verb": "commit", "branch": "phase-work"}, "tool": "Bash"}),
            attachments: vec![],
        }],
    }
}

fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
fn stats(v: &mut Vec<f64>) -> Value {
    json!({"n": v.len(), "p50_ms": pct(v, 0.5), "p99_ms": pct(v, 0.99), "max_ms": pct(v, 1.0)})
}

fn load(home: &Path, projects: usize, profile: &Path) -> Result<Value> {
    if home.exists() {
        std::fs::remove_dir_all(home)?;
    }
    let db = store::db_path(home);
    let mut s = Store::create(&db)?;
    let mut generator = Generator::new(profile)?;
    let (mut commits, mut waits) = (vec![], vec![]);
    let t = Instant::now();
    let mut events = 0;
    for p in 0..projects {
        let name = format!("p{p}");
        s.init_project(&name)?;
        let cmds = generator.commands(&name, 1000 + p as u64);
        for c in &cmds {
            events += c.events.len() + 1;
            let o = s.transact(c)?;
            commits.push(ms(o.commit));
            waits.push(ms(o.lock_wait));
        }
        let n = s.verify(&name)?;
        assert_eq!(n as usize, events_of(&s, &name)?);
        // A retried request returns its outcome and records nothing.
        let before = events_of(&s, &name)?;
        assert!(s.transact(&cmds[0])?.replay);
        assert_eq!(before, events_of(&s, &name)?);
    }
    s.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(json!({"projects": projects, "events": events, "load_s": t.elapsed().as_secs_f64(), "commit": stats(&mut commits), "lock_wait": stats(&mut waits), "size": size(&s, &db)?}))
}

fn events_of(s: &Store, p: &str) -> Result<usize> {
    Ok(s.conn.query_row("SELECT count(*) FROM event WHERE project_id=?1", [p], |r| r.get::<_, i64>(0))? as usize)
}

fn size(s: &Store, db: &Path) -> Result<Value> {
    let q = |sql: &str| -> Result<i64> { Ok(s.conn.query_row(sql, [], |r| r.get::<_, Option<i64>>(0))?.unwrap_or(0)) };
    let file = std::fs::metadata(db)?.len();
    let wal = std::fs::metadata(db.with_extension("db-wal")).map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "db_mb": file as f64 / 1e6, "wal_mb": wal as f64 / 1e6,
        "event_payload_json_mb": q("SELECT sum(length(payload_json)) FROM event")? as f64 / 1e6,
        "payload_body_mb": q("SELECT sum(length(body)) FROM payload")? as f64 / 1e6,
        "payload_raw_mb": q("SELECT sum(bytes) FROM payload")? as f64 / 1e6,
        "payloads": q("SELECT count(*) FROM payload")?,
        "events": q("SELECT count(*) FROM event")?,
    }))
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn bench(home: &Path, profile: &Path) -> Result<Value> {
    let db = store::db_path(home);
    let mut report = serde_json::Map::new();

    // Open with the ownership, mode, link, filesystem and epoch checks (guard, CLI).
    let mut opens = vec![];
    for _ in 0..500 {
        let t = Instant::now();
        let s = Store::open(&db, true)?;
        opens.push(ms(t.elapsed()));
        drop(s);
    }
    report.insert("open_checked".into(), stats(&mut opens));

    // Server start: open plus quick_check over the whole database.
    let mut starts = vec![];
    for _ in 0..5 {
        let t = Instant::now();
        let s = Store::open(&db, true)?;
        let r: String = s.conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        assert_eq!(r, "ok");
        starts.push(ms(t.elapsed()));
    }
    report.insert("server_start_quick_check".into(), stats(&mut starts));

    // Get one view document by key.
    {
        let s = Store::open(&db, false)?;
        let keys: Vec<(String, String)> = s.conn.prepare("SELECT project_id, doc_key FROM view_dispatch")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        let mut stmt = s.conn.prepare("SELECT doc_json FROM view_dispatch WHERE project_id=?1 AND doc_key=?2")?;
        let mut gets = vec![];
        let mut r = Rng(3);
        for _ in 0..20_000 {
            let (p, k) = &keys[r.below(keys.len() as u64) as usize];
            let t = Instant::now();
            let _: String = stmt.query_row([p, k], |r| r.get(0))?;
            gets.push(ms(t.elapsed()));
        }
        report.insert("view_get".into(), stats(&mut gets));
    }

    // Cold guard: a new process per call, as the hook runs.
    let (mut walls, mut stores) = (vec![], vec![]);
    for i in 0..200 {
        let t = Instant::now();
        let out = Proc::new(exe()).args(["guard-once", home.to_str().unwrap(), "p0", &format!("cold-{i}")]).output()?;
        walls.push(ms(t.elapsed()));
        let us: f64 = String::from_utf8(out.stdout)?.trim().parse()?;
        stores.push(us / 1000.0);
    }
    report.insert("guard_cold_process_wall".into(), stats(&mut walls));
    report.insert("guard_cold_store_work".into(), stats(&mut stores));

    // Eight writer processes committing continuously.
    let secs = "20";
    let children: Vec<_> = (0..8)
        .map(|i| Proc::new(exe()).args(["writer", home.to_str().unwrap(), &format!("p{}", i % 5), &i.to_string(), secs]).stdout(std::process::Stdio::piped()).spawn())
        .collect::<std::result::Result<_, _>>()?;
    let (mut waits, mut commits, mut total, mut errors) = (vec![], vec![], 0, 0);
    for c in children {
        let out = c.wait_with_output()?;
        let v: Value = serde_json::from_slice(&out.stdout)?;
        total += v["n"].as_u64().unwrap();
        errors += v["errors"].as_array().map(|a| a.len()).unwrap_or(0);
        waits.extend(v["waits"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()));
        commits.extend(v["commits"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()));
    }
    report.insert("contention_8_writers".into(), json!({"gate": std::env::var("BENCH_GATE").unwrap_or_else(|_| "1".into()), "commands": total, "errors": errors, "seconds": 20, "lock_wait": stats(&mut waits), "commit": stats(&mut commits)}));

    // Rebuild every view of the reference project into shadows, in batches, then swap.
    let mut s = Store::open(&db, false)?;
    for v in store::VIEWS {
        s.conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS view_{v}_shadow AS SELECT * FROM view_{v} WHERE 0; CREATE UNIQUE INDEX IF NOT EXISTS view_{v}_shadow_pk ON view_{v}_shadow(project_id, doc_key);"))?;
    }
    let t = Instant::now();
    let mut batches = vec![];
    let head: i64 = s.conn.query_row("SELECT head_seq FROM project WHERE project_id='p0'", [], |r| r.get(0))?;
    let mut from = 0;
    while from < head {
        let b = Instant::now();
        let tx = s.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut stmt = tx.prepare("SELECT seq, stream, type, payload_json FROM event WHERE project_id='p0' AND seq>?1 AND seq<=?2 ORDER BY seq")?;
            let rows: Vec<(i64, String, String, String)> = stmt.query_map([from, from + 200], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
            for (seq, stream, etype, payload) in rows {
                let payload: Value = serde_json::from_str(&payload)?;
                let phase = payload.get("phase").and_then(Value::as_i64).unwrap_or(0);
                let e = NewEvent { stream, etype, phase, payload, attachments: vec![] };
                store::project(&tx, "p0", seq, &e, "_shadow")?;
            }
            tx.execute("INSERT INTO view_meta VALUES ('all','p0',1,?1) ON CONFLICT(view, project_id) DO UPDATE SET applied_seq=excluded.applied_seq", [from + 200])?;
        }
        tx.commit()?;
        batches.push(ms(b.elapsed()));
        from += 200;
    }
    let sw = Instant::now();
    let tx = s.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for v in store::VIEWS {
        tx.execute_batch(&format!("DELETE FROM view_{v} WHERE project_id='p0'; INSERT INTO view_{v} SELECT * FROM view_{v}_shadow WHERE project_id='p0'; DELETE FROM view_{v}_shadow;"))?;
    }
    tx.commit()?;
    report.insert("rebuild_reference_project".into(), json!({"total_s": t.elapsed().as_secs_f64(), "batch": stats(&mut batches), "swap_ms": ms(sw.elapsed())}));

    // Backup while in use.
    let bk = home.join("backups");
    std::fs::create_dir_all(&bk)?;
    let target = bk.join("bench.db");
    let _ = std::fs::remove_file(&target);
    let t = Instant::now();
    s.conn.execute("VACUUM INTO ?1", [target.to_str().unwrap()])?;
    report.insert("backup_vacuum_into_s".into(), json!(t.elapsed().as_secs_f64()));

    // Purge the reference project's review material, then scrub.
    let t = Instant::now();
    let tx = s.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let hashes: Vec<Vec<u8>> = tx.prepare("SELECT hash FROM payload_ref WHERE project_id='p0' AND class='material'")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    tx.execute("DELETE FROM payload_ref WHERE project_id='p0' AND class='material'", [])?;
    let mut purged = 0;
    for h in &hashes {
        let left: i64 = tx.query_row("SELECT count(*) FROM payload_ref WHERE hash=?1", [h], |r| r.get(0))?;
        if left == 0 {
            purged += tx.execute("UPDATE payload SET body=NULL, state='purged' WHERE hash=?1 AND body IS NOT NULL", [h])?;
        }
    }
    tx.commit()?;
    let purge_tx = t.elapsed();
    let c = Instant::now();
    s.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let checkpoint = c.elapsed();
    let v = Instant::now();
    s.conn.execute_batch("VACUUM;")?;
    report.insert("purge".into(), json!({"payloads": purged, "transaction_ms": ms(purge_tx), "checkpoint_ms": ms(checkpoint), "vacuum_s": v.elapsed().as_secs_f64()}));
    report.insert("size_after".into(), size(&s, &db)?);
    let _ = profile;
    Ok(Value::Object(report))
}

fn guard_once(home: &Path, project: &str, request: &str) -> Result<()> {
    let t = Instant::now();
    let mut s = Store::open(&store::db_path(home), true)?;
    let _: Option<String> = rusqlite::OptionalExtension::optional(s.conn.query_row("SELECT doc_json FROM view_misc WHERE project_id=?1 AND doc_key='guard'", [project], |r| r.get(0)))?;
    s.transact(&guard_command(project, request, 0))?;
    println!("{}", t.elapsed().as_micros());
    Ok(())
}

fn writer(home: &Path, project: &str, id: &str, secs: u64) -> Result<()> {
    let mut s = Store::open(&store::db_path(home), true)?;
    let text = Text::new();
    let end = Instant::now() + Duration::from_secs(secs);
    let (mut waits, mut commits, mut n) = (vec![], vec![], 0u64);
    let mut errors: Vec<String> = vec![];
    while Instant::now() < end {
        n += 1;
        let out = text.make(n ^ 0xABCD ^ id.len() as u64, 1024 + (n as usize % 8) * 1024, 0.6);
        let att = prepare("output", &out);
        let c = Command {
            project: project.into(),
            kind: "dispatch".into(),
            request_id: format!("w{id}-{}-{n}", std::process::id()),
            events: (0..(1 + n % 3))
                .map(|i| NewEvent {
                    stream: format!("dispatch/w{id}"),
                    etype: "task.run".into(),
                    phase: 1,
                    payload: json!({"phase": 1, "facts": {"status": "ran"}, "text": format!("writer {id} run {n}.{i}")}),
                    attachments: if i == 0 { vec![Attachment { class: att.class, hash: att.hash, bytes: att.bytes, body: att.body.clone() }] } else { vec![] },
                })
                .collect(),
        };
        match s.transact(&c) {
            Ok(o) => {
                waits.push(ms(o.lock_wait));
                commits.push(ms(o.commit));
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    println!("{}", json!({"n": n, "waits": waits, "commits": commits, "errors": errors}));
    Ok(())
}

fn machine() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap().trim().to_string()));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|s| s.trim().to_string());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    json!({"cpu": cpu, "threads": threads, "kernel": kernel, "sqlite": rusqlite::version()})
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("profile/cadence-4.0.json");
    match args.get(1).map(String::as_str) {
        Some("guard-once") => guard_once(Path::new(&args[2]), &args[3], &args[4]),
        Some("writer") => writer(Path::new(&args[2]), &args[3], &args[4], args[5].parse()?),
        Some("run") => {
            let root = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "target/bench-home".into()));
            let reference = load(&root.join("reference"), 1, &profile)?;
            let several = load(&root.join("several"), 5, &profile)?;
            let measured = bench(&root.join("several"), &profile)?;
            let fs = Proc::new("findmnt").args(["-no", "FSTYPE,SOURCE", "--target", root.to_str().unwrap()]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            let out = json!({"machine": machine(), "filesystem": fs, "reference_project": reference, "five_projects": several, "measurements": measured});
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        _ => {
            eprintln!("usage: evidence-ledger-bench run [home-root]");
            Ok(())
        }
    }
}
