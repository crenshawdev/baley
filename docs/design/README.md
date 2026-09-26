# Design process

Baley is designed on paper before it is built. This page says what gets written, where it lives, how it is kept current, how it is reviewed and how it is tracked.

## What needs a design change

A change is designed before it is built when it does any of these:

- adds or replaces a component, a storage format or a wire contract;
- changes how two components talk to each other;
- changes a workflow a user or a host sees;
- is hard to reverse once shipped.

Bug fixes, refactors inside one module and dependency updates do not.

## The documents

| Kind | Location | Purpose |
|---|---|---|
| Product requirements (PRD) | [`prd/`](prd/) | What Baley is for and what its owner can do, as numbered user stories |
| System design | [`0002-system-design.md`](0002-system-design.md) | The architecture every area follows: who is responsible for what, the patterns, the parts and the decisions that cut across areas |
| Area design documents | `NNNN-slug.md` | The full design of one process area, in the form of [TEMPLATE.md](TEMPLATE.md). The set: 0003 configuration and routing, 0004 starting a project and changing scope, 0005 context, plans and acceptance, 0006 execution, 0007 verification, 0008 review, 0009 risk, 0010 guard, 0011 milestones, landing, undo and pause, 0012 host interface, 0013 next action and progress, 0014 support families |
| Decision records (ADRs) | [`../adr/`](../adr/) | One architectural decision each: its context, the options and why one was chosen |
| C4 model | [`c4/workspace.dsl`](c4/workspace.dsl) | The one model of Baley's structure; every structure diagram is exported from it |
| Architecture overview | `../architecture/` | The system as it is built today, updated with the code that changes it |

Numbers are four digits, assigned in order and never reused.

## Living documents

Design documents state the current design and nothing else. When the design changes, the document is edited in place, in every section the change touches. A document carries no amendment lists, no "previously" and no superseded copies; git holds the history.

The pull request that builds part of a design edits the document to match what was built, including its Build status section, and its description says what changed.

A document describes the design. It never tracks work: no task lists, no next steps, no schedule. Deciding what happens next is Baley's job.

Decision records are the one kind of history kept on purpose. An accepted record's body is never edited; a new record supersedes it, and the old record's status then reads "Superseded by NNNN" or "Accepted, superseded in part by NNNN". Records follow Michael Nygard's format through the [MADR](https://adr.github.io/madr/) template. Design documents list the decisions they produce, and each decision links back to its design document.

## The form of an area document

Every area document has the same twelve sections, in order, from [TEMPLATE.md](TEMPLATE.md): purpose and scope, terms, requirements, roles and actors, commands and operations, records, states, workflows, settings, instructions served, build status and open questions. A section that does not apply keeps its heading and says "Not applicable" with the reason, so every document has the same shape.

A document is detailed enough that an engineer who has never seen the project can understand the whole area from it alone.

## Requirements and traceability

Each design document states its requirements with stable identifiers: the document's short prefix and a number, such as `EVD-R1` or `SYS-R3`. Identifiers are never renumbered or reused. A requirement's status is `Active` (the design now), `Backlog` (decided, for a later release; the design says what it is, the release says when) or `Withdrawn` (dropped; the row and id stay). Quality requirements (speed, memory, security, reliability) have their own identifiers and a pass/fail check.

Build issues, pull requests, tests and the instructions Baley serves cite the identifiers they satisfy, so any rule in the code traces back to its requirement.

A reference to something not written yet is written `[TARGET]`, so it is visible and searchable (`grep -rn "\[TARGET\]" docs/`). A document is finished when no unresolved `[TARGET]` remains.

## Diagrams

Diagrams are [Mermaid](https://mermaid.js.org/) inside the Markdown, so they render on GitHub and in common editors with nothing installed, and diff as text.

Structure diagrams (system context, containers, components) come from the C4 model in [`c4/workspace.dsl`](c4/workspace.dsl), written in [Structurizr](https://docs.structurizr.com/) DSL. The model is defined once and every view is generated from it, so the diagrams cannot disagree. A document marks where a view goes:

```
<!-- c4:context -->
<!-- /c4:context -->
```

and `docs/design/c4/run.sh export <documents>` replaces the text between the markers with the current Mermaid for that view. Never edit inside the markers by hand; change the model and export again. `run.sh validate` checks the model. Both need only Java; `run.sh` downloads `structurizr.war` once.

Other diagrams are written by hand in Mermaid:

| Question the diagram answers | Diagram |
|---|---|
| Who does what, in what order, across actors | Swim lane: `sequenceDiagram`, one participant per actor, `alt` for branches |
| How one request moves between parts | `sequenceDiagram` |
| What states a record passes through | `stateDiagram-v2` |
| How types relate | `classDiagram` |
| What the stored data looks like | `erDiagram` |

Every diagram has a caption saying what it shows, and leaves its background unset so it reads in light and dark themes.

## Review and approval

1. The author opens a pull request with the document and links its design issue.
2. A reviewer independent of the author writes an adversarial review: what is wrong, missing or unproven, and against which requirement or section. When the author is an AI model, the reviewer is a model from a different family. The review goes to the owner.
3. The author checks every finding against the document and the code, and brings the owner each one that holds, in plain terms, with the options for fixing it. The owner rules on each.
4. The author edits the document to match the rulings. The pull request lists every finding and the change made for it.
5. The document is accepted when its open questions hold only what is genuinely deferred to another document, and any acceptance gate it names (a benchmark, a check on each supported host) has passed. The owner approves and merges.

## Tooling

The documents are written with the `baley-design` plugin for Claude Code: `prd`, `design-doc` (area documents), `nfr`, `api`, `adr` and `c4`. The plugin keeps every project's documents in the same shape.

## Tracking

Work is tracked on GitHub.

- **Milestones** take their names from Asimov's Robot and Foundation stories, and each one's description states its theme in plain words. Never a version number: versions are assigned when a release ships. Working branches take names from the same stories.
- **Design issues** are labelled `design`. Their pull requests carry the design documents.
- **Build issues** are opened from an accepted design, one per slice of work, each citing the requirements it delivers.
- **Pull requests** link their issue and cite requirement identifiers. Commits follow [Conventional Commits](https://www.conventionalcommits.org/) and are signed.

## What these documents are not

They are technical records for an engineer who has never met the authors. They carry no conversation transcripts, no private notes and no quotes from discussions. Anything a reader needs to understand the design is stated in the document itself.

## Index

| Document | Status |
|---|---|
| [Product requirements](prd/baley.md) | Draft |
| [0001: The evidence ledger](0001-evidence-ledger.md) | Accepted |
| [0002: System design](0002-system-design.md) | Draft |
| [0003: Configuration and routing](0003-configuration-and-routing.md) | Draft |
| [0004: Starting a project and changing scope](0004-starting-a-project-and-changing-scope.md) | Draft |
| [0005: Context, plans and acceptance](0005-context-plans-and-acceptance.md) | Draft |
| [0006: Execution](0006-execution.md) | Draft |
| [0007: Verification](0007-verification.md) | Draft |
| [0008: Review](0008-review.md) | Draft |
| [0009: Risk](0009-risk.md) | Draft |
| [0010: Guard](0010-guard.md) | Draft |
