# 0005: Put storage behind a port with engine adapters

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | [0010](0010-projector-traits-in-the-port.md) (in part: where projectors live) |

## Context and problem

In the current code, domain services read and write raw JSON values of the whole store. Storage concerns (integrity digests, generations, namespace equality) are mixed into domain logic across the codebase. Choosing SQLite (ADR 0002) should not repeat that coupling with SQL in its place, and the engine should be replaceable.

## Decision drivers

- Domain code must not depend on the engine.
- The boundary must be enforced, not a convention.
- The interface must fit Baley's access patterns, not generic storage.
- Every adapter must behave the same way.

## Considered options

1. Call SQLite directly from domain services
2. A generic repository interface (create, read, update, delete over entities)
3. Ports and adapters: a domain-shaped port, projectors in the domain, a conformance suite

## Decision

Chosen option: **3, ports and adapters**, enforced by crate boundaries. `baley-core` holds views, projectors and domain rules. `baley-store` holds the event, its canonical form and the hash chain, which both sides of the port speak, the port traits (ledger, transaction, views, payloads, search, admin) and the conformance suite. `baley-store-sqlite` implements the port. The core crate does not depend on rusqlite, so domain code cannot reach SQL. Projectors, the code that turns events into view changes, live in the core and are handed to the adapter, so no business rule lives in an adapter.

## Consequences

### Positive

- The engine can change by writing an adapter that passes the suite.
- Domain code reads as domain code; storage failures surface as typed errors.
- The conformance suite defines the port's behaviour precisely: ordering, atomicity, idempotency, chain integrity, payload round trips.

### Negative

- A layer to design and maintain.
- Engine features must be exposed as named capabilities (search, backup) rather than used directly.
- The current single crate must be split, which touches most modules as their record family moves.

## Options in detail

### Option 1: SQLite directly

Least code at first, and the same coupling the JSON store had, with SQL instead of JSON values.

### Option 2: generic repository

A well-known anti-pattern for this case: it hides the access patterns that matter, and engine specifics leak back through query options.

### Option 3: ports and adapters

The standard way to keep a domain independent of infrastructure, with the boundary checked by the compiler.
