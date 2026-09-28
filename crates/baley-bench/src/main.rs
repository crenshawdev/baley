//! Manual measurements of the real adapter, never an end-to-end test.
mod generator;
mod report;
mod workload;

use baley_store::*;
use baley_store_sqlite::{Monotonic, SqliteStore, Timing};
use generator::{Generator, Profile, Rng, Text, guard_command};
use report::{Figure, Pause, Run};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workload::{AT, Result, fixture_options};

fn now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Unix clock");
    UtcInstant::from_unix(
        d.as_secs().try_into().expect("clock range"),
        d.subsec_nanos(),
    )
    .expect("UTC clock")
    .to_string()
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
fn profile(path: &Path) -> Result<Profile> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn open(home: &Path, p: &Profile) -> Result<SqliteStore> {
    Ok(SqliteStore::open(home, &now(), fixture_options(p))?)
}
fn figure(run: &mut Run, name: &str, value: f64) {
    run.insert(
        name.into(),
        Figure {
            value: Some(value),
            longest: None,
        },
    );
}
fn latency(run: &mut Run, name: &str, values: &[f64]) {
    figure(run, name, report::percentile(values, 0.99));
}
fn load(home: &Path, projects: usize, g: &Generator) -> Result<(Vec<f64>, BTreeSet<Hash>)> {
    fs::create_dir(home)?;
    let store = open(home, &g.profile)?;
    let mut commits = Vec::new();
    let mut material = BTreeSet::new();
    for n in 0..projects {
        let project = format!("p{n}");
        store.create_project(&ProjectId(project.clone()), &project, AT)?;
        for command in g.commands(&project, 1000 + n as u64) {
            let start = Instant::now();
            let hashes = workload::transact(&store, &command)?;
            commits.push(ms(start.elapsed()));
            if n == 0 {
                material.extend(hashes);
            }
        }
    }
    Ok((commits, material))
}
fn child_output(mut command: Process) -> Result<Vec<u8>> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("child failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(output.stdout)
}
fn children(
    home: &Path,
    p: &Path,
    count: usize,
    seconds: u64,
    rebuild: bool,
) -> Result<Vec<Child>> {
    let mut children = Vec::new();
    for n in 0..count {
        let mut command = Process::new(std::env::current_exe()?);
        command
            .arg("writer")
            .arg(home)
            .arg(p)
            .arg(if rebuild {
                "p0".into()
            } else {
                format!("p{}", n % 5)
            })
            .arg(n.to_string())
            .arg(seconds.to_string())
            .arg(if rebuild { "rebuild" } else { "contention" })
            .stdout(Stdio::piped());
        children.push(command.spawn()?);
    }
    for (n, child) in children.iter_mut().enumerate() {
        let ready = home.join(format!(
            "ready-{}-{n}",
            if rebuild { "rebuild" } else { "contention" }
        ));
        while !ready.exists() {
            if let Some(status) = child.try_wait()? {
                return Err(format!("writer exited before ready: {status}").into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fs::write(
        home.join(if rebuild {
            "start-rebuild"
        } else {
            "start-contention"
        }),
        b"",
    )?;
    Ok(children)
}
fn collect(children: Vec<Child>) -> Result<Vec<f64>> {
    let mut waits = Vec::new();
    for child in children {
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(format!("writer failed: {}", output.status).into());
        }
        let v: Value = serde_json::from_slice(&output.stdout)?;
        waits.extend(
            v["waits"]
                .as_array()
                .ok_or("missing waits")?
                .iter()
                .filter_map(Value::as_f64),
        );
    }
    Ok(waits)
}
struct RecordingTiming {
    pauses: Mutex<Vec<Pause>>,
    probe: SqliteStore,
    cursor: Option<Cursor>,
    errors: Mutex<Vec<String>>,
}
fn cursor_query(cursor: Option<Cursor>) -> IndexQuery {
    IndexQuery {
        index: "phase_state".into(),
        equals: vec![],
        page: PageRequest {
            limit: 1,
            after: cursor,
        },
    }
}
impl Timing for RecordingTiming {
    fn now(&self) -> Duration {
        Monotonic.now()
    }
    fn pause(&self, duration: Duration) {
        let refused = if let Some(cursor) = &self.cursor {
            match self.probe.find(
                &ProjectId("p0".into()),
                "phase",
                &cursor_query(Some(cursor.clone())),
            ) {
                Err(StoreError::Refused(Refusal::InvalidCursor)) => true,
                Ok(_) => false,
                Err(e) => {
                    self.errors.lock().unwrap().push(e.to_string());
                    false
                }
            }
        } else {
            false
        };
        self.pauses.lock().unwrap().push(Pause {
            ms: ms(duration),
            cursor_refused: refused,
        });
        Monotonic.pause(duration);
    }
}
fn measure(root: &Path, g: &Generator, p: &Path, seconds: u64) -> Result<(Run, Value)> {
    fs::create_dir(root)?;
    let reference = root.join("reference");
    let five = root.join("five");
    let (mut commits, material) = load(&reference, 1, g)?;
    let reference_size = ["baley.db", "baley.db-wal"]
        .iter()
        .map(|name| fs::metadata(reference.join(name)).map_or(0, |m| m.len()))
        .sum::<u64>();
    let (more, _) = load(&five, 5, g)?;
    commits.extend(more);
    let mut run = Run::new();
    latency(&mut run, "Commit a command", &commits);
    figure(&mut run, "Size", reference_size as f64 / 1e6);
    let mut samples = Vec::new();
    for _ in 0..1000 {
        let t = Instant::now();
        let store = open(&five, &g.profile)?;
        samples.push(ms(t.elapsed()));
        drop(store);
    }
    latency(&mut run, "Open with checks (slice 1)", &samples);
    let store = open(&five, &g.profile)?;
    samples.clear();
    for n in 0..300u64 {
        let events = (0..10)
            .map(|i| workload::NewEvent {
                stream: format!("dispatch/31/ten{}", n / 10),
                etype: "task.run".into(),
                phase: 31,
                payload: g.inline(n * 100 + i, 31, "task.run", 900),
                attachments: if i % 3 == 0 {
                    vec![g.attachment(n * 1000 + i, "output", 2048 + i as usize * 1024)]
                } else {
                    vec![]
                },
                git: true,
            })
            .collect();
        let command = workload::Command {
            project: "p1".into(),
            kind: "dispatch".into(),
            request_id: format!("ten-{n}"),
            phase: 31,
            external: false,
            authority: Some(("plan.approved".into(), "plan/31/".into())),
            events,
        };
        let t = Instant::now();
        workload::transact(&store, &command)?;
        samples.push(ms(t.elapsed()));
    }
    latency(&mut run, "Commit a command of 10 events", &samples);
    let mut keys = Vec::new();
    let mut query = cursor_query(None);
    query.page.limit = 100;
    loop {
        let page = store.find(&ProjectId("p0".into()), "phase", &query)?;
        keys.extend(page.items.into_iter().map(|d| d.key));
        if page.next.is_none() {
            break;
        }
        query.page.after = page.next;
    }
    if keys.is_empty() {
        return Err("reference project has no view keys".into());
    }
    let mut rng = Rng(91);
    samples.clear();
    for _ in 0..20000 {
        let key = &keys[rng.below(keys.len() as u64) as usize];
        let t = Instant::now();
        std::hint::black_box(store.get(&ProjectId("p0".into()), "phase", key)?);
        samples.push(ms(t.elapsed()));
    }
    latency(
        &mut run,
        "Get one view document by key and parse it",
        &samples,
    );
    drop(store);
    samples.clear();
    for n in 0..500 {
        let mut command = Process::new(std::env::current_exe()?);
        command
            .arg("guard-once")
            .arg(&five)
            .arg(p)
            .arg(format!("guard-{n}"));
        samples.push(serde_json::from_slice::<f64>(&child_output(command)?)?);
    }
    latency(
        &mut run,
        "Guard hook, store work only, in a new process",
        &samples,
    );
    let writers = children(&five, p, 8, seconds, false)?;
    let waits = collect(writers)?;
    run.insert(
        "Wait for the write lock, 8 sessions".into(),
        Figure {
            value: Some(report::percentile(&waits, 0.99)),
            longest: Some(report::percentile(&waits, 1.0)),
        },
    );
    let probe = open(&five, &g.profile)?;
    let cursor = probe
        .find(&ProjectId("p0".into()), "phase", &cursor_query(None))?
        .next;
    let timing = Arc::new(RecordingTiming {
        pauses: Mutex::new(vec![]),
        probe,
        cursor,
        errors: Mutex::new(vec![]),
    });
    let mut options = fixture_options(&g.profile);
    options.timing = timing.clone();
    let rebuilding = SqliteStore::open(&five, &now(), options)?;
    let writers = children(&five, p, 4, seconds, true)?;
    let t = Instant::now();
    let result = rebuilding.rebuild(&ProjectId("p0".into()));
    let total = t.elapsed();
    fs::write(five.join("stop-rebuild"), b"")?;
    let rebuild_waits = collect(writers)?;
    result?;
    figure(
        &mut run,
        "Rebuild every view while writers run",
        total.as_secs_f64(),
    );
    let pauses = timing.pauses.lock().unwrap().clone();
    if !timing.errors.lock().unwrap().is_empty() {
        return Err(format!("cursor probe failed: {:?}", timing.errors.lock().unwrap()).into());
    }
    figure(
        &mut run,
        "Longest rebuild batch",
        report::longest_batch(&pauses),
    );
    run.insert(
        "Rebuild final flip".into(),
        Figure {
            value: report::flip(&pauses),
            longest: None,
        },
    );
    drop(rebuilding);
    drop(timing);
    let store = open(&reference, &g.profile)?;
    let t = Instant::now();
    let report = store.purge(
        &workload::command("p0", "payload.purge", "purge"),
        &material.into_iter().collect::<Vec<_>>(),
        "benchmark material purge",
    )?;
    figure(&mut run, "Purge including scrub", ms(t.elapsed()));
    if !report.scrubbed {
        return Err("purge scrub incomplete".into());
    }
    let t = Instant::now();
    if !store.scrub()?.scrubbed {
        return Err("standalone scrub incomplete".into());
    }
    figure(&mut run, "Standalone scrub", ms(t.elapsed()));
    let t = Instant::now();
    let verified = store.verify(&ProjectId("p0".into()), None)?;
    figure(&mut run, "Verify reference project", ms(t.elapsed()));
    if !verified.chain.is_intact() || !verified.payloads.is_empty() {
        return Err("reference verification failed".into());
    }
    Ok((
        run,
        json!({"rebuild_pauses":pauses,"rebuild_writer_waits":rebuild_waits,"contention_waits":waits,"reference_bodies_checked":verified.bodies_checked,"reference_tombstones":verified.tombstones_checked}),
    ))
}
fn writer(home: &Path, p: &Path, project: &str, id: &str, seconds: u64, mode: &str) -> Result<()> {
    let profile = profile(p)?;
    let waits = Arc::new(Mutex::new(Vec::new()));
    let captured = waits.clone();
    let mut options = fixture_options(&profile);
    options.queue_wait = Some(Arc::new(move |d| captured.lock().unwrap().push(ms(d))));
    let store = SqliteStore::open(home, &now(), options)?;
    let text = Text::new();
    let wid: u64 = id.parse()?;
    let pid = u64::from(std::process::id());
    fs::write(home.join(format!("ready-{mode}-{id}")), b"")?;
    while !home.join(format!("start-{mode}")).exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut rng = Rng(wid.wrapping_mul(7919) ^ pid);
    let mut n = 0;
    while if mode == "rebuild" {
        !home.join("stop-rebuild").exists()
    } else {
        Instant::now() < end
    } {
        n += 1;
        let roll = rng.below(100);
        let count = if roll < 10 { 10 } else { 1 + rng.below(3) };
        let external = (10..30).contains(&roll);
        let events = (0..count).map(|i| {
            let attachments = if i == 0 { let seed = if rng.below(10) == 0 { 0xC0FFEE^rng.below(32) } else { (wid<<48)^(pid<<24)^n }; vec![workload::prepare("output",&text.make(seed,1024+rng.below(8) as usize*1024,0.6),false)] } else { vec![] };
            workload::NewEvent { stream:format!("dispatch/1/w{id}-{pid}"),etype:if external { "suite.result" } else { "task.run" }.into(),phase:1,payload:json!({"phase":1,"facts":{"status":"ran"},"text":format!("writer {id} run {n}.{i}")}),attachments,git:true }
        }).collect();
        workload::transact(
            &store,
            &workload::Command {
                project: project.into(),
                kind: "dispatch".into(),
                request_id: format!("w{id}-{pid}-{n}"),
                phase: 1,
                external,
                authority: Some(("plan.approved".into(), "plan/1/".into())),
                events,
            },
        )?;
    }
    println!("{}", json!({"committed":n,"waits":*waits.lock().unwrap()}));
    Ok(())
}
fn guard_once(home: &Path, p: &Path, request: &str) -> Result<()> {
    let profile = profile(p)?;
    let options = fixture_options(&profile);
    let at = now();
    let t = Instant::now();
    let store = SqliteStore::open(home, &at, options)?;
    store.get(
        &ProjectId("p0".into()),
        "guard_policy",
        &workload::key("guard"),
    )?;
    workload::transact(&store, &guard_command("p0", request, 0))?;
    println!("{}", ms(t.elapsed()));
    Ok(())
}
fn seed(home: &Path) -> Result<()> {
    fs::create_dir(home)?;
    let store = SqliteStore::open(home, &now(), workload::seed_options())?;
    let project = ProjectId("seed".into());
    store.create_project(&project, "Owner hand run", &now())?;
    let mut hash = None;
    for n in 0..3 {
        let mut command = workload::command("seed", "seed", &format!("seed-{n}"));
        command.recorded_at = now();
        let recorded = store.transact(&command, &mut |_| {
            Ok(Decision {
                kind: OutcomeKind::Done,
                answer: json!({"note":format!("synthetic seed answer {n}")}),
                sensitive: n == 1,
                observed: Observed::default(),
                git: None,
            })
        })?;
        if let Recorded::New {
            outcome:
                Outcome {
                    answer: Answer::Stored(reference),
                    ..
                },
            ..
        } = recorded
        {
            hash = Some(reference.hash);
        }
    }
    println!(
        "project {}\npayload {}",
        project.0,
        hash.ok_or("seed has no sensitive answer")?.to_hex()
    );
    Ok(())
}
fn machine(root: &Path) -> Value {
    let cpu = fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
    });
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").ok();
    let filesystem = Process::new("findmnt")
        .args(["-no", "FSTYPE,SOURCE", "--target"])
        .arg(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    unsafe extern "C" {
        fn sqlite3_libversion() -> *const std::ffi::c_char;
    }
    // The adapter links SQLite, whose version pointer names a static C string.
    let sqlite = unsafe { std::ffi::CStr::from_ptr(sqlite3_libversion()) }.to_string_lossy();
    json!({"cpu":cpu,"kernel":kernel,"filesystem":filesystem,"sqlite":sqlite,"cache":"warm after loading"})
}
fn execute(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("seed") if args.len() == 3 => seed(Path::new(&args[2])),
        Some("guard-once") if args.len() == 5 => guard_once(Path::new(&args[2]),Path::new(&args[3]),&args[4]),
        Some("writer") if args.len() == 8 => writer(Path::new(&args[2]),Path::new(&args[3]),&args[4],&args[5],args[6].parse()?,&args[7]),
        Some("run") if args.len() >= 3 => {
            let root = PathBuf::from(&args[2]); let mut runs = 5usize; let mut seconds = 30; let mut results = root.join("results");
            let mut p = PathBuf::from("spikes/evidence-ledger-bench/profile/baseline-store.json");
            let (flags, remainder) = args[3..].as_chunks::<2>();
            for pair in flags { match pair[0].as_str() { "--runs" => runs = pair[1].parse()?, "--seconds" => seconds = pair[1].parse()?, "--results" => results = PathBuf::from(&pair[1]), "--profile" => p = PathBuf::from(&pair[1]), _ => return Err(format!("unknown option {}",pair[0]).into()) } }
            if !remainder.is_empty() || runs == 0 || seconds == 0 { return Err("run requires positive runs and seconds, with values for every option".into()); }
            fs::create_dir_all(&root)?; fs::create_dir_all(&results)?; let p = fs::canonicalize(p)?; let g = Generator::new(&p)?;
            let mut measured = Vec::new();
            for n in 1..=runs { eprintln!("measuring run {n}/{runs}"); let (run,detail) = measure(&root.join(format!("run-{n}")),&g,&p,seconds)?; fs::write(results.join(format!("run-{n}.json")),serde_json::to_vec_pretty(&json!({"figures":run,"details":detail}))?)?; measured.push(run); }
            let table = report::markdown(&measured); fs::write(results.join("summary.json"),serde_json::to_vec_pretty(&json!({"machine":machine(&root),"runs":measured,"table":table}))?)?; println!("{table}"); Ok(())
        }
        _ => Err("usage: baley-bench run HOME [--runs N] [--seconds N] [--results DIR] [--profile FILE], or seed HOME".into()),
    }
}
fn main() -> std::process::ExitCode {
    match execute(&std::env::args().collect::<Vec<_>>()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("baley-bench: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
