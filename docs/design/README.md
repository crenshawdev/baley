# Design process

Every significant change to Baley is designed on paper, reviewed and approved before it is built. This page says what gets written, where it lives, how it is reviewed and how it is tracked.

## What counts as significant

A change needs a design document when it does any of these:

- adds or replaces a component, a storage format or a wire contract;
- changes how two components talk to each other;
- changes a workflow a user or a host sees;
- is hard to reverse once shipped.

Bug fixes, refactors inside one module and dependency updates do not need one. When in doubt, write a short one.

## The documents

| Kind | Location | Purpose | Lifecycle |
|---|---|---|---|
| Design document | `docs/design/NNNN-slug.md` | The proposed design for one piece of work, before it is built | Draft, In review, Accepted, Rejected, Superseded |
| Decision record (ADR) | `docs/adr/NNNN-slug.md` | One architectural decision, its context and its consequences | Proposed, Accepted, Superseded |
| Architecture overview | `docs/architecture/` | The system as it is built today | Updated with the code that changes it |

Numbers are four digits, assigned in order and never reused. Start a design document from [TEMPLATE.md](TEMPLATE.md) and a decision record from [../adr/TEMPLATE.md](../adr/TEMPLATE.md).

**Design documents** describe what will be built and why. They are written before the code, and their diagrams come before the code too. Once accepted, a design document is a record of the approved design. It is not rewritten to track later changes: a later change gets its own design document, and the old one is marked Superseded with a link forward.

**Decision records** follow Michael Nygard's format through the [MADR](https://adr.github.io/madr/) template. Each one holds exactly one decision. An accepted record is never edited; a new record supersedes it. Design documents list the decisions they produce, and each decision links back to its design document.

**The architecture overview** shows only what exists in the code. It is updated in the same pull request as the code that changes it, never ahead of it.

## Diagrams

Diagrams are [Mermaid](https://mermaid.js.org/), fenced as ` ```mermaid ` inside the Markdown file, so they render on GitHub and in common editors with nothing to install and diff as text in review. No image exports, no separate diagram files.

| Question the diagram answers | Diagram |
|---|---|
| What is the system and what surrounds it | C4 system context (`C4Context`) |
| What runs, and what talks to what | C4 container (`C4Container`) |
| What is inside one container | C4 component (`C4Component`) |
| Who does what, in what order, across actors | Swim lane: `flowchart` with one `subgraph` per actor |
| How one request moves between parts | UML sequence (`sequenceDiagram`) |
| What states a record passes through | UML state machine (`stateDiagram-v2`) |
| How types relate | UML class (`classDiagram`) |
| What the stored data looks like | Entity relationship (`erDiagram`) |

Every diagram has a title and a one-sentence caption saying what it shows. Draw the level the reader needs: a design document for the store starts with the container view and goes down only where the design is decided.

## Requirements and traceability

Each design document states its requirements with stable identifiers: the document's short prefix and a number, such as `STORE-R1`. Identifiers are never renumbered; a dropped requirement stays in the table marked Withdrawn.

Build issues, pull requests and tests cite the identifiers they satisfy, so any line of code can be traced back to the requirement and the decision behind it.

## Review and approval

1. The author opens a pull request with the design document at status Draft and the design issue linked.
2. The document moves to In review. A reviewer independent of the author writes an adversarial review. When the author is an AI model, the reviewer is a model from a different family. The review says what is wrong, missing or unproven, and against which requirement, and is posted on the pull request.
3. The author answers every finding on the pull request, either with a change to the document or with a reason.
4. The owner approves the pull request. The author's last commit sets the status to Accepted and the owner merges. A rejected design is merged at status Rejected with the reason, so the reasoning is kept.

Decision records produced by the design are merged in the same pull request.

## Tracking

Work is tracked on GitHub.

- **Milestones** are named for their theme, never a version number. Versions are assigned when a release ships.
- **Each milestone opens with a design issue** labelled `design`. Its pull request carries the design document.
- **Build issues** are opened from the accepted design, one per slice of work, each citing the requirements it delivers. None starts until the design is merged.
- **Pull requests** link their issue and cite requirement identifiers. Commits follow [Conventional Commits](https://www.conventionalcommits.org/) and are signed.

## What these documents are not

They are technical records for an engineer who has never met the authors. They carry no conversation transcripts, no private notes and no quotes from discussions. Anything a reader needs to understand the design is stated in the document itself.

## Index

No design documents yet.
