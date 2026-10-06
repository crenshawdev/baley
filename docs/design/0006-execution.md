# 0006: Execution

| | |
|---|---|
| Status | Accepted |
| Design issue | [#51](https://github.com/crenshawdev/baley/issues/51), [#55](https://github.com/crenshawdev/baley/issues/55); build issues [#25](https://github.com/crenshawdev/baley/issues/25), [#40](https://github.com/crenshawdev/baley/issues/40) |
| Requirement prefix | EXE |
| Applies | [0002: System design](0002-system-design.md) |
| Related | ADRs: [0008](../adr/0008-host-sandbox-isolation.md), [0009](../adr/0009-served-instructions.md), [0018](../adr/0018-lease-enforced-at-close.md), [0033](../adr/0033-host-security-bar.md) · C4 view: components ([0002](0002-system-design.md) Figure 4) |

The current design of this area, and nothing else. Edit it in place when the design changes; git holds the history. It describes the design only, never the work still to do.

## 1. Purpose and scope

This area decides how an approved plan becomes committed, tested code:

- admission: binding a phase's approved plans and their checks to an execution that can begin;
- the executor's work order and what the executor may and may not do;
- tasks: one signed commit each, red then green for every check, the narrowest verify per task;
- the lease: what a task may change, how a violation is caught, and what the owner rules;
- the command runner: Baley runs every test and check command itself and judges by exit code;
- the circuit breaker on a task that keeps failing;
- the suite gate at plan close, with one owner-approved repair;
- retiring a task, gap plans, plan completion, the owner's inspection of checks;
- a worker that exits without finishing, reconciliation, deviations.

It does not decide the plan's content or its evidence map ([0005](0005-context-plans-and-acceptance.md)); the verdict on each evidence item and a truth's status ([0007: Verification](0007-verification.md)); when a diff review or a risk gate fires after execution ([0008](0008-review.md), [0009](0009-risk.md)); what the guard does with git commands and protected branches ([0010: Guard](0010-guard.md)); which branch work lands on and how execution is undone ([0011](0011-milestones-landing-undo-pause.md)); or how a work order reaches a worker as a subagent of the session ([0012](0012-host-interface.md)). Off-roadmap tasks (`baley task`) are [0014](0014-support-families.md).

Hand-offs: 0005 approves plans; this area admits them. The work order composer ([0002](0002-system-design.md) section 8) builds the executor's work order with the route from [0003](0003-configuration-and-routing.md). 0007 verifies what this area recorded. Every command Baley runs goes through the process port ([0002](0002-system-design.md), SYS-P8) under claim, act, record ([0001](0001-evidence-ledger.md), EVD-R26).

In the component view of [0002](0002-system-design.md) (Figure 4) this area is one of the domain areas.

## 2. Terms

| Term | Meaning |
|---|---|
| Admission | Binding the phase's approved plans, their evidence maps and the allocation of each check to a task, at exact versions, so execution can begin. |
| Allocation | Which task delivers which check. Every check has exactly one task. |
| Dispatch | One work order to the executor for one plan: the unfinished tasks, their checks, the lease, the commands, the route. One dispatch is active per phase. |
| Attempt | One executor run on a dispatch. A retry is a new attempt on the same dispatch. |
| Task | One unit of work: one signed commit, one or more verify commands, the checks it delivers. |
| Verify | A task's narrowest command that settles it: one test, one binary, never the suite. |
| Red, green | For a check: the run where its test fails before the code exists, and the run where it passes after; each is bound to a commit. |
| Run | One execution of a command by Baley: claimed, launched, recorded with exit code, output and a classification. |
| Lease | The files and directories a plan's tasks may change (0005). |
| Deviation | Something the executor did or found outside the plan: an out-of-lease path the owner accepted, or a stated finding that a truth or decision is wrong or unachievable. |
| Checkpoint | A stop where the executor needs the owner's answer before going on. |
| Suite | The project's test command (`workflow.test_command`), run once at plan close. |
| Repair | The one owner-approved fix and relaunch after a red suite. |
| Retire | Ending a task that cannot be done, releasing its checks. |
| Gap plan | A plan added to a phase after admission to deliver what a blocked plan did not. |
| Inspection | The owner's per-check confirmation at plan completion that the check tests what it claims and stubs nothing it asserts about. |
| Orchestrator | The host session in its relay job (0002): launches the executor, reports its exit, relays owner answers, requests the suite and completion. |

## 3. Requirements

| Id | Rule | Why | Depends on | Status |
|---|---|---|---|---|
| EXE-R1 | Execution of a phase begins with an admission that binds its approved plans, their evidence maps and an allocation giving every check exactly one task, all at exact versions; a plan, map or truth that changed since is refused (`stale-binding`). A later extension admits more plans and keeps every earlier binding verbatim. | The executor builds against exactly what the owner approved. | PLN-R14, PLN-R15, SYS-P4 | Active |
| EXE-R2 | One dispatch is active per phase. It is issued for the first admitted plan with no outcome, or the plan the owner names. Its id binds the plan, the admission, the base commit and the unfinished tasks; the same state issues the same dispatch, and a changed state supersedes it (`dispatch-superseded`). | A worker can be handed the same work twice and never a different work under the same id. | SYS-P2, SYS-R6 | Active |
| EXE-R3 | The executor's work order carries everything: identity, goal, context, notes, each unfinished task with its verify commands and checks, each check's spec, the completed history, the continuation, the suite command, the lease, the command policy, the route, and the instructions of section 10. The executor reads it from Baley by id and reads nothing else of Baley's. | Baley hands the model everything it needs; the model decides nothing about the process. | SYS-P2, SYS-P9 | Active |
| EXE-R4 | A task closes on exactly one signed commit whose subject follows Conventional Commits and names the task id, reachable from HEAD and after the dispatch base; one commit closes one task. | One unit of work, one record of it. | SYS-P6 | Active |
| EXE-R5 | For every check a task delivers: a red run of its command, at a commit where its test exists and compiles and fails; then a green run at a later commit where it passes; the check's test file is byte-identical at red, green and the closing commit; the red commit is an ancestor of the green, the green of the closing commit. A close without this pair, or with a pair whose test file changed, is refused (`red-green`). | A failure that was watched is the only proof the test reaches the code. | PLN-R12 | Active |
| EXE-R6 | Every verify command a task names has a passing run at the closing commit within the closing attempt; a close without one is refused (`verify-missing`). | The task's own claim is settled by its narrowest command. | EXE-R7 | Active |
| EXE-R7 | Baley runs every command itself: the executor asks Baley to run a named verify, red, green or suite command; Baley claims the run, checks that the working tree is clean and the material (HEAD, tree, test file, command) unchanged, launches the command in the project root through the process port, and records the exit code, the output (bounded, keeping every result line) and a classification. The judgment is the exit code; a standard report file (JUnit XML) when present names the failing tests. A command not admitted for the task is refused (`command-not-admitted`). | Evidence is first-hand and every language works. | SYS-R8, SYS-P7, PLN-R18 | Active |
| EXE-R8 | A task close is refused (`lease`) when a committed or staged path lies outside the plan's lease, naming each path. The owner rules per path: accept it as a deviation, or have the executor split the commit. The guard denies an out-of-lease file-tool write as it happens ([0010](0010-guard.md)). A shell command can write files whose paths the guard cannot tell from its text, such as an in-place edit, a redirect or a script it runs, so the close refusal is what catches every out-of-lease path (ADR 0018, ADR 0033). A lease never blocks reads and never blocks another plan's tasks. | The lease holds however a file was written, and the owner decides exceptions. | SYS-P11 | Active |
| EXE-R9 | Baley never edits source. The hosts' own tools edit source; Baley reads source and writes records. | One write surface, and it is the host's. | EVD-R18 | Active |
| EXE-R10 | Baley counts failed runs of one task's verify within one attempt. After three, a fourth run is refused (`circuit-breaker`) and a checkpoint goes to the owner naming the task, the three failures and the executor's last stated cause. The owner answers continue (one more attempt of three), retire, or stop. The number is Baley's and is never told to the model. | An executor does not loop on its own failure. | SYS-P5, PLN-R17 | Active |
| EXE-R11 | The executor closes its last task, reports and stops. The orchestrator, never the executor, requests the suite and plan completion. | The party that did the work does not hold the gate on it. | SYS-P5 | Active |
| EXE-R12 | The suite runs once at plan close, after every task is closed. A red suite raises one question to the owner naming the failing tests and the paths the executor proposes to touch. Yes buys exactly one repair (signed commits after the failed run, paths outside the lease recorded as deviations) and one relaunch. No, a second red, or a refused repair blocks the plan; there is never a third launch. | Repair is bounded and on the record. | EXE-R7, EXE-R11 | Active |
| EXE-R13 | A task may be retired by the owner with a reason. Retirement releases the task's checks for a later plan and blocks the plan; a blocked plan's remaining checks are released the same way. | A blocked task does not block the phase silently. | EXE-R1 | Active |
| EXE-R14 | A gap plan is a plan approved after admission (0005) and admitted by extension, linked to the plan it repairs. A phase is complete only when every admitted plan is complete or every check of a blocked plan is delivered by a later plan. | Nothing blocked is quietly counted as done. | EXE-R13, PLN-R20 | Active |
| EXE-R15 | A plan completes when every task is closed, the latest suite launch passed, the owner has inspected every check the plan delivered, and any risk the phase raised is settled ([0009](0009-risk.md)). The inspection is per check, given at plan completion, and states that the check tests what it claims and stubs nothing it asserts about. | Done is proven and the owner has looked. | EXE-R5, EXE-R12 | Active |
| EXE-R16 | The orchestrator reports the executor's exit with its outcome. A dispatch whose executor exited without closing every task is interrupted; nothing continues until the owner says so. There is no timeout. | A dead worker is seen from its exit report, and nothing continues past it unseen. | | Active |
| EXE-R17 | Before any continuation, Baley reconciles the checkout: commits after the last acknowledged progress, or a dirty tree, refuse the continuation (`reconciliation-required`) until the owner rules on them. | Work Baley did not see is never built on blindly. | SYS-P7 | Active |
| EXE-R18 | Deviations are recorded on the plan's outcome and shown to the verifier: out-of-lease paths the owner accepted, repair paths outside the lease, and the executor's stated findings that a truth or a decision is wrong or unachievable. A surprise in a verify's output, even a passing one, is a stated deviation. | The verifier and the owner see everything that went beside the plan. | EXE-R8, EXE-R12 | Active |
| EXE-R19 | Every executor stop is a checkpoint answered by the owner: a decision the plan did not make, a blocked package or tool, a contradiction with a truth or decision, a lease the task cannot honor, the circuit breaker. An owner stop is lifted only by the owner. | The owner decides; the executor does not work around. | SYS-P5 | Active |
| EXE-R20 | Each dispatch round records the host's token count as reported by the orchestrator. | Cost per round is a fact the owner can see. | | Active |
| EXE-R21 | Execution is sequential: one dispatch per phase, plans in admitted order, no parallel agents and no worktrees. | One writer to one checkout. | SYS-R6 | Active |
| EXE-R22 | The cheap git facts a task close and a run depend on (HEAD, whether the index matches) are read inside the write transaction that records them, and the command is refused if either moved. | The record binds to the checkout as it was at the moment of recording. | EVD-R7 | Active |

## 4. Roles and actors

| Actor | Receives | Returns | Model and effort from |
|---|---|---|---|
| Owner | Checkpoints; the suite repair question; lease rulings; inspection requests; retire and stop decisions | Answers with owner and time | Not applicable |
| Executor (dispatched) | The work order (EXE-R3) | Task starts, run requests, progress, checkpoints, task closes, a short exit report | `roles.executor.*` ([0003](0003-configuration-and-routing.md)); a retry may move one rung (CFG-R16) |
| Orchestrator (host session) | The dispatch id and the route | Launches the executor; reports its exit; relays owner answers; requests the suite, repair, and completion | Not applicable |
| Baley: this area | Admission, dispatch, run and close requests | Refusals, receipts, runs, outcomes | Not applicable |
| Hardin | What may happen next in the phase | Admit, dispatch, continue, suite, complete, or the refusal | Not applicable |
| Process port | A command, a directory | Exit code, output, report file | Not applicable |

## 5. Commands and operations

Operations are typed operations on the host interface; the owner-only ones also have command-line entries under `baley exec`. Only the `baley exec --key` credential wrapper is removed by [ADR 0039](../adr/0039-session-owned-provider-credentials.md); the execution group keeps its name. Every request carries a request id and is answered once (EVD-R26).

### execution admit, execution extend

- **Inputs:** phase; the plans with their approval digests and map versions; the allocation (task per check).
- **Outputs:** the admission record and its version.
- **Refusals:** `stale-binding` (a plan, map or truth changed), `allocation-incomplete` (a check with no task or two), `check-command` (an allocated check's command is not one of its task's verify commands), `admitted-set` (`admit` on a phase already admitted; use `extend`) (EXE-R1).

### execute next

- **Inputs:** phase; optional plan number (owner's choice).
- **Outputs:** the dispatch id and route to launch, or `complete`, or a refusal.
- **Refusals:** `interrupted` (an unanswered worker exit), `reconciliation-required` (EXE-R17), `continuation-required` (a checkpoint or stop unanswered), `active-plan-conflict` (another plan's dispatch is active), `suite-failed` (every plan has an outcome and the phase is not complete) (EXE-R2, EXE-R16, EXE-R19).

### task start, task progress, task checkpoint, task close

- **Inputs:** `start`: dispatch, task, attempt. `progress`: acknowledged commit, or a stated deviation with evidence. `checkpoint`: kind and question. `close`: the closing commit, per check the red and green commits and runs, the verify runs.
- **Outputs:** the task's new version; on close, the plan's outcome so far.
- **Refusals:**

  | Code | When | Requirement |
  |---|---|---|
  | `dispatch-superseded` | The dispatch id is not the active one | EXE-R2 |
  | `red-green` | A check without its pair, a pair out of order, or a test file that changed | EXE-R5 |
  | `verify-missing` | A verify command without a passing run at the closing commit | EXE-R6 |
  | `lease` | A committed or staged path outside the lease, named | EXE-R8 |
  | `commit` | Unsigned, wrong subject, not after the base, not reachable from HEAD, or already closed another task | EXE-R4 |
  | `checkout-moved` | HEAD or the index moved between observation and record | EXE-R22 |

### run

- **Inputs:** dispatch, task, the command, the stage (`verify`, `red`, `green`), the check for red and green.
- **Outputs:** the run record: exit code, bounded output, classification, report file summary when present.
- **Refusals:** `command-not-admitted`, `tree-dirty`, `material-changed`, `circuit-breaker` (EXE-R7, EXE-R10).

### lease rule (owner)

- **Inputs:** the task, each named path, the ruling (`deviation` or `split`).
- **Outputs:** the ruling recorded; a `deviation` ruling lets the close proceed; a `split` ruling returns the task to the executor with the paths named.
- **Refusals:** `no-such-path` (EXE-R8).

### checkpoint answer, task retire, stop (owner)

- **Inputs:** the checkpoint and the answer (`continue`, `retire`, `stop`); `retire` carries a reason.
- **Outputs:** the answer recorded; retire blocks the plan and releases the task's checks.
- **Refusals:** `no-such-checkpoint`, `already-answered` (EXE-R10, EXE-R13, EXE-R19).

### suite run, suite repair answer, suite repair, plan complete

- **Inputs:** `suite run`: the plan (orchestrator). `repair answer`: the question, yes or no (owner). `repair`: the repair commits (executor). `plan complete`: the plan, with the owner's inspections (orchestrator).
- **Outputs:** the run record; the repair question; the relaunch; the plan's outcome.
- **Refusals:** `tasks-open`, `suite-launched` (a second launch without an approved repair), `repair-refused`, `inspection-missing`, `suite-red`, `risk-unsettled` (EXE-R11, EXE-R12, EXE-R15).

### worker exit (orchestrator)

- **Inputs:** the dispatch, the outcome (`exited`, `failed`), detail.
- **Outputs:** the dispatch marked interrupted when tasks remain.
- **Refusals:** `no-such-dispatch` (EXE-R16).

### round record (orchestrator)

- **Inputs:** the dispatch round, the host token count.
- **Outputs:** recorded (EXE-R20).

## 6. Records

### plan.admitted (event, `phase/<n>` stream)

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | 1 for the admission, then each extension |
| `plans` | list | plan number, approval digest, map version |
| `allocation` | list | check id, check version, task id |
| `repairs` | list | for a gap plan, the plan it repairs |

### dispatch.issued (event, `phase/<n>` stream)

| Field | Type | Meaning |
|---|---|---|
| `id` | digest | Binds plan, admission version, base commit, unfinished tasks |
| `plan`, `attempt` | integers | |
| `route` | route ([0003](0003-configuration-and-routing.md)) | Model, rung, sources |
| `work_order` | payload reference | The work order as served |

### task events (`phase/<n>` stream)

| Event | Fields |
|---|---|
| `task.started` | dispatch, task, attempt, base commit |
| `task.progress` | task, acknowledged commit, or deviation (text, evidence) |
| `task.checkpoint` | task, kind (`decision`, `blocked`, `contradiction`, `lease`, `circuit-breaker`), question |
| `checkpoint.answered` | checkpoint, answer, owner, time |
| `task.closed` | task, closing commit, per check: red commit, red run, green commit, green run, test file digest; verify runs |
| `task.retired` | task, reason, owner, time |
| `lease.ruled` | task, path, ruling, owner, time |

### run events (`phase/<n>` stream)

| Event | Fields |
|---|---|
| `run.claimed` | dispatch, task, command, stage, check, material (HEAD, tree, test file digest) |
| `run.recorded` | run, exit code, output (payload reference; first and last 64 KiB plus every result line), classification (`passed`, `failed`, `unknown`), report summary |

### suite and plan events (`phase/<n>` stream)

| Event | Fields |
|---|---|
| `suite.launched`, `suite.recorded` | plan, launch number (1 or 2), run |
| `suite.repair_asked` | plan, failing tests, proposed paths |
| `suite.repair_answered` | question, yes or no, owner, time |
| `suite.repaired` | question, commits, paths, out-of-lease paths |
| `check.inspected` | check, test file digest, red run, green run, no subject stub, owner, time |
| `plan.completed` | plan, closing commits, deviations |
| `plan.blocked` | plan, reason (`task-retired`, `suite-failed`, `repair-refused`, `owner-stop`), released checks |
| `worker.exited` | dispatch, outcome, detail, interrupted |
| `round.recorded` | dispatch, attempt, host token count |

### Views

| View | Key | Content |
|---|---|---|
| `dispatch` | project, phase | The active dispatch, its tasks and their state, checkpoints, runs |
| `plan` | project, phase, plan | Adds admission version, outcome, deviations, suite state, inspections |
| `run` | project, run | One run's launch and result |

## 7. States

```mermaid
stateDiagram-v2
  [*] --> Allocated: plan.admitted
  Allocated --> Started: task.started
  Started --> Started: run, task.progress
  Started --> Checkpoint: task.checkpoint
  Checkpoint --> Started: checkpoint.answered continue
  Checkpoint --> Retired: checkpoint.answered retire, or task.retired
  Checkpoint --> Stopped: checkpoint.answered stop
  Started --> LeaseRefused: task close refused lease
  LeaseRefused --> Closed: lease.ruled deviation, close again
  LeaseRefused --> Started: lease.ruled split
  Started --> Closed: task.closed
  Retired --> [*]
  Stopped --> Started: owner lifts the stop
```

*Figure 1. States of a task.*

```mermaid
stateDiagram-v2
  [*] --> Admitted: plan.admitted
  Admitted --> Dispatched: dispatch.issued
  Dispatched --> Interrupted: worker.exited with tasks open
  Interrupted --> Dispatched: owner continues, dispatch reissued
  Dispatched --> TasksClosed: last task.closed
  Dispatched --> Blocked: task.retired, owner stop
  TasksClosed --> SuiteGreen: suite.recorded passed
  TasksClosed --> SuiteRed: suite.recorded failed
  SuiteRed --> Repairing: suite.repair_answered yes
  SuiteRed --> Blocked: suite.repair_answered no
  Repairing --> SuiteGreen: suite.repaired, second launch passed
  Repairing --> Blocked: second launch failed
  SuiteGreen --> Complete: plan.completed (inspections given, risk settled)
  Blocked --> [*]: checks released to a gap plan
  Complete --> [*]
```

*Figure 2. States of a plan under execution.*

```mermaid
stateDiagram-v2
  [*] --> Claimed: run.claimed
  Claimed --> Recorded: run.recorded
  Claimed --> Abandoned: process exit seen with no record; reconciled
  Recorded --> [*]
```

*Figure 3. States of a run.*

## 8. Workflows

```mermaid
sequenceDiagram
  participant O as Owner
  participant H as Orchestrator
  participant B as Baley
  participant E as Executor
  participant P as Process port
  participant L as Ledger
  H->>B: execute next
  alt interrupted, reconciliation or continuation needed
    B-->>H: refusal naming what the owner must answer
  else
    B->>L: dispatch.issued
    B-->>H: dispatch id, route
    H->>E: launch with the dispatch id
    E->>B: read the work order
    loop each task
      E->>B: task start
      E->>B: run (red) for each check
      B->>P: command
      P-->>B: exit code, output
      B->>L: run.claimed, run.recorded
      E->>B: run (green), run (verify)
      alt fourth failed verify
        B-->>E: circuit-breaker
        B->>L: task.checkpoint
        H->>O: checkpoint
        O->>H: continue / retire / stop
        H->>B: checkpoint answer
      end
      E->>B: task close
      alt out-of-lease path
        B-->>E: lease, naming the paths
        H->>O: rule per path
        O->>H: deviation or split
        H->>B: lease rule
      else red-green or verify missing
        B-->>E: red-green / verify-missing
      else
        B->>L: task.closed
      end
    end
    E-->>H: exit report
    H->>B: worker exit
  end
```

*Figure 4. A dispatch: from issue to the executor's exit.*

```mermaid
sequenceDiagram
  participant O as Owner
  participant H as Orchestrator
  participant B as Baley
  participant E as Executor
  participant L as Ledger
  H->>B: suite run
  alt tasks open
    B-->>H: tasks-open
  else
    B->>L: suite.launched, suite.recorded
    alt passed
      H->>O: inspect each check
      O->>H: inspections
      H->>B: plan complete with inspections
      B->>L: check.inspected, plan.completed
      B-->>H: outcome
    else failed, first launch
      B->>L: suite.repair_asked
      H->>O: failing tests, proposed paths
      alt no
        O->>H: no
        H->>B: repair answer no
        B->>L: plan.blocked repair-refused
      else yes
        O->>H: yes
        H->>B: repair answer yes
        H->>E: relaunch for the repair
        E->>B: suite repair (commits)
        B->>L: suite.repaired
        H->>B: suite run (second launch)
        alt passed
          B-->>H: proceed to completion
        else failed
          B->>L: plan.blocked suite-failed
        end
      end
    end
  end
```

*Figure 5. The suite gate, the one repair, and plan completion.*

## 9. Settings

| Setting | Type | Default | Scope | Owner | Effect |
|---|---|---|---|---|---|
| `workflow.test_command` | command | absent | project | 0006 | The suite Baley runs at plan close (EXE-R12); a project with none cannot admit a plan |
| `workflow.lint_command` | command | absent | project | 0006 | Run once at plan close beside the suite; a failure is reported, never a gate |
| `roles.executor.model`, `roles.executor.effort`, `escalate_on_failure` | see [0003](0003-configuration-and-routing.md) | | both | 0003 | The executor's route and its retry rung |
| `git.protected_branches`, `git.on_protected`, `git.auto_branch`, `git.base_branch` | see [0003](0003-configuration-and-routing.md) | | project | [0010](0010-guard.md), [0011](0011-milestones-landing-undo-pause.md) | Which branch a dispatch works on and what the guard does there |

## 10. Instructions served

| Instruction | Served to | Carries requirements |
|---|---|---|
| Executor | The executor, in its work order: read the work order by id; for each check write the test first in a tests-only file, commit it compiling and failing, ask Baley to run it red, implement, ask Baley to run it green, never touch the test file again until close; run only the task's named verify commands through Baley; predict each verify's output first and state any surprise as a deviation; one signed conventional commit per task naming the task id; stage files by name, never everything; never push, force-push, amend, reset or spawn a reviewer; stop at a checkpoint and wait; close the last task, report, and stop; the default test style of [0005](0005-context-plans-and-acceptance.md) when the project has set nothing | EXE-R3 to EXE-R7, EXE-R11, EXE-R18, EXE-R19 |
| Orchestrator | The host session, as the execute stub: admit, execute next, launch the named worker with the dispatch id and route, report its exit, relay checkpoints and lease rulings to the owner, collect inspections, request the suite, relay the repair question, request completion, record the round's token count | EXE-R11, EXE-R15, EXE-R16, EXE-R20 |

The lease is stated to the executor as the exact files and directories, with the rule that a change outside them stops the close until the owner rules.

## 11. Build status

The binary parks the inherited engine for Build 9 to delete, and nothing in production reaches it (`crates/baley/src/inherited.rs:1-4`). The session server answers `execution-history`, `evidence-read` and `execute-next` (`crates/baley/src/mcp/operations.rs:105-106, 114`) and the execution apply spellings (`crates/baley/src/mcp/operations.rs:137-155`) as unavailable, and the operation baseline names Build 5 for them. Production reaches the executor contract and its front door through `baley executor-instructions` (`crates/baley/src/instruction_surfaces.rs:19-22`). The guard no longer protects the list of rendered project files (`crates/baley/src/execution/render.rs:290-441`): its write rule protects Baley's folders, the project files and the stubs ([0010](0010-guard.md), GRD-R11), so only the parked engine and the instruction lint test read that list. The parked engine's native execution path is close to this design, and the rows below list the differences.

| Requirement | Status | Where |
|---|---|---|
| EXE-R1 | Not built | Only the parked engine admits a phase: `contribute` binds the plans, maps and allocation (`crates/baley/src/execution/admission.rs:106-203`), `validate` checks them (`crates/baley/src/execution/admission.rs:333-598`), and the allocation gives each check one task (`crates/baley/src/execution/allocation.rs:26-129`). The session server answers `execution-admit` and `execution-extend` as unavailable (`crates/baley/src/mcp/operations.rs:153-154`) until Build 5 |
| EXE-R2 | Not built | Only the parked engine issues a dispatch: its identity (`crates/baley/src/execution/dispatch.rs:43-55`), the binding and its change check (`crates/baley/src/execution/dispatch.rs:192-259`) and the one active dispatch (`crates/baley/src/execution/dispatch.rs:286-342`). The session server answers `execute-next` as unavailable (`crates/baley/src/mcp/operations.rs:114`) until Build 5 |
| EXE-R3 | Not built | Only the parked engine serves the work order: the dispatch's parts (`crates/baley/src/read/document.rs:47-352`) and its operational input (`crates/baley/src/execution/dispatch.rs:163-190`). The session server's `document` reads captures only (`crates/baley/src/mcp/operations.rs:100`, `crates/baley/src/mcp/document.rs:39-49`), so no work order is read by identity until Build 5, and the server answers `execute-next` as unavailable naming Build 5 (`crates/baley/src/mcp/operations.rs:114`). The executor contract that tells the worker to read its work order by id still renders through `baley executor-instructions` (`crates/baley/src/execution/instructions.rs:16-24`, `crates/baley/src/instruction_surfaces.rs:19-22`) |
| EXE-R4 | Not built | Only the parked engine closes a task on a commit, and it checks the signature of the closing commit alone (`crates/baley/src/execution/receipts.rs:1127-1129, 1181-1190`). The session server answers `execution-task-close` as unavailable (`crates/baley/src/mcp/operations.rs:141`) until Build 5 |
| EXE-R5 | Not built | Only the parked engine checks the red and green pair at a close (`crates/baley/src/execution/receipts.rs:632-738`). The session server answers `execution-task-close` as unavailable (`crates/baley/src/mcp/operations.rs:141`) until Build 5 |
| EXE-R6 | Not built | Only the parked engine requires a passing run of each named verify command at the closing commit (`crates/baley/src/execution/receipts.rs:761-785`). The session server answers `execution-task-close` as unavailable (`crates/baley/src/mcp/operations.rs:141`) until Build 5 |
| EXE-R7 | Not built | Only the parked engine runs a command itself: it claims and launches the run (`crates/baley/src/execution/runner.rs:486-700`), observes the child (`crates/baley/src/execution/runner.rs:928-1056`) and classifies the output, understanding only cargo and nextest lines, with no exit-code rule and no report file (`crates/baley/src/execution/runner.rs:1119-1160`). The session server answers `execution-run` and `execution-classify-run` as unavailable (`crates/baley/src/mcp/operations.rs:142, 145`) until Build 5 |
| EXE-R8 | Not built | Only the parked engine checks a lease at a close: it records out-of-lease commit paths as deviations (`crates/baley/src/execution/receipts.rs:1166-1179`) and refuses staged paths (`crates/baley/src/execution/receipts.rs:1191-1201`), and the guard denies no lease path. The session server answers `execution-task-close` as unavailable (`crates/baley/src/mcp/operations.rs:141`) until Build 5 |
| EXE-R9 | Built | No source edit exists |
| EXE-R10 | Not built | Documented in 3.x only |
| EXE-R11 | Not built | The parked runner service accepts the suite and completion from any caller (`crates/baley/src/execution_runner_service.rs:60-210`). The session server answers `execution-suite` and `execution-plan-complete` as unavailable (`crates/baley/src/mcp/operations.rs:148, 152`) until Build 5 |
| EXE-R12 | Not built | Only the parked engine runs the suite once at plan close and bounds its repair (`crates/baley/src/execution/history.rs:1863-2126`). The session server answers the suite spellings as unavailable (`crates/baley/src/mcp/operations.rs:148-151`) until Build 5 |
| EXE-R13 | Not built | Only the parked engine retires a task and releases its checks (`crates/baley/src/execution/history.rs:617-664, 809-859`). The session server answers `execution-task-retire` as unavailable (`crates/baley/src/mcp/operations.rs:137`) until Build 5 |
| EXE-R14 | Not built | Only the parked engine admits a gap plan by extension (`crates/baley/src/execution/admission.rs:169-177`), links no gap plan to the plan it repairs, and counts a phase complete when any later plan completed (`crates/baley/src/execution/history.rs:1525-1547`). The session server answers `execution-extend` and `execution-plan-complete` as unavailable (`crates/baley/src/mcp/operations.rs:152, 154`) until Build 5 |
| EXE-R15 | Not built | Only the parked engine completes a plan on its owner inspections (`crates/baley/src/execution/history.rs:2128-2266`, `crates/baley/src/execution/receipts.rs:179-319`). The session server answers `execution-plan-complete` and `execution-owner-attest` as unavailable (`crates/baley/src/mcp/operations.rs:143, 152`) until Build 5 |
| EXE-R16 | Not built | Only the parked engine records a worker's exit with its outcome (`crates/baley/src/execution/runner.rs:163-340`) and finds the interrupted dispatch (`crates/baley/src/execution/history.rs:1467-1491`). The session server answers `execution-worker-exit` as unavailable (`crates/baley/src/mcp/operations.rs:146`) until Build 5 |
| EXE-R17 | Not built | Only the parked engine reconciles the checkout before a continuation (`crates/baley/src/execution_service.rs:1832-1838`, `crates/baley/src/execution/runner.rs:377-433`). The session server answers `execute-next` as unavailable (`crates/baley/src/mcp/operations.rs:114`) until Build 5 |
| EXE-R18 | Not built | The parked engine does not copy the executor-stated deviations onto the plan outcome (`crates/baley/src/execution/history.rs:1564-1676`). The session server answers `execution-plan-complete` as unavailable (`crates/baley/src/mcp/operations.rs:152`) until Build 5 |
| EXE-R19 | Not built | Only the parked engine records and answers a checkpoint (`crates/baley/src/execution_service.rs:364-501, 709-798`). The session server answers `execution-authorize`, `execution-task-checkpoint` and `execution-task-answer` as unavailable (`crates/baley/src/mcp/operations.rs:139-140, 155`) until Build 5 |
| EXE-R20 | Not built | Only the parked engine records the host's token count for a round (`crates/baley/src/execution/history.rs:1825-1862`). The session server answers `execution-round-record` as unavailable (`crates/baley/src/mcp/operations.rs:147`) until Build 5 |
| EXE-R21 | Not built | Only the parked engine takes plans in numeric order (`crates/baley/src/execution/plan.rs:495-522`). The session server answers `execute-next` as unavailable (`crates/baley/src/mcp/operations.rs:114`) until Build 5 |
| EXE-R22 | Not built | `GitObservation` values supplied by the caller (`crates/baley-store/src/command.rs:104-112`, compared in `crates/baley-store-sqlite/src/transact.rs:563-576`, #40) |

## 12. Open questions

| Question | Decided by |
|---|---|
| The exact report formats Baley reads beside the exit code, per language | [0012: Host interface](0012-host-interface.md) with the process port |
