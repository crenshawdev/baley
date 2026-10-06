# 0036: Keep guard records per user and bound the guard's access

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-06 |
| Deciders | John Crenshaw |
| Design document | [0010: Guard](../design/0010-guard.md), [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The guard hook answers Claude Code's pre-tool call before each `Bash`, `Monitor`, `PowerShell`, `Read`, `Grep`, `Glob`, `Write`, `Edit` and `NotebookEdit` call runs. GRD-R9 says an ask that cannot be recorded is denied, because an ask nobody can audit or replay is not a gate. GRD-R10 says a call the host delivers again gets the answer it got the first time, even after the policy changed. Both need a record the hook can always write.

Design 0010 first put the guard's records on the session project's own chain. That chain exists only once the project is in this machine's ledger, and a fresh clone is not there until the owner runs `baley init`. The hook must never create a project, admit a checkout or record a policy: those are the owner's steps and the server's preparation, and checkout admission is what refuses a fork. So a per-project record fails exactly where an agent first works in a new clone, and every ask there would be denied.

The hook is also a process Claude Code starts once per tool call and stops after 10 seconds. Before Build 3 T9 the inherited hook waited without limit on its old store's lock and could run git for up to 9 seconds. Whatever the hook records, it has to record inside that timeout, with every wait bounded, and say so when it cannot.

## Decision drivers

- An ask is recorded or denied, in a fresh clone, an unmanaged checkout or outside any project alike.
- The hook never creates or admits the session project, never runs the policy step and never writes to a chain it does not own.
- A redelivered call gets its confirmed answer, and one call id never has two records.
- Torn settings never open the door, and nothing remembered ever allows.
- The hook answers inside Claude Code's 10-second timeout, with every wait bounded.
- No storage port change, no new store error and no new dependency.

## Considered options

1. Per-user records in `user`, with an answer view keyed by host, session and call id, and remembered denials keyed by session project root, target checkout root and host
2. Per-project records on the session project's chain
3. A request id derived from host, session and call, with the store's `request` view as the redelivery key
4. A redelivery key of session and call only
5. Remembered policy keyed by the project file's id
6. Keeping an older `refuse` until the owner turns it off
7. Rebuilding `user` when a session's server starts
8. One store open held across git

## Decision

Chosen option: **1**, with the bounded access Build 3 T9 built as its supporting policy.

**Where records go.** Every guard record is one `guard.record` command in the per-user project `user`, on stream `guard`, at policy version 0, by `Actor::Baley`, with an empty scope and the hook form of the caller: the host, the working directory and the call's `tool_use_id`, with `CLAUDE_PROJECT_DIR` as given and the host's session when the call has them. The hook creates `user` when it is missing. The session project and the target path are facts in the caller and the payload, never the ledger project, so recording needs no project in this machine's ledger and admits no checkout. `guard.answered` keeps the digest of the input fields the guard read, never the command, and a torn settings file's words with git's stderr excerpt replaced by `[redacted]`.

**Redelivery.** The `guard` view in `user` is keyed by host, native session and call id, and holds the confirmed answer with its input digest, project directory and working directory. A call that matches all of them is replayed, even after the policy changed. The hook looks for it before it reads git or any policy, and the audit transaction looks again before it appends, so two deliveries racing each other leave one record. Request ids stay fresh random UUIDs. A call id already recorded for another input digest, project directory or working directory is judged on its own and is unrecordable: nothing is appended, its ask is denied, a deny stands and a pass on failure stays a loud pass.

**Remembered denials.** The `guard_policy` view in `user` is keyed by the session project's canonical root, the canonical root of the checkout the hook's working directory is in, and the host. It keeps only the denials of the last complete policy recorded beside an answer: whether `git.on_protected` was `refuse`, whether hard fail was on, and the protected list when either was. `guard.policy_recorded` is appended only inside the audit transaction beside a `guard.answered`, and only when the denials changed, so a newer complete policy with no denial clears an older `refuse` once a call under it is recorded. A call that passes records nothing, so it clears nothing. It is read only when the settings are torn, inside that transaction.

**Supporting policy: bounded access.** The hook keeps one budget per call: 8 seconds of work inside the 10-second timeout, git at most 5 seconds shared by the branch lookup and the bounded reader of HEAD's `baley.toml`, storage waits at most 2 seconds, and at least 1 second kept for writing the answer. The guard's two git callers are the only launches the launch validator lets run at a timeout other than their registered deadline: each runs on the time its grant gives, above zero and at most 5 seconds. The hook opens the store twice at most, each time briefly on the storage time left: once for the lookup before git, once for the audit transaction after it. A store opened for the guard never rebuilds views and never takes the maintenance lock. It answers views behind this binary's as needs-rebuild, which the hook treats like busy: the decision is unrecordable, and the hook's stderr line names `baley rebuild user`. That owner command, or any normal use of `user`, brings the views current.

**Answers.** The hook renders its answer in Claude Code's pre-tool hook form, within the host and security bar of [ADR 0033](0033-host-security-bar.md): an ask is Claude Code's own ask, a plain pass prints nothing and is never `allow`, and an ask that cannot be recorded becomes a deny under GRD-R9's audit rule, not as an adaptation for a host.

## Consequences

### Positive

- A fresh clone gets recorded answers with no `baley init` first. In an unmanaged checkout or outside any project, commands pass with nothing recorded, and a path tool's denial is still recorded.
- No hook writes to a project's chain, so checkout admission and the policy step stay with the server's preparation and the command line.
- A redelivery after a timeout gets the same answer, and two racing deliveries leave one record.
- Every wait is bounded, so the hook answers inside the host's timeout, and a busy or stale store denies an ask instead of passing it.
- The storage port, the store's errors and `Cargo.lock` are unchanged.

### Negative

- Guard records are not on the session project's chain, so that project's export, verification and anchors do not carry them. They are read from `user`.
- The replay binding is exact. A genuine redelivery whose classified input, project directory or working directory text differs is a clash, and its ask is denied.
- Until the owner runs `baley rebuild user`, or normal use brings `user` current, every ask is denied. Claude Code's hooks reference sends a hook's stderr on exit 0 to its debug log only, so the line naming the command may not reach the owner there.
- The remembered denials are per checkout root, so a new worktree of a project has none until a complete policy is recorded from it.
- Each guard process's write checks that every event type in `user`'s chain is readable, because a store keeps the head it last checked only in memory, so the cost grows with `user`'s chain.

### Follow-up

- Build 3 T11 renders the hook and stub content and supplies the placement seam the guard's stub list plugs into, and T15 installs the stubs.
- Build 3 T12 observes how Claude Code handles each answer, whether its matcher accepts `Monitor` and `PowerShell`, a real redelivery, lock contention, and what the readability check costs each guard process.
- Build 3 T13's doctor reports the hook's observations and a `user` that needs a rebuild.
- Build 5 adds the lease to the write rule, and Build 9 bounds the readability check.

## Options in detail

### Per-user records in `user` (chosen)

`user` already exists for the model catalog, carries policy version 0 and is created by Baley, so the hook can create it without touching a project. Keeping the session project and the target as facts keeps them in the record without needing their chains. It meets every driver. Its costs are the negatives above.

### Per-project records on the session project's chain

What design 0010 first described. It needs the project in this machine's ledger, so a fresh clone before `baley init` could record nothing and every ask there would be denied, or the hook would have to create or admit the project, which only the owner's `baley init` and checkout admission may do. It would also put checkout admission and the policy step in front of every recorded answer, outside the 10-second budget.

### A request id derived from host, session and call

A name-based UUID from host, session and call would let the store's own `request` view find a redelivery. It adds `sha1_smol` to `Cargo.lock` for UUID version 5, request ids stop being fresh random UUIDs, and the `request` view knows nothing of the input, project directory or working directory a replay must match.

### A redelivery key of session and call only

A call id is unique only within its host, and the caller records the host anyway. Leaving it out of the key saves nothing and would let a second host's ids collide with Claude Code's.

### Remembered policy keyed by the project file's id

The id is read from the working tree's `baley.toml`, which is the file most likely to be torn exactly when the remembered denials are needed. The canonical project root and checkout root come from the walk alone.

### Keeping an older `refuse` until the owner turns it off

A `refuse` that only an explicit change clears would outlive the owner's own policy: after the owner commits `on_protected = "ask"`, a later torn file would still deny. The latest complete policy is the owner's word, so it replaces the remembered denials whole.

### Rebuilding `user` when a session's server starts

Stale views would rarely reach the guard. It changes the server's startup, makes every session start wait on the maintenance lock behind any rebuild in progress, and still leaves the time between an upgrade and the next session. The owner's existing `baley rebuild user`, and any normal use of `user`, already bring it current.

### One store open held across git

A store opened for the guard fixes when its storage time ends at open and tries nothing once that time is spent. Holding one open across git, which may take up to 5 seconds, would spend the 2-second storage time, and every record after a slow git would answer busy. Opening once after git instead would read the policy before the redelivery check.
