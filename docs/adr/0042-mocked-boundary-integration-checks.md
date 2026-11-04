# 0042: Prove assembled behavior with mocked-boundary integration checks

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-08 |
| Deciders | John Crenshaw |
| Design document | [0005: Context, plans and acceptance](../design/0005-context-plans-and-acceptance.md), [0006: Execution](../design/0006-execution.md), [0007: Verification](../design/0007-verification.md), [0002: System design](../design/0002-system-design.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The planner derives tests for the projects Baley manages under rules that allow only unit tests: one behavior of one unit, at most one simulated seam, no program started ([PRD Testing Decisions](../design/prd/baley.md#testing-decisions), [0002](../design/0002-system-design.md) section 10, [0005](../design/0005-context-plans-and-acceptance.md) section 10). A truth whose outcome depends on several of the project's units working together has no check that can prove it, so the served rules send it to a pending observation, and the owner has to look at the running program to settle it ([VER-R9](../design/0007-verification.md#3-requirements)). Unit checks leave the wiring between units unproven, and each such observation costs the owner time at verification ([issue 229](https://github.com/crenshawdev/baley/issues/229)).

The unit-only rule exists so that a test gives the same result on any machine the project builds on. A test that needs a real database, a network, a host program or pre-existing machine state does not. An integration test whose outside boundaries are all mocked keeps that property: the project's own units run together for real, and each database, network, third-party API, clock, filesystem or host program it would reach is replaced by a mock behind an interface the project owns.

## Decision drivers

- A truth about assembled behavior is proven by a test Baley runs, not left to the owner.
- Every test still gives the same result wherever the project builds and starts no program.
- One check per truth stays, so evidence does not multiply.
- The project's own test command remains the one suite. No new command or setting is added.
- What a test cannot establish is still stated and left to the owner.

## Considered options

1. Keep unit checks only and leave assembled behavior to owner observations (today)
2. Add mocked-boundary integration checks inside the existing check kind
3. Add integration tests against real throwaway databases, containers and services now

## Decision

Chosen option: **2**, because it proves the wiring between the project's units while keeping every test deterministic, and needs no new evidence kind, command or setting.

A check carries a `level`, `unit` or `integration`. Exactly one check per truth version remains (PLN-R12). The planner writes an integration check when the truth's outcome depends on two or more of the project's own units working together, and a unit check otherwise. An integration check's `boundary` names the project units it runs together, and its `fakes` names every outside boundary it mocks.

A mock stands in only for an outside boundary: a database, the network, a third-party API, the clock, the filesystem or a host program. It sits behind an interface the project owns and serves the results and the failures the test needs. No project unit is mocked inside an integration check, and a check that mocks a unit it claims to run together is rejected by the verifier under VER-R6. Where the project has no owned interface at a boundary, the planner says so in the plan and plans the seam, the same way a unit that asks the outside world in the middle of judging is split today. The rule that a test depends only on the project's language toolchain and test libraries, mocking libraries included, applies unchanged, and so does the fresh temporary directory as the one filesystem seam.

An integration check runs red then green in its task like any check (EXE-R5). Integration tests belong to the project's own test command (`workflow.test_command`), so they run in every plan-close suite (EXE-R12), and the latest suite gates phase completion (VER-R12). PLN-R8 stands: there is no separate whole-application test command.

An observation is written only for what an integration check cannot establish: real persistence, a real service, GUI interaction, performance and a live run of the assembled program.

## Consequences

### Positive

- Truths about assembled behavior reach `met` from Baley's own runs instead of waiting on the owner.
- The wiring between units is proven in every plan-close suite, and a break in it fails the suite.
- Mocks behind owned interfaces push managed projects toward seams at their outside boundaries.
- The executor, the suite gate and the verifier keep their existing shapes.

### Negative

- A mock can agree with code that a real dependency would reject. Real persistence and real services stay unproven until tests against them exist.
- A project with no owned interfaces at its boundaries needs seams planned before its first integration check, which adds tasks.
- The verifier has one more way a check can fail to measure what it claims, a mocked project unit, and must read for it.

### Follow-up

- Build 4 (#25) adds `level` to the check spec and the evidence map grammar, and serves the integration rules to the planner.
- Build 5 (#26) serves the verifier the rule that rejects an integration check mocking a project unit.
- Tests against a real throwaway database or container, adapter tests against real services, and GUI interaction tests are left for after the first release. Until then they stay observations.
- Baley's own test rules are not changed by this decision.

## Options in detail

### Unit checks only

Keeps every check at one unit and one seam. Every truth about assembled behavior becomes an owner observation, so the owner settles by hand what a deterministic test could settle, and a regression in the wiring is not caught by the suite.

### Mocked-boundary integration checks

Runs the project's own units together with every outside boundary mocked behind an owned interface. Results stay the same on every machine, the existing check, red-then-green and suite rules carry it, and observations shrink to what needs a real dependency or a person. A mock can be wrong about the real dependency, which this option does not address.

### Real-dependency tests now

Proves persistence and service behavior for real, but needs containers, services or installed programs on every machine that runs the suite, breaks the same-result-anywhere rule, and adds machinery the first release does not need.
