# 0022: Report owner-acknowledged restores behind a remote anchor

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | [0035](0035-restore-purge-uncertainty.md), in part: the reporting and recovery contract |

## Context and problem

An earlier copy of the database can be locally intact but end before, or disagree at, the latest forge anchor. The immutable tag makes this difference detectable ([ADR 0007](0007-forge-anchors.md)). An ordinary anchor push must refuse that copy, even after its chain grows past the tag's sequence.

The owner may still need to continue from the copy after accepting that the missing or changed interval cannot be recovered. The acceptance must stay visible in later verification.

## Decision drivers

- Preserve the remote anchor as an outside witness.
- Let the owner resume anchoring an intact restored chain.
- Keep every accepted gap visible after later anchors land.

## Considered options

1. Refuse every push until the missing history is recovered.
2. Let the owner record an acknowledgement event naming the remote anchor and restored head.
3. Move or replace the remote tag.

## Decision

Chosen option: **Option 2**. The owner-only `anchor.acknowledge_restore` command checks the latest remote anchor and the local chain, then records `anchor.restore_acknowledged` with both heads. Its `anchor` scope waits for an open anchor claim to be reconciled. Verification recognizes an exact owner event bound to the chain head before it and reports `Acknowledged` against that remote anchor. Later matching anchors do not erase the acknowledgement from the report.

## Consequences

### Positive

- Anchoring can resume from an intact restored copy without changing immutable tags.
- `verify` and `doctor` continue to show the accepted gap.

### Negative

- History between the copy and the old remote anchor is not recovered.
- A process able to rewrite the database can also append an apparent owner acknowledgement. The event is therefore always reported, never treated as hidden repair or proof that the owner actually approved it.

### Follow-up

- The CLI exposes the command only to the owner. The MCP server does not offer it to agents.

## Options in detail

### Refuse every push

This keeps the original witness strict but makes a usable earlier copy permanently unable to anchor when lost history cannot be recovered.

### Record an acknowledgement

The event preserves the remote witness and the local restore point in the chain. A push at a sequence whose tag already exists can be refused; a later anchor above the old latest sequence can land.

### Move or replace the tag

This would destroy the outside evidence of the earlier head and violate the immutable tag rule.
