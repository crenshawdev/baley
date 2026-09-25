# Evidence ledger benchmark

Measurement code for acceptance gate 1 of [design 0001](../../docs/design/0001-evidence-ledger.md): does the evidence ledger meet its performance and size budgets on SQLite? It is a spike. Nothing here ships, and it lives outside the Cargo workspace.

## Run it

```
cargo build --release
./target/release/evidence-ledger-bench run target/bench-home > results.json
```

The home directory must be on the local disk you want to measure. A `tmpfs` directory measures memory, not durability.

## What it builds

A prototype of the design's SQLite adapter, with the parts that cost time or space:

- events with the per-project hash chain over canonical JSON, and a `command.completed` event per command;
- payloads compressed with zstd, stored once by SHA-256, with per-reference rows;
- one view table per view family, updated by a projector in the same transaction, with provenance on every document;
- the request view, checked first in every transaction, so a retry records nothing;
- an FTS5 search table fed from each event's text;
- `BEGIN IMMEDIATE`, write-ahead log, `synchronous=FULL`, `secure_delete=ON`, page size 8 KiB;
- the open checks: owner, modes, symbolic links, network filesystem, compatibility epoch.

It leaves out what does not change the numbers: real domain rules, upcasters, anchors, the claim protocol and the port traits.

## The workload

`profile/cadence-4.0.json` describes the store left by the Cadence 4.0 build (about three weeks, 40 phases): 1,551 commands and 1,615 events, each with the inline size it had and the size, retention class and identity of each attachment. It holds numbers only; no content from the store is in it. It was produced once, locally, from that store, mapping each old record to the event the design would record for it and dropping the copies the design removes: plan receipts, copies of execution records inside verification attempts, and copies of plan files inside verification runs. Retired prompt text is kept, as the size budget requires.

The generator replays the profile with synthetic text. Text for each retention class is tuned to the compression ratio measured on the real content of that class, and identical real content produces identical synthetic content, so deduplication behaves as it would have. Guard events are not in the old store; the generator adds one guarded git command for every third task command. Everything is deterministic.

## What it measures

Every budget in the design's Performance table, on two databases: the reference project alone, and five projects.

| Measurement | How |
|---|---|
| Size of the reference workload | Database after a checkpoint, with every payload body under default retention |
| Commit of one command | Time from holding the write lock to commit, while loading |
| Open with checks | 500 opens in one process |
| Server start | Open plus `quick_check` of the five-project database |
| View get by key | 20,000 random keys, prepared statement |
| Cold guard | 200 new processes, each opening, reading a view and recording one guard event; wall time and store time |
| Write-lock wait | 8 writer processes committing continuously for 20 seconds across five projects |
| Rebuild | Every view of the reference project into shadow tables in batches of 200 events, then one swap transaction |
| Backup, purge, checkpoint | `VACUUM INTO` while open; purge of the reference project's review material, checkpoint and `VACUUM` |

## Results

Five runs on each of two drives, 2026-09-25: `results/2026-09-25-t700/` (Crucial T700) and `results/2026-09-25-p3plus/` (Crucial P3 Plus), each with one file per run and a `summary.json`. They are summarized against each budget in design 0001's Performance section, together with the design changes the runs led to. Measurements follow loading on the same machine, so the page cache is warm.

`./target/release/evidence-ledger-bench run <home-root> <runs> <results-dir>` reproduces a set. `BENCH_GATE=0` in a writer's environment turns the writer queue off; the harness runs both modes itself.
