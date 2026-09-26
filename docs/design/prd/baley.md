# Baley: product requirements

| | |
|---|---|
| Status | Accepted |
| Covers | The whole product, first release |
| System design | [0002: System design](../0002-system-design.md) |

## Problem Statement

AI coding agents now write much of the code in a project, but the person who owns the project still answers for every line. Today that person has no dependable way to know what an agent actually did, whether its claims are true, or whether the process they meant to follow was followed at all.

The agent itself keeps the process in its head. It decides what comes next, which step to skip, whether a test really ran and whether the work is finished. Rules written as instructions to the model are followed when the model remembers them and ignored when it does not. When a session ends, crashes or runs out of context, the state of the work goes with it. Reviews happen when someone remembers to ask for them. A green result can mean a test passed, or that no test ran. The owner ends up re-checking everything by hand, or trusting what they cannot see.

## Solution

Baley is the owner's control plane for AI-assisted engineering. The owner approves the plan, agents do the engineering, and Baley decides every step of the process with deterministic code. Nothing counts until Baley has recorded the evidence that proves it.

Baley keeps one record of everything that happens to a project: plans, approvals, what each agent did, the tests that ran and what they printed, reviews, rulings and verdicts. It decides what may happen next from that record and refuses anything the evidence does not support, naming what is missing. It builds the complete instructions for every agent it dispatches, so the model never decides the process, only the engineering. It runs the tests itself, so a result is something Baley saw, not something an agent reported.

The owner works in a familiar loop: start a project, discuss a phase, approve a plan, let the agents build it, review and verify it, then land it. Baley runs inside Claude Code or Codex, the host the owner already uses, and works the same way in both.

## User Stories

### Starting and shaping a project

1. As an owner, I want to bring a new or existing repository under Baley with one command, so that I can start working without hand-editing configuration.
2. As an owner, I want Baley to write the first requirements and roadmap with me, so that the work has an approved scope before any agent builds anything.
3. As an owner, I want to add, edit, insert and remove phases in the roadmap through Baley, so that the roadmap changes only with a record of why.
4. As an owner, I want every earlier version of an approved requirement or phase kept, so that I can see how the scope changed and who changed it.
5. As an owner, I want Baley to find the project from any folder inside the repository, so that I do not have to tell it where I am.

### Settings, roles and models

6. As an owner, I want to set up Baley through a guided interview the first time, so that I get working defaults without reading every setting.
7. As an owner, I want global settings shared by all my projects and project settings that override them, so that I configure once and adjust per project.
8. As an owner, I want settings that differ between Claude Code and Codex kept in their own section for each host, so that each host uses the models it actually has.
9. As an owner, I want to choose which model and effort level each role uses (planner, assumptions analyzer, plan checker, executor, verifier, reviewer), so that I spend the strongest models where judgment matters.
10. As an owner, I want a retry to step up one effort level when I allow it, so that a failed attempt gets more capacity without me intervening.
11. As an owner, I want to see every setting, its value and where it came from, so that I never have to guess why Baley made a choice.
12. As an owner, I want Baley to record which settings were in effect for each decision, so that I can explain any past outcome.
13. As an owner, I want my branch, forge and repository settings kept with the project, so that they are the same whichever host I use.

### Planning

14. As an owner, I want to refine each story with its acceptance criteria (truths, the outcomes a person can observe) before it is planned, so that the plan is measured against what I actually want.
15. As an owner, I want each truth written as a single observable outcome, so that it can be proven or disproven.
16. As an owner, I want to set a sprint's capacity in tasks and have Baley refuse a plan that would exceed it, so that sprints stay small enough to finish without anyone estimating.
17. As an owner, I want a planner agent to write the plan, with exactly one check for each truth, so that every promised outcome has a test that proves it.
18. As an owner, I want an assumptions analyzer to surface what the plan takes for granted, so that I catch wrong assumptions before code is written.
19. As an owner, I want a plan checker to judge the plan independently, so that a bad plan is caught before it is built.
20. As an owner, I want each phase to deliver something I can actually use, so that progress is real at every step.
21. As an owner, I want to approve splitting a phase in two before it happens, so that scope never changes without me.
22. As an owner, I want my approval to bind to the exact plan I read, so that a plan changed after I approved it is refused.
23. As an owner, I want nothing to run until I have approved the plan, so that agents never act on a plan I have not seen.

### Execution

24. As an owner, I want Baley to dispatch the executor with a complete work order (role, model, effort, instructions, inputs and the result it expects), so that the agent does engineering, not process.
25. As an owner, I want every test written and seen failing before the code that makes it pass, so that no check can pass without testing anything.
26. As an owner, I want one signed commit per task, so that the history shows exactly what each task changed.
27. As an owner, I want Baley to run the test suite itself when a plan closes, so that a passing suite is something Baley saw, not something an agent claimed.
28. As an owner, I want a failing suite to stop and ask me whether to allow one repair, so that an agent never loops on a red suite on its own.
29. As an owner, I want work outside a task's declared files recorded as a deviation, so that I see where an agent went beyond its brief.
30. As an owner, I want to retire a task that cannot be done and plan the gap, so that a blocked task does not block the whole phase silently.
31. As an owner, I want Baley to work with any language's tests, so that projects that are not written in Rust are first-class.
32. As an owner, I want long operations such as test runs to keep running while my session continues, so that I am not blocked waiting on a tool call.

### Review and risk

33. As an owner, I want Baley to decide from my policy when a plan or a diff must be reviewed and how strictly, so that reviews happen every time they should, not when someone remembers.
34. As an owner, I want reviews from outside models (for example OpenAI, Gemini or DeepSeek) when my policy asks for them, so that the work is attacked by a different model than the one that wrote it.
35. As an owner, I want each outside provider to use either its own command-line login or an API key, as I choose, so that I am never forced to hand over a key.
36. As an owner, I want every review finding checked against the code and presented to me in plain words with the options for fixing it, so that I rule on real problems, not raw model output.
37. As an owner, I want my ruling on each finding to be what clears a review gate, so that no finding is dropped or applied without me.
38. As an owner, I want Baley never to act on a finding by itself, so that fixes happen only when I decide.
39. As an owner, I want changes that touch risky areas (such as authentication, migrations, secrets or destructive operations) to trigger a stricter review, so that the dangerous changes get the most scrutiny.

### Verification

40. As an owner, I want a verifier agent to give one verdict for each piece of evidence, so that every truth is checked against what was actually built.
41. As an owner, I want Baley, not the model, to decide whether each truth is met, so that a model cannot declare its own work done.
42. As an owner, I want Baley to re-run each check itself during verification, so that verification rests on fresh evidence.
43. As an owner, I want to waive a truth with a recorded reason, or overrule a rejected verdict, so that I stay in charge of exceptions and they stay visible.
44. As an owner, I want a phase to complete only when every truth is met or waived, so that "done" always means proven.

### Milestones, landing and release

45. As an owner, I want to close a milestone only when its phases are complete and its reviews are ruled, so that nothing half-finished is shipped.
46. As an owner, I want each external step of landing (push, pull request, merge, tag) to need my approval, so that nothing is published without me.
47. As an owner, I want every landing step recorded before and after it runs, so that an interrupted landing is recovered, not repeated.
48. As an owner, I want to undo a change by the exact commits Baley recorded, so that undo never guesses.
49. As an owner, I want to pause work and resume it later from exactly where it stopped, so that a stopped session loses nothing.

### Everyday tools

50. As an owner, I want to capture an idea or a to-do in one line without leaving my work, so that nothing is lost.
51. As an owner, I want small one-off tasks, debugging sessions and exploratory spikes recorded the same way as planned work, so that everything I do leaves evidence.
52. As an owner, I want to ask why the code is the way it is and get the decision and evidence behind it, so that I can trust or challenge it.
53. As an owner, I want to search everything Baley recorded, so that past decisions are easy to find.
54. As an owner, I want to see progress and the next allowed step at any time, so that I always know where the work stands.

### Guard and trust

55. As an owner, I want every git commit and push an agent runs checked against my protected branches, so that agents cannot commit to or push a branch I protect.
56. As an owner, I want agents kept out of Baley's own records, so that the record of the work cannot be rewritten by the work.
57. As an owner, I want tampering with the record detected, including by checking against copies anchored on the forge, so that I can trust the history.
58. As an owner, I want an API key to reach only the one call that needs it, with any copy in the output hidden, so that keys do not leak into conversations or logs.
59. As an owner, I want Baley to store my API keys itself and manage them only through its command line, so that keys do not sit in plain files or environment variables.

### Hosts and running

60. As an owner, I want Baley to work the same way under Claude Code and Codex, so that I can use whichever host suits the task.
61. As an owner, I want one Baley to serve all my sessions and projects at once, so that several terminals and agents work against the same record without conflict.
62. As an owner, I want Baley to work with no background service at all, so that I install nothing extra.
63. As an owner, I want to choose at install time to run Baley in the background, with Baley managing that itself, so that my hosts can use the newer protocol without me configuring a service.
64. As an owner, I want Baley to adapt to each host's capabilities (such as being notified when long work finishes), so that I get the best behavior each host offers.
65. As an owner, I want two sessions changing the same thing at once to be handled safely, with the second one refused rather than merged, so that the record never becomes inconsistent.

### The record itself

66. As an owner, I want to verify, back up and export the record at any time, so that my project's history outlives any machine or checkout.
67. As an owner, I want to purge a secret that ended up in the record, so that a leak can be cleaned up.
68. As an owner, I want Baley's memory use to stay bounded however large the record grows, so that it runs for months without slowing my machine.
69. As an owner, I want every refusal to say what is missing and where, so that I know how to proceed.

## Implementation Decisions

The architecture is in [0002: System design](../0002-system-design.md); the store is in [0001: The evidence ledger](../0001-evidence-ledger.md). The decisions that shape the product:

- **Three parties, one authority each.** The owner holds intent and accountability. Baley holds the truth about the process, every process decision and orchestration. The model holds engineering judgment only. The host's main session relays Baley's work orders and adjudicates other models' output when Baley assigns it.
- **Complete work orders.** Baley builds every dispatch whole: role, model, effort, instructions, inputs and the typed result expected back. The model never writes a prompt, never reads settings and never chooses a role, model or effort.
- **Evidence-gated lifecycle.** The state of the work is derived from the record, never stored beside it. A step is allowed only when the recorded evidence supports it.
- **One record.** Every fact is an append-only, hash-chained event in one SQLite ledger per user, outside any checkout. Views are rebuilt from events. No Markdown file is a record.
- **Instructions are part of the binary.** Every instruction a model sees is compiled into Baley and served in parts. Files a host must load are stubs Baley renders.
- **One shared Baley per user.** A single server serves every session and worker. It speaks MCP over stdio and HTTP, and both the 2025-11-25 protocol and MCP 2 (2026-07-28). At install the owner chooses whether it runs in the background (Baley manages its own systemd or launchd entry); otherwise a small stdio launcher starts or joins it.
- **Host adapters.** Baley identifies its host and version on every connection and chooses mechanisms per host, such as notifying the model when long work finishes where the host supports it, and waiting in steps elsewhere.
- **Optimistic concurrency.** Writes go through one writer, one short transaction at a time. Each decision is made inside its write from inputs read there; a command whose inputs changed is refused, never merged. Long work runs outside the write and is recorded after it.
- **Baley runs tests and checks.** Results are judged by exit code, with an optional standard report for which tests failed, so every language works.
- **Outside models are called by the host session, never by Baley.** Baley decides the review and builds its prompt and material. Each provider uses its own command-line login or an API key, as the owner chooses. A key reaches a call only through Baley, set for that one command and hidden in its output.
- **Settings are TOML.** One global file and one project file that overrides it. Branch, forge and repository settings live only in the project file. Settings that differ per host sit in a section for each host. Baley writes the files; the model is never told about them.
- **Nothing is required to work the way Cadence did.** The familiar working loop (project, plan, milestone, land) is kept; everything underneath is designed afresh.

## Testing Decisions

- **What a good test is.** A test checks one behavior of one unit through its public surface, using plain values. It depends only on the language toolchain and its own test libraries, starts no program, and gives the same result on any machine the project builds on. A test may use a fresh temporary directory it owns. There are no end-to-end tests.
- **Seam 1: the storage port.** Every store adapter passes one shared conformance suite that defines the port's behavior.
- **Seam 2: the decision core.** Hardin and the domain rules are plain synchronous code, tested directly with plain values. This is the highest seam where every process decision can be checked without a host.
- **Seam 3: the MCP operation boundary.** A typed request in, a typed answer or refusal out, tested inside the process with no host attached.
- **Live behavior** on real hosts is checked by an acceptance run on both hosts before release, not by tests.
- **Prior art.** The SQLite store adapter's tests and the conformance harness in the store crates.

## Out of Scope

- Windows in the first release. It is planned for a later release.
- Several people sharing one project's record, and a shared server for teams.
- Moving the record between machines. Only the anchors of its chain leave the machine.
- A separate operating-system user for Baley.
- Defending the record or the keys against a determined agent. The design guards against accidental exposure; tampering is detected, not prevented.
- Running agents in parallel or in separate worktrees on one phase.
- Tracking work in documents. Deciding what happens next is Baley's job, not a document's.
- Importing records from Cadence.

## Further Notes

- Baley grew out of Cadence, a planning and gate system for Claude Code. Its working loop is Baley's heritage; its code and records are not carried over.
- The area-by-area design is written as a set of living design documents that follow this PRD and the system design.
