# 0008: Use host sandboxes to keep agents out of the ledger

| | |
|---|---|
| Status | Proposed |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Agents run as the owner's user, as Baley does. File modes keep other users out of Baley's home but not the owner's own agents: an agent's shell can open the database directly. The guard hook watches named paths for file tools and checks git commands, but it does not see arbitrary shell writes. Something must keep agents on Baley's typed operations rather than the raw database.

## Decision drivers

- Agents must not be able to read or write the ledger directly.
- Baley's own server and hook must still write it.
- Work on both supported hosts, Claude Code and Codex.
- No daemon and no privileged install.

## Considered options

1. State the gap, and rely on the guard and on detection
2. Use each host's sandbox to deny agents access to Baley's home
3. Run the store under a separate operating-system user behind a local socket

## Decision

Chosen option: **2, host sandboxes**, on top of option 1. Baley's setup adds a rule to each host's configuration that denies agents access to Baley's home: Codex's `workspace-write` sandbox already confines agent writes to the workspace, and Claude Code's sandbox denies named paths for reading and writing. A host matrix, run on both hosts before acceptance and before each release, shows an agent's attempt refused while Baley's server and hook still write. The first run (2026-09-25, `spikes/host-matrix`) showed both hosts run the hook and the MCP server outside the agent sandbox, Claude Code denies reads and writes, and Codex denies writes only: every Codex sandbox policy grants full disk read, so on Codex the sandbox protects the ledger's integrity, not its confidentiality. The guard also refuses file-tool writes into the home and shell commands that name it, as a best-effort second layer, and forge anchors (ADR 0007) detect anything that gets through.

## Consequences

### Positive

- Real prevention on both hosts, without a daemon or a privileged install.
- Defence in depth: sandbox, guard, then detection.

### Negative

- Depends on each host's sandbox being enabled and correctly configured. `baley doctor` checks the rule is present.
- A host without a sandbox gets only the guard and detection; the design says so.
- Host sandbox behaviour can change between host releases; the matrix is re-run before each release.

## Options in detail

### Option 1: guard and detection only

Honest and cheap, but prevention is best-effort: a shell command that reaches the file without naming it slips past the guard.

### Option 2: host sandboxes

Uses the isolation the hosts already provide for exactly this kind of boundary.

### Option 3: separate operating-system user

The strongest separation, but it needs a daemon, a privileged install step and different setups on Linux and macOS. Future work.
