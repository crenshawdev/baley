# 0024: Separate port conformance from adapter mechanism tests

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The evidence ledger's [Testing table](../design/0001-evidence-ledger.md#testing) includes behaviours every storage adapter must provide, mechanisms specific to SQLite, and the core's anchor command path. A second adapter needs the same behavioural contract without inheriting SQLite's writer queue, timing dependency or physical schema.

The storage port exposes commands, history, views, payloads and administration. It does not expose raw rows, writer queue turns or pause timing. The placement of the suite and its factory in `baley-store` follows [ADR 0005](0005-storage-port.md) and [ADR 0021](0021-claim-rules-in-the-port.md).

## Decision drivers

- Prove the same observable contract for every adapter.
- Keep engine mechanisms outside the production port.
- Preserve assertions the port cannot express.
- Name each failing behaviour separately.

## Considered options

1. Separate port conformance checks from adapter mechanism tests and core tests.
2. Expose timing and writer queues through the port and include their checks in the suite.
3. Put every storage-related test in the conformance suite.

## Decision

Chosen option: **Option 1**. Every behaviour observable through the port that a second adapter must also provide is a public generic check in `baley-store::conformance`. Each adapter invokes `conformance_suite!`, which creates one test and one fresh factory per check. The suite owns scenarios and assertions; the factory owns engine operations for creating, reopening, damaging, copying, restoring and exporting stores, and interrupting or interleaving rebuilds.

Each adapter proves its mechanisms with its own tests. These include rebuild pause timing, cleanup time bounds and writer-queue interleaving, because the port exposes neither the queue nor timing. They also include facts the port cannot observe or write: another project's raw rows in an export, trace writes fenced by the epoch, malformed compressed bodies, an anchor row stored with another tag, and scratch rows left after failed verification. Raw generation numbers, stamps, lease-row counts and stored JSON text remain adapter assertions where a port read cannot distinguish their damage.

The core's tests over an adapter continue to prove the anchor call path, remote observations, heartbeat and reconciliation decisions. The suite uses supplied times, anchors and findings and never calls a forge.

## Consequences

### Positive

- A new adapter runs the same named behavioural checks.
- SQLite details do not become requirements of the production port.
- Mechanism assertions remain beside the implementation they protect.

### Negative

- Passing the suite does not prove an adapter's mechanism tests are complete.
- Each adapter must supply its own corruption and rebuild harness.
- Unit tests do not prove process scheduling, physical durability or the assembled workflow.

### Follow-up

- Every new adapter passes the suite and supplies its own mechanism tests.
- Migration checks arrive with the first migration; search checks arrive with Build 8.

## Options in detail

### Separate conformance and mechanisms

Portable assertions run once through the shared suite. Each adapter retains the assertions requiring its own internals, and core tests exercise domain decisions through the port. This preserves coverage without requiring a common physical implementation.

### Expose timing and writer queues

This would make timing checks portable by imposing a queue and clock interface on every adapter. Those are implementation choices, and widening the production port for them would couple domain-facing traits to maintenance machinery.

### Put everything in the suite

This would require engine-specific escape hatches for raw rows and the core's forge path. The suite would cease to describe the port's contract, and `baley-store` would need responsibilities owned by the adapter or core.
