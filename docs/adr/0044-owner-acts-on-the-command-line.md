# 0044: Give the owner's acts and reads command-line forms, and serve approvals only there

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-13 |
| Deciders | John Crenshaw |
| Design document | [0004: Starting a project and changing scope](../design/0004-starting-a-project-and-changing-scope.md), [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

Designs 0004 and 0005 said every operation is reachable from a session through the stub skill that names it and from the command line under `baley project`, `baley scope`, `baley phase`, `baley story`, `baley backlog`, `baley questions` and `baley plan`. Some steps exist only to issue a work order that a session launches, such as a refinement's analyzer work order. A command line has no subagent to launch, so a command-line form of such a step does nothing useful.

Approvals were to reach Baley through the session: design 0012 had the session put the owner's questions to the owner and relay the answer over MCP (HST-R9), and the first build of scope approval served `scope-approve` on `baley_apply`. Over MCP every call comes from the model. Nothing in the call proves a person answered, so an agent that submitted a draft could approve it as the owner in the next call. The terminal is different: Claude Code's sandbox denies agents reads and writes of Baley's home, where the ledger lives ([ADR 0033](0033-host-security-bar.md)), so an agent cannot run the command line against the owner's ledger, and a command there is the owner's own.

The question is which operations get command-line forms, and where an approval may be given.

## Decision drivers

- An agent can never approve its own work, or anything else, as the owner.
- The owner can approve and read without a session open.
- Each operation is implemented once, whichever surface serves it.
- No command exists that cannot do its job from where it runs.

## Considered options

1. Every operation on the command line as well as over MCP (the earlier wording)
2. Session-only operations, with no command line for the owner
3. Approvals relayed by the session over MCP, with command-line forms beside them
4. The owner's acts and reads on the command line, with approvals there only

## Decision

Chosen option: **4**, because it is the only one in which an agent cannot approve as the owner, and it still lets the owner read and act without a session.

Each operation is implemented once, in the library, and served over MCP. Every owner approval, of scope now and of truths and plans in later builds, is served on the command line only and never over MCP. Over MCP the call comes from the model and nothing proves a person answered, while the sandbox keeps agents from running the command line against Baley's home, where the ledger lives.

The owner's other acts and reads, answers, deferrals, decisions on review findings, reorders, withdrawals, reading rounds, previews and reviews, also have commands, under `baley project`, `baley scope`, `baley story`, `baley backlog`, `baley phase`, `baley plan`, `baley questions` and `baley review`. Each calls the same library function as the operation a session calls. A step whose only purpose is to issue a work order is session-only. An owner's approval that also releases a work order records that order and returns its id, so a session can launch it.

## Consequences

### Positive

- An agent cannot approve its own work: a draft it submits waits for the owner at the terminal.
- The owner can approve, read a draft and read accepted scope with no session open.
- One function serves the session and the terminal, so the two cannot judge the same request differently.

### Negative

- Two surfaces to keep in step, each rendering the same result its own way.
- The owner leaves the session to approve, and the session has to tell the owner the command and the digest to approve.
- A session cannot complete a step that needs an approval on its own; it stops until the owner acts.

### Follow-up

- Build 4 T3 (#25) builds `baley scope approve <digest>`, `baley scope preview <digest>`, `baley scope show`, `baley phase show <number>`, `baley backlog show` and `baley story show <id>`. `scope submit` stays session-only: a submission comes from the start planner's work.
- Every later Build 4 task supplies its own owner forms: approvals on the command line only, and the owner's other acts and reads on both surfaces.
- Design 0012 states the rule for the host interface.

## Options in detail

### Every operation on the command line

Gives every operation a command, including the steps that only issue a work order. Those commands could only print a work order nobody launches. It leaves approval reachable over MCP as well, so it does not stop an agent approving as the owner.

### Session-only operations

Keeps one surface. The owner can read and act only through a session, every approval arrives over MCP from the model, and an agent can approve its own draft.

### Approvals relayed by the session over MCP

The session puts the preview to the owner and relays the owner's yes as an approval call, with the owner's name and Baley's clock added by Baley. The call is the model's either way: Baley cannot tell a relayed yes from one the agent made up, so an agent can submit a draft and approve it in the next call.

### The owner's acts and reads on the command line, with approvals there only

Approvals come only from the terminal, which agents cannot reach against the ledger under the sandbox. The owner's other acts and reads have commands as well as session operations, both calling one function, and work-order steps stay with the session. The owner has to switch to the terminal to approve.
