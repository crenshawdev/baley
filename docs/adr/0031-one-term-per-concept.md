# 0031: Use one term per concept, kept in a glossary: phase, not sprint, and story, not requirement

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-28 |
| Deciders | John Crenshaw |
| Design document | [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md), [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0011: Milestones, landing, undo and pause](../design/0011-milestones-landing-undo-pause.md), [0013: Next action and progress](../design/0013-next-action-and-progress.md), [glossary](../../CONTEXT.md) |
| Supersedes | [0017](0017-stories-and-sprints.md), in part: a phase is not called a sprint, and the capacity setting is `planning.phase_capacity` |
| Superseded by | |

## Context and problem

The design documents named two concepts twice each.

The working increment was a phase in the ledger's streams and events (`phase/<n>`, `phase.declared`, `phase.completed`) and in designs 0004, 0006, 0007, 0011 and 0013, and a sprint in [ADR 0017](0017-stories-and-sprints.md), which says a phase is a sprint and names the setting `planning.sprint_capacity`. Design 0005 followed ADR 0017 with `sprint plan`, `sprint close`, `sprint.retrospective`, `sprint-active`, `sprint-not-active`, `story-in-sprint`, `story-not-in-sprint` and a `sprint` view; design 0011 named the integration branch `baley/sprint-<n>-<slug>`; design 0013 took `--sprint <n>`.

The owner's declared work was a requirement in design 0004, with the events `requirement.declared`, `requirement.corrected`, `requirement.reassigned`, `requirement.reprioritized` and `requirement.dropped`, the commands `requirement declare`, `edit`, `reassign` and `drop` under `baley requirement`, and the refusal `no-such-requirement`; and a story in design 0005, with `story.refined`, `baley story` and the refusal `no-such-story`. Both refusals cited the same rule, PRJ-R18, for the same fault. Design 0004 also listed a `baley context` namespace for truth revision, which 0005 does as `story truths submit`. Meanwhile every design document already uses Requirement for its numbered design rows, such as CFG-R12 and PRJ-R4.

Two names for one thing make a reader, a model or a test ask whether they differ; a model echoes whichever word it was served. None of these names is built yet: the code holds no `sprint` or `requirement.*` event, command or code. Once built, an event type is permanent in the ledger, since events are never rewritten ([ADR 0001](0001-event-ledger.md)).

## Decision drivers

- One word for each concept in the design documents, the instructions Baley serves, and every setting, command, code, event, field, view and branch name.
- Rename before the names are built, while it costs only text; recorded event types cannot be renamed later.
- Keep the word the ledger and most documents already use.
- Keep Requirement for the one meaning every design document already gives it: a numbered design requirement row.
- A reader who has never seen the project finds each term, its meaning and the words to avoid in one place.

## Considered options

1. Keep both words as synonyms and say so in each document
2. Sprint and requirement as the canonical words
3. Phase and story as the canonical words, with a glossary
4. Phase and requirement as the canonical words

## Decision

Chosen option: **3**. The glossary [CONTEXT.md](../../CONTEXT.md) at the repository root gives one term per concept, its meaning, the design section that owns it and the words to avoid for it. A design document that changes a term changes the glossary in the same pull request ([the design process](../design/README.md)).

Phase replaces sprint: `planning.sprint_capacity` is `planning.phase_capacity`; `sprint plan`, `sprint close` and `baley sprint` are `phase plan`, `phase close` and `baley phase`; `--sprint <n>` is `--phase <n>`; `sprint-active`, `sprint-not-active`, `story-in-sprint` and `story-not-in-sprint` are `phase-active`, `phase-not-active`, `story-in-phase` and `story-not-in-phase`; `sprint.retrospective` is `phase.retrospective`; the branch pattern `baley/sprint-<n>-<slug>` is `baley/phase-<n>-<slug>`. The `sprint` view is renamed `phase_plan` and stays separate from the existing `phase` view: `phase_plan` answers a phase's goal, committed stories, plans, size, capacity and definition of done for 0005 and 0013, while `phase` serves Hardin the context, plans, execution and completion state (design 0004).

Story replaces requirement for what the owner declares on the roadmap: the five events are `story.declared`, `story.corrected`, `story.dropped`, `story.reassigned` and `story.reprioritized`, beside the existing `story.refined`, which keeps its meaning; no other `story.*` event had these names. The commands are `story declare`, `story edit`, `story reassign` and `story drop` under `baley story`, the namespace 0005 already uses, and 0004's `baley requirement` and `baley context` namespaces are gone. The refusals are `no-such-story`, `story-double-assigned`, `story-not-served`, `story-shipped` and `story-undecided`. The field `requirements` of `phase.declared` is `stories`, the field `requirement` of `capture.promoted` is `story`, and the `requirement-*` kinds of `scope.approved` are `story-declare`, `story-edit`, `story-reassign` and `story-drop`. Requirement stays the word for a numbered design requirement row.

## Consequences

### Positive

- Each concept has one name everywhere, so a code, an event and a sentence about the same thing read alike, and the two refusals for PRJ-R18 are one.
- Requirement means one thing: a design row with an identifier.
- A new reader, a model and a reviewer can check a word against the glossary.
- The renames cost only documents, since nothing named is built yet.

### Negative

- Phase and story are not the words of Scrum, which ADR 0017 codifies; a reader who knows Scrum has to read phase as sprint.
- ADR 0017 and other earlier decision records keep the old words, since accepted records are never edited; the glossary says how to read them.
- The glossary is one more document to keep in step with every design change.
- A command `phase plan` and a view `phase_plan` now share words; they are an operation and a view, and the glossary and 0005 say which is which.

### Follow-up

- Build 4 ([#25](https://github.com/crenshawdev/baley/issues/25)) builds the story events, commands, codes and fields of design 0004, the capacity setting, the planning codes and the `phase_plan` view of design 0005, under the new names.
- Build 5 ([#26](https://github.com/crenshawdev/baley/issues/26)) builds `phase close` and `phase.retrospective` with phase completion (designs 0005 and 0007).
- Build 6 ([#27](https://github.com/crenshawdev/baley/issues/27)) builds the integration branch `baley/phase-<n>-<slug>` (design 0011, LND-R1).
- Build 7 ([#28](https://github.com/crenshawdev/baley/issues/28)) builds next action and progress over `phase_plan`, with `--phase <n>` (design 0013).
- Build 8 ([#29](https://github.com/crenshawdev/baley/issues/29)) builds capture promotion through `story declare`, recording the `story` field of `capture.promoted` (design 0014, SUP-R2).

## Options in detail

### Keep both words as synonyms

No renames, but every reader has to learn that two words mean one thing, two refusals already name one fault, and a model is served whichever word the document it reads uses.

### Sprint and requirement

The Scrum word for the increment, and the event names design 0004 had. It means renaming the ledger's `phase` streams and events and most of the documents, and leaves Requirement meaning both the owner's work and a design row.

### Phase and story, with a glossary (chosen)

Keeps the word the ledger and most documents use for the increment, takes the word design 0005 already uses for the owner's work, and leaves Requirement to design rows. The cost is renaming the sprint and requirement names in the documents, and departing from Scrum's word for the increment.

### Phase and requirement

Keeps design 0004's events, but Requirement would still mean both the owner's work and a numbered design row, and design 0005's story names would all change.
