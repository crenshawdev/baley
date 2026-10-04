# 0028: Use one HTTP stack on tokio and hyper: reqwest for outgoing calls, axum for the MCP server

| | |
|---|---|
| Status | Accepted, superseded in part by 0034 |
| Date | 2026-09-28 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | [0034](0034-one-server-per-session.md), in part: the MCP server over HTTP on axum |

## Context and problem

Baley speaks HTTP in two directions. It calls each provider's model-list endpoint for detection (design 0003, CFG-R20), and the shared server accepts MCP over HTTP when it runs as a background service (ADR 0011, SYS-R2, HST-R1). Build 2 ([#23](https://github.com/crenshawdev/baley/issues/23)) makes the calls; Build 3 ([#24](https://github.com/crenshawdev/baley/issues/24)) builds the server.

`reqwest` 0.12 is already a direct dependency, built on hyper 1 and tokio, with rustls and the ring provider. tokio already runs the server. The MCP server is built with `rmcp` 3.2.0, whose streamable HTTP server transport serves MCP over HTTP and needs an HTTP server to host it. `rmcp` tests that transport on axum 0.8: axum 0.8 is a dev-dependency in rmcp 3.2.0's `Cargo.toml`. New MCP protocol revisions reach Baley through `rmcp`, so on the host `rmcp` tests they arrive already exercised.

Measured in a copy of the workspace, with rmcp's streamable HTTP server feature turned on: hosting it on a hand-written hyper accept loop adds 8 packages to `Cargo.lock` (`async-trait`, `core_detect`, `httpdate`, `multiversion_no_op`, `scopeguard`, `simdutf8`, `sse-stream`, `tokio-stream`), and hosting it on axum 0.8 adds 13 (those 8 plus `axum`, `axum-core`, `matchit`, `mime`, `serde_path_to_error`). `cargo deny --locked check` passes for both, and each adds one new duplicate version (`cpufeatures`).

## Decision drivers

- One HTTP stack and one runtime in the binary.
- New MCP protocol revisions arrive through `rmcp` on a host `rmcp` itself tests.
- Baley does not own connection handling a maintained library already provides.
- Few new packages, all passing `cargo deny`.

## Considered options

1. One stack on tokio and hyper 1: `reqwest` 0.12 for outgoing calls, axum 0.8 hosting rmcp's streamable HTTP server
2. rmcp's streamable HTTP service behind a hyper accept loop Baley writes
3. `ureq` or `curl` for outgoing calls, beside a hyper server

## Decision

Chosen option: **1**. Baley's outgoing calls, the provider model lists, use `reqwest` 0.12. The MCP server over HTTP uses rmcp's streamable HTTP server, hosted on axum 0.8. Build 3, which builds the server, adds axum and turns the feature on; Build 2 adds none of it. Stdio through the launcher stays the fallback (ADR 0011).

## Consequences

### Positive

- One HTTP and TLS stack and one runtime serve both directions.
- The server runs on the host `rmcp` tests, so newer MCP protocol revisions arrive through `rmcp` already exercised there.
- Connection handling is axum's and hyper's, not Baley's.

### Negative

- Build 3 adds 13 packages to `Cargo.lock` and one more duplicate version, which `cargo deny` warns on.
- axum's releases join the ones Baley follows.

### Follow-up

- Build 3 adds axum 0.8 and rmcp's streamable HTTP server feature with the HTTP transport.

## Options in detail

### One stack, axum hosting rmcp (chosen)

Adds 13 packages to `Cargo.lock`, 5 of them for axum, and passes `cargo deny`. `reqwest` and axum share hyper 1, tokio and the TLS stack already built. The server is mounted the way `rmcp`'s own tests mount it.

### A hand-written hyper accept loop

Adds 8 packages to `Cargo.lock` against option 1's 13, and passes `cargo deny`. Baley would own the connection handling: accepting connections, HTTP/1 and keep-alive, shutdown and errors, on a host `rmcp` does not test.

### `ureq` or `curl` for outgoing calls

`ureq` is synchronous and small, and `curl` adds no package, but either is a second HTTP stack beside the hyper stack the server needs anyway. `curl` also runs as a separate program that is not present on every Linux install.
