# 0001: Record evidence as an append-only, hash-chained event ledger

| | |
|---|---|
| Status | Proposed |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Baley's records are its product: they are the proof that a plan was approved, that work was done, that tests passed and that a verdict was reached. The current store keeps the same fact in up to three places (a snapshot namespace, a mirror line in a log, a Markdown file) and spends most of its code keeping those copies in agreement. It keeps only current state in the snapshot, so history lives in the log copies.

The question is what the unit of record is: current state that is updated in place, or facts that are appended and never changed.

## Decision drivers

- A record must be attributable and must not change without detection.
- Every fact must exist once.
- Current-state questions must be cheap.
- Schema changes must not require rewriting history.

## Considered options

1. Mutable state documents (the current model)
2. Mutable state tables with an audit table beside them
3. An append-only event ledger with derived views (event sourcing)

## Decision

Chosen option: **3, an append-only event ledger with derived views**. Each fact is an immutable, typed, attributed event. Each project's events form a SHA-256 hash chain. Current state is held in views computed from the events by domain code, in the same transaction as the append, and rebuildable from the ledger at any time.

## Consequences

### Positive

- One copy of every fact; the reconciliation machinery goes away.
- Full history by construction, which is what an evidence record needs.
- Tampering with the database is detectable, and the first changed event is located.
- A view can change shape freely: change the projector, rebuild the view.

### Negative

- Two representations to reason about: the events and the views.
- Event payload schemas are permanent. Changes need versioned types and upcasters.
- The log grows without bound. Payload retention and purge (with the hash kept in the chain) are needed from the start.

### Follow-up

- Event type catalogue and payload schemas, per record family.
- Retention policy defaults.

## Options in detail

### Option 1: mutable state documents

What exists today. Simple to write, but history is lost on overwrite unless copied elsewhere, and the copies are the problem this redesign removes.

### Option 2: state tables plus an audit table

Familiar and simple to query. The audit table is a second copy of every change and drifts from the state it describes unless every write updates both, which is the same reconciliation problem in a new form.

### Option 3: event ledger with derived views

The events are the only source of truth and the views are disposable. Costs are the two representations and permanent event schemas, both well understood in practice.
