# 0037: Choose plan re-check scope explicitly

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-06 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0008: Review](../design/0008-review.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

[PLN-R16](../design/0005-context-plans-and-acceptance.md#3-requirements) allows one re-check after a planner revision. Its earlier definition did not specify the material that re-check reads. Checking only prior blockers leaves changes outside those blockers without another check. [Issue #134](https://github.com/crenshawdev/baley/issues/134) proposes full and diff scopes.

[REV-R10](../design/0008-review.md#3-requirements) grants one further round after a fix to a triggered review. This decision supplies the reading scope left unspecified by [ADR 0019](0019-reviews-adjudicated-and-ruled.md), while preserving its authorization and two-round limit.

## Decision drivers

- Review changes beyond blocker closure.
- Preserve an explicit choice of reading scope.
- Keep plan settings independent of reviews of completed code.
- Record what each round was asked to read.

## Considered options

1. Check only earlier blockers.
2. Always repeat the whole plan check.
3. Default to a full plan re-check with an explicit diff option.
4. Apply one scope setting to all review triggers.

## Decision

Chosen option: **3**, accepted on 2026-10-02. Default `review.triggers.plan.recheck` to `full` and allow `diff` for plan checks and plan-triggered review rounds. The first round reads the whole submitted plan against its stories' versioned truths. Full reads the whole revised plan against the same truths. Diff reads every addition, modification and deletion since the first round's draft, with before-and-after context. Both scopes check closure and may report new defects. The plan checker checks every first-round blocker (PLN-R16). A plan-triggered review checks each finding the owner ruled `fix` (REV-R10), as diff and risk reviews do. Scope changes neither the gate's effect nor the round limit.

Diff and risk review re-checks read their whole revised target independently of this setting: the original reviewed changes and the completed fixes, with the original base, revised endpoint and exact included commits retained.

Resolve the scope when the round-2 work order is created. Bind its baseline, truth versions for a plan, actual scope, material references and selecting policy version to that work order. Retain the original and revised plan snapshots before discarding a replaced draft, and record each round's scope and retained inputs. Retries keep the binding. A missing or mismatched required input refuses the re-check; Baley substitutes neither current source nor another scope. Changed truths require a new plan submission and an initial full check.

## Consequences

### Positive

- A revised plan receives a complete second pass unless the configured scope selects the narrower pass.
- The record distinguishes a full pass from a check of changes and blockers, and preserves the inputs and policy that selected it.

### Negative

- Full repeats work over unchanged plan material.
- Diff can miss defects outside the changes and supplied context.
- Neither scope guarantees discovery.
- Retention or purge can make a required body unavailable, preventing a re-check even though its reference remains.

### Follow-up

- Build 4 supplies plan scope selection, the setting's first reader and validation, work-order material and domain records and projections. Design 0005 leaves open how the checker's revision and a plan review's revision combine for one plan, owned by Build 4.
- Build 5 supplies revised targets for diff and completion risk reviews.

## Options in detail

### Check only earlier blockers

Limits the input to earlier claims, omitting new changes outside them. Closing those claims does not establish that a revision has no new defects.

### Always repeat the whole plan check

Revisits the entire revised plan and its truths, but removes the narrower choice and repeats work over unchanged material.

### Full with an explicit diff option

Makes the coverage tradeoff explicit. Full is the default; diff deliberately narrows the reading to changes. Both scopes check closure of first-round blockers for the checker and findings ruled `fix` for a plan review. Both can report new defects, and the record preserves which scope ran.

### One scope setting for all review triggers

Couples plan review cost to completed-code review coverage. A diff review's target is already a committed range; narrowing its re-check to changes since round 1 would require a separate baseline choice.
