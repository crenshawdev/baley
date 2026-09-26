# 0017: Codify Scrum: a requirement is a story that carries its truths, a phase is a sprint

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Cadence's acceptance design (2026-09-09) put truths on a phase, capped them at seven, and left requirements as rows a phase served. The owner's process is Scrum: a prioritized backlog of stories, each with acceptance criteria, committed to sprints that deliver a usable increment, sized without estimating, with a definition of done and a retrospective. A fixed cap on truths and a t-shirt size derived from counts were both considered and rejected: a number handed to the model becomes a target, and a size that changes with every edit churns.

## Decision drivers

- Acceptance criteria belong to the thing promised.
- A sprint delivers something the owner can use.
- Nobody estimates; size is counted from the plan.
- No number is handed to the model.

## Considered options

1. Truths on the phase with a hard cap of seven (the 09-09 design)
2. Phases with a t-shirt size derived from counts and one ceiling setting
3. Stories carry their truths; a phase is a sprint; size is the tasks in approved plans against an owner-set capacity

## Decision

Chosen option: **3**. A requirement is a story and carries its own truths, written with the owner at refinement and versioned. A phase is a sprint: a goal and the stories committed to it, one active per project, a working increment proven by its stories' truths and the suite. A story's size is the number of tasks in its approved plans; `planning.sprint_capacity` is the only ceiling, applied by Baley at plan submit and reported as a refusal. No cap on truths per story; the planner proposes splits and the owner approves them. Velocity is shown, never used to decide. A sprint closes with the owner's approval and an optional retrospective record. Stories and sprints are mirrored to the forge in a later release.

## Consequences

### Positive

- The owner's own process, enforced by Baley's rules rather than a vocabulary layer.
- Sizes are facts with no churn: they change only when a plan is approved or replaced.
- The seven-truth cap and its refusal are gone; the capacity setting replaces them with one owner number Baley applies.

### Negative

- 0004's project start and scope commands changed to backlog terms after they were designed; the documents were amended in the same day.
- Plans bind to story truth versions, so a revised story invalidates plans that served it, by design.

### Follow-up

- The backlog, sprint and plan views are built together (Build 4).

## Options in detail

### Truths on the phase, cap of seven

Reviewable, but the promise sits on the wrong object, and the cap is a number the planner plans to.

### T-shirt sizes from counts

Familiar words, but the size flips with every plan edit and the numbers behind it are targets.

### Stories and sprints with capacity in tasks (chosen)

Truths on the story, one ceiling the owner sets, counted after planning, no estimation.
