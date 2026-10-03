# 0033: Support only hosts whose sandboxing and execution controls meet Baley's requirements

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-02 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md), [0002: System design](../design/0002-system-design.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md), [0006: Execution](../design/0006-execution.md), [0010: Guard](../design/0010-guard.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | [0029](0029-a-host-may-offer-more.md), in part: the host that offers less sets the floor; [0020](0020-sandbox-is-a-write-barrier.md), in part: reads of Baley's home are the host's policy; [0018](0018-lease-enforced-at-close.md), in part: the reason for the close check and the Codex follow-up; [0008](0008-host-sandbox-isolation.md), in part: the driver naming both hosts; [0027](0027-vendor-folders-and-plain-keys.md), in part: the Codex consequence |
| Superseded by | |

## Context and problem

Baley was designed for Claude Code and Codex as hosts. Design 0002's host neutrality pattern (SYS-P12) judged every decision on both and let the host that offers less set the floor; ADR 0029 let the other host add to it.

The host matrix (`spikes/host-matrix`, 2026-09-25, Codex CLI 0.156.1 and Claude Code 2.1.282) measured the floor. Claude Code's sandbox denies agents reads and writes of a named path. Every Codex sandbox policy grants full disk read. Codex rejects the hook answer `ask` as unsupported and runs the command. A check of Codex CLI 0.160.0 on 2026-10-02 found MCP protocol 2026-07-28 behind an off-by-default flag marked under development, and Streamable HTTP servers still failing to initialize (openai/codex#11284).

Those limits shaped the design. ADR 0020 dropped the promise that agents cannot read Baley's home. GRD-R12 turns every `ask` into `deny` on Codex. ADR 0018 gave Codex's hook coverage as the reason the lease is checked at task close. ADR 0011 kept the stdio launcher as the floor because Codex over HTTP is unproven. HST-R4 serves an unknown host at the floor. Since ADR 0027, provider keys sit in a plain `keys.env` in the config folder, so on Codex an agent can read them.

The question is whether Baley keeps designing to the host that offers least, or supports the host that meets its security promises and states what another host must show to be added.

## Decision drivers

- Baley does not lower its security defaults to accommodate a host.
- Every security promise Baley makes holds on every host it supports, with no per-host exception.
- Provider keys and the ledger stay out of agents' reach.
- A host is added by measurement, not by argument.
- Adding a host back takes an adapter, not a redesign (SYS-P8).

## Considered options

1. Keep both hosts, with the host that offers less setting the floor
2. Support Claude Code only and remove the host seam
3. Support Claude Code as the reference host, and add another host only when it meets a stated security bar

## Decision

Chosen option: **3**. Codex support has been removed because its sandboxing and execution controls do not meet Baley's requirements. Support may return when those requirements are met. Baley will not lower its security defaults to accommodate a host.

Claude Code is the only host Baley supports, and every requirement is stated for it. SYS-P12 becomes the security bar below; its rule that every capability reaches every agent, not only the main session, stands.

**The security bar.** A host is added when the host matrix, run on that host, shows each of the following as it shows it on Claude Code.

Sandboxing:

- it denies agents reads and writes of Baley's home and config folder, both through shell commands and their child processes and through its built-in file tools;
- Baley's hook and MCP server run outside that sandbox and still write the home.

Execution controls:

- its hook runs before an agent's shell commands and file writes, and receives the command or the path;
- a command or write the guard does not allow does not run, whatever answer the guard gives.

Adding a host takes a new ADR that cites the matrix run, and an adapter for that host.

**Reads are denied again.** On Claude Code, agents can neither read nor write Baley's home or its config folder (EVD-R24, GRD-R13). Three mechanisms carry this, because Claude Code's sandbox covers only shell commands and their child processes, and Claude Code applies `Read` rules to its Grep and Glob tools only on a best-effort basis: the sandbox's read and write denials for shell commands; `Read` and `Edit` deny rules for its built-in file tools; and Baley's guard hook, which refuses any Read, Grep or Glob call whose path lies inside or contains the home or the config folder. An unavailable sandbox is reported, never passed over. With all three in place, `keys.env` and the ledger are out of an agent's reach. That holds for `keys.env` as a file in the config folder (or the home); where it is a symbolic link, which CFG-R24 follows, the target is protected only when it too lies inside those folders. The host matrix is run on Claude Code before each release. Both probes stay in `spikes/host-matrix`: Claude Code's for the release run, Codex's to measure Codex against the bar.

**An unknown host is refused.** A connection from a host Baley does not support is refused with the supported host named (HST-R4), not served at a floor. The host's name is the client's own report, so this states what Baley supports; it does not stop a client that names itself Claude Code.

**The seam stays.** `Host` keeps its type with one value, `claude-code`. Host sections in the settings files, `--host` and the per-host model catalog keep the host. The rung map stays (CFG-R15); Claude Code's map takes each rung to the effort of the same name. Not every Claude Code model takes all five: Haiku takes no effort level, and Opus and Sonnet 4.6 have no `xhigh`. Claude Code runs an unsupported level as the highest supported level at or below it, so Baley relies on that fallback and records the requested rung and the effective level apart, from a per-model table of supported levels. The guard's answer adapter stays as the step that renders an answer in the host's hook form (GRD-R12); the rule that turns `ask` into `deny` goes with Codex. A host that offers more than Claude Code may offer the extra as a declared addition, and no step of the process depends on it (ADR 0029).

**The lease is still checked at close.** ADR 0018's rule stands on a reason of its own. On Claude Code the guard sees the paths of `Write` and `Edit` calls, but from a shell command's text it cannot tell every file the command writes: an in-place edit, a redirect or a script run by the command. The check at task close sees every changed path, however it was written.

## Consequences

### Positive

- One set of guarantees, stated once, measured on the one supported host.
- Provider keys in `keys.env` and the ledger cannot be read by an agent.
- The design documents lose their per-host branches for Codex: launch flags, hook coverage, sandbox reads and the `ask` rule.
- A host that comes back is judged against a written bar with a probe that already exists.

### Negative

- An owner who works in Codex cannot use Baley until Codex meets the bar.
- With one value of `Host`, the tests that kept two hosts' catalogs apart have nothing to separate and are removed; that behaviour is untested until a second host is added. The ADR that adds a host restores them, at the `Host` type.
- Every promise rests on Claude Code's sandbox, which on Linux needs `bubblewrap` and `socat`, and which the owner's own Claude Code settings can loosen. `baley doctor` reports what an agent can reach (GRD-R13).
- A ledger event naming a `codex` host catalog would be refused by the catalog projector. None exists: Baley has no release, and the stores in use record provider catalogs only.

### Follow-up

- The Build 3 ([#24](https://github.com/crenshawdev/baley/issues/24)) task that lands this decision removes the `codex` value of `Host`, its catalog row and the help text naming it, and rewrites designs 0001, 0002, 0003, 0006, 0010 and 0012, the PRD, the roadmap, the C4 model, the glossary, the README and the bug report template to match. The inherited engine's Codex transcript reader goes in the same change; Build 9 removes the rest of that engine.
- Build 3 refuses an unknown host (HST-R4), configures Claude Code's sandbox and its `Read` and `Edit` deny rules to deny agents reads and writes of the home and the config folder, has the guard hook cover every tool that runs a shell command and the built-in tools that read files, and extends the Claude Code probe to the config folder and to the built-in file tools.

## Options in detail

### Keep both hosts at the floor

Serves owners on either host. Every promise is limited to what the weaker host enforces: on Codex an agent can read the ledger and `keys.env`, and an `ask` the host does not support has to become a refusal. Each later decision is bent to the same limits.

### Claude Code only, without the seam

The least code to keep. Removing the host from settings, routing and the stored policy view changes the stored key, and every part would be rebuilt when a host is added.

### Claude Code as the reference host, with a security bar (chosen)

Meets every driver. The guarantees are Claude Code's, and they are the bar for any host after it. The seam costs one type with one value and the settings sections already built, and a host that meets the bar is added through an adapter and an ADR.
