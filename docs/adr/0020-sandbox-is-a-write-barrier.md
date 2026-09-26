# 0020: State what each host's sandbox denies; reads are the host's policy

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md), [0010: Guard](../design/0010-guard.md) |
| Supersedes | [0008](0008-host-sandbox-isolation.md), in part: the driver "agents must not be able to read or write the ledger directly" and the claim that each host denies access |
| Superseded by | |

## Context and problem

ADR 0008 chose host sandboxes to keep agents out of Baley's home and wrote its driver as "read or write". The host matrix measured the two hosts on 2026-09-25: Claude Code's sandbox denies agents both reads and writes of a named path; Codex's `workspace-write` denies writes only, and every Codex sandbox policy grants full disk read. The design documents recorded the measurement (0001, host neutrality) but EVD-R24 and ADR 0008 kept the stronger wording. A reviewer asked whether `workspace-write` is enough, given that an agent under it can look anywhere.

The read permission is Codex's policy. It applies to everything the owner's user can read: the owner's keys, other repositories, browser profiles. Baley's home is one directory in that set. Baley cannot change the policy, and no Codex setting was found that denies reads of a path.

## Decision drivers

- The requirement text says exactly what was measured on each host.
- Baley's guarantees do not depend on a read barrier it cannot provide.
- The boundary Codex leaves open is the operating-system user; Baley does not claim to close it.
- No service, no second user, no privilege at install.

## Considered options

1. State what each host denies and keep everything else as it is
2. Run the Baley service as its own user so the home is unreadable to agents running as the owner
3. Search Codex for a read-deny setting

## Decision

Chosen option: **1**. EVD-R24 now says each host's sandbox denies writes, and reads where the host can (Claude Code); GRD-R13 says the same and adds that `baley doctor` reports reads; CFG-R24 states that the Linux Secret Service answers any process running as the owner, so an agent under a host that allows reads could reach the master key there. Baley's guarantees are restated without a read barrier: writes are refused on both hosts, tampering is detected by the chain and the forge anchors, keys are encrypted with the master key in the OS secret store, and nothing sensitive is kept outside purgeable payloads. Reads of the ledger by a process that already runs as the owner are that host's policy and the owner's operating-system user boundary, outside Baley's scope.

Option 2 was ruled out by the owner: no service user. Option 3 was not pursued: the matrix already showed every Codex policy reads the whole disk, and protection resting on a host setting an agent could argue about would not be a barrier.

## Consequences

### Positive

- The documents claim only what the matrix shows, per host.
- No new mechanism, no privilege at install, nothing to run.
- The reviewer's question has a written answer in the design.

### Negative

- On Codex, an agent can read the ledger; on Codex under Linux, it could reach the master key through the user keyring. Both are stated limits.

### Follow-up

- `baley doctor` reports what an agent can read on the connected host (GRD-R13).
- The host matrix is re-run before each release; a Codex read restriction, if one appears, is recorded there.

## Options in detail

### State what each host denies (chosen)

Wording follows measurement. Costs nothing; changes no behaviour; leaves the Codex read open and says so.

### A service user

The background service runs as a `baley` account with a 0700 home; agents running as the owner cannot read it on any host. The real read barrier, at the cost of a privileged install step, a socket permission model and a second user on the machine. Ruled out by the owner.

### A Codex read-deny setting

Would close the read on Codex only, if it existed; the matrix found none, and it would depend on host configuration rather than on Baley.
