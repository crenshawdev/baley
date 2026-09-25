# 0002: Use SQLite as the storage engine

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The ledger (ADR 0001) needs an engine that gives atomic multi-record transactions, safe access from several processes on one machine, keyed and indexed reads, full-text search, and durability across power loss. Baley ships as one binary per platform with no runtime dependencies, so the engine must embed.

## Decision drivers

- Atomic transactions across events, views and payloads.
- Several processes (MCP servers, the guard hook, the CLI) on one machine.
- Keyed and indexed queries, and full-text search with ranking.
- Durable once acknowledged.
- Embeddable, no daemon, no system library.
- Proven stability and a long-term file format.

## Considered options

1. SQLite through rusqlite with the bundled build
2. An embedded key-value store (redb or sled)
3. PostgreSQL
4. Git objects and references
5. JSON files (the current store)

## Decision

Chosen option: **1, SQLite**, through rusqlite with the `bundled` feature, which compiles SQLite from source (3.53.2 with rusqlite 0.40.1) with FTS5 enabled. The database runs in write-ahead-log mode with `synchronous=FULL` and `secure_delete` on. Acceptance of the design is gated on a benchmark of an adapter prototype against a reproducible workload.

## Consequences

### Positive

- ACID transactions, and readers that never block the writer.
- Real queries and indexes, JSON functions over payloads, and FTS5 search with BM25 ranking, which replaces the custom recall index.
- Online backup and integrity checking are built in.
- A file format that is stable since 2004 and recommended by the US Library of Congress for preservation. Limits far beyond Baley's needs: 281 TB per database, about 1 GB per value.

### Negative

- One writer at a time for the whole database. Write transactions must stay short, so no slow work may happen inside one.
- Write-ahead-log mode does not work over a network filesystem. The store must detect that and refuse.
- A long-lived reader can stop checkpoints and let the log grow. Reads must be short and the server checkpoints when idle.
- A C dependency compiled into the binary.

### Follow-up

- Benchmark harness for the performance budgets in the design document.

## Options in detail

### Option 1: SQLite

Meets every driver. The single-writer limit is acceptable because Baley's writes are a few small transactions per second at most, measured in milliseconds each.

### Option 2: redb or sled

Pure Rust and fast. No query language, no full-text search; every index is hand-built, which is the mistake the JSON store made.

### Option 3: PostgreSQL

Excellent engine, wrong shape: a daemon to install, run and upgrade on every developer machine, for concurrency a single-user tool does not need. Remains possible as a future adapter.

### Option 4: git

Content addressing, signing and transport built in. Every query still needs a local index, writers serialize on reference updates, and query latency on large histories is poor. Kept as a future transport, not the engine.

### Option 5: JSON files

The current store. No transactions across files, no indexes, and every read loads everything.
