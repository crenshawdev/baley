# 0014: Have Baley run tests and checks itself and judge by exit code

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0006: Execution](../design/0006-execution.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Today's engine launches a check's command and classifies its outcome by parsing cargo and nextest summary lines; any other output is unknown and needs an owner's classification, so a project not written in Rust cannot close a task. The rewrite considered letting the executor run its own verifies and report them.

Evidence is first-hand only when Baley saw the run. Every language's test runner sets an exit code; many can also write a standard report (JUnit XML) naming the failing tests.

## Decision drivers

- Evidence of a red or green run comes from Baley's own launch, never from a model's report.
- Every language the owner uses is first-class.
- The rule is one line: exit code decides; a report adds names.
- No runner-specific parser is required to accept a project.

## Considered options

1. The executor runs its own tests and reports the result
2. Baley runs tests and parses each runner's summary format (today)
3. Baley runs tests and judges by exit code, with an optional standard report for the failing test names

## Decision

Chosen option: **3**. Baley runs every verify, red, green and suite command itself through the process port, claims the run before launching, checks the working tree and material, records the exit code, the bounded output and the classification. The judgment is the exit code: zero passes, anything else fails. When the runner writes a JUnit XML report (or another standard report the process port knows), Baley reads it for the names of the failing tests; without one, the output is kept and the names are not claimed. The executor asks Baley to run; it never runs a test itself for the record.

## Consequences

### Positive

- A Python, Go, JavaScript or any other project closes tasks the same way a Rust one does.
- The record holds first-hand runs with exact commands, exit codes and output.
- No parser to maintain per runner; a report format is an addition, never a requirement.

### Negative

- A runner that exits zero on failure (rare, misconfigured) is judged passed; the owner's inspection at plan completion is the check on that.
- Baley must bound and retain output for every run.

### Follow-up

- The process port's first report formats are chosen when execution is built and recorded in 0006.

## Options in detail

### The executor runs its own tests

Cheapest, but the evidence is the model's word. Fails the first driver.

### Parse each runner's summary

Works for the runners Baley knows and no other; every new language is a parser. Fails the second and fourth drivers.

### Exit code plus optional report (chosen)

Universal, first-hand, one rule; names of failures where the runner can give them.
