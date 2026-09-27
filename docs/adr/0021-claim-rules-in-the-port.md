# 0021: Claim liveness and scope rules in the port

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | [0010](0010-projector-traits-in-the-port.md), in part: its placement of claim liveness and scope rules in the core; the core keeps domain judgements such as anchor reconciliation |
| Superseded by | |

## Context and problem

ADR 0010 put the store-owned `request` projector in the port and left domain decisions in the core. The Build 1 plan also placed request, claim and reconciliation rules in `baley-core`. The SQLite adapter must judge a lease and scope inside the write transaction for every command. It cannot depend on `baley-core`, because the core already depends on the port.

The meaning of a remote tag's annotation is a domain judgement. It does not belong to a lease or scope check and stays in `baley-core`.

## Decision drivers

- Keep the crate graph one way from the core and adapter to the port.
- Keep business rules out of the adapter.
- Give every adapter the same claim contract, testable by the conformance suite without the core.
- Keep domain judgements in the core.

## Considered options

1. Put claim rules in `baley-core` and have the adapter call back into it.
2. Put lease state, scope blocking, the claim event shapes and `claim_scope` in `baley-store`, while the core judges anchor observations.
3. Put every rule, including anchor judgement, in `baley-store`.

## Decision

Chosen option: **2**. The port owns the lease state and 60-second expiry, exact-token scope blocking, `command.claimed` and `command.reconciled` shapes, and the `claim_scope` view. The adapter applies them inside the shared command transaction. The core judges the remote observation for anchor reconciliation and supplies the resulting decision.

## Consequences

### Positive

- Every adapter enforces one claim contract, including EVD-R26's scope rule for every command.
- The conformance suite can test the contract without depending on the core.
- The core retains the decision about what a remote finding means.

### Negative

- The port grows two event shapes, one view and the lease rules. Changes to them are reviewed together as port changes.
- Build 1 plan decision 1's placement of claim and reconciliation rules is corrected.

### Follow-up

- T10 implements this placement. Slices 4 to 6 add their scope tokens in the core.

## Options in detail

### Option 1: rules in the core

The adapter would need to call `baley-core` while processing every scoped command. That creates a dependency cycle and fails the crate graph driver.

### Option 2: store rules in the port, anchor judgement in the core

The port gives adapters the same lease and scope decisions they must enforce inside their transactions. The core keeps the remote-specific judgement. This meets all four drivers at the cost of a larger port.

### Option 3: every rule in the port

The adapter would have a shared contract, but the port would also decide what a remote tag means. That moves a domain judgement out of the core and expands the port with each external effect.
