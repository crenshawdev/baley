# 0018: Enforce the lease at task close on both hosts, with the guard as an early stop where it sees writes

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0006: Execution](../design/0006-execution.md), [0010: Guard](../design/0010-guard.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

A plan's lease names the files and directories its tasks may change. During the 4.0 build, plans were written against code still being built, so an out-of-lease commit was allowed to close and recorded as a deviation. In a project Baley manages that reason does not hold. The hook that could stop a write as it happens sees `Write` and `Edit` on Claude Code and only `Bash` on Codex (host matrix), so a write-time rule alone behaves differently per host. A separate proposal to let Baley edit source itself, checking the lease as it wrote, was rejected: Baley reads source and writes records.

## Decision drivers

- The lease means something to an agent, on both hosts alike.
- The owner decides exceptions, on the record.
- A lease is not a lock: reads and other plans' tasks are never blocked.
- No new write surface in Baley.

## Considered options

1. Deviation only (the build-time rule)
2. Refuse at task close; the owner rules per path
3. Refuse at write time through the guard
4. Refuse at close on both hosts, with the guard as an early stop where it sees writes

## Decision

Chosen option: **4**. A task close is refused when a committed or staged path lies outside the plan's lease, naming each path; the owner rules per path: accept it as a deviation, or have the executor split the commit. This is the floor on both hosts. On a host whose hook sees file writes, the guard denies an out-of-lease write as it happens. Baley never edits source. A lease never blocks reads and never blocks another plan's tasks.

## Consequences

### Positive

- The same rule on Claude Code and Codex; the guard adds only an earlier warning.
- Every exception is the owner's, recorded per path.

### Negative

- A wrong lease from the planner stalls a task until the owner rules.
- Whether Codex runs a hook before `apply_patch` is unknown; until probed, Codex has no early stop.

### Follow-up

- Probe Codex for a hook before `apply_patch` when the guard is built (0012 open question).

## Options in detail

### Deviation only

Nothing stalls, and the lease means nothing.

### Refuse at close

Same on both hosts, one owner round per violation. The floor of the chosen option.

### Refuse at write time

Catches it earliest, but only on the host whose hook sees writes; breaks host neutrality if it is the rule.

### Close refusal plus guard early stop (chosen)

The floor from option 2, the early stop from option 3 where it exists.
