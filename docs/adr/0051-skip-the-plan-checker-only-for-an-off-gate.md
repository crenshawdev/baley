# 0051: Skip the plan checker only for the owner's off gate

| | |
|---|---|
| Status | Accepted |
| Date | 2026-11-01 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0008: Review](../design/0008-review.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The plan checker reviews a submitted plan before the owner sees it. [PLN-R16](../design/0005-context-plans-and-acceptance.md#3-requirements) has it run whenever `review.triggers.plan.gate` is not `off`. That setting belongs to the owner, who can turn the check off in either settings file, and its default, `advisory`, runs the check.

[ADR 0048](0048-port-what-cadence-hands-agents.md) makes Cadence the source of what Baley hands its agents, so the check steps of `bal-plan` are ported from the `check_gate` step of Cadence's `cadence-core/workflows/plan.md` at tag `v3.9.4`. That step has two ways round the check that PLN-R16 does not. "Skip when workflow.plan_check is false or `--skip-check` was passed" lets the session running the workflow pass a flag. "Empty or unmarked return -> report it, ask whether to proceed unchecked" lets a check that returned nothing end with the plan going on unchecked.

ADR 0048 keeps a Cadence rule that contradicts only a design document and corrects the document; only an accepted ADR stands against a ported rule. PLN-R16 is a design requirement, so without an ADR those two rules would stand and PLN-R16 and Build 4's acceptance criteria would be corrected to match. The question is who may skip the checker, and what a check that produced nothing means.

## Decision drivers

- The owner's gate holds what the owner set: the owner's setting is the one place that says whether a plan is checked.
- An agent cannot bypass it: over MCP the call comes from the model and nothing proves a person asked for it ([ADR 0044](0044-owner-acts-on-the-command-line.md)), so a session's choice is never the owner's.
- A failure is not a pass: a check that did not run or returned nothing says nothing about the plan.

## Considered options

1. Port the session's skip and the ask, and correct PLN-R16 and the Build 4 criterion
2. Only the owner's effective `off` gate skips the checker

## Decision

Chosen option: **2**, because it is the only option where a gate the owner turned on holds whatever a session does.

Only an effective `off` for `review.triggers.plan.gate` skips the checker, whichever settings layer gave it. Every other value, set in a file or by default, issues the checker's order with the submit. `plan-submit` and `plan-replace` take no argument that asks to skip it, and nothing a session sends changes the gate.

A failed or empty checker return is a failed order, never a pass. A checker that cannot launch, returns nothing or returns something that is not a findings list has its order failed and retried by the session with `work-order-fail` and `work-order-retry`, and a return refused for its shape goes the same way. Until the check's last round returns, the draft's question set does not open and `baley plan approve` is refused `check-unfinished`. An empty findings list is not an empty return: it is the checker's own answer that it found nothing, and it is recorded as that round's result.

`bal-plan` lists both drops, `--skip-check` and "ask whether to proceed unchecked", beside its ported text with this record as their reason (`crates/baley/src/instruction/plan.rs`, its module comment).

## Consequences

### Positive

- The owner can read a plan's preview knowing the check ran, or that the owner's own setting turned it off.
- Every draft that reaches the owner under a gate that is not `off` has a recorded result for its check.
- PLN-R16 and the code say the same thing, so the design keeps its wording.

### Negative

- A checker that keeps failing holds the plan: the session can only retry, and the owner's way past it is to set the gate to `off`, or to wait. A session cannot get the plan in front of the owner to ask.
- Baley's `bal-plan` differs from the Cadence step it was ported from in behaviour, not only in names, so a later port of that step has to keep these two drops.

### Follow-up

- Build 4 T11 extends the plan-gate function (`crates/baley-core/src/policy/merge.rs`, `EffectivePolicy::plan_gate`) with the risk raise, recorded before the checker's order is issued.
- Design 0005 describes the checker as built, and cites this record in its PLN-R16 status row.

## Options in detail

### Option 1: port the session's skip and the ask

`bal-plan` keeps Cadence's text as it stands, `plan-submit` gains a way to skip, and a session asks the owner whether to go on after an empty return. PLN-R16 and the Build 4 criterion are corrected to say so. This keeps the ported rule whole and gives a stuck check a way out. But an agent can bypass a gate the owner turned on, and because the ask reaches the owner through the session, the record could not show that the owner gave the answer. Fails the first two drivers, and the third when the session proceeds.

### Option 2: only the owner's effective `off` gate skips the checker (chosen)

The skip is one setting the owner controls, read under each submit's own policy, and a check that produced nothing is retried until it produces something. Meets every driver. Costs the way out of a stuck check, which is now the owner's setting alone, and two listed differences from Cadence's step.
