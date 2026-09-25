# 0001: The evidence ledger

| | |
|---|---|
| Status | In review |
| Author | John Crenshaw |
| Reviewers | Codex, adversarial review, 2026-09-25 (25 findings, all addressed in this revision) |
| Design issue | #4 |
| Milestone | Evidence |
| Requirement prefix | EVD |
| Supersedes | |
| Superseded by | |

## Summary

Baley records every fact about the work it governs (plans the owner approved, what each agent did, the test runs that prove it, the verdicts, the reviews) in one append-only, hash-chained event ledger per project, kept in a single SQLite database per user and anchored to the forge so that a rewrite is detectable from outside the machine. Current state is a set of views derived from the ledger in the same transaction, large or sensitive payloads are stored once by content hash and can be purged by reference, and all of it sits behind a storage port that domain code cannot see past. This replaces the JSON store, the `.planning/` directory and every Markdown copy the binary reads or writes today.

## Context

### What Baley is for

Baley is an owner's control plane for AI-assisted engineering. The owner decides and answers for the work; agents (the Daneels) do it; the part of Baley that decides what may happen next (Hardin) allows a step only when the recorded evidence supports it. The records are the product: a claim that has no record behind it does not count.

### What exists today

The binary keeps its records in three files under `<project>/.planning/`: `state.json`, one JSON document holding about 30 namespaces; `decisions.jsonl`, a log that mirrors many of those records in full; and `items.jsonl`, the capture queue. It also renders Markdown copies of its records into the same directory (phase `CONTEXT.md`, `PLAN-k.md`, `SUMMARY.md`, `UAT.md`, task, spike and debug records), writes deferred-review JSON files there, and reads `ROADMAP.md` and `REQUIREMENTS.md` as inputs.

The same fact is routinely held three times: once in a snapshot namespace, once as a mirror line in the log, once as a Markdown file. Most of the store's machinery exists to keep those copies in agreement:

- every write re-renders and re-hashes the whole snapshot and both logs, and a multi-file intent journal carries the full new bytes of every changed file;
- a commit validator of about 650 lines compares whole namespaces before and after each write and re-derives each transition;
- records are bound to the physical directory through device and inode numbers, so the store cannot be moved, copied or rebuilt elsewhere;
- almost every read deserializes or deep-clones the whole store, and several readers scan the whole log to find one entry.

On the store left by the Cadence 4.0 build (about three weeks, 40 phases), `state.json` was 99.1 MB and `decisions.jsonl` 76.3 MB. The long-running server reached 8.7 GB resident against a 62 MB store, and 2.5 GB after one cold read. A first estimate, made on that JSON with duplicate copies and retired prompt text removed, puts the unique large content at 3.6 MB (0.7 MB compressed) and the rest at no more than 38.9 MB (4.0 MB compressed). That estimate is not evidence for the new design; the benchmark in [Performance](#performance) is.

The Markdown inputs are also a second source of truth. Phases exist only as lines in `ROADMAP.md`; execution parses its task list out of `PLAN-k.md`; two features (`why` and `recall`) read old Markdown out of git history.

### Why now

Baley starts with an empty store (the Cadence records are not imported), so the record model can be chosen on its merits without a data migration.

## Goals

1. Every fact is recorded once, attributed, and cannot be altered without detection, including by a process running as the owner.
2. Any current-state question is answered by key, reading only what the answer needs.
3. Several processes on one machine use the store at once without corrupting it or waiting noticeably.
4. The storage engine is replaceable without touching domain code.
5. A project's history outlives any checkout of it and can be exported, backed up, verified and purged.
6. Baley's records never live in the working tree.

## Non-goals

- **Moving records between machines.** The ledger is designed so it can travel later (see [Future work](#future-work)); only chain-head anchors leave the machine in this milestone.
- **Multi-user or server deployment.** One user, one machine. The port leaves room for a server adapter; none is built.
- **A separate operating-system user for Baley.** Real process separation is future work; this design uses the hosts' sandboxes (see [Threat model](#threat-model)).
- **Importing Cadence records.** The store starts empty.
- **Redesigning the domain rules.** Plans, execution, verification, review, guard, pause and milestones keep their current rules; only how they are recorded changes. [Rules preserved](#rules-preserved) lists each rule and where it lives in the new design. Where a rule exists only to reconcile copies, it is removed.
- **Windows.** Baley ships for Linux and macOS.

## Threat model

| Actor | Can | Defended by |
|---|---|---|
| Accidental failure: crash, power loss, disk error, a bug | Leave a transaction half-done, corrupt pages | SQLite transactions and durability, `integrity_check`, verified backups |
| A person or program editing the database outside Baley | Change rows directly | The hash chain detects naive edits; forge anchors detect edits that recompute the chain |
| An agent running as the owner's user | Everything the owner's files allow, including running `sqlite3` on the database and signing with the owner's cached GPG key | The host sandbox denies the agent access to Baley's home (prevention); the guard refuses file tools and shell commands that name the home (best effort); forge anchors detect a rewrite, truncation or rollback of anything before the latest anchor (detection) |
| Another local user | Read or write files they have access to | Private file modes and ownership checks on every open |
| A remote attacker | Nothing directly | The store makes no network calls; only the chain head is pushed to the forge |

Not defended in this milestone: an attacker with the owner's forge credentials and local access at once, who could both rewrite the database and push a matching anchor; and changes made after the latest anchor, which are detectable only against the local chain. The project id is an identity, not an authorization: holding it grants nothing.

## Requirements

Identifiers are stable. Requirements changed by the review keep their number; new ones are appended.

| ID | Requirement | Source |
|---|---|---|
| EVD-R1 | Every fact is recorded exactly once, in the ledger. No other copy is maintained. | Context: triple copies |
| EVD-R2 | Recorded events are never modified or deleted. The only exception is a payload body removed under EVD-R14. | Goal 1 |
| EVD-R3 | Each project's events form a hash chain whose head is anchored outside the machine. Verification detects any modified, inserted, deleted or reordered event, any truncation, and any rollback to an older database, up to the latest anchor, and names the first bad position. | Goal 1; threat model |
| EVD-R4 | Every event records its actor (owner, agent role or Baley), its time, its project, and the git facts it depends on (commit, tree, checkout). | Goal 1 |
| EVD-R5 | A command's events and view updates commit together or not at all. | Goal 3 |
| EVD-R6 | A request id is generated by the caller, scoped to its project and command kind, and bound to a digest of the command. A retry with the same id and digest returns the original outcome and records nothing new; the same id with a different digest is refused. | Hosts retry tool calls |
| EVD-R7 | A command's decision is made inside its write transaction from inputs read in that transaction, and its cheap git facts are re-checked there. A command whose inputs moved is refused, never merged. | Goal 3 |
| EVD-R8 | The MCP servers of several sessions, the guard hook and the CLI can use the store at the same time on one machine. Readers never wait for writers. | Goal 3 |
| EVD-R9 | Every current-state question is answered from views through a declared key or index, with defined ordering, paging and bounds, without reading the log or unrelated records. Memory used by a query is proportional to its answer, not to the store. | Goal 2; store memory growth |
| EVD-R10 | Any view can be rebuilt from the ledger with an identical result, including after any purge. Rebuilds never block writers for more than one short transaction. | Goal 4 |
| EVD-R11 | Payloads are stored once, compressed, addressed by SHA-256 of their bytes. Events reference them by hash. | Context: frozen copies |
| EVD-R12 | Domain code has no dependency on the storage engine, enforced by the crate graph. Every adapter passes one conformance suite. | Goal 4 |
| EVD-R13 | Recorded text can be searched with relevance ranking, scoped by project and phase, with semantics defined independently of the engine. | Replaces custom recall index |
| EVD-R14 | Payload bodies can be removed by retention policy or by command without breaking EVD-R3 or EVD-R10. Retention applies per reference. Each removal is an event in every affected project's chain, and removes every derived copy Baley manages. | Goal 5 |
| EVD-R15 | A project's ledger can be exported as a standalone database that verifies on its own. The whole store can be backed up while in use. | Goal 5 |
| EVD-R16 | There is one database per user, outside any checkout, in the platform data directory on Linux and macOS. `BALEY_HOME` overrides the location. | Goal 5 |
| EVD-R17 | A checkout maps to its project through a committed project file holding the project id. No record holds a filesystem identity. | Goal 5 |
| EVD-R18 | Baley never writes its records into the working tree. The only working-tree writes are the ones named in [Working-tree writes](#working-tree-writes). | Goal 6 |
| EVD-R19 | The database carries a compatibility epoch, checked in every write transaction. A process that finds a newer epoch stops writing. Migrations run in one transaction after an automatic backup. Views only ever rebuild forward. Stored events are never rewritten. | Development builds share the machine |
| EVD-R20 | A command acknowledged to its caller survives power loss. | Goal 1 |
| EVD-R21 | Performance and size budgets hold on the reference workload, measured before acceptance, as set out in [Performance](#performance). | Goal 3 |
| EVD-R22 | The store files are owned by and private to the owning user. Every open checks ownership, modes and symbolic links on the real database path. | Records hold source and output |
| EVD-R23 | The store refuses to open on a network filesystem, judged at the real database path, and says why. | SQLite write-ahead log constraint |
| EVD-R24 | The design works with Claude Code and Codex as host, proven by the host matrix. Each host's sandbox denies agents access to Baley's home while Baley's server and hook can still write it. | Host neutrality; threat model |
| EVD-R25 | An owner can see the state and history of any record without reading files: through the CLI, through the MCP document query, and through an explicit export. | Replaces Markdown copies |
| EVD-R26 | A command with an effect outside the database claims its request and records its intent before acting, and records the result after. Retries and duplicates never repeat the effect. An active claim blocks only its own scope; an interrupted claim (lease expired) is reconciled before work in its scope continues. | External effects cannot be rolled back |
| EVD-R27 | A decision that grants authority (admission, completion, landing, release) confirms its deciding facts against events inside its transaction, so an edited view cannot grant authority. | Views are derived data |
| EVD-R28 | Every rule listed in [Rules preserved](#rules-preserved) behaves as it does today, proven by an equivalence test. | Non-goal: redesigning domain rules |

## Design

### Overview

Three ideas carry the design.

**The ledger is the only source of truth.** Everything Baley learns or decides is appended to the ledger as an event: a small, typed, attributed record of one fact, such as "plan 5-2 approved by the owner" or "suite run R7 passed". Events are never edited. A correction is a new event.

**Current state is a projection.** Hardin, the Daneels and the owner mostly ask what is true now: what phase 5's status is, which plan is approved, what the next allowed step is. Those answers live in views: keyed documents computed from the events by domain code and updated in the same transaction that appends the events. Views can be thrown away and rebuilt from the ledger, so they never become a second source of truth, and decisions that grant authority check the events themselves.

**Storage is behind a port.** Domain code sees traits that speak Baley's language (append these events, get this view by key, store this payload) and never SQL. SQLite is one adapter behind that port.

```mermaid
flowchart LR
  owner(["Owner<br/><small>Decides and answers for the work</small>"])
  host["Host agent<br/><small>Claude Code or Codex, running the Daneels</small>"]
  baley["Baley<br/><small>Records the evidence, decides what may happen next</small>"]
  git["Git repository<br/><small>Source, commits, the project file</small>"]
  forge["Forge<br/><small>Immutable chain-head anchors</small>"]
  prov["Review providers<br/><small>Outside models for adversarial review</small>"]
  owner -->|directs work| host
  owner -->|queries, approves: CLI| baley
  host -->|tool calls: MCP| baley
  baley -->|reads history, runs git| git
  baley -->|pushes anchors| forge
  baley -->|review material: HTTPS| prov
  classDef person fill:#08427b,stroke:#052e56,color:#fff
  classDef system fill:#1168bd,stroke:#0b4884,color:#fff
  classDef external fill:#6b6b6b,stroke:#4d4d4d,color:#fff
  class owner person
  class baley system
  class host,git,forge,prov external
```

*Figure 1. System context, in the C4 model's sense. Baley sits between the owner, the host agent that runs the Daneels, the repository, the forge that holds its anchors, and the outside reviewers.*

```mermaid
flowchart TB
  owner(["Owner"])
  host["Host agent<br/><small>Claude Code or Codex, sandboxed</small>"]
  subgraph baley [Baley]
    direction TB
    server["MCP server<br/><small>one per host session; Hardin decides the next step</small>"]
    guard["Guard hook<br/><small>one per tool call; refuses unsafe actions</small>"]
    cli["CLI<br/><small>show, verify, export, backup, doctor, purge</small>"]
    db[("Ledger database<br/><small>SQLite, one per user</small>")]
  end
  checkout["Project checkout<br/><small>git working tree with the project file</small>"]
  host -->|tool calls| server
  host -->|before each tool call| guard
  owner -->|commands| cli
  server --> db
  guard --> db
  cli --> db
  server -.->|finds project file, runs git| checkout
  guard -.->|finds project file| checkout
  classDef person fill:#08427b,stroke:#052e56,color:#fff
  classDef container fill:#438dd5,stroke:#2e6295,color:#fff
  classDef external fill:#6b6b6b,stroke:#4d4d4d,color:#fff
  class owner person
  class server,guard,cli,db container
  class host,checkout external
```

*Figure 2. Containers, in the C4 model's sense. Every solid arrow into the database goes through the same storage port. The host's sandbox keeps its agents out of Baley's home; only Baley's own processes reach the database.*

### Terms

| Term | Meaning |
|---|---|
| Event | One recorded fact. Immutable, typed, attributed, hash-chained. |
| Stream | The events of one thing that changes over time, such as one plan or one dispatch. Named, for example `plan/5-2`. Each stream has its own version counter. |
| Project sequence | The position of an event in its project's ledger. The hash chain follows this order. |
| Anchor | A copy of a project's chain head (sequence and hash) pushed to the forge as an immutable tag. |
| View | A keyed collection of documents computed from events, answering one kind of current-state question. |
| Projector | Domain code that updates views from an event. Pure: event and current documents in, document changes out. |
| Payload | Content stored once by hash, outside the event: large content, and every sensitive kind of content regardless of size. |
| Reference | One event's use of a payload, carrying the retention class for that use. |
| Command | One request from a caller that may record events. It carries a request id. |
| Claim | The intent event a command with an external effect records before acting. |
| Port | The set of storage traits the domain depends on. |
| Adapter | An implementation of the port for one engine. |
| Hardin | The part of Baley that reads views and names the one allowed next step. |
| Daneels | The agents, driven by a host, that do the work and record it through Baley's tools. |

### Detailed design

#### Crate structure (EVD-R12)

```mermaid
flowchart TB
  subgraph workspace [Cargo workspace]
    server["baley<br/>binary: MCP server, guard hook, CLI"]
    core["baley-core<br/>events, views, projectors, domain rules, Hardin"]
    port["baley-store<br/>storage port traits, conformance suite"]
    sqlite["baley-store-sqlite<br/>SQLite adapter"]
  end
  rusqlite[("rusqlite, bundled SQLite")]
  server --> core
  server --> sqlite
  core --> port
  sqlite --> port
  sqlite --> rusqlite
```

*Figure 3. Only the binary and the adapter know the engine exists. `baley-core` cannot import rusqlite, so domain code cannot reach SQL.*

The binary wires one adapter into the core at start-up. Tests of the domain run against the real SQLite adapter in a temporary directory or in memory; no fake store exists.

#### The storage port

The port is shaped around Baley's access patterns: by id, by phase, by (phase, plan), by request id, the audit history of one thing, and search. It is not a generic create-read-update-delete repository. [Appendix B](#appendix-b-reads-mapped-to-views) maps every current read to the view key that serves it.

```mermaid
classDiagram
  class Ledger {
    <<trait>>
    +transact(project, command, decide) Outcome
    +stream(project, stream, from_version, page) Events
    +history(project, range, filter, page) Events
    +verify(project) ChainReport
  }
  class Transaction {
    <<trait>>
    +get(view, key) Document
    +find(view, index, range, page) Documents
    +event_exists(project, type, match) bool
    +expect(stream, version)
    +append(stream, event) Seq
    +put_payload(bytes, class) Hash
  }
  class Views {
    <<trait>>
    +get(view, key) Document
    +find(view, index, range, page) Page
  }
  class Payloads {
    <<trait>>
    +open(hash) PayloadStream
    +status(hash) Present or Reduced or Purged
  }
  class Search {
    <<trait>>
    +search(project, query, scope, page) Hits
  }
  class Admin {
    <<trait>>
    +backup(target)
    +export(project, target)
    +purge(project, selector) PurgeReport
    +rebuild(view, project)
    +anchor(project) Anchor
    +doctor() Health
  }
  class Projector {
    <<trait, implemented in baley-core>>
    +view() ViewSpec
    +apply(event, documents) Changes
  }
  Ledger ..> Transaction : decide runs inside
  Ledger ..> Projector : runs after append
  Transaction ..> Payloads : attachments
```

*Figure 4. The storage port. The decision runs inside the transaction with read access; projectors live in the core and are handed to the adapter, so the adapter never contains business rules.*

- **`transact`** runs one command's decision. The caller does its slow work first (tests, model calls, git work) and passes the results in. The adapter opens the write transaction, checks the request (see [Commands](#commands)), and calls `decide` with a `Transaction`. `decide` re-reads every input it depends on from views inside the transaction, re-checks the command's cheap git facts, confirms authority against events where required, and appends events. The adapter then runs every projector registered for those event types, writes the view changes and search entries, and commits. If any step fails, nothing is recorded (EVD-R5). Because SQLite admits one writer at a time, nothing can change between `decide`'s reads and the commit (EVD-R7).
- **`expect`** additionally names the stream that serializes a contested decision, so two commands that would both pass their own checks are ordered by one version counter. Each contested decision names its stream in [Serializing streams](#serializing-streams).
- **Views** are declared by the core as a `ViewSpec`: a name, a version, a key shape, indexed fields, the ordering of each index and a page-size bound. `find` reads by a declared index, never by scan, and returns a page with a cursor. One query runs against one read snapshot.
- **Payloads** are read through a stream handle, never loaded with the view that references them. A purged or reduced payload returns its status and the tombstone, not an error.
- **Search** is a capability with defined semantics: terms and quoted phrases, scoped by project and optionally phase, results in descending relevance with stable tie-breaking by sequence. The SQLite adapter implements it with FTS5 and BM25; nothing in the core depends on FTS5 syntax.
- **Admin** covers everything an owner does to the store as a whole.

#### Events

Every event has an envelope and a payload.

| Field | Meaning |
|---|---|
| `project_id` | The project the event belongs to. |
| `seq` | Project sequence, starting at 1, no gaps. |
| `stream`, `stream_version` | The stream and its version after this event. Unique together within a project. |
| `type`, `type_version` | The event type, such as `plan.approved`, and the version of its payload schema. |
| `actor` | `owner`, an agent role (for example `daneel:executor`), or `baley`. |
| `recorded_at` | UTC time the event was recorded. |
| `request_id` | The command that recorded it. |
| `git` | The git facts the event depends on: `commit`, `tree`, and the `checkout` it was observed in. Absent when the event depends on none. |
| `policy_version` | The effective policy the command ran under. |
| `payload` | The event's typed content, as canonical JSON. Every fact a projector or the search index needs is inline. Attachments (outputs, review material, prompts, plan and context text) are references `{ "payload": "<sha256>", "bytes": n, "class": "<retention class>" }`. |
| `prev_hash`, `hash` | The hash chain. |

Payloads are JSON so that the ledger stays readable with standard tools and queryable through SQLite's JSON functions. For hashing, the envelope (without `hash`) and the payload are serialized with the JSON Canonicalization Scheme (RFC 8785), so the same event always hashes the same way on any platform.

Event types are named `<family>.<fact>` in the past tense, and each has a payload schema version. A payload schema change adds a new version; old events are never rewritten and are read through an upcaster that converts old versions to the current shape (EVD-R19). An event type or version a binary does not know makes that project read-only for that binary.

Streams used by the record families:

| Stream | Examples of events |
|---|---|
| `project` | `project.initialized`, `policy.effective`, `checkout.seen` |
| `roadmap` | `phase.declared`, `phase.renamed`, `phase.reordered`, `requirement.declared` |
| `phase/<n>` | `context.approved`, `dispatch.issued` (serializes one active dispatch per phase), `phase.completed`, `completion.invalidated`, `phase.undone` |
| `plan/<n>-<k>` | `plan.submitted`, `plan.approved`, `plan.superseded` |
| `admission/<n>` | `execution.admitted`, `execution.extended` |
| `dispatch/<id>` | `task.started`, `task.run`, `task.closed`, `suite.run`, `dispatch.ended`, `worker.exited`, `worker.interrupted` |
| `verification/<id>` | `verification.started`, `verification.run`, `verdict.claimed`, `truth.waived`, `waiver.revoked`, `human.result`, `verification.completed` |
| `review/<id>` | `review.admitted`, `review.enqueued`, `review.delivered`, `review.returned`, `review.deferred`, `review.adjudicated`, `review.closed` |
| `risk/<n>` | `risk.observed`, `risk.fired`, `risk.receipt` |
| `milestone/<name>` | `milestone.close_ready`, `milestone.archived`, `release.proposed`, `release.confirmed`, `landing.started`, `landing.step`, `landing.completed` |
| `pause` | `pause.recorded`, `pause.resumed` |
| `task/<slug>`, `debug/<slug>`, `spike/<slug>` | the off-roadmap records |
| `capture` | `item.captured`, `item.resolved` |
| `guard` | `guard.allowed`, `guard.asked`, `guard.refused`, `guard.policy_recorded` |
| `command/<kind>` | `command.claimed`, `command.completed`, `command.reconciled` |
| `retention` | `payload.reduced`, `payload.purged` |

The full mapping from today's namespaces is in [Appendix A](#appendix-a-mapping-from-the-current-store).

#### Commands

A command is one request from a caller, carrying a request id.

**Request ids (EVD-R6).** The caller generates a fresh UUID for each command. Baley scopes it to the project and the command kind, so two sessions, two hosts or two command kinds can never collide or receive each other's answers. The request digest is the SHA-256 of the canonical form of the command kind and every field that carries authority (the approved content, the phase and plan, the target dispatch, the policy version). Guard decisions keep today's identity built from the host session and tool-call id.

**Outcomes.** Every command that reaches a domain outcome, success or a real refusal, records a `command.completed` event carrying the request digest and the answer, including commands that record no other event. The `request` view is built from those events and is rebuildable like any other. Infrastructure failures (store busy, disk full) and stale-input refusals are not outcomes: nothing is recorded, and the caller re-reads and tries again with the same request id.

**Database-only commands** run in one transaction: check the request first, then decide, append, project and commit, as in Figure 6.

**Commands with an external effect (EVD-R26)** use three steps, because SQLite cannot roll back a git revert, a provider call or a pushed ref:

1. **Claim.** One short transaction checks the request and records `command.claimed` with the command's inputs and intended effect. A retry or a duplicate finds the claim and receives the in-progress or finished answer; it never repeats the effect.
2. **Act.** The external work runs outside any transaction.
3. **Record.** One more transaction re-checks the claim is still current, records the result events and `command.completed`.

**Active and interrupted claims.** A claim records its owner (process, host session and start time) and a scope. The owner holds a lease on the claim and renews it every 10 seconds while it works; the lease expires 60 seconds after the last renewal. Leases are liveness, not evidence: they live in a `claim_lease` table outside the chain, and renewing one records no event.

- A claim whose lease is current is **active**. It blocks only commands in its own scope; everything else proceeds.
- A claim whose lease has expired is **interrupted**: its owner died or lost its connection. Only an interrupted claim is reconciled.

| Claim | Scope it blocks while active or interrupted |
|---|---|
| Verification run or test | Another run of the same check |
| Undo's revert | Commands on that phase, and any command that writes git in that checkout |
| Landing step, release | The other steps of that milestone |
| Anchor push | Another anchor push for the project |
| Provider review call | Another delivery of the same review |

**Reconciliation.** When Baley finds an interrupted claim, at start or when a command in its scope arrives, it reconciles automatically wherever the real state can be read: an interrupted anchor push checks the remote for its tag, an interrupted test run is recorded as interrupted, an interrupted provider call is recorded as undelivered and may be retried under a new request. It stops for the owner only where the real state is ambiguous, as with a revert that may be half applied, which is today's undo rule. Either way, `command.reconciled` records what was found.

**Failures are outcomes.** An effect that fails cleanly (the remote refused the push, the provider returned an error) is a domain outcome: the record step commits `command.completed` with that result, and the claim is finished, not interrupted. A failed anchor push therefore never blocks work; the unanchored range grows, the next anchor point retries, and `doctor` warns once the unanchored range is more than a day old.

```mermaid
stateDiagram-v2
  [*] --> Active : claim transaction commits, lease taken
  Active --> Active : lease renewed every 10 s
  Active --> Completed : result recorded, success or clean failure
  Active --> Interrupted : lease expires, owner gone
  Interrupted --> Reconciling : start, or a command in its scope
  Reconciling --> Completed : real state read and recorded
  Reconciling --> AwaitingOwner : real state ambiguous
  AwaitingOwner --> Completed : owner reconciles
  Completed --> [*]
```

*Figure 5. A command with an external effect. An active claim blocks only its scope; only an interrupted claim is reconciled, automatically where the real state can be read. A claim is never re-executed.*

```mermaid
sequenceDiagram
  autonumber
  participant D as Daneel (host agent)
  participant S as MCP server
  participant C as baley-core handler
  participant L as Ledger port
  participant Q as SQLite
  D->>S: tool call with request id
  S->>C: command
  C->>C: slow work: run tests, read git, call models
  C->>L: transact(project, command, decide)
  L->>Q: BEGIN IMMEDIATE
  L->>Q: check compatibility epoch, look up request
  alt request already answered
    L->>Q: ROLLBACK
    L-->>C: original outcome, nothing recorded
  else new request
    L->>C: decide(transaction)
    C->>Q: re-read inputs from views, re-check cheap git facts
    C->>Q: confirm authority against events
    alt inputs moved
      L->>Q: ROLLBACK
      L-->>C: refused: stale, retry
    else current
      C->>L: append events, put payloads
      L->>Q: insert payloads and references, events with hash chain
      L->>Q: run projectors, write views and search entries
      L->>Q: record command.completed, advance project head
      L->>Q: COMMIT (synchronous, survives power loss)
      L-->>C: outcome
    end
  end
  C-->>S: answer
  S-->>D: tool result
```

*Figure 6. A database-only command. The request is checked before anything else, and the decision is made inside the transaction from inputs read there.*

#### Serializing streams

| Contested decision | Serialized on |
|---|---|
| One active dispatch per phase | `phase/<n>` |
| Plan approval and supersession within a phase | `phase/<n>` |
| Admission and extension | `admission/<n>` |
| Verification completion and invalidation | `phase/<n>` |
| Milestone close, archive, release and landing | `milestone/<name>` |
| Roadmap changes | `roadmap` |
| Policy changes | `project` |

#### Git facts and checkouts (EVD-R4, EVD-R7)

Git state changes without any stream moving, so every event that depends on git records the facts it saw: HEAD commit, tree, and the checkout it came from. The cheap facts (HEAD and whether the index matches what the command observed) are re-checked inside the write transaction; anything that moved refuses the command. Slow git work stays before the transaction, as today, so the race window is no wider than it is now. Execution permission records the checkout it was granted to; another worktree of the same project must be admitted on its own.

The existing source checks are kept and run at these points:

| Check | Today | Runs |
|---|---|---|
| Verification claim recomputed at commit | `store/writer.rs:1196` | Inside `decide` for `verdict.claimed` |
| Source reachability, commit signatures, staged-path leases | `execution/receipts.rs:700-712` | Before `transact`, facts recorded; HEAD re-checked inside |
| Changed source or index refused on re-observation | `verification/inputs.rs:248` | Inside `decide` |
| HEAD unchanged during an execution request | `execution_service.rs:2131` | Inside `decide` |

#### The hash chain and anchors (EVD-R3)

Each project's events are chained in project-sequence order:

- `hash(1) = SHA-256("baley-ledger/1" || project_id || JCS(envelope(1)) || JCS(payload(1)))`
- `hash(n) = SHA-256(hash(n-1) || JCS(envelope(n)) || JCS(payload(n)))`

`prev_hash` is stored with each event so a break is located without recomputing from the start. A payload enters the chain as its reference, so the chain commits to the content's hash and length, not its bytes: it proves what was committed to, and when, even after the body is purged.

A chain whose head lives only in the database protects against accidents, not against someone who can rewrite the file and recompute every hash. So Baley anchors the head outside the machine. At every verified phase, every milestone step, and at least daily while a project is active, Baley pushes the tag `baley-anchor/<project_id>/<seq>`, whose annotation holds the sequence and head hash, to the project's forge remote. The repository's tag ruleset makes tags impossible to move or delete, for anyone. The anchor push is a command with an external effect and follows the claim, act, record steps.

`baley verify` walks the chain, recomputes every hash, checks every present payload against its hash, and compares the chain with the latest anchor fetched from the forge. It reports the first mismatch, a chain shorter than the anchor (truncation), or a head that differs from the anchor at the anchored sequence (rewrite or rollback). Changes after the latest anchor are checked against the local chain only; the report states the unanchored range. A project with no forge remote gets local verification only, and `doctor` says so.

The chain is per project, so one project's ledger can be exported and verified without the others (EVD-R15). Appending takes the database's single write lock, so a chain never forks.

#### Views and projectors (EVD-R9, EVD-R10, EVD-R27)

A projector is registered for the event types it cares about. For each appended event, the adapter loads the documents the projector names by key, calls `apply`, and writes the returned changes. Every view document records the project sequence and projector version that produced it.

**Authority.** Views are fast, derived data. A decision that grants authority (admission, completion, landing, release) confirms its deciding facts against the events themselves inside its transaction, through `Transaction::event_exists`, so an edited view cannot grant authority.

**Rebuild.** A view is rebuilt per project into a shadow table, in batches of short transactions, each replaying a bounded range of events. When the shadow reaches the project head, one short transaction applies the remaining events and swaps the shadow in. A crash discards the shadow. Progress is tracked per view per project. Writers are never blocked for longer than one batch.

**Versions.** A view whose stored projector version is older than the running binary's is rebuilt forward before it is used. A binary never rebuilds a view whose stored version is newer than its own; it treats that project as read-only.

**Verification of views.** `baley verify --views` rebuilds every view of a project into scratch tables and compares them with the live ones, reporting any document that differs.

Views planned for the first build, by the question they answer. Query contracts (keys, indexes, ordering, page bounds) are in [Appendix B](#appendix-b-reads-mapped-to-views).

| View | Key | Indexed by | Answers |
|---|---|---|---|
| `roadmap` | project | | The ordered phases and their declared requirements |
| `phase` | (project, phase) | status | Status, context, completion and whether it still applies |
| `plan` | (project, phase, plan) | status | Current content reference, approval binding, readiness |
| `evidence_map` | (project, phase, plan) | | The acceptance evidence a plan must produce |
| `admission` | (project, phase) | checkout | What execution may touch, and in which checkout |
| `dispatch` | (project, dispatch id) | phase, state | Active and ended dispatches, task and suite outcomes |
| `run` | (project, run id) | dispatch, phase | One run: launch, result, output reference |
| `verification` | (project, attempt id) | phase, state | Attempts, runs, claims, waivers, completion |
| `review` | (project, review id) | phase, state | Review attempts and their outcomes |
| `review_queue` | (project, item) | phase, state | Deferred reviews awaiting adjudication |
| `risk` | (project, phase) | | Observations and receipts |
| `milestone` | (project, name) | state | Close, archive, release and landing state |
| `pause` | project | | The active pause and its resume bindings |
| `policy` | (project, checkout) | | Effective policy and its layers |
| `guard_policy` | project | | The remembered denial policy |
| `capture` | (project, item id) | phase, disposition | The capture queue |
| `request` | (project, command kind, request id) | | Outcome of each command, for retries |

Hardin reads only views, and confirms authority against events. The next-action rules, progress and gate checks become queries over `phase`, `plan`, `dispatch`, `verification`, `review_queue`, `pause` and `capture`, not a walk over the whole store.

#### Payloads, retention and purge (EVD-R11, EVD-R14)

Content is stored as a payload when it is larger than 4 KiB, or when it is of a sensitive kind at any size: command output, review material, prompts. Payloads are compressed with zstd and keyed by the SHA-256 of the uncompressed bytes, so storing the same content twice stores it once. SQLite's own measurements put the crossover at about 100 KB: smaller content reads faster inside the database, larger content faster from files. The `Payloads` trait hides where bodies live, so an adapter can move large bodies to a content-addressed directory later without the ledger changing.

**Retention is per reference.** Each event that uses a payload records a reference with its own retention class:

| Class | Examples | Default retention |
|---|---|---|
| `record` | plan and context text, verdict detail | Kept for the life of the project |
| `output` | test and command output | Kept until the milestone that produced it closes, then reduced |
| `material` | review material, prompts sent to models | Kept for 90 days after its review closes |

These are the defaults. A project can change any of them in `baley.toml`, and `baley purge` removes a body at once regardless of class.

A body is deleted only when no remaining reference still requires it. A purge in one project removes only that project's references and is recorded in that project's chain; a body shared with another project survives until that project's references also expire.

**Reduction** does not alter a payload. It stores the kept excerpt (the first and last 64 KiB) as a new payload with its own hash, records `payload.reduced` naming the original hash, the excerpt hash and the byte ranges kept, and tombstones the original body. Verification checks the excerpt in full and the original as a commitment.

**Purge** removes, in one transaction, the body and every derived copy Baley manages: search entries, view fields that quoted it, saved request answers and trace rows. The database runs with `secure_delete` on, and a purge ends with a checkpoint, `VACUUM`, and a final `wal_checkpoint(TRUNCATE)`: in write-ahead-log mode `VACUUM` writes every page through the log, so only the last truncating checkpoint leaves no copy behind. Backups in Baley's home receive the same purge; exports outside it cannot be reached and are listed in the purge report. A purge records `payload.purged` naming the hashes, the policy or reason and the actor, in every affected project's chain.

Purge removes a secret from everything Baley manages. A secret that has already reached a review provider, an export or any other system must still be rotated; the purge report says so.

**Invariant (EVD-R10).** Every fact a projector or the search index needs is inline in the event. Payloads are attachments only. So a purge never changes what a view knows, only whether an attachment's body can be shown, and a rebuild after any purge yields the same views, with tombstones in place of purged attachments.

```mermaid
stateDiagram-v2
  [*] --> Stored : first reference
  Stored --> Stored : another reference to the same hash
  Stored --> Reduced : output retention ends, excerpt stored as a new payload
  Stored --> Purged : no reference still requires it, or owner purges
  Reduced --> Purged : owner purges
  Purged --> [*]
  note right of Purged
    Hash, length and references remain.
    The chain still verifies.
  end note
```

*Figure 7. Payload lifecycle.*

#### Physical schema (SQLite adapter)

```mermaid
erDiagram
  PROJECT ||--o{ EVENT : "records"
  PROJECT ||--o{ CHECKOUT : "is seen at"
  PROJECT ||--o{ ANCHOR : "is anchored by"
  EVENT ||--o{ PAYLOAD_REF : "uses"
  PAYLOAD ||--o{ PAYLOAD_REF : "is used by"
  PROJECT ||--o{ VIEW_DOC : "has current state"
  PROJECT ||--o{ SEARCH_ENTRY : "is searchable by"
  PROJECT {
    text project_id PK
    text name
    text created_at
    integer head_seq
    blob head_hash
  }
  CHECKOUT {
    text project_id FK
    text path
    text root_commit
    text remote_url
    text last_seen
  }
  ANCHOR {
    text project_id PK
    integer seq PK
    blob head_hash
    text tag
    text pushed_at
  }
  EVENT {
    text project_id PK
    integer seq PK
    text stream
    integer stream_version
    text type
    integer type_version
    text actor
    text recorded_at
    text request_id
    text git_commit
    text git_tree
    text git_checkout
    integer policy_version
    text payload_json
    blob prev_hash
    blob hash
  }
  PAYLOAD {
    blob hash PK
    integer bytes
    text encoding
    blob body
    text state
  }
  PAYLOAD_REF {
    text project_id PK
    integer seq PK
    blob hash PK
    text class
    text expires_at
  }
  VIEW_DOC {
    text view PK
    text project_id PK
    text doc_key PK
    integer produced_seq
    integer projector_version
    text doc_json
  }
  SEARCH_ENTRY {
    text project_id
    text phase
    integer seq
    text body
  }
```

*Figure 8. Tables of the SQLite adapter. `VIEW_DOC` stands for one table per view, each with its declared key and index columns. `SEARCH_ENTRY` is an FTS5 virtual table. The `request` view is one of the views.*

Further tables: `schema_meta` (compatibility epoch, created and migrated times), `view_meta` (per view per project: projector version, applied sequence, shadow state), and `trace` (diagnostics, outside the chain, size-capped).

Indexes: `event(project_id, stream, stream_version)` unique; `event(project_id, type, seq)`; `event(project_id, git_commit)` for `why`; `payload_ref(hash)`; each view's declared indexes.

Connection settings: `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`, `secure_delete=ON`, `busy_timeout=5000`, page size 8 KiB. rusqlite's bundled build compiles SQLite from source (3.53.2 with rusqlite 0.40.1) with FTS5 enabled, so no system SQLite is used.

#### Location, layout and file safety (EVD-R16, EVD-R22, EVD-R23)

The home directory is `BALEY_HOME` if set, otherwise the platform data directory: `$XDG_DATA_HOME/baley`, falling back to `~/.local/share/baley`, on Linux; `~/Library/Application Support/baley` on macOS.

```
<home>/
  baley.db          the ledger database
  baley.db-wal      SQLite write-ahead log (managed by SQLite)
  baley.db-shm      SQLite shared memory index (managed by SQLite)
  backups/          automatic backups before migrations, and scheduled backups
```

On every open, Baley resolves the real path of the database and checks: the home and database are owned by the current user; the home is mode 0700 and the files 0600, and anything more permissive is refused with the fix named; neither the home nor the database is a symbolic link; and the filesystem holding the real database path is local. Backups and exports are created private. User configuration lives in the platform configuration directory (`$XDG_CONFIG_HOME/baley`, `~/.config/baley` on Linux; `~/Library/Application Support/baley/config` on macOS).

Development builds and tests set `BALEY_HOME` so they never touch the owner's real ledger.

#### Project identity and policy (EVD-R17)

A project is initialized once with `baley init`. That creates the project in the ledger with a new random project id (UUID version 4) and writes the project file, `baley.toml`, at the repository root, which the owner commits. TOML allows comments, which a hand-edited policy file needs. The file holds the project id, the project name, and the project's policy: reviewers, routing and protected branches, the settings that today live in the repository config.

Baley finds a checkout's project the way git finds a repository: it walks up from the working directory to the first directory holding the project file, stopping at the repository root. The guard uses the same discovery. A directory with no project file is not managed and the guard stays silent. Every checkout Baley sees is recorded with `checkout.seen` (path, root commit, remote URL) for diagnosis only. If two checkouts whose remotes differ claim the same project id (a fork cloned beside its upstream), Baley refuses to record for the second and tells the owner to give it its own id with `baley init --new-id`.

**Effective policy.** Policy has layers, as today: built-in defaults, the owner's user configuration, and the project file, merged with today's precedence, and with settings that today are allowed only in the global file allowed only in the user configuration. Whenever the merged result changes, for any layer, Baley records `policy.effective` with the full merged policy and the layer each value came from. Every command records the policy version it ran under. Each checkout runs under the project file committed at its own HEAD; if two checkouts of one project run under different policies, Hardin reports the divergence and names both. The routing-admission rule is kept: a dispatch whose routing inputs changed since admission is refused.

Agents may not edit the project file; the guard refuses writes to it, as it refuses writes to the repository config today.

#### What replaces the Markdown and JSON files (EVD-R18, EVD-R25)

| Today | Replacement |
|---|---|
| `ROADMAP.md` phase list, read by every status query | `roadmap` stream and view. New commands declare, rename and reorder phases, which also gives the missing "add a phase" operation. |
| `REQUIREMENTS.md` traceability rows | `requirement.declared` events and the `roadmap` view. Completion and undo record events instead of editing rows. |
| `PROJECT.md` milestone version | The `milestone` view. |
| `phases/<n>/CONTEXT.md` | `context.approved` event; text is a `record` payload. |
| `phases/<n>/PLAN-k.md`, parsed by execution | `plan.approved` event carrying the approval binding; execution reads the typed plan from the `plan` view, never parses Markdown. |
| `phases/<n>/SUMMARY.md` | A query over the `dispatch` and `run` views. Git source accounting no longer needs an exemption for Baley's own files, because Baley writes none. |
| `phases/<n>/UAT.md` | `human.result` events. |
| `DEFERRED-*.json`, `ADJUDICATION-*.json` | `review.deferred` and `review.adjudicated` events; the `review_queue` view feeds next-action with today's precedence. |
| task, spike and debug Markdown | Their own streams and views. |
| `why` and `recall` reading Markdown from git history | `why` joins commits to the events that name them (`event(project_id, git_commit)`); `recall` uses the `Search` capability. Nothing is lost when a milestone is archived, because nothing is deleted. |
| Pause committing store files | `pause.recorded` carries today's resume bindings (preserved HEAD, branch, policy version, occurrence, next step); `pause.resumed` records a resume that passed those checks. Pause still commits the owner's work in progress; it never commits Baley's records. |
| Milestone prune deleting phase directories | `milestone.archived`, a separate owner-approved step bound to one exact `milestone.close_ready`, as prune is today. Archived phases leave the active roadmap; nothing is deleted. |

The owner reads records with `baley show <thing>` (for example `baley show plan 5-2`) and can write any record to a Markdown file with `baley export <thing> --to <path>`. Agents read them through the existing MCP `document` query, which is served from views. No file Baley exports is ever read back.

#### Working-tree writes

Baley's records never live in the working tree (EVD-R18). The only writes Baley makes there are:

- the project file, at `baley init` and when the owner changes policy through Baley;
- exports, to a path the owner names;
- the git operations that belong to commands: undo's revert, the release manifest bump, pause's work-in-progress commit, and landing.

#### Processes and concurrency (EVD-R8, EVD-R19)

Each process opens its own connection. SQLite's write-ahead log lets any number of readers run alongside one writer, and readers see a consistent snapshot.

- **MCP server.** One per host session. It keeps one write connection behind a single writer task, so its own commands queue in order, and a small pool of read connections.
- **Guard hook.** Starts per tool call, opens a connection without an integrity scan, reads the views it needs and, for a decision worth recording, appends one `guard` event. It holds no write transaction while it evaluates.
- **CLI.** Opens connections on demand.

**Writer queue.** Before `BEGIN IMMEDIATE`, every writer takes a blocking exclusive lock on `<home>/baley.db.writer`. The kernel parks waiting writers and wakes one each time the lock is released, so writers take turns instead of polling; SQLite's own busy handler sleeps and retries, and under sustained load it starved writers for seconds (see [Performance](#performance)). Write transactions then start with `BEGIN IMMEDIATE`, so a writer takes the database lock before reading and two writers never deadlock on an upgrade. `busy_timeout` stays at 5 seconds as a backstop, after which a writer fails with a clear "store busy" error. Because every slow step happens before `transact`, a write transaction holds the lock for milliseconds.

**Compatibility epoch.** Every write transaction reads the compatibility epoch from `schema_meta` before anything else. A process whose epoch is older than the stored one stops writing, answers read-only, and tells the caller which binary is needed. Migrations raise the epoch in the same transaction that changes the schema, so an older process that is already running is fenced at its next write.

**Checkpoints.** SQLite folds the write-ahead log into the database automatically every 1,000 pages. A reader that never finishes would stop that and let the log grow without bound; Baley's reads are short-lived by construction, and the server runs a passive checkpoint when idle. `baley doctor` reports the log size.

#### Opening the store

```mermaid
stateDiagram-v2
  [*] --> Locating
  Locating --> Refused : unsafe owner, modes or links, or a network filesystem
  Locating --> Opening
  Opening --> Creating : no database
  Creating --> Ready
  Opening --> ReadOnly : epoch newer than this binary
  Opening --> BackingUp : epoch older than this binary
  BackingUp --> Migrating
  Migrating --> Ready : migration committed, epoch raised
  Migrating --> Refused : migration failed, database unchanged
  Opening --> Checking : epoch current, server start only
  Checking --> Ready
  Checking --> ReadOnly : quick_check failed
  Opening --> Ready : epoch current, guard or CLI
  Ready --> Reconciling : interrupted claims found
  Reconciling --> Ready
  Refused --> [*]
```

*Figure 9. Opening the store. A binary never writes to an epoch it does not understand, a failed migration leaves the database as it was, and interrupted claims are reconciled before work in their scope continues.*

The MCP server runs SQLite's `quick_check` when it starts. The guard and the CLI do not, so a guard call never scans the database. The full `integrity_check`, chain and view verification run in `baley doctor` and before every backup.

### Workflows

#### From an approved plan to a verified phase

```mermaid
sequenceDiagram
  actor O as Owner
  participant D as Daneels (host)
  participant B as Baley (Hardin decides)
  participant L as Ledger
  O->>B: approve plan 5-2
  B->>L: plan.approved (exact submission, owner, time)
  D->>B: ask to execute
  B->>L: re-read plan, context, admission, confirm against events
  alt evidence supports execution
    B->>L: execution.admitted (for this checkout)
    B-->>D: allowed, dispatch issued
  else proof missing
    B-->>D: refused, naming the missing proof
  end
  loop each task
    D->>B: record task closed and suite run, with outputs
    B->>L: task.closed, suite.run
  end
  D->>B: ask to verify
  B->>L: read dispatch and evidence views
  alt all checks closed and proven
    B-->>D: allowed
    D->>B: record verification run and verdict
    B->>L: verification.run, verdict.claimed
    O->>B: accept verification
    B->>L: verification.completed, then anchor pushed
  else a check is open or unproven
    B-->>D: refused, naming the missing proof
  end
```

*Figure 10. One column per actor. Nobody but Baley writes to the ledger; each step's fact is recorded before Hardin allows the next one, a refusal names the proof that is missing, and a verified phase pushes an anchor.*

#### A retried tool call

A host may deliver the same tool call twice after a timeout. The second delivery carries the same request id, finds the stored outcome at step 6 of Figure 6, and returns it. For a command with an external effect, the retry finds the claim and waits for, or returns, its result. Nothing is recorded twice and no effect runs twice.

#### Two sessions writing at once

Two sessions on different projects, or two agents on one project, each hold the write lock only for their commit. The second waits milliseconds at `BEGIN IMMEDIATE`. If both decided on inputs the first one changed, the second's `decide` re-reads them inside its transaction, sees the change, and refuses as stale; its caller re-reads and decides again.

## Rules preserved

Every rule below keeps its current behaviour (EVD-R28). Each gets an equivalence test that exercises today's case against the new design.

| Rule | Today | In the ledger |
|---|---|---|
| A plan's approval binds its exact submitted content, the owner and the time | `plan/persistence.rs:135` | `plan.approved` carries the submission digest, owner and time; admission confirms it against the event |
| Plan replay is scoped to its phase occurrence | `plan/persistence.rs:165` | Request scope (project, command kind) plus the phase in the digest |
| Completion binds context, publications, admissions and task and plan history, and stops applying when any of them changes or execution is undone | `verification/completion.rs:80-125` | The `phase` view computes applicability from the bound facts; `completion.invalidated` is projected when a bound fact changes; completion is confirmed against events when used |
| Verification records its launch before running | `verification/runner.rs:144-164` | Claim, act, record |
| Verification claims are recomputed at commit | `store/writer.rs:1196` | Inside `decide` |
| Undo records a pending state before its first revert and refuses to continue until an interrupted revert is reconciled | `undo_service.rs:108-112` | Claim, act, record, and reconciliation |
| Undo keeps its refusal receipts | `undo_service.rs:18` | `command.completed` for refusals |
| Landing records an intent per step and requires reconciliation | `landing_service.rs:245-287` | Claim, act, record per landing step |
| Release requires an exact unstarted landing and a retained confirmation | `milestone/release.rs:122-184` | `release.proposed` and `release.confirmed` as separate owner steps |
| Milestone close records readiness; prune is a later step bound to that exact close | `milestone_service.rs:113`, `milestone/prune.rs:94` | `milestone.close_ready`, then `milestone.archived` bound to it |
| Resume checks the preserved HEAD, branch, configuration and occurrence | `pause_service.rs:458-490` | `pause.recorded` bindings, checked by `pause.resumed` |
| Deferred reviews feed next-action with their precedence | `next_action/observations.rs:125`, `next_action/select.rs:93` | `review_queue` view |
| Config layers merge with repo over global, and some settings are global-only | `config/merge.rs:131`, `config/mod.rs:14` | `policy.effective` with the same precedence and scope rules |
| A dispatch whose routing inputs changed is refused | `config/reload.rs:190` | Kept; routing inputs are part of the policy version bound at admission |
| The guard falls back to its remembered denial policy when current config is missing or malformed; it remembers denials only, never permissions | `guard/bash.rs:169`, `guard/bash.rs:375`, `guard/audit.rs:165` | `guard.policy_recorded` events and the `guard_policy` view |
| The guard answers a re-delivered call with its confirmed answer, even after policy changes | `guard/bash.rs:486` | Request lookup on the guard's session and tool-call id |
| Guard outcomes: ask, deny, pass, pass on failure, redelivery, audit-storage failure | `guard/bash.rs`, `guard/audit.rs` | Same outcomes, recorded as `guard` events; storage failure keeps today's behaviour |
| One run can be read by its id | `execution_runner_service.rs:93` | `run` view |

## Cross-cutting concerns

### Security

See [Threat model](#threat-model) for who is defended against.

- **Agent isolation (EVD-R24).** Agents run as the owner's user, so file modes cannot keep them out. The barrier is the host's sandbox: Baley's setup adds a rule to each host's configuration that denies agents read and write access to Baley's home (Codex's `workspace-write` sandbox already confines agent writes to the workspace; Claude Code's sandbox denies named paths). The host matrix proves on both hosts that agents are blocked while Baley's server and hook still write. As a second, best-effort layer, the guard refuses file-tool writes into the home and shell commands that name it.
- **Detection.** Anchors on the forge make a rewrite, truncation or rollback detectable up to the latest anchor, even by an agent that can run `sqlite3` and sign with the owner's key.
- **Secrets.** Test output and review material can carry secrets. They are always payloads, so they can always be purged, with the limits stated in [Payloads, retention and purge](#payloads-retention-and-purge-evd-r11-evd-r14). The existing fence that screens outbound review payloads stays.
- **Network.** The store makes no network calls. Anchors are pushed with git to the project's own remote.

### Failure modes and recovery

| Failure | Detection | What the user sees | Recovery |
|---|---|---|---|
| Process killed in a database-only command | SQLite rolls back the uncommitted transaction | The command did not happen | Retry with the same request id |
| Process killed after a claim, during or after the external effect | The claim's lease expires | Commands in the claim's scope wait for reconciliation, naming the claim; other work continues | Reconcile against the real state; `command.reconciled` records it |
| Power loss after acknowledgement | None needed | Nothing is lost (`synchronous=FULL`) | None |
| Store busy longer than 5 s | `SQLITE_BUSY` after the timeout | "store busy" | Retry; `doctor` shows long-running holders |
| Disk full | `SQLITE_FULL` | The command is refused, nothing recorded | Free space; retry |
| Database corruption | `quick_check` at server start, `integrity_check` in doctor | Read-only, with the check's report | Restore the latest verified backup |
| Chain differs from its anchor | `baley verify` | The first bad sequence, truncation or rollback, and the unanchored range | Restore from backup; the difference is itself evidence |
| A view differs from its rebuild | `baley verify --views` | The documents that differ | Rebuild the view; authority was never granted from it |
| Older process after an upgrade | Epoch check in its next write | Read-only, naming the needed binary | Restart with the new binary |
| Migration fails | Transaction error | Refused; database unchanged; backup kept | Report the bug; the old binary still works |
| Unsafe home or a network filesystem | Checks on open | Refused with the reason and the fix | Fix modes or ownership, or set `BALEY_HOME` to a local path |
| Anchor push fails cleanly (remote unreachable or refused) | `command.completed` with the failure | Nothing blocks; `doctor` warns once the unanchored range is over a day old | Retried at the next anchor point |
| Write-ahead log grows | Log size in doctor | Warning in doctor | Idle checkpoint; find the long reader |

### Performance

Budgets on the reference workload, and what the benchmark measured (p99 unless stated):

| Operation | Budget | Measured |
|---|---|---|
| Open a connection with the ownership, link, filesystem and epoch checks (guard, CLI) | 10 ms | 0.27 ms |
| Server start with `quick_check` (five projects, 105 MB) | 2 s | 150 ms |
| Commit a command of up to 10 events, excluding the command's own work | 10 ms | 6.9 ms (reference load), 5.5 ms (8 writers) |
| Get one view document by key | 2 ms | under 0.01 ms |
| Guard hook, store work only, in a new process | 25 ms | 6.6 ms (9.2 ms including process start) |
| Longest wait for the write lock with 8 sessions writing continuously | 250 ms | 23.6 ms p99, 34 ms max, no errors |
| Rebuild every view of the reference project, without blocking writers longer than one batch | 30 s total, 50 ms per batch | 0.14 s total, 4.1 ms largest batch |

Size budget: the reference workload stores in at most 50 MB, counting the database, its write-ahead log after a checkpoint, and every payload body under default retention, with retired prompt text included. **Measured: 21.5 MB**, against 175 MB for the same history in the JSON store: 5.3 MB of event JSON, 4.2 MB of compressed payload bodies (41.3 MB before compression), and the rest indexes, views and the search index. Memory: no operation loads more than its answer; the server's resident memory does not grow with the store. The prototype has no long-running server, so this is verified during the build, not by the benchmark.

**How it was measured.** A prototype of the SQLite adapter in [`spikes/evidence-ledger-bench`](../../spikes/evidence-ledger-bench/README.md) replays a numbers-only profile of the Cadence 4.0 build (1,551 commands, 1,615 events, 618 attachments with their sizes, retention classes and identities) as synthetic content tuned to each class's measured compression ratio, for the reference project alone and for five projects. It records the hash chain, payloads and references, projector-written views, the request view and the search index, with the connection settings above, and adds one guarded git command per three task commands, since the old store kept none. Results are in [`results/2026-09-25-ryzen-9800x3d-t700-btrfs.json`](../../spikes/evidence-ledger-bench/results/2026-09-25-ryzen-9800x3d-t700-btrfs.json): AMD Ryzen 7 9800X3D, Crucial T700 NVMe, btrfs, Linux 7.2, SQLite 3.53.2. A slower disk raises commit and lock-wait times roughly in proportion to its `fsync` time; the budgets leave room for a disk several times slower.

**What the benchmark changed.** With SQLite's busy handler alone, 8 continuous writers starved each other: p99 wait 429 ms, one writer waiting 5 s and failing, commands per writer ranging from 204 to 729. Hence the writer queue in [Processes and concurrency](#processes-and-concurrency-evd-r8-evd-r19), which brought the same run to a 21 ms p99 wait and 540 or 541 commands per writer. It also showed that `VACUUM` in write-ahead-log mode leaves a full copy of the database in the log (117 MB after the purge run), hence the final truncating checkpoint in the purge sequence.

SQLite's limits sit far beyond these numbers: 281 TB per database and about 1 GB per stored value.

### Observability

- `baley doctor` reports the compatibility epoch, `integrity_check`, chain verification against the latest anchor per project, view verification, write-ahead log size, database size by record family and retention class, view versions and lag, active and interrupted claims, the age of the unanchored range, and backups present.
- The `trace` table records diagnostics (timings, retries, busy waits) outside the chain, with its own size cap and rotation.
- Every refusal carries a stable code and the facts that caused it, as refusals do today.

### Host neutrality (EVD-R24)

| Concern | Claude Code | Codex | Evidence required before acceptance |
|---|---|---|---|
| Project discovery from the working directory | Hook and MCP server start in the project | Same | Discovery works from subdirectories on both |
| Hook contract (tool names, event names, answer format) | `PreToolUse`, `Bash`, `Write`, `Edit` | Codex's own hook events and tool names | A guard adapter per host; both refuse the same unsafe actions |
| Re-delivered tool calls | Retries after timeout | Retries after timeout | The retry returns the original answer on both |
| Access to Baley's home | Server and hook write; agents denied by sandbox rule | Server and hook write; agents confined by `workspace-write` | Shown on both, with an agent attempt refused |
| Store unreachable | Guard applies today's rules for a missing or failed audit store | Same | The same outcome shown on both |

### Compatibility and migration

Nothing is migrated from the Cadence store; Baley starts empty. Ledgers written while the migration is under way are disposable, and the builds in between are development builds: nobody else uses Baley yet. No data rollback is promised; each slice can be reverted as code.

Families are moved by what is written together, not one at a time:

| Family | Written in the same transaction as | Read together with |
|---|---|---|
| Roadmap, requirements | Plan publication (seeds requirements), completion (ticks phase and requirements), undo | Progress, next action, audit |
| Context | Plan publication (validates against it) | Admission, verification, review selection |
| Plans, evidence maps | Requirements | Admission, dispatch, verification |
| Admission, execution, dispatch, runs | Native summaries, evidence | Verification, progress, undo |
| Native evidence | Task checkpoints | Next action, execution |
| Verification | Completion edits roadmap and requirements | Progress, milestone |
| Review | Review families only | Selection reads plans and context |
| Risk | Risk families only | Suggest, milestone preflight |
| Milestones, landing, undo | Undo edits roadmap and requirements | Progress |
| Captures, config and routing, task, debug, spike, guard | Their own families | Their own readers |

Slices:

1. **Foundation.** The workspace split, the port, the SQLite adapter, the conformance suite, payloads and references, the hash chain, anchors, `verify`, `doctor`, `backup`. Nothing uses it yet.
2. **Identity, location and policy.** The home directory and its checks, `baley init`, the project file, discovery for the server and the guard, `policy.effective`, host sandbox rules.
3. **Standalone families.** Captures; guard; task, debug and spike.
4. **The lifecycle slice.** Roadmap, requirements, context, plans, evidence maps, admission, execution, dispatch, runs, native evidence and verification move together, built as a series of pull requests on one branch and merged when the whole slice works.
5. **Review and risk.**
6. **Milestones, landing, undo and pause.**
7. **Hardin on views.** Progress, next action and the gates read only views.
8. **Search and why.**
9. **Removal.** The JSON store, the intent journal, participants, root binding, the Markdown renderers and parsers, and every `.planning` path.

## Alternatives considered

**Keep the JSON documents.** Rejected. The copies and the machinery to reconcile them are the problem, and a single document cannot answer a keyed query without reading the whole store (Context).

**SQLite as plain state tables, without a ledger.** Simpler: update rows in place. Rejected because it keeps only the present. Baley's product is the proof of what happened; overwriting rows discards it, and an audit trail bolted on afterwards becomes a second copy again.

**Git as the database**, as git-bug and git-appraise do. Attractive: content addressing, signing and transport come built in. Rejected for now because every query still needs a local index (both projects keep one), concurrent writers must serialize on reference updates, and query latency on large histories is poor. Git is used for what it does best here: holding the immutable anchors.

**An embedded key-value store** (redb, sled). Fast and pure Rust, but no query language, no full-text search, and every index would be hand-built, which is the mistake the current store made with JSON.

**A database server** (PostgreSQL). Rejected for a single-user tool: a daemon to install, run and upgrade, for concurrency Baley does not need. The port keeps it possible as a future adapter for teams.

**One database file per project.** It contains the blast radius and makes per-project export trivial. Rejected in favour of one database per user because cross-project questions become simple queries and there is one schema to migrate and one file to back up; per-project chains and `export` give the same per-project verifiability. See ADR 0003.

**The database inside `.git/`.** Shared by all worktrees and never committed, but deleted with the clone. Rejected because the ledger is the historical record and must outlive any checkout. See ADR 0003.

**Filesystem identity for projects** (today's inode binding). Rejected: the store could not move, be restored or be rebuilt. See ADR 0004.

**A chain anchored only inside the database.** Rejected: the attacker Baley exists to stop, an agent running as the owner, can recompute it. See ADR 0007.

**Tracking every read's version instead of deciding inside the transaction.** Correct in principle, but a single missed read is a silent bug. Deciding inside the single-writer transaction is correct by construction.

**A separate operating-system user for Baley.** Real separation, but it needs a daemon, a privileged install step and different setups on Linux and macOS. Kept as future work. See ADR 0008.

## Testing

| Requirement | How it is proven |
|---|---|
| EVD-R2, R3 | Conformance tests append events, verify the chain, then in a copy of the database in a temporary directory: alter, insert, delete and reorder rows; recompute the whole chain after an edit; truncate the tail; restore an older copy. Against a local anchor fixture, `verify` names the first bad position, the truncation or the rollback. |
| EVD-R5 | A conformance test fails a projector mid-command and asserts no event, view change, payload, reference or outcome was recorded. |
| EVD-R6 | Replay of a request id with the same digest returns the original outcome; with a different digest it is refused; two sessions using the same id for different command kinds do not collide. |
| EVD-R7 | A command whose view input, absence query or HEAD changes between its slow work and its transaction is refused as stale. |
| EVD-R8 | Several connections to one database in a temporary directory interleave reads and writes. |
| EVD-R9 | Each view's query contract (key, index, ordering, paging, bound) is tested against a fixture. |
| EVD-R10 | Each projector has unit tests: event and documents in, changes out. A conformance test rebuilds every view, including through a simulated crash mid-rebuild, and compares it with the live one; the same after a policy purge and after a targeted purge. |
| EVD-R11, R14 | Identical content is stored once; the body decompresses to the original bytes and matches its hash; reduction stores an excerpt with its own hash; a shared body survives one project's purge; a purge removes search entries, view fields, request answers and trace rows. |
| EVD-R12 | The crate graph is the test: `baley-core` has no path to rusqlite, checked by `cargo tree` in CI. |
| EVD-R13 | Search returns hits in relevance order with stable ties, scoped by project and phase, over a fixture corpus. |
| EVD-R15 | An exported project verifies alone. |
| EVD-R16, R17, R22, R23 | Location resolution, project discovery and the open checks run against directory trees built in a temporary directory with `BALEY_HOME` set: wrong owner, permissive modes, a symbolic-linked home or database, and a filesystem classified as networked. |
| EVD-R18 | Each command's test asserts it writes nothing in the working tree beyond the named exceptions. |
| EVD-R19 | Opening a database stamped with a newer epoch yields read-only; a second connection at the old epoch is fenced at its next write; a migration test upgrades a fixture of the previous schema and verifies a backup was taken first; a newer view is never rebuilt backward. |
| EVD-R20 | Relies on SQLite's documented durability with `synchronous=FULL`. Power loss is not reproducible in a portable test and is not re-tested. |
| EVD-R21 | The benchmark harness measures every budget on the reference workload. Timings are measured, not asserted in tests, because timing is not portable. |
| EVD-R24 | The host matrix, run by hand on both hosts before acceptance, and again before each release. |
| EVD-R25 | `show` and `export` render every record family from views. |
| EVD-R26 | While a claim is active, commands outside its scope proceed and commands inside it wait; a retry during the effect never repeats it. After the owner is killed and the lease expires, a command in scope triggers reconciliation; an anchor claim reconciles automatically from the remote; a revert claim waits for the owner. A cleanly failed anchor push completes the claim and blocks nothing. |
| EVD-R27 | An edited view document that says "approved" or "complete" does not grant admission or completion, because the event is missing. |
| EVD-R28 | One equivalence test per row of [Rules preserved](#rules-preserved). |

The conformance suite lives in `baley-store` and runs against every adapter.

## Decisions

- [ADR 0001: Record evidence as an append-only, hash-chained event ledger](../adr/0001-event-ledger.md)
- [ADR 0002: Use SQLite as the storage engine](../adr/0002-sqlite.md)
- [ADR 0003: Keep one ledger database per user, outside any checkout](../adr/0003-per-user-database.md)
- [ADR 0004: Identify projects by a committed project file](../adr/0004-project-identity.md)
- [ADR 0005: Put storage behind a port with engine adapters](../adr/0005-storage-port.md)
- [ADR 0006: Keep every operational record in the ledger](../adr/0006-no-markdown-records.md)
- [ADR 0007: Anchor chain heads on the forge](../adr/0007-forge-anchors.md)
- [ADR 0008: Use host sandboxes to keep agents out of the ledger](../adr/0008-host-sandbox-isolation.md)

## Future work

- **Records that travel.** Push a project's chain to a git ref so another machine can fetch and verify it.
- **Signed checkpoints** with a key the agent cannot use, for example a hardware key that needs a touch.
- **A separate operating-system user** for the store.
- **A server adapter** for teams sharing one ledger.

## Open questions

1. **Host matrix.** Run every row of [Host neutrality](#host-neutrality-evd-r24) on both hosts. Must be answered before acceptance.

## Appendix A: Mapping from the current store

| Current namespace or file | Becomes |
|---|---|
| `context` | `phase/<n>` stream, `context.approved`; `phase` view |
| `plan_publications` (publications, receipts) | `plan/<n>-<k>` stream; `plan` view; receipts replaced by `command.completed` and the `request` view |
| `acceptance_maps` | `plan.approved` payload; `evidence_map` view |
| `native_admissions` | `admission/<n>` stream; `admission` view |
| `execution` (occurrences, active, issues) | `phase/<n>` and `dispatch/<id>` streams; `dispatch` view |
| `native_tasks`, `native_plans` | Task and suite events on `dispatch/<id>`; `run` view; outputs become `output` payloads |
| `native_execution_summaries` | Dropped: the summary is a query |
| `native_execution_material` | `task.closed` payload |
| `rail_execution_material` | Dropped: written only by the legacy executor-patch path |
| `worker_exits`, `worker_interruptions` | `worker.exited` and `worker.interrupted` events |
| `verification` (attempts, runs, claims, patches, waivers, humans, completions) | `verification/<id>` stream; `verification` view. The copy of execution records inside each attempt is replaced by the sequence range it observed |
| `native_evidence` | Evidence events on the stream they concern; order comes from the project sequence, not a log scan |
| `review` | `review/<id>` stream; `review` and `review_queue` views; retained material becomes `material` payloads |
| `rail_observations`, `rail_receipts` | `risk/<n>` stream; `risk` view |
| `milestones`, `milestone_prunes`, `milestone_releases`, `landings` | `milestone/<name>` stream with separate close, archive, release and landing steps; `milestone` view |
| `undos`, `undo_requests` | `phase.undone` events through claim, act, record; refusals through `command.completed` |
| `task`, `debug`, `spike` | Their own streams and views |
| `derivation.memo` | Dropped: the views are the derived state |
| `guard_audit` (initialization, `denial_policy`) | `guard` stream; `guard.policy_recorded` and the `guard_policy` view keep the remembered denial policy |
| `import`, `layers` | Dropped: no import; configuration history is `policy.effective` events |
| Envelope `operations`, `generation`, `integrity`, log digests | Replaced by `command.completed`, stream versions, the compatibility epoch and the hash chain |
| `decisions.jsonl` gate, routing and boundary lines | Replaced by the events themselves; nothing is mirrored |
| `items.jsonl` | `capture` stream; `capture` view |
| `trace.jsonl` | `trace` table |
| `phases/*/DEFERRED-*.json`, `ADJUDICATION-*.json` | `review.deferred`, `review.adjudicated`; `review_queue` view |
| `.planning/*.md`, `phases/**` | See [What replaces the Markdown and JSON files](#what-replaces-the-markdown-and-json-files-evd-r18-evd-r25) |

Found unused in the current code and not carried forward: the store operations `AdmitExecution`, `ApplyExecutionPatch` and `RecordExecutionRefusal`, and the helpers `installed_spikes`, `installed_debug` and `installed_tasks`.

## Appendix B: Reads mapped to views

Every read the MCP server serves today, and the view and key that serve it. All queries page with a cursor and are bounded; lists are ordered as stated.

| Query operation | View and key | Order |
|---|---|---|
| `progress` | `roadmap` (project); `phase` by status; `dispatch` by state; `capture` by disposition; `review_queue` by state; `pause` (project) | Roadmap order |
| `execute-next` | `phase`, `plan`, `admission`, `dispatch` for (project, phase); evidence events for the phase | |
| `verify-next` | `phase`, `plan`, `evidence_map`, `dispatch`, `verification` for (project, phase) | |
| `verification-read` | `verification` (project, attempt id), or by phase index | Newest first |
| `verification-audit` | `roadmap` requirements; `phase` and `verification` by phase | Roadmap order |
| `execution-history` | `dispatch` by phase; `run` (project, run id) with a payload stream for output | Sequence |
| `evidence-read` | `evidence_map` (project, phase, plan); evidence events for the plan | Sequence |
| `plan-read` | `plan` by phase; `evidence_map` | Plan number |
| `context-intake` | `phase` (project, phase); `roadmap` | |
| `document` | By identity: `phase` for phase context; `plan` for a phase plan; `dispatch` for a dispatch; `verification` for an attempt; `run` for run output; `review` for a review entry; `roadmap` for a roadmap row; `task`, `debug`, `spike` by slug | |
| `document-search` | `Search` scoped to (project, phase), returning identities and parts | Relevance |
| `recall` | `Search` scoped to the project, optionally a phase | Relevance |
| `why` | `event(project_id, git_commit)` index, then the events' streams | Sequence |
| `suggest` | `risk` by phase; routing and outcome events by type | Newest first |
| `risk-status` | `risk` (project, phase) | |
| `route`, `config-entry`, `config-facts`, `config-interview` | `policy` (project, checkout) | |
| `milestone-read` | `milestone` (project, name) | |
| `land-read` | `milestone` (project, name), landing steps | Step order |
| `undo-read` | `phase` undo history; `request` for refusals | Newest first |
| `debug-list`, `debug-status`, `debug-continue` | `debug` (project) and (project, slug) | Newest first |
| `help`, `schema`, `detect-surfaces` | No store read | |
