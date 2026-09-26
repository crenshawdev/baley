# 0011: Run one shared Baley server per user over stdio and HTTP

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

The ledger is one database per user (ADR 0003). Today's engine binds one project per process and is started by each host session over stdio, so two sessions, a worker and the guard hook are four processes writing one database, each with its own connection, cache and view state. Every session also pays the server's start-up, and a worker launched by a host starts a second server of its own.

Both hosts start an MCP server with the session's directory as its working directory and speak to it over stdio by default; Claude Code also connects to HTTP servers, and Codex has HTTP support with an open reliability issue. Operating systems offer a way to keep a user process running (systemd user units, launchd agents).

## Decision drivers

- One writer process for the one per-user ledger, so views, caches and the writer queue live once.
- Every session and worker reaches the same server without knowing how it was started.
- No service is required of anyone; a background service is available to anyone who wants it.
- Both hosts, as they are today.

## Considered options

1. One server per session and worker, stdio only (today)
2. One shared server per project
3. One shared server per user, reached over stdio through a launcher or over HTTP as a background service

## Decision

Chosen option: **3**. One Baley server runs per user and serves every session and worker. It listens on HTTP on a local port and on stdio through a small launcher that starts the server when none runs and joins it otherwise, relaying the session's stdio and passing the host's client information and the session's working directory on every request. `baley install` asks whether Baley runs in the background; yes writes and starts a systemd user unit or launchd agent and registers the host over HTTP; no registers the launcher. Both routes lead to the same single server, which exits after a quiet period when it was started on demand and nothing is connected. Every request carries the caller's working directory, from which the server resolves the project (0003, CFG-R4).

## Consequences

### Positive

- One process owns the writer queue, the views and the caches; several sessions never hold competing state.
- A worker joins the same server as its session; no second server is ever started by a host.
- The record knows which host, session and worker made each call.
- An owner who wants a service gets one; one who does not needs nothing running.

### Negative

- A launcher is one more small program to ship and test on each host.
- The server must handle many connections at once safely (SYS-R5) and survive an upgrade while sessions reconnect.
- Codex over HTTP is unproven (openai/codex#11284); the launcher is the floor until it is.

### Follow-up

- Build the launcher and the HTTP transport together; measure Codex over HTTP and record the result in 0012.
- The service unit and agent files are written by `baley install` and checked by `baley doctor`.

## Options in detail

### One server per session and worker, stdio only (today)

Each host session starts its own server; a worker started by the host starts another. Simple to register, but the per-user ledger is written by several processes with separate caches, views are rebuilt per process, and the writer queue serializes through the file lock alone. Fails the first two drivers.

### One shared server per project

One server per checkout, started by the first session in it. Sessions in one project share state, but the ledger is per user, so two projects still write it from two processes, and cross-project questions need two servers. Fails the first driver.

### One shared server per user (chosen)

One process for the one database. The launcher makes stdio hosts work without a service; HTTP serves a service or any host that connects that way. The cost is the launcher and the concurrency the server must handle, both bounded and testable.
