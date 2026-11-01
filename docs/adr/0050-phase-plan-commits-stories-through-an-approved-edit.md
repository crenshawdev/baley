# 0050: Commit a phase plan's stories through an approved phase edit that issues the planner's order

| | |
|---|---|
| Status | Accepted |
| Date | 2026-11-01 |
| Deciders | John Crenshaw |
| Design document | [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md), [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

`phase plan` starts a phase's planning. A session sends the phase's number and the stories to commit to it, in order, and the answer is the planner's work order, which the session launches. When the stories sent are the ones the phase already commits, nothing about scope changes and the order can be issued at once. When they differ, because a story is added, left out or moved, the call changes what the phase commits, and every scope change is a draft the owner approves by digest at the terminal ([0004](../design/0004-starting-a-project-and-changing-scope.md) PRJ-R19, [ADR 0043](0043-submissions-recorded-as-drafts.md), [ADR 0044](0044-owner-acts-on-the-command-line.md)). The session cannot approve it, since over MCP nothing proves a person answered.

So one call has two acts in it: a change of commitment that only the owner may make, and a work order for a planner. A planner launched on stories nobody approved would write a plan for a commitment that may never stand, and the owner would then be asked to approve the plan with the commitment under it already assumed. The question is where the planner's order comes from when the commitment changes, and which step records it.

The refinement path has a precedent: a story's drafting order is issued by the call that closes its question set, or, after a close at the terminal, by the session's next `story refine` ([0005](../design/0005-context-plans-and-acceptance.md) section 5). That keeps work orders out of the terminal, but it leaves the owner's approval and the order it makes possible in two separate steps.

## Decision drivers

- No planner order exists for a commitment the owner has not approved.
- The owner's approval of the commitment is the act that starts planning, so approving it and getting the planner's order are one step, not two the owner must remember to join.
- A crash or a restart between any two steps leaves nothing half done: an approved commitment either has its order recorded or has neither.
- A rerun of the approval, or a second `phase plan`, never issues a second order for the same commitment, since a phase has one live planner order.
- Approvals are given only at the terminal, and a work order is launched only by a session ([ADR 0044](0044-owner-acts-on-the-command-line.md)).

## Considered options

1. The approval records only the commitment, and the session's next `phase plan` issues the order
2. `phase plan` commits the stories directly and issues the order
3. The approval records the commitment and the planner's order together, and the session's next `phase plan` hands that order over

## Decision

Chosen option: **3**, because it is the only option where the owner's approval is what starts planning, while the order still reaches the planner only through a session.

`phase plan` with a selection that differs from the phase's committed stories, a reorder included, records the `phase edit` draft of the phase's `stories` and, in the same transaction, a planning continuation on the phase's stream: `plan.continuation`, naming the phase, the selection in order, the edit's digest and the id the planner's order will have. It answers with the draft's digest and the commands that preview and approve it, and issues no order.

`baley scope approve` on that digest is a commitment approval. Its first step rechecks the selection over facts read at approval, with the same rules `phase plan` judged it by: another phase active (`phase-active`), the phase and each story held, each story refined (`story-unrefined`), no story committed to another phase (`story-in-phase`), and no planner order of the phase pending (`planner-order-pending`). A refusal records nothing. Its second step records, in one transaction, the edit's `phase.declared` and `scope.approved` and the planner's `work_order.issued` under the id the continuation named, and the terminal prints that id. Running the approval again is refused `draft-approved`, naming the same order.

The session's next `phase plan` with the same selection finds the approved continuation and its pending order, records `plan.continuation_followed`, and answers with that order's receipt, which the session launches as it would a fresh order. A further `phase plan` while the order is pending is refused `planner-order-pending`, naming it.

## Consequences

### Positive

- No planner order is recorded until the owner has approved the stories it plans, and a stale or refused approval changes neither the stories nor the work orders.
- The commitment and its order are one transaction, so after a restart the ledger holds both or neither, and the order id is fixed when the edit is drafted, so every read of the approval names the same order.
- The owner approves once, and the session's next call picks up the order without the owner doing anything more.
- An unchanged selection still issues its order directly, so planning a phase whose stories were already approved costs no extra step.

### Negative

- The terminal issues a work order, which until now only a session did. It must compose the order's route, and it composes it for Claude Code, the one supported host, under the checkout's effective policy for that host. Supporting a second host needs a way to choose which host a terminal-issued order is routed for.
- `scope approve` treats a `phase edit` differently when a continuation names it, so the same kind of draft has two approval paths to keep in step.
- Two new events, `plan.continuation` and `plan.continuation_followed`, exist only to carry the selection and the hand-over across the gap between the session and the terminal.

### Follow-up

- Design 0004 describes the continuation in `phase edit` and `scope approve`, and design 0005 describes `phase plan`, its paths and its refusals, and the two events.

## Options in detail

### Option 1: the approval records only the commitment

The approval records the edit as any other, and the session's next `phase plan`, now finding the selection equal to the committed stories, issues the order, as a story's drafting order is issued by the next `story refine` after a close at the terminal. No new event and no route at the terminal. But the owner's approval does not start planning: it is one step, and the order is another that happens only if a session calls again, and between them the phase has an approved commitment and no planner. Fails the second driver.

### Option 2: `phase plan` commits the stories directly

The call records the new commitment itself and issues the order. One step, and no draft. But the commitment changes on a call that comes from the model, with no owner approval, which every other scope change requires. Fails the first driver and PRJ-R19.

### Option 3: the approval records the commitment and issues the order (chosen)

The edit is drafted with a continuation, approved at the terminal, and the approval records the order in the same transaction; the session's next call hands it over. Meets every driver. Costs a route composed at the terminal for the one supported host, two continuation events, and a second approval path for a `phase edit` a continuation names.
