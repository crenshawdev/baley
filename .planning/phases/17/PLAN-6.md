---
phase: 17
plan: 6
requirements: ["T6"]
files: ["crates/cadence/src/git_process.rs","crates/cadence/src/lib.rs","crates/cadence/src/execution_service.rs","crates/cadence/src/pause/git.rs","crates/cadence/src/why/git.rs","crates/cadence/src/execution/runner.rs","crates/cadence/src/pause/branch.rs","crates/cadence/src/rail/git.rs","crates/cadence/src/guard/bash.rs","crates/cadence/src/rail/commit.rs","crates/cadence/src/recall/history.rs","crates/cadence/src/read/document.rs","crates/cadence/src/landing/effects.rs","crates/cadence/src/store/mod.rs","crates/cadence/src/rail_service.rs","crates/cadence/src/pause_service.rs","crates/cadence/src/why_service.rs","crates/cadence/src/task_service.rs","crates/cadence/src/recall/mod.rs","crates/cadence/src/guard/audit.rs","crates/cadence/tests/subprocess_deadlines.rs","crates/cadence/tests/support/deadline_fixtures.rs","crates/cadence/tests/support/serve_process.rs","crates/cadence/tests/support/production_source.rs","docs/architecture/store.md"]
directories: []
execution: {"schema":1,"suite":"cargo nextest run --workspace --no-fail-fast","tasks":[{"id":"P17-6-T1","verify":["cargo nextest run -p cadence --test subprocess_deadlines git_subprocesses_run_under_a_deadline"]},{"id":"P17-6-T2","verify":["cargo nextest run -p cadence --test subprocess_deadlines git_subprocesses_run_under_a_deadline"]}]}
---
## Goal

Every production git spawn has a registered deadline and returns a named limit after reaping its child.

## Must be true when done

- T6. When a git subprocess exceeds its deadline, the caller sees a limit disposition naming the command and the bound, with the child reaped, and no git subprocess in crates/ left without a registered deadline.

## Context

HEAD census, production only: crates/cadence/src/execution_service.rs:2455; crates/cadence/src/execution_service.rs:2477; crates/cadence/src/pause/git.rs:63; crates/cadence/src/pause/git.rs:254; crates/cadence/src/why/git.rs:25; crates/cadence/src/why/git.rs:42; crates/cadence/src/execution/runner.rs:222; crates/cadence/src/pause/branch.rs:82; crates/cadence/src/rail/git.rs:18; crates/cadence/src/guard/bash.rs:262; crates/cadence/src/rail/commit.rs:24; crates/cadence/src/rail/commit.rs:41; crates/cadence/src/recall/history.rs:21; crates/cadence/src/read/document.rs:77. These are fourteen sites in ten files and none has a deadline. crates/cadence/src/landing/effects.rs:27 separately spawns invocation.program (git or gh), with the existing 60-second loop at :38 and process-group cleanup at :45. The grounded four-caller inventory was narrower, not the full census. The existing production_sites scanner is crates/cadence/tests/support/production_source.rs:7. Plan 5 already bounds acquisitions; this plan bounds the child processes that supply git observations, retaining its capped streams.

## Evidence map

```json
{
  "mode": "attached",
  "items": [
    {
      "kind": "check",
      "id": "check/git_subprocesses_run_under_a_deadline",
      "spec": {
        "command": "cargo nextest run -p cadence --test subprocess_deadlines git_subprocesses_run_under_a_deadline",
        "expected": {
          "kind": "property",
          "value": "Guard reports a limit naming symbolic-ref and 10 seconds before the hook's 10-second host cutoff. Other three callers name their actual command and 60-second bound and answer within 62 seconds; the sleeping child no longer exists and has been waited. Immediate controls yield ordinary policy/risk/recall/task results, never a limit. The census equals the shared spawn plus registered callers and names the location of any unregistered direct or variable git spawn. Expected bounds/margins and disposition words are handwritten, not imported from production constants."
        },
        "test": {
          "file": "crates/cadence/tests/subprocess_deadlines.rs",
          "function": "git_subprocesses_run_under_a_deadline"
        },
        "setup": "Use real Client::open_with_env at crates/cadence/tests/support/serve.rs:30, real guard probe at crates/cadence/tests/support/support_records.rs:74, and real git/risk fixtures at crates/cadence/tests/support/landing_fixtures.rs:16 and :159. support/deadline_fixtures.rs creates a PATH git shim that delegates to the captured absolute real git except for one caller-specific argv pattern; that pattern writes its PID/argv to a fixture file and execs a sleep past 60s. Repositories and .git are genuine. Prepare independent valid cases for guard symbolic-ref, risk-check's committed rail diff, recall history with actual committed planning history, and task-open's reused pause branch observation. The immediate-control shim delegates every call to real git. For the guard fixture set git.guard_hard_fail=true so failed branch evidence is an observable refusal, not an implicit allow.",
        "call": "Run real serve for risk-check, recall and task-open, and the actual cadence guard for branch observation. In each case match only the target argv, so prerequisite git commands succeed; require the shim trace proves that exact caller was reached. Wait for the real deadline (no substituted clock or shortened timeout), inspect each limit answer, then verify the recorded child PID is gone/reaped. Run immediate controls with the same inputs and require their known normal dispositions. Run the source/caller census inside this same test function.",
        "boundary": "Real serve stdio and real guard hook, genuine git repos and actual sleeping children. Census scope is all production Rust under crates/cadence/src with the stated test exclusions.",
        "fakes": [
          "Only PATH git is replaced: selected commands exec a real sleeper, all prerequisites delegate to actual git."
        ]
      },
      "reason": "This one check causes T6's trigger and inspects its stated outcome.",
      "associations": [
        {
          "truth_id": "T6",
          "truth_version": 1,
          "reason": "The trigger and outcome are exercised at the real boundary for T6."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/git-deadline-registry",
      "spec": {
        "locators": [
          "crates/cadence/src/git_process.rs",
          "crates/cadence/src/execution_service.rs",
          "crates/cadence/src/pause/git.rs",
          "crates/cadence/src/why/git.rs",
          "crates/cadence/src/execution/runner.rs",
          "crates/cadence/src/pause/branch.rs",
          "crates/cadence/src/rail/git.rs",
          "crates/cadence/src/guard/bash.rs",
          "crates/cadence/src/rail/commit.rs",
          "crates/cadence/src/recall/history.rs",
          "crates/cadence/src/read/document.rs",
          "crates/cadence/src/landing/effects.rs"
        ],
        "substance": "A per-caller registry makes all fourteen former direct git sites plus landing's variable git path use one deadline/reaping runner: nominal 10s guard, 60s every other git caller."
      },
      "reason": "A per-caller registry makes all fourteen former direct git sites plus landing's variable git path use one deadline/reaping runner: nominal 10s guard, 60s every other git caller.",
      "associations": [
        {
          "truth_id": "T6",
          "truth_version": 1,
          "reason": "This artifact is required for T6's stated outcome."
        }
      ]
    },
    {
      "kind": "artifact",
      "id": "artifact/git-deadline-census",
      "spec": {
        "locators": [
          "crates/cadence/tests/subprocess_deadlines.rs",
          "crates/cadence/tests/support/production_source.rs"
        ],
        "substance": "The single deadline check contains a production-site census that rejects a new unregistered caller naming its file and line."
      },
      "reason": "The single deadline check contains a production-site census that rejects a new unregistered caller naming its file and line.",
      "associations": [
        {
          "truth_id": "T6",
          "truth_version": 1,
          "reason": "This artifact is required for T6's stated outcome."
        }
      ]
    }
  ]
}
```

## Tasks

### Task 1: Route all git callers through registered deadlines and deliver the deadline check

- **ID:** P17-6-T1
- **Files:** crates/cadence/src/git_process.rs, crates/cadence/src/lib.rs, crates/cadence/src/execution_service.rs, crates/cadence/src/pause/git.rs, crates/cadence/src/why/git.rs, crates/cadence/src/execution/runner.rs, crates/cadence/src/pause/branch.rs, crates/cadence/src/rail/git.rs, crates/cadence/src/guard/bash.rs, crates/cadence/src/rail/commit.rs, crates/cadence/src/recall/history.rs, crates/cadence/src/read/document.rs, crates/cadence/src/landing/effects.rs, crates/cadence/src/store/mod.rs, crates/cadence/src/rail_service.rs, crates/cadence/src/pause_service.rs, crates/cadence/src/why_service.rs, crates/cadence/src/task_service.rs, crates/cadence/src/recall/mod.rs, crates/cadence/src/guard/audit.rs, crates/cadence/tests/subprocess_deadlines.rs, crates/cadence/tests/support/deadline_fixtures.rs, crates/cadence/tests/support/serve_process.rs, crates/cadence/tests/support/production_source.rs
- **Action:** Deliver git_subprocesses_run_under_a_deadline red then green. Create git_process with a required typed Caller enum and registry: ExecutionOutput, ExecutionStatus, PauseRead, PauseIndex, WhyRead, WhyInput, ExecutionRunner, PauseMergeBase, RailRead, GuardBranch, RailCommitInput, RailConfig, RecallHistory, ReadDocumentHead, and LandingGit. GUARD_GIT_DEADLINE=10s and OTHER_GIT_DEADLINE=60s cover the entire spawn/stdin/output/wait lifecycle; guard reserves 1s for reaping and answers before 10s. Move all fourteen production Command::new(git) sites to this runner; route landing's git invocation through LandingGit while preserving gh's current 60s behavior. Preserve each caller's env removal/overrides, exit-code interpretation, stdin bytes, temporary index, literal pathspec and no-lazy-fetch protections. Drain pipes concurrently, bound acquisition/output as required by each existing caller, close stdin on timeout, kill the process group, wait/reap the child and terminate inherited-pipe descendants before returning. Typed limit carries the argv command and nominal seconds; propagate it through guard unavailable/refusal diagnostics, rail errors, recall incomplete notes, pause/task and why/read answers rather than flattening it to generic git-failed or an empty diff. Do not fabricate a successful empty observation. No timeout is adjustable by fixture env/config.
- **Verify:**
  - cargo nextest run -p cadence --test subprocess_deadlines git_subprocesses_run_under_a_deadline

### Task 2: Keep the production census equal to registered caller sites

- **ID:** P17-6-T2
- **Files:** crates/cadence/src/git_process.rs, crates/cadence/tests/subprocess_deadlines.rs, crates/cadence/tests/support/production_source.rs, docs/architecture/store.md
- **Action:** Within the same sole T6 acceptance function, call production_sites rooted exactly at crates/cadence/src, skipping test modules and tests.rs/*_tests.rs per its implementation. Assert that every literal git spawn is the one common runner and every runner caller tag has a registry entry, with handwritten identities corresponding to the fourteen HEAD sites plus LandingGit. Enumerate dynamic process construction too: classify landing's remaining gh, rail gpg and execution sh explicitly so a renamed program variable cannot hide a new git caller. A new direct spawn or unregistered caller must fail with its actual file:line; do not weaken the scan to the original four modules. Keep the common runner enum exhaustive, and document the timeout/reap disposition without adding a second acceptance check or census-only evidence item.
- **Verify:**
  - cargo nextest run -p cadence --test subprocess_deadlines git_subprocesses_run_under_a_deadline

## Notes

D-213 and D-216. One shared git_process runner replaces all fourteen direct spawns; retain fourteen stable caller registrations, each with its own name and constant selection, plus LandingGit for the existing variable-program arm. GUARD_GIT_DEADLINE=10s, OTHER_GIT_DEADLINE=60s. Reserve GUARD_REAP_RESERVE=1s inside the nominal 10s envelope: terminate work at 9s so the caller returns before the hook's 10s host timeout. No change to the named limits. crates/cadence/src/rail/commit.rs's gpg child at :55 and crates/cadence/src/execution/runner.rs's sh child at :520 get no new deadline here because neither is git; ordinary shell task commands are not D-213's subprocess census. The modern pause coordinator is internal at HEAD (crates/cadence/src/server.rs:135); T1's task-open reuses pause::branch::observe and pause::git::run, which gives T6 a real stdio route to that production caller. Guard itself is necessarily the real cadence guard hook child. No new wire operation, tools/list ceiling or rendered pin.
