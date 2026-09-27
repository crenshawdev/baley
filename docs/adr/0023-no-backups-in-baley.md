# 0023: Keep whole-store backups outside Baley

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | [0003: Keep one ledger database per user, outside any checkout](0003-per-user-database.md), in part |
| Superseded by | |

## Context and problem

A whole-store copy can preserve a ledger against database or disk loss, but restoring it returns the ledger only to the moment the copy was taken. Baley cannot make that copy current with later events it no longer has. A purge also cannot reach arbitrary copies outside Baley's home, including system snapshots.

The store already has a per-project export that verifies on its own and records its target so a later purge can list affected exports.

## Decision drivers

- Keep the restoration limit explicit.
- Avoid suggesting that a Baley-managed directory makes all copies reachable by purge.
- Retain a verifiable per-project export for the owner.

## Considered options

1. Make and manage whole-store backups inside Baley's home.
2. Leave whole-store copying to outside tools and keep Baley's per-project export.
3. Offer no way to make a copy.

## Decision

Chosen option: **Option 2**. Baley makes no backups and has no backup command. The owner copies the store using SQLite's backup API, a filesystem snapshot, or a plain copy with Baley stopped. Restore puts that copy in place while Baley is stopped; `verify` checks it after opening. If the restored chain falls behind the latest remote anchor, the owner may run `acknowledge-restore` under [ADR 0022](0022-acknowledged-restore.md).

## Consequences

### Positive

- Copy and restore procedures state their limits without a second Baley-managed retention path.
- Per-project exports remain standalone and verifiable.

### Negative

- A restore loses events committed after the copy's moment.
- A purge cannot reach outside copies and cannot promise removal from whole-home system backups.

### Follow-up

- The CLI offers export and doctor, with no backup command or backup benchmark figure.

## Options in detail

### Manage backups in Baley

This could automate a copy, but it would still be stale at restore and would not cover copies made by the operating system or other tools.

### Use outside tools

The owner chooses when and how to copy the full store. SQLite's backup API or a filesystem snapshot can copy a running store; a plain file copy requires Baley to be stopped.

### Offer no copy path

This would leave database or disk loss without a recovery path.
