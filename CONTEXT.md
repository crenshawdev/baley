# Glossary

The words Baley's design uses, one term per concept. Each entry gives the term to use, what it means in a sentence or two with a link to the design section that owns it, and the words to avoid for it: synonyms that name the same thing, and words easily mistaken for it.

The terms come from the Terms sections of the design documents in [docs/design](docs/design/), the [product requirements](docs/design/prd/baley.md) and this repository's [README](README.md). When a design document changes a term, this page changes in the same pull request.

Two notes on history:

- **Phase** is the one term for the working increment ([ADR 0031](docs/adr/0031-one-term-per-concept.md)). Decision records written before that choice say sprint, for example [ADR 0017](docs/adr/0017-stories-and-sprints.md), which also names the setting `planning.sprint_capacity`, now `planning.phase_capacity`, and a sprint view, now `phase_plan`, which is separate from the `phase` view. Read sprint there as phase.
- **Story** is the one term for what the owner declares on the roadmap ([ADR 0031](docs/adr/0031-one-term-per-concept.md)), and its events are named for it: `story.declared`, `story.corrected`, `story.reassigned`, `story.reprioritized`, `story.dropped` and `story.refined`. Decision records written before that choice also say requirement for it, for example ADR 0017. Read requirement there as story, except where it names a numbered design requirement row such as `CFG-R12`, which keeps the word Requirement.

## The parties

| Term | Meaning | Avoid |
|---|---|---|
| Owner | The person who decides and answers for the work: approves plans, rules on findings, grants waivers and landing steps, sets policy ([0002 section 2](docs/design/0002-system-design.md#2-separation-of-concerns)). | user, customer, operator |
| Baley | The one Rust binary, run as one server per Claude Code session, that keeps the record and makes every process decision from it ([0002 section 2](docs/design/0002-system-design.md#2-separation-of-concerns)). | the framework, the agent |
| Model | The AI model that does the engineering judgment; it never decides process state, order, routing or whether proof is enough ([0002 section 2](docs/design/0002-system-design.md#2-separation-of-concerns)). | the AI, the assistant |
| Host | Claude Code, the program the owner works in, whose session starts workers as subagents and connects to Baley over MCP. A host is supported only when it meets the security bar ([0012](docs/design/0012-host-interface.md#2-terms), [ADR 0033](docs/adr/0033-host-security-bar.md)). | client, IDE, harness |
| Host session | The host's main conversation with the owner, in two jobs: relaying Baley's work orders, questions and answers, and adjudicating other models' output ([0002 section 2](docs/design/0002-system-design.md#2-separation-of-concerns)). | orchestrator, main thread, main agent |
| Session | One host conversation, with its own Baley server over stdio ([0012](docs/design/0012-host-interface.md#2-terms)). | chat, thread |
| Worker | A subagent the host session starts for one work order. It shares its session's connection ([0012](docs/design/0012-host-interface.md#2-terms)). | Daneel, Daneels, agent (as a Baley term) |
| Role | A kind of worker Baley dispatches: planner, analyzer, checker, executor, verifier, reviewer ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | persona, agent type |
| Hardin | The part of Baley that reads views and names the one allowed next step ([0001](docs/design/0001-evidence-ledger.md#terms)). | scheduler, state machine |
| Relay | The host session putting Baley's question to the owner and returning the answer unchanged ([0012](docs/design/0012-host-interface.md#2-terms)). | proxy, paraphrase |

## The ledger

| Term | Meaning | Avoid |
|---|---|---|
| Ledger | Baley's append-only, hash-chained record of a project's events, kept outside the repository ([0001](docs/design/0001-evidence-ledger.md#summary)). | database, log, store (the store is the storage crate and port) |
| Event | One recorded fact: immutable, typed, attributed, hash-chained ([0001](docs/design/0001-evidence-ledger.md#terms)). | entry, row, log line |
| Stream | The events of one thing that changes over time, named like `plan/5-2`, with its own version counter ([0001](docs/design/0001-evidence-ledger.md#terms)). | topic, channel, table |
| Project sequence | The position of an event in its project's ledger; the hash chain follows this order ([0001](docs/design/0001-evidence-ledger.md#terms)). | offset, index |
| Anchor | An immutable tag on the forge, `baley-anchor/<project>/<seq>`, carrying a copy of the project's chain head ([0001](docs/design/0001-evidence-ledger.md#terms), [0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | checkpoint, snapshot, backup |
| View | A keyed collection of documents computed from events, answering one kind of current-state question ([0001](docs/design/0001-evidence-ledger.md#terms)). | cache, table, report |
| Generation | One complete set of a project's view documents; a rebuild builds the next one beside the live one ([0001](docs/design/0001-evidence-ledger.md#terms)). | epoch, copy |
| View set version | The version a binary declares for its whole set of registered views, raised when a view is added, removed or renamed ([0001](docs/design/0001-evidence-ledger.md#terms)). | schema version |
| Projector | Pure code that updates one view from an event ([0001](docs/design/0001-evidence-ledger.md#terms)). | handler, reducer |
| Payload | Content stored once by hash, outside the event: large content and every sensitive kind of content ([0001](docs/design/0001-evidence-ledger.md#terms)). | blob, attachment |
| Reference | One event's use of a payload, carrying the retention class for that use ([0001](docs/design/0001-evidence-ledger.md#terms)). | pointer, link (a link is an evidence kind) |
| Command | One request from a caller that may record events; it carries a request id ([0001](docs/design/0001-evidence-ledger.md#terms)). | call, action |
| Claim | The intent event a command with an external effect records before it acts ([0001](docs/design/0001-evidence-ledger.md#terms)). | lock, reservation |
| Claim lease | A claim's renewal time outside the chain: liveness only, expiring 60 seconds after renewal ([0001](docs/design/0001-evidence-ledger.md#terms), where it is called Lease). | heartbeat, lock, lease alone (a lease is also a plan's files) |
| Scope token | An exact string declared on a command; an open claim holds its tokens and blocks other commands that declare one of them ([0001](docs/design/0001-evidence-ledger.md#terms)). | lock key, mutex, scope alone |
| Port | The set of storage traits the domain depends on ([0001](docs/design/0001-evidence-ledger.md#terms)). | interface, API |
| Adapter | An implementation of a port for one engine, host or forge: the SQLite adapter, a host adapter, the forge adapter, the guard answer adapter that renders an answer in the host's hook form ([0001](docs/design/0001-evidence-ledger.md#terms), [0012](docs/design/0012-host-interface.md#2-terms)). | driver, plugin, backend |

## Configuration and routing

| Term | Meaning | Avoid |
|---|---|---|
| Setting | One named value Baley reads, such as `git.on_protected`, with a type, a default and a scope ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | option, flag, config key |
| Layer | One source of settings: the built-in defaults, the global file, its host section, the project file or its host section ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | level, profile |
| Host section | A table `[host.<name>]` in a settings file whose values apply only when that host is connected ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | host profile, override block |
| Setting scope | Where a setting may be set: `global`, `project` or `both` ([0003](docs/design/0003-configuration-and-routing.md#2-terms), where it is called Scope). | scope alone (scope is also a project's description, stories and roadmap) |
| Effective policy | The result of merging every layer for one project and the connected host, if there is one, with the layer each value came from ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | config, merged settings |
| Policy version | The identity of one recorded effective policy; every command records the version it ran under ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | config hash, revision |
| Rung | One of Baley's five effort levels, in order: `low`, `medium`, `high`, `xhigh`, `max` ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | effort tier, level |
| Route | The model and rung resolved for one role and one dispatch, with the settings that decided them ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | model choice, routing table |
| Model catalog | The model names Baley accepts, per host and per provider, with the source each name came from ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | model list, registry |
| Host alias | A short model name a host resolves itself, such as `opus` in Claude Code ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | nickname, shorthand |
| Provider | An outside model vendor reached by API key or its own command-line login: OpenAI and DeepSeek are reached by key, and Anthropic only through the Claude Code login ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | vendor, backend |
| Detection | Asking a provider's list endpoint, with the owner's key, which model names that key can use ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | discovery, probing |
| Hint table | A table compiled into Baley that tags known model names with a tier and whether they accept high effort ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | model database |
| Tier | A model class, `flagship`, `balanced` or `cheap`, mapped to a model name per provider ([0003](docs/design/0003-configuration-and-routing.md#2-terms), [0008](docs/design/0008-review.md#2-terms)). | size, rank, rung (a rung is effort) |
| Config folder | Baley's own folder under the crenshawdev vendor folder, holding the global file and the keys file ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | home directory, dotfolder |
| Keys file | `keys.env` in the config folder: one `NAME=value` line per provider API key, written by the owner and only read by Baley ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | secrets file, vault, keychain |
| Key name | The name on the left of a line in the keys file, such as `OPENAI_API_KEY` ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | environment variable |
| Keys | The part of Baley that reads a key from the keys file for one use ([0003](docs/design/0003-configuration-and-routing.md#2-terms)). | secret store, key manager |

## Scope and the roadmap

| Term | Meaning | Avoid |
|---|---|---|
| Project | One managed repository with its own id, ledger records and settings ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#1-purpose-and-scope)). | repo (as a record), workspace |
| Project description | What the project is, its core value and its constraints, in the owner's words ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | vision, charter |
| Story | One numbered statement of what the project must do, declared by the owner on the roadmap, with a status (`active`, `deferred`, `excluded`), carrying its own truths ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | requirement (a requirement is a numbered design requirement row), user story, feature, ticket |
| Backlog | The stories in priority order; one committed to no phase waits in the backlog ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | queue, to-do list |
| Phase | One working increment of the roadmap: a number, a name, a goal, detail text, dependencies on other phases, and the stories committed to it. One phase is active per project ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | sprint, iteration, milestone (a milestone is a group of phases), build (a build is a slice of Baley's own development) |
| Roadmap | The ordered list of phases ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | plan (a plan is one typed document for a phase), schedule |
| Scope | The project description, the stories and the roadmap together ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | scope alone for a setting's scope or a scope token |
| Submission | A typed draft of a change, whole and validated, identified by its digest; nothing in it is recorded until it is approved ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | proposal (a proposal is a suggested setting change), request |
| Approval | The owner's yes to one submission, bound to its digest, with the owner and the time ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | sign-off, acceptance |
| Preview | The changes a submission would make, shown before approval ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | dry run, diff |
| Phase number | A whole number given to a phase when it is declared; it never changes and is never reused ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | index, position (that is its order) |
| Order | The position of a phase in the roadmap, a separate fact from its number, changed by reorder ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | rank, number |
| Context | A phase's approved goal and the truth versions of its committed stories at the time ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | discussion, context file |
| Withdraw | Remove a phase from the roadmap, keeping every record of it ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | delete, cancel |
| Brief | A file the owner points the project start at, holding a description of the project written elsewhere ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | spec, work order |
| Survey | The planner's reading of an existing codebase before drafting the first scope ([0004](docs/design/0004-starting-a-project-and-changing-scope.md#2-terms)). | audit, scan (a scan is the risk classifier's run) |

## Refinement, plans and acceptance

| Term | Meaning | Avoid |
|---|---|---|
| Truth | One acceptance criterion of a story: one sentence saying what a person can observe when it is delivered. A truth has a version ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | acceptance criterion, criterion, assertion, must-have |
| Refinement | Writing or revising a story's truths with the owner: the analyzer asks its questions, the owner answers them in rounds, then the analyzer drafts truths from the answers ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | discussion, discuss step, grooming |
| Question | One decision the owner must make, typed: an id, the question, a recommended answer and the ids of the questions it depends on ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | assumption, prompt, checkpoint (a checkpoint is an executor stop) |
| Question set | The questions of one refinement, or of one plan draft, put to the owner and recorded together ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | questionnaire, survey |
| Open question | A question that is neither answered nor deferred ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | pending question, unanswered question |
| Question round | The open questions of a set whose dependencies are all answered, worked out by Baley; nobody chooses it ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms), where it is called Round). | batch, round alone (a review also has rounds) |
| Deferral | The owner's recorded choice to leave a question undecided for now, with a reason ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | skip, postponement |
| Phase planning | Committing stories to a phase and writing its plans ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | sprint planning |
| Increment | What a completed phase delivers: something the owner can use, proven by its truths and the suite ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | deliverable, release (a release is a tagged version) |
| Size | The number of tasks a story's approved plans need; a phase's size is the sum ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | estimate, story points |
| Capacity | The owner's ceiling on a phase's size in tasks, `planning.phase_capacity` ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | budget, sprint capacity |
| Velocity | Tasks completed per closed phase, shown as a fact and never used to decide ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | throughput, speed |
| Plan | One typed document for a phase: the stories it serves, its file lease, its tasks, its suite, its evidence map and its questions. A phase may have several ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | roadmap, spec, PLAN file |
| File lease | The exact files and directories a plan's tasks may change ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms), where it is called Lease). | allowlist, lease alone (a claim also has a lease) |
| Suite | The project's test command, `workflow.test_command`, run by Baley at plan close ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | CI, test run, whole-application test |
| Evidence map | For each truth a plan serves, the items that have to exist and be connected for it to hold, and how each is checked ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | test plan, coverage map |
| Check | The one test that proves a truth: cause its trigger, look for its outcome ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | acceptance test, verify (a verify is a task's command) |
| Artifact, link, observation | The other evidence kinds: a thing that must exist, a value one part hands another, something a person or live system must see ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | reference (for a link), observation alone for a debug observation |
| Plan check | A review of a submitted plan by the checker, before the owner sees it ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | plan review (a plan review is a triggered review by reviewers) |
| Definition of done | The conditions a phase must meet to be complete: every committed story's truths met or waived, the suite green, every review ruled ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | acceptance, exit criteria |
| Retrospective | The owner's notes at phase close, recorded in the owner's words as `phase.retrospective` ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | retro, summary |
| Analyzer | The worker role that, during refinement, finds facts in the code and the records, asks questions only for the decisions left to the owner, and drafts truths from the answers ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | assumptions analyzer |
| Planner | The worker role that writes plans and evidence maps, and asks questions for the decisions a plan needs from the owner ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | architect |
| Checker | The worker role that reviews a submitted plan, also called the plan checker; its settings are `roles.checker.*` ([0005](docs/design/0005-context-plans-and-acceptance.md#2-terms)). | plan reviewer, critic |

## Execution

| Term | Meaning | Avoid |
|---|---|---|
| Admission | Binding a phase's approved plans, their evidence maps and the allocation of each check to a task, at exact versions, so execution can begin ([0006](docs/design/0006-execution.md#2-terms)). | start, kickoff |
| Allocation | Which task delivers which check; every check has exactly one task ([0006](docs/design/0006-execution.md#2-terms)). | assignment, mapping |
| Dispatch | One work order to the executor for one plan: the unfinished tasks, their checks, the lease, the commands, the route. One is active per phase ([0006](docs/design/0006-execution.md#2-terms)). | job, run (a run is one command) |
| Execution attempt | One executor run on a dispatch; a retry is a new attempt on the same dispatch ([0006](docs/design/0006-execution.md#2-terms), where it is called Attempt). | try, attempt alone (verification and debug also have attempts) |
| Task | One unit of work in a plan: one signed commit, one or more verify commands, the checks it delivers ([0006](docs/design/0006-execution.md#2-terms)). | unit of work, step, ticket, subtask |
| Verify | A task's narrowest command that settles it: one test, one binary, never the suite ([0006](docs/design/0006-execution.md#2-terms)). | check (a check proves a truth), test command |
| Red, green | For a check, the run where its test fails before the code exists and the run where it passes after, each bound to a commit ([0006](docs/design/0006-execution.md#2-terms)). | fail/pass pair, before/after |
| Run | One execution of a command by Baley: claimed, launched, recorded with exit code, output and a classification ([0006](docs/design/0006-execution.md#2-terms)). | job, invocation |
| Deviation | Something the executor did or found outside the plan: an accepted out-of-lease path, or a stated finding that a truth or decision is wrong ([0006](docs/design/0006-execution.md#2-terms)). | exception, drift |
| Checkpoint | A stop where the executor needs the owner's answer before going on ([0006](docs/design/0006-execution.md#2-terms)). | pause (a pause is the owner's work-in-progress commit), question (a question belongs to a question set) |
| Repair | The one owner-approved fix and relaunch after a red suite ([0006](docs/design/0006-execution.md#2-terms)). | hotfix, retry |
| Retire | End a task that cannot be done, releasing its checks ([0006](docs/design/0006-execution.md#2-terms)). | cancel, drop |
| Gap plan | A plan added to a phase after admission to deliver what a blocked plan did not ([0006](docs/design/0006-execution.md#2-terms)). | follow-up plan, patch plan |
| Inspection | The owner's per-check confirmation at plan completion that the check tests what it claims and stubs nothing it asserts about ([0006](docs/design/0006-execution.md#2-terms)). | review, sign-off |
| Executor | The worker role that writes tests and code, one signed commit per task ([0006](docs/design/0006-execution.md#4-roles-and-actors)). | coder, implementer |

## Verification

| Term | Meaning | Avoid |
|---|---|---|
| Verification attempt | One verification of one phase at one basis ([0007](docs/design/0007-verification.md#2-terms), where it is called Attempt). | attempt alone, audit (an audit is a read-only query) |
| Basis | The digest of everything a verification attempt judges: the phase's context, plans and evidence maps, admissions, execution outcomes and the source at HEAD ([0007](docs/design/0007-verification.md#2-terms)). | baseline, snapshot |
| Verdict | The verifier's judgment on one evidence item: `accepted`, `rejected` or `not_seen`, with what was observed ([0007](docs/design/0007-verification.md#2-terms)). | status (a status belongs to a truth), result |
| Independent run | Baley's own run of a check's command for a verification attempt, separate from the executor's red and green runs ([0007](docs/design/0007-verification.md#2-terms)). | rerun, CI run |
| Status | A truth's derived state: `pending`, `met`, `concerns`, `unmet`, `waived`; never written by a model ([0007](docs/design/0007-verification.md#2-terms)). | verdict, result |
| Observation record | The owner's record that an observation item was seen, or not, with the time ([0007](docs/design/0007-verification.md#2-terms)). | UAT, manual test |
| Overrule | The owner's record, with evidence, that a rejected item is accepted ([0007](docs/design/0007-verification.md#2-terms)). | override (an override excuses a risk fire), waiver |
| Truth waiver | The owner's record that an unmet truth is accepted as it is, with a reason ([0007](docs/design/0007-verification.md#2-terms), where it is called Waiver). | exemption, waiver alone (a surface can also be waived) |
| Completion | The record that a phase is done, bound to the basis it was judged at ([0007](docs/design/0007-verification.md#2-terms)). | sign-off, close (a close belongs to a milestone) |
| Audit | A read-only query from a story to the truths, evidence, verdicts and status that prove it ([0007](docs/design/0007-verification.md#2-terms)). | report, trace |
| Verifier | The worker role that inspects every evidence item ([0007](docs/design/0007-verification.md#2-terms)). | tester, QA |

## Review

| Term | Meaning | Avoid |
|---|---|---|
| Review | One critique of one target by one or more reviewers, ending in the owner's rulings ([0008](docs/design/0008-review.md#2-terms)). | plan check (that is the checker's), audit |
| Trigger | What raised a review: `plan`, `diff` or `risk_surface` ([0008](docs/design/0008-review.md#2-terms)). | event, cause |
| Kind | An on-demand review: `minimalism`, `decision` or `diagnosis`; a review has a trigger or a kind, never both ([0008](docs/design/0008-review.md#2-terms)). | type, mode |
| Diagnosis review | The on-demand review of a stuck debug episode, over its symptom, reproduction, recorded runs, hypotheses and observations ([0008](docs/design/0008-review.md#3-requirements) REV-R14). | consult |
| Gate | What a trigger's review holds: `off`, `advisory`, `deferred`, `blocking`, `adjudicated` ([0008](docs/design/0008-review.md#2-terms)). | mode, strictness |
| Reviewer | A voice that critiques: `host` (the host's own subagent) or a provider ([0008](docs/design/0008-review.md#2-terms)). | critic, checker (the checker judges plans) |
| Review round | One pass of every reviewer over the target; a review has at most two ([0008](docs/design/0008-review.md#2-terms), where it is called Round). | iteration, round alone (question sets also have rounds) |
| Material | The exact bytes reviewed or scanned, retained by content hash ([0008](docs/design/0008-review.md#2-terms), [0009](docs/design/0009-risk.md#2-terms)). | input, source, diff |
| Finding | One typed claim from a reviewer: file, line, severity, claim, failure scenario ([0008](docs/design/0008-review.md#2-terms)). | issue (an issue is on the forge), comment, bug |
| Adjudication | The host session's check of each finding, or other model output, against the code before it reaches the owner ([0008](docs/design/0008-review.md#2-terms)). | triage, filtering |
| Ruling | The owner's decision on one finding: `fix`, `track` or `dismiss` with a reason ([0008](docs/design/0008-review.md#2-terms)). | verdict (a verdict is the verifier's), decision |
| Standing dismissal | For one fingerprint in one project, the latest ruling that covers it, when that ruling is `dismiss` ([0008](docs/design/0008-review.md#2-terms)). | suppression, ignore list |
| Recurring finding | A returned finding whose fingerprint, or a merged finding's, has a standing dismissal in the same project ([0008](docs/design/0008-review.md#2-terms)). | duplicate, known issue |
| Confirm, reverse | What the owner's ruling on a recurring finding does to the earlier dismissal: `dismiss` confirms it, `fix` or `track` reverses it ([0008](docs/design/0008-review.md#2-terms)). | overrule, reopen |
| Settlement | The state where everything owed is answered: every finding of a review's current round ruled, or every risk fire of a plan answered ([0008](docs/design/0008-review.md#2-terms), [0009](docs/design/0009-risk.md#2-terms)). | resolution, closure |
| Deferred queue | Reviews with a `deferred` gate whose rulings are still owed ([0008](docs/design/0008-review.md#2-terms)). | backlog (the backlog holds stories), inbox |
| Filing | Creating an issue on the forge for a finding the owner ruled `track` ([0008](docs/design/0008-review.md#2-terms)). | posting, exporting |
| Fingerprint | The stable identity of a finding, computed by Baley from its file, claim and failure scenario ([0008](docs/design/0008-review.md#2-terms)). | hash, finding id |

## Risk

| Term | Meaning | Avoid |
|---|---|---|
| Surface | A category of change that is dangerous when wrong, such as `auth`, `migrations` or `secrets` ([0009](docs/design/0009-risk.md#2-terms)). | area, hotspot |
| Declared surfaces | The surfaces the project says it has, in `baley.toml`; unanswered means all eight ([0009](docs/design/0009-risk.md#2-terms)). | risk profile |
| Signal | A path segment, file name, extension or changed-line pattern that points at a surface ([0009](docs/design/0009-risk.md#2-terms)). | rule, heuristic |
| Scan | Baley's run of the classifier over a range of changes, giving matched surfaces or inconclusive ([0009](docs/design/0009-risk.md#2-terms)). | review, survey |
| Fire | A scan that matched a declared surface, or was inconclusive; it raises the `risk_surface` review ([0009](docs/design/0009-risk.md#2-terms)). | alert, hit |
| Override | The owner's record that a fire is excused for one plan, with a reason ([0009](docs/design/0009-risk.md#2-terms)). | overrule (an overrule accepts a rejected item), waiver |
| Gate raise | Setting a plan's review gate to `blocking` at submit because its lease touches a declared surface ([0009](docs/design/0009-risk.md#2-terms)). | escalation |
| Surface waiver | A declared surface excluded from the gate raise, per project; never from the scan ([0009](docs/design/0009-risk.md#2-terms), where it is called Waiver). | exemption, waiver alone (a truth can also be waived) |
| Inconclusive | A scan that could not decide: a binary or undecodable diff, or a range past the source bound ([0009](docs/design/0009-risk.md#2-terms)). | unknown, error |

## Guard

| Term | Meaning | Avoid |
|---|---|---|
| Hook | The host's pre-tool-use call into Baley ([0010](docs/design/0010-guard.md#2-terms)). | callback, trigger (a trigger raises a review) |
| Guard | Baley's answer to a hook call: `pass`, `ask`, `deny` or `pass on failure` ([0010](docs/design/0010-guard.md#2-terms)). | firewall, policy check |
| Verb | The git subcommand a `Bash`, `Monitor` or `PowerShell` command carries: `commit` or `push` ([0010](docs/design/0010-guard.md#2-terms)). | action, operation |
| Protected branch | A branch named in `git.protected_branches` ([0010](docs/design/0010-guard.md#2-terms)). | base branch (not every base branch is protected), main |
| Torn settings | A settings file that cannot be read or parsed at the moment of the call ([0010](docs/design/0010-guard.md#2-terms)). | corrupt config |
| Guard failure | A call the guard could not decide because git or the branch could not be read ([0010](docs/design/0010-guard.md#2-terms)). | guard error, crash |
| Hard fail | The opt-in rule `git.guard_hard_fail` that turns a guard failure on a provably protected branch into a deny ([0010](docs/design/0010-guard.md#2-terms)). | strict mode, fail closed |
| Remembered policy | The last complete policy read for the project, kept so a denial can still be given when the settings are torn; it never supplies an allow ([0010](docs/design/0010-guard.md#2-terms)). | cached policy, fallback |
| Redelivery | The host calling the hook again with the same call id after a timeout ([0010](docs/design/0010-guard.md#2-terms)). | retry, replay |
| Sandbox | The host's own restriction on what an agent process may read and write ([0010](docs/design/0010-guard.md#2-terms)). | jail, container |

## Milestones, landing, undo and pause

| Term | Meaning | Avoid |
|---|---|---|
| Base branch | The branch work is integrated into: `git.base_branch`, or the repository's default ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | trunk (as a noun for the branch), main |
| Integration branch | Where a phase's task commits go: a branch per phase (`baley/phase-<n>-<slug>`), per milestone, or the base branch itself ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | feature branch, working branch |
| Landing | Taking an integration branch to the base branch on the forge: push, open a pull request, merge, then local cleanup ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | shipping, merging, deploy |
| Authorization | The owner's grant for exactly one external step of one landing, bound to the landing's exact state ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | approval (an approval binds a submission), permission |
| External step | A step that changes something outside the checkout: push, open, merge, tag push, forge create ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | side effect, remote action |
| Reconciliation | Reading the real state after an interrupted step to record what happened ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | recovery, repair (a repair follows a red suite) |
| Merge confirmation | The owner's record that the pull request merged, with the cleanup choices ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | merge approval |
| Reap | Deleting the merged integration branch locally ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | prune, cleanup |
| Tracker check | Landing's read-only report of the forge's open issues for the project ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | issue check |
| Milestone | A named group of phases (Asimov names in this repository; any name in a managed project) ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | epic, release, version |
| Milestone close | The record that a milestone's phases are complete, its reviews ruled and its risk settled ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms), where it is called Close). | completion (a completion belongs to a phase), close alone |
| Archive | The later owner step, bound to one exact close, that removes the milestone's phases from the active roadmap; nothing is deleted from the ledger ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | prune, delete |
| Release | Tagging the managed project at a version, with its version manifest bumped, over a landing ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | milestone, deploy |
| Undo | Reverting a phase's task commits, newest first, before they are pushed ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | rollback, reset |
| Pause | A work-in-progress commit of the authorized paths and a resume record ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | stop, suspend, checkpoint |
| Stop | The owner's order to halt; lifted only by the owner ([0011](docs/design/0011-milestones-landing-undo-pause.md#2-terms)). | pause, cancel, kill |

## Host interface

| Term | Meaning | Avoid |
|---|---|---|
| Server | The Baley process a session starts: one per session, over stdio, serving the session and its subagents ([0012](docs/design/0012-host-interface.md#2-terms)). | daemon, instance |
| Client info | What a host sends when it connects: its name and version ([0012](docs/design/0012-host-interface.md#2-terms)). | user agent |
| Operation | One typed request under the `query` or `apply` tool, named by a string that is only ever added ([0012](docs/design/0012-host-interface.md#2-terms)). | endpoint, method |
| Refusal | A typed answer that a request was not done, with a code and a place ([0012](docs/design/0012-host-interface.md#2-terms)). | error, failure, rejection |
| Identity | The name by which a record or instruction is read: a kind plus keys, never a path ([0012](docs/design/0012-host-interface.md#2-terms)). | path, URL, file name |
| Part | One bounded piece of a served record or instruction, at most 24,576 bytes ([0012](docs/design/0012-host-interface.md#2-terms)). | page, chunk |
| Work order | The complete dispatch for one worker: identity, role, route, instructions, inputs and expected result, read by id ([0002 section 8](docs/design/0002-system-design.md#8-work-orders)). | prompt, brief, ticket |
| Stub | A file a host needs on disk to list or launch something, rendered by Baley from its tables and pointing at Baley ([0012](docs/design/0012-host-interface.md#2-terms)). | template, prompt file |
| Elicitation | The MCP request by which a server asks the host to show the person a form ([0012](docs/design/0012-host-interface.md#2-terms)). | prompt, dialog |

## Next action and progress

| Term | Meaning | Avoid |
|---|---|---|
| Derived state | A status computed from the ledger's views at the moment of the question, never stored ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | cached state, saved status |
| Current phase | The active phase: the one with committed stories that is not complete or withdrawn ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | current sprint, open phase |
| Next action | The one step Baley names as what may happen now, with the operation that does it and who acts ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | suggestion, recommendation |
| Held | Said of a step that may not happen yet, with the record that holds it named ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | blocked, waiting |
| Disagreement | A view that says one thing and the events another; a refusal, never repaired in place ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | inconsistency, drift |
| Proposal | A setting change `suggest` derives from the record, with the payload the owner may apply ([0013](docs/design/0013-next-action-and-progress.md#2-terms)). | recommendation, submission |

## Support families

| Term | Meaning | Avoid |
|---|---|---|
| Capture | A short record the owner or a session makes in passing: a `note`, or a `story` candidate for the backlog ([0014](docs/design/0014-support-families.md#2-terms)). | memo, todo |
| Promotion | Turning a `story` capture into a story on the backlog with `story declare` ([0014](docs/design/0014-support-families.md#2-terms)). | conversion, import |
| Hotfix task | A unit of work outside any phase: a description, one or a few signed commits, a report, a risk scan at close ([0014](docs/design/0014-support-families.md#2-terms), where it is called Task (hotfix)). | task alone (a task is also a step of a plan), quick fix |
| Episode | One debug investigation: a symptom, a reproduction, hypotheses, observations, attempts, a resolution ([0014](docs/design/0014-support-families.md#2-terms)). | debug session, incident |
| Hypothesis | A candidate cause with a rank reason and a state: untested, testing, refuted, confirmed ([0014](docs/design/0014-support-families.md#2-terms)). | theory, guess |
| Debug observation | A test of a hypothesis with its result and what it rules in or out ([0014](docs/design/0014-support-families.md#2-terms), where it is called Observation). | finding, observation alone (an observation is also an evidence kind) |
| Reproduction | One command that shows the episode's symptom, with its symptom signature and its reproduction files; Baley runs it ([0014](docs/design/0014-support-families.md#2-terms)). | repro script, test case |
| Reproduction files | The files the reproduction command runs that make up the reproduction itself, such as a test file, script or fixture; Baley records their digests at the red run and refuses a resolve if any changed ([0014](docs/design/0014-support-families.md#2-terms)). | test file alone (a check also has a test file), the code under test (never a reproduction file) |
| Symptom signature | Non-blank text the reproduction's output contains when the symptom is present ([0014](docs/design/0014-support-families.md#2-terms)). | error pattern, fingerprint (a fingerprint identifies a finding) |
| Red run | The run of the reproduction that exits non-zero with the symptom signature in its output ([0014](docs/design/0014-support-families.md#2-terms)). | failing run |
| Green run | The run of the same reproduction, at resolve, over reproduction files byte-identical to the red run's, that exits zero ([0014](docs/design/0014-support-families.md#2-terms)). | passing run |
| Unreproduced | An episode with no red run yet ([0014](docs/design/0014-support-families.md#2-terms)). | unconfirmed, new |
| Closed unreproduced | The end of an episode whose symptom never reproduced: the owner's record, with a reason, recorded as `debug.closed_unreproduced`; the episode leaves the open debug list ([0014](docs/design/0014-support-families.md#2-terms)). | abandoned, cancelled, resolved (a resolve needs a green run) |
| Stuck | An episode past the attempt threshold or with every hypothesis refuted ([0014](docs/design/0014-support-families.md#2-terms)). | blocked, failed |
| Spike | A question, the decision it informs, ordered criteria, one observation each, and a verdict ([0014](docs/design/0014-support-families.md#2-terms)). | prototype, experiment |
| Throwaway location | The directory outside the project where a spike's code lives; never inside the checkout ([0014](docs/design/0014-support-families.md#2-terms)). | scratch folder, sandbox (the sandbox is the host's restriction) |
| Recall | Full-text search over the ledger's recorded text ([0014](docs/design/0014-support-families.md#2-terms)). | memory, code search |
| Why | The events that name a commit, task, plan or story, joined by the ledger's git facts ([0014](docs/design/0014-support-families.md#2-terms)). | blame, history |

## Baley's own development

| Term | Meaning | Avoid |
|---|---|---|
| Build | One slice of Baley's own development in the [roadmap](docs/roadmap.md), from the build under way to the first release. It is not a phase of a managed project. | phase, sprint, release |
| Requirement | One numbered row of a design document's Requirements table, such as `CFG-R12`, with a stable identifier, a rule, a reason and a status ([design process](docs/design/README.md#requirements-and-traceability)). | story (a story is the owner's declared work in a managed project), rule, spec |

## Words with more than one meaning

Some words name different things in different areas. Qualify them, as the entries above do:

| Word | Meanings |
|---|---|
| Attempt | An execution attempt (one executor run on a dispatch), a verification attempt (one verification of a phase at one basis), and a debug attempt (something tried in an episode). |
| Round | A question round (0005) and a review round (0008). |
| Lease | A claim lease (0001) and a plan's file lease (0005, 0006). |
| Scope | A setting scope (0003), a project's scope (0004) and a scope token (0001). |
| Observation | An evidence kind (0005), the owner's observation record (0007) and a debug observation (0014). |
| Waiver | A truth waiver (0007) and a surface waiver (0009). |
| Task | A plan's task (0006) and a hotfix task (0014). |
| Close | A task close, a phase close, a milestone close, and the close of a debug episode that never reproduced. |
| Admission | Phase admission (0006), which binds a phase's approved plans so execution can begin, and checkout admission (0001), which judges a checkout against its project's other checkouts and records `checkout.seen` before a command records. |
