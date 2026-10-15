# 0043: Record every submission in the ledger as a draft

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-13 |
| Deciders | John Crenshaw |
| Design document | [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md), [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0001: Evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

A scope change, a story's truths and a plan each reach the owner as a submission: a whole, validated draft identified by its digest, which the owner approves by that digest. Designs 0004 and 0005 had Baley hold a submission as a draft until it was approved, replaced or the session that made it ended, and recorded nothing until the approval. No earlier decision record decided where drafts live.

A draft held by one server for one session is lost when that session ends or the server restarts. Each session runs its own Baley server ([ADR 0034](0034-one-server-per-session.md)), and the server mints a new session id at every start ([0012](../design/0012-host-interface.md) section 7), so a restarted server cannot even tell which drafts were its own. No other session sees such a draft, and neither does the owner's terminal, which is a separate process from every session's server. The inherited engine kept its context and plan drafts in memory the same way and lost them on every restart.

The owner reviews a draft at their own pace, sometimes over more than one sitting, and approves it at the terminal ([ADR 0044](0044-owner-acts-on-the-command-line.md)). The question is where a draft lives so that it outlasts the session that made it and the terminal can read and approve it.

## Decision drivers

- A draft survives the session that submitted it ending and its server restarting.
- The owner reads and approves a draft from the terminal, outside any session, and any session can read it.
- The owner approves exactly what was submitted, named by its digest.
- Every operational record lives in the ledger ([ADR 0006](0006-no-markdown-records.md)).
- Accepted scope, truths and plans change only on the owner's approval.

## Considered options

1. Hold drafts in the server, per session (the earlier design)
2. Write drafts as files in the repository
3. Record drafts as ledger events

## Decision

Chosen option: **3**, because it is the only one that keeps a draft past a restart, lets the terminal and every session read it by digest, and keeps it in the ledger with the rest of the record.

Every scope, truth and plan submission is recorded in the ledger when it is submitted, as an event whose payload holds the submission's digest and kind and a reference to its body, stored as a `record` payload. A scope submission is `scope.submitted` on the `project` stream. The digest is taken over the submission's content alone, so identical content has one digest from any session.

A recorded draft survives a restart. It can be read by its digest from any session and from the terminal, and it is approved by its digest only at the terminal ([ADR 0044](0044-owner-acts-on-the-command-line.md)). Recording a submission changes no accepted scope, truths or plans: the views that hold them ignore submissions, and only an approval changes them.

A newer submission for the same change ends an earlier draft's eligibility: approving the earlier one is refused `stale-draft`. The earlier draft keeps its record, and any snapshot a checker needs from it stays. The latest submission is the newest draft even when it repeats an earlier digest, so after A, then B, then A again, approving A succeeds and approving B is `stale-draft`. A session's end alone discards no draft and abandons no question set.

## Consequences

### Positive

- A server restart or the end of a session loses no draft, and another session can pick up the work.
- The owner reads and approves at the terminal, with no session open.
- Every draft's provenance, who submitted it, when and under which policy, is in the hash chain beside its approval.
- A draft's body follows the ledger's retention rules: a body this project purged answers its tombstone and reason and can no longer be approved.

### Negative

- Every submission appends an event and stores its body, and a body is kept for the life of the project unless purged, including drafts never approved.
- Reading a draft opens a payload, so the approval reads the body before its transaction, which has no payload read, and confirms inside the transaction that nothing it read has moved.

### Follow-up

- Build 4 T3 (#25) builds it for the start submission: `scope.submitted`, the `draft` view and the approval by digest.
- Build 4 T6, T7, T9 and T10 apply it to question sets, truths and plans.
- How a pending round-2 check is keyed, and how a draft whose re-check was refused leaves that state, stay with Build 4 T9 and T10 ([0005](../design/0005-context-plans-and-acceptance.md) section 12).

## Options in detail

### Drafts held in the server, per session

The server keeps each draft in memory, or in a store keyed by its session, until it is approved, replaced or the session ends. Nothing is written before approval. A restart or a closed session loses the draft, no other session can read it, and the terminal cannot approve it, since it is a separate process with no access to the server's memory. Storing drafts per session across a restart does not help, because the restarted server mints a new session id.

### Draft files in the repository

The session writes each draft to a file in the checkout, where the terminal and other sessions can read it. [ADR 0006](0006-no-markdown-records.md) keeps every operational record out of the repository and in the ledger. A file in the checkout can also be edited by an agent between the preview and the approval, and it travels with branches and commits that have nothing to do with the draft.

### Ledger events

Each submission is an event with its body stored as a `record` payload, and a `draft` view finds a draft and the newest of its kind by digest. Drafts survive restarts, every session and the terminal read the same record, and the chain shows each draft and its approval. Each submission costs an event and a stored body.
