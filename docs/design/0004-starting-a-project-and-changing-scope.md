# 0004: Starting a project and changing scope

| | |
|---|---|
| Status | Accepted |
| Design issue | [#46](https://github.com/crenshawdev/baley/issues/46), [#45](https://github.com/crenshawdev/baley/issues/45); build issues [#23](https://github.com/crenshawdev/baley/issues/23), [#25](https://github.com/crenshawdev/baley/issues/25) |
| Requirement prefix | PRJ |
| Applies | [0002: System design](0002-system-design.md) |
| Related | ADRs: [0004](../adr/0004-project-identity.md), [0006](../adr/0006-no-markdown-records.md), [0007](../adr/0007-forge-anchors.md), [0009](../adr/0009-served-instructions.md), [0017](../adr/0017-stories-and-sprints.md), [0031](../adr/0031-one-term-per-concept.md) · C4 view: components ([0002](0002-system-design.md) Figure 4) |

The current design of this area, and nothing else. Edit it in place when the design changes; git holds the history. It describes the design only, never the work still to do.

## 1. Purpose and scope

This area decides how a project gets its first scope and how an approved scope changes afterwards:

- starting a project: the project description, the first stories and the first roadmap, as one submission the owner approves;
- the roadmap commands: declaring, editing, reordering and withdrawing a phase;
- the story commands: declaring, correcting, reassigning and dropping a story;
- the backlog: the stories in priority order;
- what the start checks on the forge and in the settings, and what it offers to fix.

It does not decide a story's acceptance criteria (truths), their revision, phase planning or plans ([0005: Context, plans and acceptance](0005-context-plans-and-acceptance.md)), how execution is undone ([0011: Milestones, landing, undo and pause](0011-milestones-landing-undo-pause.md)), how a repository or ruleset is created on the forge (0011), how settings are written ([0003: Configuration and routing](0003-configuration-and-routing.md)), or how the owner's approval reaches Baley from a host session ([0012: Host interface](0012-host-interface.md)).

Hand-offs: `baley init` ([0001](0001-evidence-ledger.md), EVD-R17) gives the repository its project id and file; this area's start runs it when it has not been run. The `roadmap` and `phase` views ([0001](0001-evidence-ledger.md)) serve what this area records to Hardin, context authoring and progress ([0013](0013-next-action-and-progress.md)). The planner that drafts the first scope is dispatched with a work order ([0002](0002-system-design.md) section 8) and routed by [0003](0003-configuration-and-routing.md).

In the component view of [0002](0002-system-design.md) (Figure 4) this area is one of the domain areas; it uses Hardin for what may happen next, the work order composer for the planner's dispatch, and the ports for the ledger, git and the forge.

## 2. Terms

| Term | Meaning |
|---|---|
| Project description | What the project is, its core value and its constraints, in the owner's words. |
| Story | One numbered statement of what the project must do, declared by the owner on the roadmap, with a status: `active`, `deferred` or `excluded`. A story carries its own acceptance criteria (truths), written in [0005](0005-context-plans-and-acceptance.md). |
| Backlog | The stories in priority order. A story in no phase waits in the backlog. |
| Phase | One working increment of the roadmap: a number, a name, a goal, detail text, dependencies on other phases, and the stories committed to it. One phase is active per project; its planning, size, capacity and close are designed in [0005](0005-context-plans-and-acceptance.md). |
| Roadmap | The ordered list of phases. |
| Scope | The description, the stories and the roadmap together. |
| Submission | A typed draft of a scope change, whole and validated, identified by its digest. Nothing in it is recorded until it is approved. |
| Approval | The owner's yes to one submission, bound to its digest, with the owner and the time. |
| Preview | The roadmap and story changes a submission would make, shown before approval. |
| Phase number | A whole number given to a phase when it is declared. It never changes and is never reused. |
| Order | The position of a phase in the roadmap: a separate fact from its number, changed by reorder. |
| Truth | One acceptance criterion of a story, in the sentence form [0005](0005-context-plans-and-acceptance.md) defines. A truth has a version. |
| Context | A phase's approved goal and the truth versions of its committed stories at the time ([0005](0005-context-plans-and-acceptance.md)). |
| Withdraw | Remove a phase from the roadmap, keeping every record of it. |
| Brief | A file the owner points the start at, holding a description of the project written elsewhere. |
| Survey | The planner's reading of an existing codebase before drafting the first scope. |

## 3. Requirements

| Id | Rule | Why | Depends on | Status |
|---|---|---|---|---|
| PRJ-R1 | One command, `project start`, starts a project in a new repository and in an existing codebase alike. Baley reads the checkout: when it holds source files and commits, the planner's work order includes a survey of what exists before drafting; when it is empty, it does not. The records are the same either way. | One contract to learn and test; Baley can tell the two cases apart itself. | SYS-P2 | Active |
| PRJ-R2 | The start records one submission, approved by the owner by digest, all or nothing: the project description, the first stories and the first roadmap. Nothing is recorded before the approval. | Scope exists only once the owner has said yes to the whole of it. | SYS-P5, EVD-R2 | Active |
| PRJ-R3 | Every field of a submission is validated before any event is recorded; a malformed submission is refused naming the section and the field at fault. | A refusal that names the fault is fixable at once. | SYS-P3 | Active |
| PRJ-R4 | Stories carry a priority order, and an active story is committed to at most one phase that is not complete; one committed to none waits in the backlog. A submission that commits one to two such phases is refused naming the story. | The backlog is the order of what is promised; a phase commits from it. | PRJ-R3, PLN-R1 | Active |
| PRJ-R5 | A brief named with `--brief <file>` is read before anything else and handed to the planner inside its work order; an unreadable brief is refused naming the file, and nothing is recorded. | The owner's own writing is the best input, and a failure to read it must not leave a half-started project. | PRJ-R1 | Active |
| PRJ-R6 | On a repository with no project, the start runs `baley init` first; `baley init` remains its own command. On a project that already has a roadmap, the start is refused with `project-already-started`; changes go through the scope-change commands. | One front door for a new project; one record of how the project began. | EVD-R17, ADR 0004 | Active |
| PRJ-R7 | There is no research step. Reading the codebase and the problem is part of the planner's work order; only the approved submission is recorded. | Research prose is not evidence of anything; the approved scope is. | ADR 0006 | Active |
| PRJ-R8 | The start checks the forge: a remote exists, the forge is reachable, and the tag ruleset that anchors rely on is in place; it records what it found as `forge.checked`. When something is missing it offers to create it; creation is an external step run only on the owner's approval, under claim, act, record, by [0011](0011-milestones-landing-undo-pause.md). A project with no forge runs unanchored and the ledger says so. A forge that cannot hold a tag ruleset, such as a private GitHub repository without a paid plan, is recorded as unprotected and anchoring goes on. | Anchors need a forge, and the owner should learn that at the start, not at the first landing. | ADR 0007, ADR 0026, SYS-P5, SYS-P7 | Active |
| PRJ-R9 | When the start finds no global settings file, or no settings in the project file, it runs the settings interview of [0003](0003-configuration-and-routing.md) as part of the start: the global file first when it is missing, then the project's settings. When both exist it runs nothing. The interview reaches the owner through the terminal when the start is run there, and through the host's question mechanism (HST-R9) when it is run from a session. | A new user gets a working setup in one sitting; an existing user is not asked again. | CFG-R11, HST-R9 | Active |
| PRJ-R10 | A phase's number is assigned when it is declared, as the next whole number, and never changes. Its order is a separate recorded fact, changed by `phase reorder`. Inserting a phase is declaring it and reordering it. Decimal numbers do not exist. | Every reference to a phase in the record stays true forever. | EVD-R2 | Active |
| PRJ-R11 | One phase declaration shape is used by the start, by `phase declare` and by every later edit: name, goal, detail text, dependencies on other phases, and the stories it serves. Editing records a new declaration and keeps the earlier one. | One shape to validate and serve. | PRJ-R3 | Active |
| PRJ-R12 | A phase with no execution recorded (declared, context approved, or plans approved) may be withdrawn directly. The owner decides, for each active story it served, the phase it moves to or that it is dropped, and approves the previewed roadmap and story changes. | Scope can shrink, and every promise it carried is accounted for. | PRJ-R4, PRJ-R13 | Active |
| PRJ-R13 | A phase whose execution has started is withdrawn only after its execution is undone (`phase.undone`, [0011](0011-milestones-landing-undo-pause.md)). Until then `phase withdraw` is refused with `execution-present`, naming the phase and the undo it needs. A complete phase is never withdrawn; the request is refused with `phase-complete`. | Removing a phase from the roadmap must not leave its code in the repository unaccounted for, and finished work stays finished. | SYS-P4 | Active |
| PRJ-R14 | A story's truths may be revised after they were approved, including after a phase that committed the story has started execution. A truth whose text changes gets the next version; an unchanged truth keeps its version; earlier versions are kept. The operation is [0005](0005-context-plans-and-acceptance.md)'s `story truths submit`. | The owner's understanding changes, and the record must follow it without losing history. | EVD-R2 | Active |
| PRJ-R15 | After a revision, a plan bound to an older truth version is no longer approved for execution; the phase is re-planned before more work runs, and committed code stays. A completion based on older truths stops applying. | Work is measured against the truths in force. | PRJ-R14, PLN-R4, [0007](0007-verification.md) | Active |
| PRJ-R16 | A revision is refused with `dispatch-active` while a worker is dispatched on a phase that committed the story. | A running worker must finish or be stopped under the truths it was given. | PRJ-R14 | Active |
| PRJ-R17 | A story's wording is corrected by `story edit`, at any time, for any active story; every earlier wording is kept. Correcting a story no phase serves (dropped or excluded) is refused with `story-not-served`. | One place to fix wording, and no editing of what the project no longer promises. | EVD-R2 | Active |
| PRJ-R18 | A request that names a phase, story or plan that does not exist is refused with `no-such-phase`, `no-such-story` or `no-such-plan`, naming it. "Unavailable" is answered only for a temporary condition. | A mistake must read as a mistake. | SYS-P10 | Active |
| PRJ-R19 | Every scope change (project start; phase declare, edit, reorder and withdraw; story declare, edit, reassign and drop; backlog reorder; and a revision of a story's truths, `story truths submit` in [0005](0005-context-plans-and-acceptance.md)) is a submission with a preview and an owner approval by digest, recorded as events that keep every earlier record. | Scope changes only with a record of what changed, who approved it and when. | SYS-P5, SYS-P6 | Active |
| PRJ-R20 | The project description is recorded as `project.described` on the project stream and served to refinement and planning ([0005](0005-context-plans-and-acceptance.md)) beside the roadmap. It is revised the same way as any scope change. | The planner and the analyzer work from what the owner said the project is. | PRJ-R2 | Active |

## 4. Roles and actors

| Actor | Receives | Returns | Model and effort from |
|---|---|---|---|
| Owner | Previews; the interview questions; the forge report and offers | The description and answers in the session; approvals by digest; story decisions on a withdraw | Not applicable |
| Planner (dispatched worker) | A work order: the project description so far, the brief if any, the survey instruction when the checkout has code, the declaration shape, the truth and story rules | A typed submission: description, stories, roadmap | `roles.planner.model`, `roles.planner.effort` ([0003](0003-configuration-and-routing.md)) |
| Host session | The planner's work order to launch; the preview and the questions to relay | The owner's approval and answers | Not applicable |
| Baley: this area | Submissions and approvals | Previews, refusals, receipts, events | Not applicable |
| Hardin ([0002](0002-system-design.md)) | A scope change request | Whether it may happen now (execution present, dispatch active, phase complete) | Not applicable |
| Forge adapter ([0011](0011-milestones-landing-undo-pause.md)) | A check request; a create request on approval | Remote, reachability and ruleset facts; the created repository or ruleset | Not applicable |

The analyzer and the plan checker take no part in this area; they act on a story's refinement and a phase's plans ([0005](0005-context-plans-and-acceptance.md)).

## 5. Commands and operations

Every operation here is a typed operation on the host interface, reachable from a session through the stub skill that names it ([ADR 0009](../adr/0009-served-instructions.md)) and from the command line under `baley project`, `baley scope`, `baley phase`, `baley story` and `baley backlog`, the same `baley story` namespace that holds the refinement operations of [0005](0005-context-plans-and-acceptance.md). Each change follows the same three steps: submit (returns a preview and a digest), approve (the owner's yes to that digest), record. A submission that is not approved is a draft held by Baley until it is approved, replaced or the session ends; a draft is never a record.

### project start

- **Inputs:** the working directory; optional `--brief <file>`.
- **Outputs:** a receipt naming what the start did and found: `baley init` run or already done; settings interview run or not needed; the forge report (`forge.checked`) with any offer to create; the checkout classification (empty or existing code); the planner's work order id.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `not-a-repository` | The directory is not inside a git repository | EVD-R17 |
  | `not-repository-root` | The directory is inside a git repository but is not its root (names the root) | EVD-R17 |
  | `project-name-required` | The repository root has no folder name Baley can use as the project's name (fix: `baley init --name <name>`) | EVD-R17 |
  | `project-already-started` | The project already has a roadmap | PRJ-R6 |
  | `brief-unreadable` | The brief cannot be read (names the file) | PRJ-R5 |
  | `config-unavailable` | Inherited from `baley init`: `baley.toml` is not a regular file, cannot be read, does not parse, lacks the project's id or name, or holds an id that is not a lower-case UUID version 4 (names the file); the global file is not a regular file, cannot be read or is invalid, or HEAD's copy of `baley.toml` cannot be read or is invalid (names the file and the fault); or the repository root, or a settings file's path, is not UTF-8 (names the path). Also when the settings cannot be read after the interview | EVD-R17, CFG-R9 |

### scope submit

- **Inputs:** a submission: description (what, core value, constraints), stories (id, sentence, status), roadmap (phases in order, each with the declaration shape), the work order id it answers.
- **Outputs:** the preview (the roadmap and stories as they would be recorded), the submission digest.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `malformed-submission` | A field is missing, blank or off type (names section and field) | PRJ-R3 |
  | `story-double-assigned` | An active story is committed to two open phases (names it) | PRJ-R4 |
  | `dependency-cycle` | Phase dependencies form a cycle (names the phases) | PRJ-R11 |
  | `project-already-started` | A roadmap exists | PRJ-R6 |

### scope approve

- **Inputs:** the submission digest; the owner and the time ([0012](0012-host-interface.md) says how a session establishes both).
- **Outputs:** the events recorded: `project.described`, one `story.declared` per story, one `phase.declared` and `phase.reordered` per phase, `scope.approved` binding the digest; the receipt.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `stale-draft` | A newer submission for the same change exists (names the first differing part) | PRJ-R19 |
  | `unknown-draft` | No draft with that digest is held | PRJ-R19 |

### phase declare

- **Inputs:** the declaration shape: name, goal, detail, dependencies, stories served; optional position (default: last).
- **Outputs:** preview and digest; on approval `phase.declared` with the next number and `phase.reordered`.
- **Refusals:** `malformed-submission`, `story-double-assigned`, `dependency-cycle`, `no-such-story`, `no-such-phase` (a dependency that does not exist) (PRJ-R3, PRJ-R4, PRJ-R11, PRJ-R18).

### phase edit

- **Inputs:** a phase number and the fields that change (name, goal, detail, dependencies, stories served).
- **Outputs:** preview and digest; on approval a new `phase.declared` for the same number, keeping the earlier one.
- **Refusals:** as `phase declare`, plus `no-such-phase`, plus `phase-complete` when the stories served would change on a complete phase (PRJ-R13).

### phase reorder

- **Inputs:** the full order of phase numbers.
- **Outputs:** preview and digest; on approval `phase.reordered`.
- **Refusals:** `no-such-phase`, `order-incomplete` (a phase missing or repeated), `dependency-order` (a phase placed before one it depends on) (PRJ-R10, PRJ-R11, PRJ-R18).

### phase withdraw

- **Inputs:** a phase number; for each active story it serves, the phase it moves to or `drop`.
- **Outputs:** preview (the roadmap without the phase, the stories' new phases or dropped status) and digest; on approval `phase.withdrawn`, one `story.reassigned` or `story.dropped` per story, `phase.reordered`.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `no-such-phase` | The phase does not exist | PRJ-R18 |
  | `execution-present` | Execution has started and is not undone (names the phase and the undo) | PRJ-R13 |
  | `phase-complete` | The phase is complete | PRJ-R13 |
  | `story-undecided` | An active story it serves has no decision (names it) | PRJ-R12 |
  | `no-such-phase` | A story is moved to a phase that does not exist | PRJ-R18 |

### story declare, story edit, story reassign, story drop

- **Inputs:** `declare`: sentence, status, phase served. `edit`: id, new sentence. `reassign`: id, new phase. `drop`: id.
- **Outputs:** preview and digest; on approval `story.declared`, `story.corrected`, `story.reassigned` or `story.dropped`.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `no-such-story` | The id does not exist | PRJ-R18 |
  | `no-such-phase` | The phase does not exist | PRJ-R18 |
  | `story-not-served` | `edit` on a dropped or excluded story | PRJ-R17 |
  | `story-shipped` | `reassign` or `drop` on a story a complete phase served | PRJ-R13 |
  | `malformed-submission` | Blank sentence or unknown status | PRJ-R3 |

### backlog reorder

- **Inputs:** the full priority order of active and deferred story ids.
- **Outputs:** preview and digest; on approval `story.reprioritized`.
- **Refusals:** `no-such-story`, `order-incomplete` (an id missing or repeated) (PRJ-R4, PRJ-R18).

### Revising a story's truths

The operation is `story truths submit` and `story truths approve` in [0005](0005-context-plans-and-acceptance.md); PRJ-R14 to PRJ-R16 are the rules it applies to a story already committed to a phase.

## 6. Records

### project.described (event, `project` stream)

| Field | Type | Meaning |
|---|---|---|
| `what` | text | What the project is |
| `core_value` | text | The one thing it must do well |
| `constraints` | list of text | What it must not do or depend on |
| `submission` | digest | The submission this came from |

### story.declared, story.corrected, story.reassigned, story.reprioritized, story.dropped (events, `roadmap` stream)

| Field | Type | Meaning |
|---|---|---|
| `id` | story id | Assigned at declaration, never reused |
| `sentence` | text | The wording (declared, corrected) |
| `status` | `active`, `deferred`, `excluded`, `dropped` | The status after the event |
| `phase` | phase number or absent | The phase it is committed to (declared, reassigned); absent means the backlog |
| `order` | list of story ids | The full priority order after the event (reprioritized) |
| `submission` | digest | The submission this came from |

### phase.declared, phase.reordered, phase.withdrawn (events, `roadmap` stream)

| Field | Type | Meaning |
|---|---|---|
| `number` | integer | The phase's permanent number (declared, withdrawn) |
| `name`, `goal` | text | (declared) |
| `detail` | payload reference | The detail text as a `record` payload (declared) |
| `depends_on` | list of phase numbers | (declared) |
| `stories` | list of story ids | The active stories served (declared) |
| `order` | list of phase numbers | The full order after the event (reordered) |
| `submission` | digest | The submission this came from |

### scope.approved (event, `project` stream)

| Field | Type | Meaning |
|---|---|---|
| `submission` | digest | The approved submission |
| `kind` | enum | `start`, `phase-declare`, `phase-edit`, `phase-reorder`, `phase-withdraw`, `story-declare`, `story-edit`, `story-reassign`, `story-drop`, `backlog-reorder`. A revision of a story's truths is approved by `story truths approve` and recorded as `story.refined` ([0005](0005-context-plans-and-acceptance.md)), not here |
| `owner`, `at` | actor, time | Who approved and when |

### forge.checked (event, `project` stream)

| Field | Type | Meaning |
|---|---|---|
| `remote` | URL or absent | The remote found |
| `reachable` | bool | Whether the forge answered |
| `ruleset` | `present`, `missing`, `unknown` | The tag ruleset anchors rely on |
| `offered` | list | What the start offered to create |

### story.refined (event, `roadmap` stream; defined in [0005](0005-context-plans-and-acceptance.md))

This area adds one rule to it: each truth carries a `version`, and a revision records a new `story.refined` whose unchanged truths keep their versions.

### Views

| View | Key | Content |
|---|---|---|
| `roadmap` | project | The ordered phases with their current declaration, status and committed stories; the stories in priority order with status, wording, phase and truth count; the project description |
| `phase` | project, phase | Adds the current context (truth versions), plans, execution and completion state Hardin needs |

## 7. States

```mermaid
stateDiagram-v2
  [*] --> Declared: phase.declared
  Declared --> Declared: phase.declared (edit), phase.reordered
  Declared --> ContextApproved: every committed story refined (story.refined)
  ContextApproved --> ContextApproved: story.refined (revision)
  ContextApproved --> Planned: plan.approved
  Planned --> ContextApproved: story.refined (revision; plans invalid)
  Planned --> Executing: first task admitted
  Executing --> Planned: phase.undone
  Executing --> Complete: phase.completed
  Complete --> Executing: completion.invalidated
  Declared --> Withdrawn: phase.withdrawn
  ContextApproved --> Withdrawn: phase.withdrawn
  Planned --> Withdrawn: phase.withdrawn
  Withdrawn --> [*]
```

*Figure 1. States of a phase as this area sees them. Execution and completion transitions belong to [0006](0006-execution.md) and [0007](0007-verification.md); a withdraw from Executing is refused until `phase.undone` returns it to Planned; Complete is never withdrawn.*

```mermaid
stateDiagram-v2
  [*] --> Active: story.declared (active)
  [*] --> Deferred: story.declared (deferred)
  [*] --> Excluded: story.declared (excluded)
  Active --> Active: story.corrected, story.reassigned, story.reprioritized
  Deferred --> Active: story.reassigned (given a phase)
  Active --> Dropped: story.dropped
  Deferred --> Dropped: story.dropped
  Active --> Met: phase.completed of its phase
  Met --> Active: completion.invalidated
```

*Figure 2. States of a story. Excluded and Dropped are final; Met is derived from its phase's completion ([0007](0007-verification.md)).*

```mermaid
stateDiagram-v2
  [*] --> Draft: submit
  Draft --> Draft: submit again (new digest)
  Draft --> Recorded: approve (matching digest)
  Draft --> Discarded: session ends or replaced
  Recorded --> [*]
```

*Figure 3. States of a submission. Only Recorded produces events.*

## 8. Workflows

```mermaid
sequenceDiagram
  participant O as Owner
  participant H as Host session
  participant B as Baley
  participant P as Planner
  participant F as Forge
  participant L as Ledger
  O->>H: start the project (optionally naming a brief)
  H->>B: project start
  B->>B: inside a repository? project file present? roadmap present?
  alt not inside a repository
    B-->>H: not-a-repository
  else below the repository root
    B-->>H: not-repository-root naming the root
  else roadmap exists
    B-->>H: project-already-started
  else
    B->>B: baley init unless already done
    alt brief named and unreadable
      B-->>H: brief-unreadable
    else
      B->>B: settings missing? run the interview through the host
      B->>F: remote, reachability, tag ruleset
      F-->>B: facts
      B->>L: forge.checked
      B->>B: checkout empty or existing code
      B-->>H: receipt, forge report with offers, planner work order
      H->>P: launch with the work order
      P-->>H: submission
      H->>B: scope submit
      alt malformed, or a story committed to two open phases
        B-->>H: malformed-submission naming section and field, or story-double-assigned naming the story
      else
        B-->>H: preview and digest
        H->>O: preview
        O->>H: yes
        H->>B: scope approve (digest, owner, time)
        B->>L: project.described, story.declared, phase.declared, phase.reordered, scope.approved
        B-->>H: receipt
      end
    end
  end
```

*Figure 4. Starting a project.*

```mermaid
sequenceDiagram
  participant O as Owner
  participant B as Baley
  participant D as Hardin
  participant L as Ledger
  O->>B: phase withdraw N, decisions per story
  B->>D: may phase N be withdrawn?
  alt complete
    D-->>B: no
    B-->>O: phase-complete
  else execution present
    D-->>B: no, undo needed
    B-->>O: execution-present, naming the undo
  else
    B->>B: every active story decided?
    alt one undecided
      B-->>O: story-undecided
    else
      B-->>O: preview and digest
      O->>B: approve
      B->>L: phase.withdrawn, story.reassigned or story.dropped, phase.reordered, scope.approved
      B-->>O: receipt
    end
  end
```

*Figure 5. Withdrawing a phase.*

The revision of a story's truths is [0005](0005-context-plans-and-acceptance.md) Figure 4.

## 9. Settings

Not applicable. This area reads no setting of its own. The start runs the settings interview of [0003](0003-configuration-and-routing.md) when settings are missing (PRJ-R9), and the planner's route comes from `roles.planner.model` and `roles.planner.effort` ([0003](0003-configuration-and-routing.md)).

## 10. Instructions served

| Instruction | Served to | Carries requirements |
|---|---|---|
| Start planner | The planner, inside the start work order: the description so far, the brief, the survey instruction when the checkout has code, the declaration shape, the story statuses, the assignment rule, the truth rules of [0005](0005-context-plans-and-acceptance.md) | PRJ-R1, PRJ-R4, PRJ-R7, PRJ-R11 |
| Scope change stubs | The host session, as the stub skills for project start, scope, phase, story and backlog: which operation to call and that the owner approves the preview | PRJ-R19 |

## 11. Build status

The binary parks the inherited engine for Build 9 to delete, and nothing in production reaches it (`crates/baley/src/inherited.rs:1-4`). The session server answers `plan-read` and `context-intake` (`crates/baley/src/mcp/operations.rs:105-106`) and `plan-submit` and `context-submit` (`crates/baley/src/mcp/operations.rs:153-154`) as `operation-unavailable`, and the operation baseline names Build 4 for them. It answers `verification-complete` (`crates/baley/src/mcp/operations.rs:133`), the roadmap tick, the same way and names Build 5. The baseline has no start, story or phase spelling. Production reaches `baley init` in this area (`crates/baley/src/init.rs:557-566`). Parsing and editing `ROADMAP.md` and `REQUIREMENTS.md` under `.planning/` is the parked engine's.

| Requirement | Status | Where |
|---|---|---|
| PRJ-R1, PRJ-R2, PRJ-R5, PRJ-R7 | Not built | No start operation exists |
| PRJ-R3 | Not built | Only the parked engine validates a context submission field by field (`crates/baley/src/context_service.rs:60-63`, `crates/baley/src/context/validation.rs:5-106`), and no scope submission exists. The session server answers `context-submit` as unavailable (`crates/baley/src/mcp/operations.rs:154`) until Build 4 |
| PRJ-R4 | Not built | The parked engine seeds `REQUIREMENTS.md` rows at plan-submit (`crates/baley/src/plan_service.rs:410-437`) and has no assignment check. The session server answers `plan-submit` as unavailable (`crates/baley/src/mcp/operations.rs:153`) until Build 4 |
| PRJ-R6 | Partly built | `baley init` is built as its own command (`crates/baley/src/init.rs:557-566`); the start operation that runs it is Build 4 (#25) |
| PRJ-R8, PRJ-R9 | Not built | |
| PRJ-R10 | Not built | The parked engine parses phase ids as floating-point numbers (`crates/baley/src/derivation/model.rs:20-22`) and takes their order from the textual order of `ROADMAP.md` (`crates/baley/src/derivation/parse.rs:150-191`) |
| PRJ-R11, PRJ-R12, PRJ-R13 | Not built | No phase declaration, edit or withdraw exists. The parked engine edits `ROADMAP.md` only to tick a phase (`crates/baley/src/verification/completion.rs:264-303`), and the session server answers `verification-complete` as unavailable (`crates/baley/src/mcp/operations.rs:133`) until Build 5 |
| PRJ-R14, PRJ-R15, PRJ-R16 | Not built | The parked engine refuses a second context approval `native-context-exists` (`crates/baley/src/context_service.rs:184-193`), writes truth version 1 always (`crates/baley/src/context/persistence.rs:33-49`), and its plan check accepts only version 1 (`crates/baley/src/plan/limits.rs:401-408`). The session server answers `context-submit` and `plan-submit` as unavailable (`crates/baley/src/mcp/operations.rs:153-154`) until Build 4 |
| PRJ-R17 | Not built | |
| PRJ-R18 | Not built | Only the parked engine answers `unavailable` at context intake for a phase not on the roadmap (`crates/baley/src/context_service.rs:35-53`). The session server answers `context-intake` as unavailable (`crates/baley/src/mcp/operations.rs:106`) until Build 4 |
| PRJ-R19 | Not built | Only the parked engine submits, drafts and approves a context by digest, with `stale-draft` and `unknown-draft` (`crates/baley/src/context_service.rs:116-207`). Its drafts are memory-only and lost on restart (`crates/baley/src/session/mod.rs:427-437`), and owner and time are any non-blank strings. The session server answers `context-submit` as unavailable (`crates/baley/src/mcp/operations.rs:154`) until Build 4 |
| PRJ-R20 | Not built | No `project.described` event is recorded. Only the parked engine reads `PROJECT.md`, to name the pause branch (`crates/baley/src/pause/branch.rs:131`), and nothing writes it |

## 12. Open questions

| Question | Decided by |
|---|---|
| How a session establishes the owner's identity and the time of an approval, and how the interview's questions reach the owner from a session | [0012: Host interface](0012-host-interface.md) |
