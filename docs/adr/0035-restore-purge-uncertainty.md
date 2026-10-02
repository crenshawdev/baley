# 0035: Report purge uncertainty after restoring a store

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-02 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | [0022](0022-acknowledged-restore.md), in part: the reporting and recovery contract |
| Superseded by | |

## Context and problem

Baley makes no backups ([ADR 0023](0023-no-backups-in-baley.md)). The owner supplies a copy of the store, and restoring one returns the store to that copy's moment. A copy holds only the purge history it had when it was taken. A tombstone exists only in the store that recorded the purge, so an older copy has no tombstone for a body a later purge removed, and it may hold that body again.

[ADR 0022](0022-acknowledged-restore.md) lets the owner accept an intact restored chain that ends before, or disagrees with, the latest remote anchor, and keeps the accepted gap in every later report. It does not say what the acceptance cannot undo. An anchor mismatch proves that two histories disagree about the project's chain. It does not say which events are missing, so it cannot identify a lost purge. A purge also cannot reach copies outside the store, including the copy that was restored.

Issue #146 reports a hand run: export a project, purge a body in the original store, restore the export, verify, and acknowledge. The body was back and nothing said so.

## Decision drivers

- An intact restored chain stays usable after the owner accepts it.
- Say plainly what a restore cannot undo.
- Claim nothing Baley cannot back: no named lost purge, no completeness claim.
- Keep the exit classes of acknowledgement and verification as they are.
- Add no store, schema or port change, and no new record for the owner to maintain inside Baley.

## Considered options

1. Explain the risk in the acknowledgement and in every report that lists the gap, and keep acknowledgement.
2. Have the owner repeat the purges they know of, from their own records.
3. Refuse the acknowledgement while a later purge is known.
4. Reapply removals automatically from surviving evidence.
5. Discard every body in the restored scope.
6. Refuse every detectable gap.

## Decision

Chosen options: **1 and 2 together**.

Acknowledgement stays available. Its success output, whether the acknowledgement is new, replayed, or recorded with an answer that cannot be read, carries a warning. So does every later `verify` (anchored and local-only), every project block of `doctor` and the output of a failed export, once per report, while the chain lists an accepted restore. That includes reports after a newer anchor matches. A refusal, an acknowledgement error and the `anchor` command print no warning, and neither does a report that lists no accepted gap.

The warning says that purges recorded in history the copy lacks may be missing, whether they came after the point the copy was taken or on a branch the copy replaced, and that bodies purged there may have reappeared. It says affected secrets must be rotated. It tells the owner to repeat the purges they know of by running `baley purge <project> <hash>... --reason <text>` for each project, from records kept outside the store. It says to review first any hash with nothing left to release, because one such hash refuses the whole request, and to check purge's report of bodies it kept because another reference still requires them. It names no copy time and no particular lost purge.

There is no purge registry and no automatic removal. Warnings change no exit code, and the acknowledgement event and its answer are unchanged. [ADR 0022](0022-acknowledged-restore.md)'s owner acknowledgement, its immutable anchors and its persistent reporting of the gap stay as they are. This record replaces only what 0022 left unsaid about reporting and recovery.

## Consequences

### Positive

- The owner learns at the moment of acceptance, and in every report after it, that a restore can bring back purged bodies.
- The recovery uses the `baley purge` command as it is.
- No store, schema, core or port change, so every existing chain, conformance and adapter test stands.

### Negative

- Baley cannot discover new loss after the latest anchor, or when there is no anchor, the remote is unreachable or the check is local-only. Such loss shows no gap and gets no warning of its own. A gap already accepted still warns in every report, local-only checks included.
- A matching or absent anchor does not prove the purge history complete. Design 0001 states this limit, and no report prints a completeness line, because Baley has nothing to back one.
- The owner must keep purge records outside the store and outside any copy of it: project ids, requested hashes and reasons. Baley does not keep them for the owner.
- One project's acknowledgement cannot reconcile a whole-store restore that holds several projects. Each project is acknowledged and re-purged on its own.
- The warning persists after newer anchors match, so a long-restored store keeps printing it.

### Follow-up

- Design 0001 and design 0011 state the contract.
- The runtime doctor in Build 3 reuses the warning text.

## Options in detail

### Explain the risk and keep acknowledgement (chosen, with the next)

Keeps the recovery 0022 chose and fills the gap in what it tells the owner. It adds text and no mechanism. It meets every driver.

### Repeat purges from the owner's records (chosen, with the previous)

The one person who knows which purges were requested is the owner, and `baley purge` already takes a project and hashes. Baley prints how to use it and does not pretend to know the list. The cost is that the owner has to keep the records.

### Refuse while a later purge is known

A later purge is known only if a surviving later store or an outside list says so, and Baley has neither at the moment of acknowledgement. Even where it is known, refusing would leave an intact copy unusable, which the first driver rules out.

### Reapply removals automatically from surviving evidence

A surviving later store holds typed purge events, so a future command could read them and purge for the owner. That is a possible extension and not an existing capability. It needs a source and branch binding, input validation, and a rule for hashes a later reference still holds, and a continually published registry would break the rule that the ledger is the single record. It is held back until an owner asks for it and says what it should read.

### Discard every body in the restored scope

Removes the question by removing the bodies, and it sacrifices attachments nobody asked to lose. The command has no purge-all argument, a project-only purge leaves shared bodies readable, and secrets would still need rotating. It would have to be an explicit owner choice and never a consequence of acknowledgement.

### Refuse every detectable gap

Makes every restore from an older copy a dead end whenever lost history cannot be recovered, which is what 0022 rejected.
