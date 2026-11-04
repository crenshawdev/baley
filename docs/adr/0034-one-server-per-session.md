# 0034: Run one Baley server per session over stdio

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-02 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md), [0002: System design](../design/0002-system-design.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | [0011](0011-one-shared-server.md); [0028](0028-one-http-stack.md), in part: the MCP server over HTTP on axum |
| Superseded by | |

## Context and problem

ADR 0011 chose one Baley server per user, reached through a stdio launcher or over HTTP from a background service. Its drivers were one writer process for the per-user ledger, so views, caches and the writer queue live once; every session and worker reaching the same server; no service required of anyone; and both hosts as they were. It turned down a server per session because the ledger would be written by several processes with separate caches, views would be rebuilt per process, and the writer queue would serialize through the file lock alone.

Several of those facts have changed or were measured since.

- Claude Code is the only host ([ADR 0033](0033-host-security-bar.md)).
- A live probe on 2026-10-02 (Claude Code 2.1.287, MCP protocol 2026-07-28 over stdio and over HTTP) found that a subagent's calls arrive on its session's connection and are served by the session's own stdio server process. Over HTTP, Claude Code's calls carry no session id and no working directory, and two sessions cannot be told apart. In that probe the stdio server ran in the session's directory. Claude Code's documentation does not state a stdio server's working directory; it sets `CLAUDE_PROJECT_DIR` in the server's environment to the project root so a server need not depend on it, and hooks receive the same value. It also sets `CLAUDE_CODE_SESSION_ID` for a stdio server: the server keeps the id it was spawned with, while the id hooks receive changes on `/clear`, and a `--continue` or `--resume` without an id may hand the server the initial id.
- The ledger is already written by several processes. The guard hook and every command-line run open their own connections (design 0001, Processes and concurrency). Every write takes the writer queue, an exclusive `flock` on `<home>/baley.db.writer` shared across processes, then `BEGIN IMMEDIATE`, then the compatibility epoch check. A command decides inside that transaction from inputs read there and is refused if they went stale (ADR 0012). Stream versions and the previous event's hash are read inside the transaction, and a unique index on project, stream and stream version refuses a duplicate.
- The store holds one piece of ledger state in memory between transactions: per project, the head through which every stored event was found readable by this binary, so a write checks only the events recorded since. Each write compares that head's hash with the stored event inside its transaction and checks from the start when they differ, so events another process wrote are checked, not skipped. Views are tables in the database; a rebuild runs once under the maintenance lock, and another process that needs it waits and finds it done.
- `crates/baley-bench` measured eight writer processes on one ledger on 2026-09-27: the wait for the writer queue was 37 to 58 ms at the 99th percentile and 136 ms at the longest, against a budget of 250 ms (design 0001, Performance).

The question is whether release 1 still needs one shared server.

## Decision drivers

- Every call is tied to its project and session without asking the host for anything it does not send.
- Concurrent sessions never corrupt the record or silently merge their work.
- Nothing for the owner to run or keep running: no service, no port, no launcher.
- The fewest moving parts that meet the other drivers.

## Considered options

1. One server per user, through a stdio launcher or over HTTP (ADR 0011)
2. One server per user, through the stdio launcher only
3. One server per session over stdio

## Decision

Chosen option: **3**. Each Claude Code session starts its own Baley server over stdio, and the session's subagents reach it through the session's connection. There is no launcher, no HTTP listener and no background service. The server serves one session and ends when the session closes its stdio.

The server takes its project from `CLAUDE_PROJECT_DIR` and records its working directory beside it. When the variable is missing or invalid, every call that needs a project fails with a typed code and records nothing. Because hooks receive the same value, the server and the guard agree on the project, and after `/cd` the session's project stays the one it started in. The server creates a session id when it starts and records it on every call; when `CLAUDE_CODE_SESSION_ID` is set, the host's id is recorded beside it. Nothing depends on the host's id being present or staying equal to the id hooks receive.

In this release every worker is a subagent of its session. A worker that runs as a program of its own, outside the session, is not part of this release; if one is added, it starts its own server under this same rule.

Sessions share the ledger through the store, as the guard hook and the command line already do. Every write takes the writer queue and decides inside its transaction (ADR 0012). There is no idle checkpoint: SQLite's automatic checkpoint folds the write-ahead log as commits grow it, and `baley doctor` warns on its size. When a server exits, it stops taking calls, gives accepted work up to ten seconds to finish, and makes one `PASSIVE` checkpoint attempt unless its store is fenced or read-only. The attempt never takes the writer queue and never waits, so it cannot hold up another session or the guard.

Baley speaks MCP over stdio only in this release, so axum is not added. Outgoing calls stay on `reqwest` (ADR 0028).

## Consequences

### Positive

- The project and session come from the process the host started, with nothing extra on the wire.
- No launcher, no service unit, no port and no idle timer to build, install or check.
- The store's concurrency rules already cover several writer processes, and that load has been measured.
- A crash ends one session's server, not every session's.

### Negative

- Each session pays the server's start-up, and each running session holds its own process and connections.
- Concurrent sessions take turns at the writer queue. The measured waits are within budget for eight writers; more sessions than that have not been measured.
- A session keeps the binary it started with until it ends. After an upgrade, an older server whose epoch is behind the ledger's answers read-only and names the binary needed (design 0001, Compatibility epoch); before the first release, ledgers are disposable.
- The host's session id the server holds is the one it was spawned with: after `/clear` it differs from the id hooks receive, and after a `--continue` it may be the initial one. The server's own id still ties every call to its session.
- After `/cd`, Baley keeps serving the project the session started in. Work in the new directory is not recorded against its own project until a new session starts there.
- Two live sessions writing one project at once have not been run. The benchmark's writers are processes, not host sessions.

### Follow-up

- Build 3 ([#24](https://github.com/crenshawdev/baley/issues/24)) rewrites SYS-R1, SYS-R2, SYS-R5 and SYS-R6 in design 0002, and HST-R1 and the install record in design 0012, and withdraws SYS-R3, SYS-R4, HST-R2 and HST-R3. The Processes and concurrency section of design 0001 is rewritten to match, in the change that builds the server.
- Build 3's acceptance includes two live Claude Code sessions writing one project at once, with the record checked afterwards.
- A shared server, or HTTP, comes back only with a new ADR, if a worker outside the session or a host that cannot start a stdio server needs one.

## Options in detail

### One server per user, through a launcher or over HTTP

One process owns the writer queue for every session. Over HTTP the server cannot tell which session or project a call is for, because Claude Code sends neither, so only the launcher route meets the first driver. It needs a launcher, a service unit, a port, an idle timer and an upgrade path for live sessions, for a benefit the store already provides.

### One server per user, through the launcher only

Meets the first driver, since the launcher knows its session's directory and environment and can pass them on every request. It keeps the launcher, the start-or-join race, the idle timer and the upgrade path, and every session still depends on one process staying up.

### One server per session over stdio (chosen)

The host already starts one stdio server per session and routes its subagents through it, so the process is the session. The cost is a process per session and turns at the writer queue, which the store was built for and the benchmark measured.
