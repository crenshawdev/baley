//! Benchmark harness for design 0001. See README.md for method and how to run.

mod generator;
mod store;

use generator::{Generator, Rng, Text, guard_command};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};
use store::{Command, NewEvent, Result, Store, VIEWS};

fn repo() -> PathBuf {
    PathBuf::from(std::env::var("BENCH_REPO").unwrap_or_else(|_| "/code/baley".into()))
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
/// Nearest-rank percentile.
fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).ceil() as usize]
}
fn stats(v: &mut Vec<f64>) -> Value {
    json!({"n": v.len(), "p50_ms": pct(v, 0.5), "p99_ms": pct(v, 0.99), "max_ms": pct(v, 1.0)})
}
fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}
fn shape(n: usize) -> &'static str {
    match n {
        0..=1 => "1",
        2..=4 => "2-4",
        5..=9 => "5-9",
        _ => "10+",
    }
}

fn load(home: &Path, projects: usize, generator: &Generator) -> Result<Value> {
    if home.exists() {
        std::fs::remove_dir_all(home)?;
    }
    let db = store::db_path(home);
    let mut s = Store::create(&db, &repo())?;
    let mut by_shape: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    let (mut claims, mut waits) = (vec![], vec![]);
    let t = Instant::now();
    let (mut events, mut commands) = (0, 0);
    let mut verify = Value::Null;
    for p in 0..projects {
        let name = format!("p{p}");
        s.init_project(&name)?;
        let cmds = generator.commands(&name, 1000 + p as u64);
        for c in &cmds {
            let o = s.transact(c)?;
            assert!(!o.replay);
            commands += 1;
            events += o.events;
            let key = format!("{}{}", shape(c.events.len()), if c.external { " external" } else { "" });
            by_shape.entry(key).or_default().push(ms(o.commit));
            if c.external {
                claims.push(ms(o.claim_commit));
            }
            waits.push(ms(o.lock_wait));
        }
        let v = Instant::now();
        let (n, payloads) = s.verify(&name)?;
        if p == 0 {
            verify = json!({"events": n, "payloads_checked": payloads, "ms": ms(v.elapsed())});
        }
        // A retried request returns its original answer and records nothing.
        let first = s.transact(&cmds[0])?;
        let stored: String = s.conn.query_row("SELECT outcome_json FROM view_request WHERE project_id=?1 AND gen=(SELECT live_gen FROM project_gen WHERE project_id=?1) AND request_id=?2", params![name, cmds[0].request_id], |r| r.get(0))?;
        assert!(first.replay && first.answer == stored && s.verify(&name)?.0 == n);
    }
    s.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let mut shapes = serde_json::Map::new();
    for (k, mut v) in by_shape {
        shapes.insert(k, stats(&mut v));
    }
    Ok(json!({"projects": projects, "commands": commands, "events": events, "load_s": t.elapsed().as_secs_f64(),
        "commit_by_events": shapes, "claim_commit": stats(&mut claims), "lock_wait": stats(&mut waits), "verify_reference": verify, "size": size(&s, &db)?}))
}

fn size(s: &Store, db: &Path) -> Result<Value> {
    let q = |sql: &str| -> Result<i64> { Ok(s.conn.query_row(sql, [], |r| r.get::<_, Option<i64>>(0))?.unwrap_or(0)) };
    let file = std::fs::metadata(db)?.len();
    let wal = std::fs::metadata(db.with_extension("db-wal")).map(|m| m.len()).unwrap_or(0);
    let mut tables = serde_json::Map::new();
    if let Ok(mut stmt) = s.conn.prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name ORDER BY 2 DESC") {
        let rows: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        for (n, b) in rows {
            tables.insert(n, json!(b as f64 / 1e6));
        }
    }
    Ok(json!({
        "db_mb": file as f64 / 1e6, "wal_mb": wal as f64 / 1e6,
        "payload_body_mb": q("SELECT sum(length(body)) FROM payload")? as f64 / 1e6,
        "payload_raw_mb": q("SELECT sum(bytes) FROM payload")? as f64 / 1e6,
        "payloads": q("SELECT count(*) FROM payload")?, "events": q("SELECT count(*) FROM event")?,
        "tables_mb": tables,
    }))
}

/// Writers run as separate processes. Each reports its own progress, waits and errors.
fn spawn_writers(home: &Path, n: usize, projects: &[&str], secs: u64, gate: &str) -> Result<Vec<std::process::Child>> {
    (0..n)
        .map(|i| {
            Proc::new(exe())
                .args(["writer", home.to_str().unwrap(), projects[i % projects.len()], &i.to_string(), &secs.to_string()])
                .env("BENCH_GATE", gate)
                .stdout(Stdio::piped())
                .spawn()
                .map_err(Into::into)
        })
        .collect()
}

fn collect_writers(children: Vec<std::process::Child>) -> Result<Value> {
    let (mut waits, mut commits, mut per, mut errors, mut events, mut bytes) = (vec![], vec![], vec![], 0, 0, 0);
    for c in children {
        let out = c.wait_with_output()?;
        let v: Value = serde_json::from_slice(&out.stdout)?;
        per.push(json!({"commands": v["committed"], "errors": v["errors"].as_array().map(|a| a.len()), "wait_p99_ms": v["wait_p99_ms"], "wait_max_ms": v["wait_max_ms"]}));
        errors += v["errors"].as_array().map(|a| a.len()).unwrap_or(0);
        events += v["events"].as_u64().unwrap_or(0);
        bytes += v["payload_bytes"].as_u64().unwrap_or(0);
        waits.extend(v["waits"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()));
        commits.extend(v["commits"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()));
    }
    let total = commits.len();
    Ok(json!({"committed": total, "events": events, "payload_bytes_inserted": bytes, "errors": errors, "per_writer": per,
        "lock_wait": stats(&mut waits), "commit": stats(&mut commits)}))
}

/// Rebuilds every view of one project into a new generation beside the live
/// one, in time-bounded batches through the writer queue, then catches up and
/// flips the live generation in one short transaction. Old rows are deleted
/// afterwards in batches. With `check`, it builds a scratch generation, compares
/// it with the live views at the head, and flips nothing (`verify --views`).
fn rebuild(s: &mut Store, project: &str, check: bool) -> Result<Value> {
    let t = Instant::now();
    let live: i64 = store::live_gen(&s.conn, project)?;
    let target = if check { -1 } else { live + 1 };
    let tables: Vec<String> = VIEWS.iter().map(|v| format!("view_{v}")).chain(std::iter::once("view_request".to_string())).collect();
    let mut maintenance = vec![];
    let clear = |s: &mut Store, generation: i64, maintenance: &mut Vec<f64>| -> Result<()> {
        for v in &tables {
            loop {
                let b = Instant::now();
                let g = s.gate()?;
                let n = if v == "view_request" {
                    s.conn.execute("DELETE FROM view_request WHERE (project_id, gen, kind, request_id) IN (SELECT project_id, gen, kind, request_id FROM view_request WHERE project_id=?1 AND gen=?2 LIMIT 200)", params![project, generation])?
                } else {
                    s.conn.execute(&format!("DELETE FROM {v} WHERE (project_id, gen, doc_key) IN (SELECT project_id, gen, doc_key FROM {v} WHERE project_id=?1 AND gen=?2 LIMIT 200)"), params![project, generation])?
                };
                drop(g);
                maintenance.push(ms(b.elapsed()));
                std::thread::sleep(b.elapsed());
                if n == 0 {
                    break;
                }
            }
        }
        Ok(())
    };
    clear(s, target, &mut maintenance)?;
    // Replay events after `from` into the target generation, stopping after 200
    // events or 15 ms, whichever comes first.
    let replay = |tx: &rusqlite::Transaction, from: i64, bounded: bool| -> Result<i64> {
        let started = Instant::now();
        let mut stmt = tx.prepare_cached("SELECT seq, stream, type, payload_json FROM event WHERE project_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;
        let rows: Vec<(i64, String, String, String)> =
            stmt.query_map(params![project, from, if bounded { 200 } else { i64::MAX }], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
        let mut last = from;
        let mut docs = store::Docs::new(target);
        for (seq, stream, etype, payload) in rows {
            let payload: Value = serde_json::from_str(&payload)?;
            let phase = payload.get("phase").and_then(Value::as_i64).unwrap_or(0);
            let e = NewEvent { stream, etype, phase, payload: payload.clone(), attachments: vec![], git: false };
            store::project_event(tx, &mut docs, project, seq, &e, &payload)?;
            last = seq;
            if bounded && started.elapsed() > Duration::from_millis(15) {
                break;
            }
        }
        docs.flush(tx, project)?;
        Ok(last)
    };
    let (mut batches, mut from) = (vec![], 0i64);
    loop {
        let b = Instant::now();
        let g = s.gate()?;
        let tx = s.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head: i64 = tx.query_row("SELECT head_seq FROM project WHERE project_id=?1", [project], |r| r.get(0))?;
        if head - from <= 50 {
            drop(tx);
            drop(g);
            break;
        }
        from = replay(&tx, from, true)?;
        tx.commit()?;
        drop(g);
        batches.push(ms(b.elapsed()));
        std::thread::sleep(b.elapsed());
    }
    let f = Instant::now();
    let g = s.gate()?;
    let tx = s.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    replay(&tx, from, false)?;
    let mut mismatched = Value::Null;
    if check {
        let mut n = 0i64;
        for v in &tables {
            let cols = if v == "view_request" { store::REQUEST_COLUMNS } else { store::VIEW_COLUMNS };
            let q = |a: i64, b: i64| format!("SELECT count(*) FROM (SELECT {cols} FROM {v} WHERE project_id=?1 AND gen={a} EXCEPT SELECT {cols} FROM {v} WHERE project_id=?1 AND gen={b})");
            n += tx.query_row(&q(live, target), [project], |r| r.get::<_, i64>(0))?;
            n += tx.query_row(&q(target, live), [project], |r| r.get::<_, i64>(0))?;
        }
        mismatched = json!(n);
    } else {
        tx.execute("UPDATE project_gen SET live_gen=?2 WHERE project_id=?1", params![project, target])?;
    }
    tx.commit()?;
    drop(g);
    let final_ms = ms(f.elapsed());
    clear(s, if check { target } else { live }, &mut maintenance)?;
    Ok(json!({"total_s": t.elapsed().as_secs_f64(), "batches": stats(&mut batches), "final_transaction_ms": final_ms,
        "final_transaction": if check { "catch-up and compare" } else { "catch-up and flip the live generation" },
        "cleanup": stats(&mut maintenance), "mismatched_rows": mismatched}))
}

fn measure(home: &Path, generator: &Generator) -> Result<Value> {
    let db = store::db_path(home);
    let mut report = serde_json::Map::new();
    let r = repo();

    let mut opens = vec![];
    for _ in 0..1000 {
        let t = Instant::now();
        let s = Store::open(&db, true, &r)?;
        opens.push(ms(t.elapsed()));
        drop(s);
    }
    report.insert("open_checked".into(), stats(&mut opens));

    let mut starts = vec![];
    for _ in 0..20 {
        let t = Instant::now();
        let s = Store::open(&db, true, &r)?;
        let ok: String = s.conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        assert_eq!(ok, "ok");
        starts.push(ms(t.elapsed()));
    }
    report.insert("server_start_quick_check".into(), stats(&mut starts));

    {
        let s = Store::open(&db, false, &r)?;
        let mut sizes: Vec<f64> = s
            .conn
            .prepare("SELECT length(doc_json) FROM view_dispatch v JOIN project_gen g ON g.project_id=v.project_id AND g.live_gen=v.gen UNION ALL SELECT length(doc_json) FROM view_phase v JOIN project_gen g ON g.project_id=v.project_id AND g.live_gen=v.gen")?
            .query_map([], |r| r.get::<_, i64>(0))?
            .map(|x| x.map(|b| b as f64))
            .collect::<std::result::Result<_, _>>()?;
        let keys: Vec<(String, String, String)> = s
            .conn
            .prepare("SELECT 'view_dispatch', v.project_id, doc_key FROM view_dispatch v JOIN project_gen g ON g.project_id=v.project_id AND g.live_gen=v.gen UNION ALL SELECT 'view_phase', v.project_id, doc_key FROM view_phase v JOIN project_gen g ON g.project_id=v.project_id AND g.live_gen=v.gen")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut gets = vec![];
        let mut rng = Rng(3);
        for _ in 0..20_000 {
            let (t, p, k) = &keys[rng.below(keys.len() as u64) as usize];
            let started = Instant::now();
            let doc: String = s.conn.prepare_cached(&format!("SELECT doc_json FROM {t} WHERE project_id=?1 AND gen=(SELECT live_gen FROM project_gen WHERE project_id=?1) AND doc_key=?2"))?.query_row([p, k], |r| r.get(0))?;
            let _: Value = serde_json::from_str(&doc)?;
            gets.push(ms(started.elapsed()));
        }
        let mut finds = vec![];
        for i in 0..2000 {
            let started = Instant::now();
            let rows: Vec<String> = s
                .conn
                .prepare_cached("SELECT doc_json FROM view_run WHERE project_id=?1 AND gen=(SELECT live_gen FROM project_gen WHERE project_id=?1) AND phase=?2 ORDER BY doc_key LIMIT 50")?
                .query_map(params![format!("p{}", i % 5), 31 + (i % 10) as i64], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            let _ = rows.len();
            finds.push(ms(started.elapsed()));
        }
        report.insert(
            "view_read".into(),
            json!({"get_by_key_and_parse": stats(&mut gets), "find_page_of_50": stats(&mut finds),
            "doc_bytes": {"p50": pct(&mut sizes, 0.5), "p99": pct(&mut sizes, 0.99), "max": pct(&mut sizes, 1.0)}}),
        );
    }

    {
        let mut s = Store::open(&db, true, &r)?;
        let mut commits = vec![];
        for n in 0..300u64 {
            let events = (0..10)
                .map(|i| NewEvent {
                    stream: format!("dispatch/31/ten{}", n / 10),
                    etype: "task.run".into(),
                    phase: 31,
                    payload: generator.inline(n * 100 + i, 31, "task.run", 900),
                    attachments: if i % 3 == 0 { vec![generator.attachment(n * 1000 + i, "output", 2048 + (i as usize) * 1024)] } else { vec![] },
                    git: true,
                })
                .collect();
            let c = Command {
                project: "p1".into(),
                kind: "dispatch".into(),
                request_id: format!("ten-{n}"),
                phase: 31,
                external: false,
                authority: Some(("plan.approved".into(), "plan/31/".into())),
                events,
            };
            commits.push(ms(s.transact(&c)?.commit));
        }
        report.insert("commit_10_events".into(), stats(&mut commits));
    }

    let (mut walls, mut stores) = (vec![], vec![]);
    for i in 0..500 {
        let t = Instant::now();
        let out = Proc::new(exe()).args(["guard-once", home.to_str().unwrap(), "p0", &format!("cold-{i}")]).output()?;
        walls.push(ms(t.elapsed()));
        let us: f64 = String::from_utf8(out.stdout)?.trim().parse()?;
        stores.push(us / 1000.0);
    }
    report.insert("guard_new_process".into(), json!({"wall_including_process_start": stats(&mut walls), "store_work": stats(&mut stores)}));

    for gate in ["1", "0"] {
        let children = spawn_writers(home, 8, &["p0", "p1", "p2", "p3", "p4"], 30, gate)?;
        report.insert(format!("contention_8_writers_gate_{gate}"), collect_writers(children)?);
    }

    // Rebuild the reference project's views while four writers keep writing to it.
    let children = spawn_writers(home, 4, &["p0"], 25, "1")?;
    std::thread::sleep(Duration::from_millis(500));
    let mut s = Store::open(&db, true, &r)?;
    let rebuilt = rebuild(&mut s, "p0", false)?;
    let verified = rebuild(&mut s, "p0", true)?;
    let during = collect_writers(children)?;
    report.insert("rebuild_under_load".into(), json!({"rebuild": rebuilt, "verify_views_after": verified, "writers_meanwhile": during}));

    // A verified backup while writers run: integrity, every chain, then the copy.
    let children = spawn_writers(home, 4, &["p1", "p2"], 8, "1")?;
    let bk = home.join("backups");
    std::fs::create_dir_all(&bk)?;
    let target = bk.join("verified.db");
    let _ = std::fs::remove_file(&target);
    let t = Instant::now();
    let ok: String = s.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    assert_eq!(ok, "ok");
    let integrity = t.elapsed();
    for p in 0..5 {
        s.verify(&format!("p{p}"))?;
    }
    let chains = t.elapsed() - integrity;
    s.conn.execute("VACUUM INTO ?1", [target.to_str().unwrap()])?;
    let total = t.elapsed();
    let during = collect_writers(children)?;
    report.insert(
        "verified_backup_under_load".into(),
        json!({"integrity_s": integrity.as_secs_f64(), "chains_s": chains.as_secs_f64(), "copy_s": (total - integrity - chains).as_secs_f64(),
            "total_s": total.as_secs_f64(), "writers_meanwhile": during}),
    );

    report.insert("purge".into(), purge(&mut s, &db, &target)?);
    s.verify("p0")?;
    report.insert("after_purge_views_match_rebuild".into(), rebuild(&mut s, "p0", true)?);
    report.insert("size_after".into(), size(&s, &db)?);
    Ok(Value::Object(report))
}

/// The complete purge of the reference project's review material: references,
/// bodies no longer referenced, derived search rows, the purge event, the same
/// bodies in the managed backup, then checkpoint, vacuum and a truncating checkpoint.
fn purge(s: &mut Store, db: &Path, backup: &Path) -> Result<Value> {
    let t = Instant::now();
    let g = s.gate()?;
    let tx = s.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let hashes: Vec<Vec<u8>> =
        tx.prepare("SELECT DISTINCT hash FROM payload_ref WHERE project_id='p0' AND class='material'")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    tx.execute("DELETE FROM payload_ref WHERE project_id='p0' AND class='material'", [])?;
    let mut purged = vec![];
    for h in &hashes {
        let left: i64 = tx.query_row("SELECT count(*) FROM payload_ref WHERE hash=?1", [h], |r| r.get(0))?;
        if left == 0 && tx.execute("UPDATE payload SET body=NULL, state='purged' WHERE hash=?1 AND body IS NOT NULL", [h])? > 0 {
            purged.push(store::hex(h));
        }
        let rows: Vec<i64> = tx.prepare("SELECT row FROM search_ref WHERE project_id='p0' AND hash=?1")?.query_map([h], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        for row in rows {
            tx.execute("DELETE FROM search WHERE rowid=?1", [row])?;
        }
        tx.execute("DELETE FROM search_ref WHERE project_id='p0' AND hash=?1", [h])?;
    }
    let event = NewEvent {
        stream: "retention".into(),
        etype: "payload.purged".into(),
        phase: 0,
        payload: json!({"phase": 0, "hashes": purged, "class": "material", "reason": "policy", "actor": "owner"}),
        attachments: vec![],
        git: false,
    };
    store::append(&tx, &s.repo, "p0", "purge-1", &[&event])?;
    tx.commit()?;
    drop(g);
    let transaction = t.elapsed();
    let b = Instant::now();
    let backup_conn = rusqlite::Connection::open(backup)?;
    backup_conn.execute_batch("PRAGMA secure_delete=ON;")?;
    for h in &purged {
        backup_conn.execute("UPDATE payload SET body=NULL, state='purged' WHERE hash=?1", [hex_to_bytes(h)])?;
    }
    backup_conn.execute_batch("VACUUM;")?;
    drop(backup_conn);
    let backup_s = b.elapsed();
    let c = Instant::now();
    s.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let first_checkpoint = c.elapsed();
    let v = Instant::now();
    s.conn.execute_batch("VACUUM;")?;
    let vacuum = v.elapsed();
    let f = Instant::now();
    s.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let last_checkpoint = f.elapsed();
    let wal = std::fs::metadata(db.with_extension("db-wal")).map(|m| m.len()).unwrap_or(0);
    let left: i64 = s.conn.query_row("SELECT count(*) FROM payload WHERE state='purged' AND body IS NOT NULL", [], |r| r.get(0))?;
    Ok(json!({"payloads_purged": purged.len(), "transaction_ms": ms(transaction), "backup_s": backup_s.as_secs_f64(), "checkpoint_ms": ms(first_checkpoint),
        "vacuum_s": vacuum.as_secs_f64(), "final_checkpoint_ms": ms(last_checkpoint), "total_s": t.elapsed().as_secs_f64(), "wal_mb_after": wal as f64 / 1e6,
        "purged_bodies_left": left}))
}

fn hex_to_bytes(h: &str) -> Vec<u8> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

fn guard_once(home: &Path, project: &str, request: &str) -> Result<()> {
    let t = Instant::now();
    let mut s = Store::open(&store::db_path(home), true, &repo())?;
    let _: Option<String> = s.conn.query_row("SELECT doc_json FROM view_guard_policy WHERE project_id=?1 AND gen=(SELECT live_gen FROM project_gen WHERE project_id=?1) AND doc_key='guard'", [project], |r| r.get(0)).optional()?;
    s.transact(&guard_command(project, request, 0))?;
    println!("{}", t.elapsed().as_micros());
    Ok(())
}

/// A writer: mixed command shapes (10 % with ten events, 20 % with an external
/// effect, the rest one to three events). One attachment per command, drawn 10 %
/// of the time from a shared pool of 32 contents, otherwise unique to the writer.
fn writer(home: &Path, project: &str, id: &str, secs: u64) -> Result<()> {
    let mut s = Store::open(&store::db_path(home), true, &repo())?;
    let text = Text::new();
    let wid: u64 = id.parse()?;
    let pid = std::process::id() as u64;
    let end = Instant::now() + Duration::from_secs(secs);
    let (mut waits, mut commits, mut n, mut events, mut bytes) = (vec![], vec![], 0u64, 0u64, 0u64);
    let mut errors: Vec<String> = vec![];
    let mut rng = Rng(wid.wrapping_mul(7919) ^ pid);
    while Instant::now() < end {
        n += 1;
        let roll = rng.below(100);
        let count = if roll < 10 { 10 } else { 1 + rng.below(3) as usize };
        let external = (10..30).contains(&roll);
        let mut evs = vec![];
        for i in 0..count {
            let attachments = if i == 0 {
                let seed = if rng.below(10) == 0 { 0xC0FFEE ^ rng.below(32) } else { (wid << 48) ^ (pid << 24) ^ n };
                let body = text.make(seed, 1024 + rng.below(8) as usize * 1024, 0.6);
                vec![store::prepare("output", &body, false)]
            } else {
                vec![]
            };
            evs.push(NewEvent {
                stream: format!("dispatch/1/w{id}-{pid}"),
                etype: if external { "suite.result" } else { "task.run" }.into(),
                phase: 1,
                payload: json!({"phase": 1, "facts": {"status": "ran"}, "text": format!("writer {id} run {n}.{i}")}),
                attachments,
                git: true,
            });
        }
        let c = Command {
            project: project.into(),
            kind: "dispatch".into(),
            request_id: format!("w{id}-{pid}-{n}"),
            phase: 1,
            external,
            authority: Some(("plan.approved".into(), "plan/1/".into())),
            events: evs,
        };
        match s.transact(&c) {
            Ok(o) => {
                waits.push(ms(o.lock_wait));
                commits.push(ms(o.commit));
                events += o.events as u64;
                bytes += o.payload_bytes as u64;
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    let mut w2 = waits.clone();
    println!(
        "{}",
        json!({"committed": commits.len(), "events": events, "payload_bytes": bytes, "waits": waits, "commits": commits, "errors": errors,
        "wait_p99_ms": pct(&mut w2, 0.99), "wait_max_ms": pct(&mut w2, 1.0)})
    );
    Ok(())
}

fn machine(root: &Path) -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap().trim().to_string()));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|s| s.trim().to_string());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let fs = Proc::new("findmnt").args(["-no", "FSTYPE,SOURCE", "--target", root.to_str().unwrap()]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    json!({"cpu": cpu, "threads": threads, "kernel": kernel, "sqlite": rusqlite::version(), "filesystem": fs,
        "cache": "warm: measurements follow loading on the same machine; the page cache is not dropped"})
}

/// For every measured number: median, minimum and maximum across runs.
fn summarize(runs: &[Value]) -> Value {
    fn walk(v: &Value, path: String, out: &mut std::collections::BTreeMap<String, Vec<f64>>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == "per_writer" || k == "tables_mb" {
                        continue;
                    }
                    walk(x, if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, out);
                }
            }
            Value::Number(n) => out.entry(path).or_default().push(n.as_f64().unwrap()),
            _ => {}
        }
    }
    let mut all = Default::default();
    for r in runs {
        walk(r, String::new(), &mut all);
    }
    let mut out = serde_json::Map::new();
    for (k, mut v) in all {
        if k.ends_with("p99_ms") || k.ends_with("max_ms") || k.ends_with("_s") || k.ends_with("_mb") || k.ends_with("_ms") || k.ends_with("mismatched_rows") || k.ends_with("errors") || k.ends_with("committed") {
            out.insert(k, json!({"median": pct(&mut v, 0.5), "min": pct(&mut v, 0.0), "max": pct(&mut v, 1.0), "runs": v.len()}));
        }
    }
    Value::Object(out)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("profile/cadence-4.0.json");
    match args.get(1).map(String::as_str) {
        Some("guard-once") => guard_once(Path::new(&args[2]), &args[3], &args[4]),
        Some("writer") => writer(Path::new(&args[2]), &args[3], &args[4], args[5].parse()?),
        Some("run") => {
            let root = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "target/bench-home".into()));
            let runs: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(5);
            let out = PathBuf::from(args.get(4).cloned().unwrap_or_else(|| "target/results".into()));
            std::fs::create_dir_all(&out)?;
            std::fs::create_dir_all(&root)?;
            let generator = Generator::new(&profile)?;
            let calibration: serde_json::Map<String, Value> = generator
                .calibration
                .iter()
                .map(|(k, (q, got, want))| (k.clone(), json!({"repeat": q, "achieved_ratio": got, "target_ratio": want})))
                .collect();
            let mut results = vec![];
            for r in 0..runs {
                let dir = root.join(format!("run-{r}"));
                let reference = load(&dir.join("reference"), 1, &generator)?;
                let several = load(&dir.join("several"), 5, &generator)?;
                let measured = measure(&dir.join("several"), &generator)?;
                let run = json!({"run": r, "reference_project": reference, "five_projects": several, "measurements": measured});
                std::fs::write(out.join(format!("run-{r}.json")), serde_json::to_string_pretty(&run)?)?;
                eprintln!("run {r} done");
                results.push(run);
                std::fs::remove_dir_all(&dir)?;
            }
            let summary = json!({"machine": machine(&root), "calibration": calibration, "runs": runs, "summary": summarize(&results)});
            std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary)?)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        _ => {
            eprintln!("usage: evidence-ledger-bench run [home-root] [runs] [results-dir]");
            Ok(())
        }
    }
}
