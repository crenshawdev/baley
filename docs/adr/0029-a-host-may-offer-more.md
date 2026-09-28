# 0029: Let a host offer more than the floor

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-28 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Baley runs with Claude Code or Codex as the host. Design 0002's host neutrality pattern (SYS-P12) says each wire, tool or instruction decision is judged on both hosts and the host that offers less sets the floor. Read strictly, a capability one host can offer and the other cannot is never offered on either.

The design already differs by host where a host allows more. The guard hooks `Bash`, `Write` and `Edit` on Claude Code and only `Bash` on Codex (design 0010, GRD-R1). The sandbox keeps agents from reading Baley's home on Claude Code, while Codex's sandbox grants reads (GRD-R13, [ADR 0020](0020-sandbox-is-a-write-barrier.md)). The guard's `ask` answer reaches the owner on Claude Code and becomes `deny` on Codex (GRD-R12). Each was decided on its own: ADR 0018 keeps the lease check at task close as the floor and adds the guard's earlier stop where the host has one, ADR 0020 keeps the read barrier where the host can enforce it, and GRD-R12 gives the floor as its reason. SYS-P12 itself states no general rule for when a host may go beyond the floor.

Further additions of this kind are coming, such as letting the owner route a stage of the process to another model family's command-line program where the host can start one. The question is whether the floor is also a ceiling.

## Decision drivers

- Baley's whole process works on either host.
- An owner on the host that offers more is not held to the other host's limits.
- A difference between hosts is declared and visible, never implied by the model or discovered by accident.
- Baley decides which mechanism runs, through the host's adapter (SYS-P1, SYS-P8).

## Considered options

1. The floor is also the ceiling: offer only what both hosts can do
2. The floor is the baseline, and a host may offer more as an addition its adapter declares
3. Design each host separately, with no common floor

## Decision

Chosen option: **2**. The host that offers less still sets the floor, and the whole process works on either host at that floor. A host that offers more may offer more, as an addition its adapter declares. No step of the process depends on an addition. On a host without it, `baley doctor` names the addition as unavailable on that host, and a setting that asks for it is refused on that host with the reason, so the owner sees the gap rather than finding it later. SYS-P12 in design 0002 carries the rule.

## Consequences

### Positive

- Owners on the more capable host get what it can do without waiting for the other host.
- The existing differences in the guard and the sandbox, each decided on its own in ADR 0018, ADR 0020 and GRD-R12, now follow from one general rule.
- Each addition is a declared adapter capability, so Baley, not the model, decides whether it is used.

### Negative

- Each addition is a second behaviour to document and to check in the acceptance run on its host.
- The floor can erode if an addition quietly becomes necessary; every addition has to be checked against the rule that no step depends on it.

### Follow-up

- Build 3 ([#24](https://github.com/crenshawdev/baley/issues/24)), which builds the host adapters, adds the declared additions to each adapter beside the mechanisms it already chooses (design 0012, HST-R4), with `baley doctor` reporting them. The seam is the host adapter.
- Routing a stage to another family's model, program or API is designed in designs 0003, 0008 and 0012 in its own change.

## Options in detail

### The floor is also the ceiling

Keeps one behaviour everywhere and the least to test. It withholds from Claude Code owners anything Codex cannot do, and it does not describe the design as it stands: the guard and the sandbox already differ by host.

### A declared addition above the floor (chosen)

The floor keeps the process whole on both hosts. Additions are optional and declared by the adapter, and their absence on the other host is stated. The cost is one more behaviour per addition to document and test.

### Design each host separately

Gives each host everything it can do, but two designs drift apart, and nothing guarantees an owner can move between hosts without losing part of the process.
