# 0007: Anchor chain heads on the forge

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Each project's events form a hash chain (ADR 0001). A chain whose head is stored in the same database proves nothing against someone who can write that database: they can change an event and recompute every hash after it, cut the tail off, or restore an older copy, and verification will pass. The realistic attacker is an agent running as the owner's user, which can open the database with `sqlite3` and can sign with the owner's cached GPG key. Signing the head locally therefore does not help either.

## Decision drivers

- Detect rewrite, truncation and rollback by a process running as the owner.
- No new service to run.
- Use infrastructure the project already has.

## Considered options

1. Keep the head only in the database
2. Sign the head locally with the owner's key
3. Push the head to the forge as an immutable tag
4. Send the head to a third-party transparency service

## Decision

Chosen option: **3, anchors on the forge**. At every verified phase, every milestone step, and at least daily while a project is active, Baley pushes the tag `baley-anchor/<project_id>/<seq>`, annotated with the sequence and head hash, to the project's remote. The repository's tag ruleset forbids moving or deleting tags, for everyone. `baley verify` compares the local chain with the latest anchor and reports a rewrite, truncation or rollback at or before it. The push is a command with an external effect and follows the claim, act, record protocol.

## Consequences

### Positive

- A rewrite of anything before the latest anchor is detectable from outside the machine.
- Uses the repository's existing remote and rulesets; no new service.
- Anchors are visible, dated and ordered in the forge's own history.

### Negative

- Changes after the latest anchor are protected only by the local chain. The verify report states the unanchored range.
- A project with no remote gets local verification only.
- An attacker holding the owner's forge credentials and local access at once could push a matching anchor. Out of scope for this milestone.
- Tags accumulate: one per anchor point.

## Options in detail

### Option 1: head only in the database

Detects accidents and naive edits, nothing more.

### Option 2: local signature

The agent runs as the owner and can use the same signing agent, so a local signature proves nothing against it. A key that needs a physical touch would, and is future work.

### Option 3: forge tags

An outside witness the agent cannot rewrite, using what every project already has.

### Option 4: transparency service

Stronger independence, but a third-party dependency and network service for every project.
