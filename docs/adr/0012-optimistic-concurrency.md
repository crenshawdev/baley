# 0012: Use optimistic concurrency in the shared server

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

With one shared server (ADR 0011), several sessions, workers and the guard hook write the ledger at once. An earlier ruling for the 4.0 rewrite allowed no cross-session concurrency at all: one writer, other sessions read-only. That ruling predates the shared server and the ledger's writer queue (0001, EVD-R6 to EVD-R8), which already serialize commits and re-check inputs inside the write.

Contention within a project is low: one dispatch is active per sprint, and most writes are short appends. Contention across projects is only at the database's single writer lock, measured in milliseconds (0001, Performance).

## Decision drivers

- Several sessions write at once without corrupting or silently merging anything.
- No record is locked while an agent works, however long that is.
- A command decides on the inputs it read, and never on inputs that changed under it.
- The rule is the same for every caller: session, worker, hook, command line.

## Considered options

1. No cross-session concurrency: one writer, other sessions read-only
2. Pessimistic locks held for the duration of a command
3. Optimistic concurrency: append-only events, short write transactions, decisions made inside the write from inputs read there, stale inputs refused

## Decision

Chosen option: **3**, which replaces the earlier no-cross-session ruling. Events are appended, never overwritten. Each command's decision is made inside one short write transaction from inputs read in that transaction; a command whose inputs changed since it was prepared is refused as stale and its caller reads again and decides again. Nothing is locked while an agent works; the writer queue holds the database for the length of one commit only. Long work (a suite, a git step) is claimed, done, then recorded (SYS-P7), so it holds no lock while it runs.

## Consequences

### Positive

- Every session, worker and hook writes through one rule with no coordination between them.
- A stale decision is refused with a code, never merged.
- Long-running work never blocks another writer.

### Negative

- A caller must be ready to read again after a stale refusal.
- Every decision must be written as a function of inputs read inside the transaction, which constrains how commands are coded (0001, `decide`).

### Follow-up

- Every command that grants authority confirms its facts against events inside its write (EVD-R27).

## Options in detail

### No cross-session concurrency

One writer, everyone else reads. Simple, but a second session or a worker cannot record anything, and the guard hook, which writes on every agent commit, cannot run beside a session. Fails the first driver outright.

### Pessimistic locks

Lock the records a command touches until it finishes. Safe, but a dispatch runs for minutes or hours; locking its sprint would block the guard, progress and every other reader. Fails the second driver.

### Optimistic concurrency (chosen)

Short writes, decisions inside them, stale inputs refused. The ledger already works this way (EVD-R6, EVD-R7, EVD-R26); the shared server keeps it and adds nothing.
