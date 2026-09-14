# The bootstrap exception (D-156)

On 2026-09-13 the rewrite branch's store refused every write it was asked
for. HEAD's snapshot had dropped the `root` field that state.json carried,
integrity no longer matched, and the binary could not record a repair to
itself. The plan that was supposed to carry that repair, phase 31 plan 5,
needed the store to run. This file is the record of the exception that
broke the loop. It is not read at runtime and carries no rule the binary
enforces; the rule it names lives here and in the six commits that cite it.

## D-156

A defect that stops the binary from recording its own repair, or suite debt
no open plan owns, may be fixed by hand. The gates stay: a red test first,
or the failing test that already exists as the red; one signed conventional
commit per fix naming what it fixes; clippy clean on what it touches; the
full suite once at the end; and the record written here, never into the
store. Everything else waits for the front door.

John, 2026-09-13, after asking whether we were finding bugs or shoehorning:
"Six. Don't send anything to codex. I want you to handle it."

## The six

1. `e05c3824` fix(store): a root whose filesystem identity changed at the
   same path is the same root when its bytes match. Re-lands 9d24bdaf, which
   8ccb543f had reverted so plan 5 could land it. state.json carried `root`,
   the snapshot did not, and every write came back "snapshot integrity or
   version mismatch".
2. `8757b24f` fix(import): keep the import manifest as history and carry the
   current layer paths beside it. One manifest field did two jobs; after any
   accepted relocation of the global config the evidence guard refused every
   gate answer with "evidence proposal must preserve import manifest". The
   session now carries `active` beside the stored manifest, and the guard
   compares the manifest against the manifest. This is phase 35's whole scope.
3. `9c7d5037` chore(34): rerender the executor contract skill after plan 1
   taught the binary to retire a task. Plan 1 added the retire paragraph to
   the compiled role and did not rerender `skills/cad-executor-contract/SKILL.md`;
   `tests/mcp.rs` holds the checked-in bytes to the binary's own rendering.
   The same commit allows dead code on the phase 34 support include, where
   `Completed::project` has no caller.
4. `2e0c9770` test(13): rehearse adoption from the pinned rewrite tree, never
   the live one. The rehearsal read the live `.planning` and asserted it had
   no native store; it has had one since dogfooding began, and the roadmap
   has 35 rows where the rehearsal counts 30. The source is now cea1f28c,
   the phase 13 close, exported from git.
5. `c27ecb61` test(31): ignore the retired T6 red until phase 32 respecifies
   it. P31-3-T1's committed red stays in the tree with the reason on the
   attribute. The four clippy lints in `support/phase31_hosts.rs` wait for
   phase 32 with it.
6. `167f7e39` test(12): bind a provisional publication's null map as the
   empty string in the admission fixture. Plan 6's repair: compact plan-read
   emits `map_revision: null` for a provisional publication, and the fixture
   kept the null, so the provisional control stopped at admission-shape
   before admission-binding could refuse it.

## What the suite said

The gate that stays is the suite once at the end, and it refused commit 1.
Fifteen tests in five targets: `store::crash_tests`, `execution_store`,
`execution_boundary_compat`, `phase7_receipts` and `phase8_dispatch`, every
one green at 97c70fbc, every one red because state.json now carried an
absolute path and a device and inode chain, a legacy store was rewritten at
first open, and a store copied to another path was refused as moved. Neither
9d24bdaf nor its re-landing had ever been run against the full suite; the
plan that would have run it, phase 31 plan 5, was the plan the store could
not execute.

`985022d5` revert(store) takes commit 1 back out. Under it, the reboot
refusal came back, so the cause got fixed instead of the symptom.

## D-157

A record's directory identity is provenance, never the key the next process
must match. Every native record keeps the device and inode chain of the
directory it was written under. Nothing compares a retained record's chain
with the live one. The in-process guard stays: the writer's live observation
of its directory, the transaction's intent checks, and a claim checked
against the directory it was built in during the same commit. A patch must
still echo the dispatched input's chain, because that is a patch against a
retained record.

`7d60cd9c` fix(execution): a record's directory identity is provenance, never
the key the next process must match. The four replay functions lose the
binding argument they no longer read. Red test:
`execution::tests::native_records_outlive_the_directory_identity_they_were_stamped_with`.

The live store carried a `root` field from the unreleased binary, sealed
into the snapshot's integrity. It was stripped by hand with the binary's own
`Snapshot::new` and `with_operations`, on a copy, re-parsed, then copied
over, generation 131 and every record kept; the original is in the session
scratchpad. John said yes to that before it happened.

## What it made moot

Phase 31 plans 5 and 6 are admitted in the store and pending; their
documents stay as written. Plan 5's work landed as commit 1 and was reversed
by D-157; plan 6's is commit 6. Phase 35 has an approved context (D-155, T1)
and no plan; commit 2 is its scope, and its roadmap row stays unchecked.
How the store closes these three is a front-door question. Nothing here
writes a store record for them.

Phase 35's row and section left ROADMAP.md by hand on 2026-09-14. Its
whole scope had shipped as commit 2 the day before, and the binary has no
way to close a phase whose scope shipped outside it: a plan for it would
own a check with no red left, verification refuses a phase with no
admitted plan, and derivation reads the store, never the box, so a checked
row would have kept it the current phase forever
(`crates/cadence/src/derivation/mod.rs:184-190`). Its approved context
(D-155, T1) stays in the store as history. That missing operation is a
candidate beside the others.

Phase 34 plan 1 is recorded failed on suite `p34-1-suite-20260913`, exit
101, on the four targets commits 3 through 6 fix. Commit 3 is plan 1's own
miss. Phase 34 plan 2 goes through the front door on a green suite.

## The seventh

Later the same evening, with phase 34 plan 2 parked on a checkpoint, a new
Claude Code session died on its first message: "tools.15.custom.input_schema:
JSON schema is nested too deeply. Tool schemas may nest at most 64 levels".
Tool 15 was `cadence_apply`. `host_schema` in `server.rs` merged each
variant's shape of a shared field by wrapping the previous union in a fresh
`anyOf`, one level per variant, and the `operation` field had reached 65.
No host session could load the resident, so nothing could reach the front
door. That is D-156's first clause.

7. `0f925a1a` fix(server): keep every host schema union flat so the API
   stops refusing the resident. Distinct shapes collect into one `anyOf` per
   field, 9 levels deep on both tools. The red is
   `tool_schemas_stay_within_host_nesting_limits` in `tests/mcp.rs`, which
   refuses any advertised schema past 32 levels so the next variant cannot
   take a session down. `cargo test --workspace --no-fail-fast` at `0f925a1a`:
   exit 101, 52 targets, 942 passed, 1 failed, 1 ignored. The one failure is
   `phase34_blocked_then_completed_phase_is_derived_executed`, plan 2's own
   committed red at cbe39c4e (`phase_status` null, not `"planned"`), which
   the seventh commit does not touch.

## The suite

`cargo test --workspace --no-fail-fast` at `7d60cd9c` on 2026-09-13:
exit 0, 52 targets, 941 passed, 0 failed, 1 ignored (the T6 red).

## What it taught

Three gaps, all open, none decided here.

D-120 repairs a failed suite through a linked gap plan, never a rerun, and
D-112 says who owns runs. Neither names a vehicle for a closed phase's test
that goes red later with no open plan to own it. Phase 13's rehearsal and
the phase 12 fixture were exactly that, debt with no owner, which is how
a phase 31 gap plan came to own a phase 12 file. The choice is a standing
vehicle for orphan suite debt, or this exception, made once and named each
time it is used.

D-152 retires a task and says nothing about the red test material the task
already committed. P31-3-T1's red is in the tree, and the attribute that
ignores it is a hand decision. Retirement should say what happens to the
material.

D-120 says a blocked plan is repaired by a later approved gap identity, and
the binary enforces the opposite at three sites. On 2026-09-14, phase 34
plan 2 was retired because its admitted check setup admitted both fixture
plans at once, which `history::phase_complete` can never count as a later
repair. Plan 3, published as the gap at `1f32fd7e`, could not carry a
corrected check: the same id with a changed spec is `evidence-item-conflict`
and a new id under T2 is `truth-check-limit`, because a retired plan's
items stay current in the phase union. Its task cannot own the check either:
an extension must keep every prior allocation entry
(`execution/admission.rs:98`) and a check has one owner
(`execution/allocation.rs:52`). And the phase cannot verify at all while
plan 1 is failed and plan 2 retired, because verification inputs demand
every admitted plan complete (`verification/inputs.rs:113`). One decision,
what a retired or failed plan leaves behind in the current union, three
enforcement sites. Plan 3 stays published and unadmitted until it is made.

A fix written by hand ran its own tests and never the suite, twice, and it
was the suite at the end that caught it. The gate held because it was kept;
a plan would have run it at close, and the plan is the thing that could not
run.

D-158 and D-159 went to phase 36's context on 2026-09-14, and D-161 to
phase 37's the same day. The orphan suite debt vehicle and the retirement
material are still candidates, and so is this one, found closing 36 and
37: a check the verifier rejects for an assertion weaker than its expected
value, on code that is right, has no red left, so no gap plan can re-own
it under D-161 and the owner's only exit is a waiver. Both phases closed
complete-with-waivers for exactly that.

## D-160

The store's own staging files are the binary's, never the user's. A source
observation that runs while a commit is between `prepare` and its rename
sees `.planning/.state.json.<pid>.<seq>.tmp` beside the target, and that
file does not make the tree dirty. Every other untracked path, under any
other name, anywhere, still does.

On 2026-09-14, with 34 and 36 both derived executed and the resident
restarted on the rebuilt binary, `verify-next 34` refused
`evidence-source-dirty` on a tree where `git status --porcelain=v1 -z
--untracked-files=all` printed nothing. A fresh stdio process said the same.
strace showed the order: `commit` in `store/transaction.rs` runs
`validate_all` three times, before `prepare`, after it, and again before
each rename, and for a verification intent `validate_all` re-observes the
source through `inputs::reobserve_external`. The second and third passes
ran with the staging files on disk. `.gitignore` names the store's targets
and not the names `prepare` gives them, so Git listed three untracked files
and the binary refused its own write, then unlinked them. Every test fixture
ignores `.planning/` whole, which is why fifty-two targets never saw it, and
execution receipts do no clean check, which is why execution ran all day
and verification failed on its first live call. A project that tracks its
`.planning` documents, which is the shape the design asks for, could not
verify at all.

This is a D-156 case by its first clause: the binary could not record its
own verification, and the plan that would carry the fix needs verification
to run.

8. `ac929d8b` test(store): a commit in flight must not dirty its own source.
   `tests/store_staging_clean.rs` drives the real store with a probe at
   `Stage::Prepared` on a fixture that tracks `.planning` and ignores only
   the store files; `runner::clean` and `inputs::source` must both read the
   tree as clean while the staging file exists. Two more tests: the exact
   names `prepare` writes, by hand, at both places it writes them; and the
   negative control, a user's untracked file under six names and places,
   and a modified tracked file, every one still refused.
9. `8acb3948` fix(store): the store's own staging files never dirty the
   source. `runner::status` is now the one reader of `git status` for both
   clean checks and drops an untracked entry under `.planning/` whose name
   is one `prepare` writes; `filesystem::is_staging_name` owns that shape
   beside the naming. The same reader serves the evidence run's before and
   after observation in `verification/runner.rs`.

Clippy is clean on the library and the new target. The four lints in
`tests/support/phase31_hosts.rs` are commit 5's, still waiting for phase 32.

`cargo test --workspace --no-fail-fast` at `8acb3948` on 2026-09-14:
exit 0, 54 targets, 950 passed, 0 failed, 1 ignored (the T6 red).

The first `verify-next 34` on the rebuilt binary opened attempt `00cd1a1c`
at head `8acb3948`, before this record was written. This commit moves HEAD,
so that attempt's basis is stale and the next `verify-next` opens another;
the store keeps the first as history. Nothing here writes a store record.

## D-162

A stored Complete terminal is history once a later extension admits a plan
with no retained outcome. `execute-next` answers complete only while
`phase_complete` still holds, and `admit_dispatch` clears the terminal as
that later plan goes active. A judgment stop still ends the occurrence.

On 2026-09-14 phase 37 (D-161, a rejected check is released like a retired
one) ran its one task through Codex. Three of the four checks went red then
green. The fourth, `P37-T4-C`, completes a plan for real, has a patch
reject its check, publishes and extends a later plan carrying the corrected
definition, authorizes, and asks `execute-next`. It got
`{"status":"ok","outcome":"complete","phase":37}`. `execution-plan-complete`
stores `occurrence.terminal = Complete` when `phase_complete` holds
(`execution/history.rs:597`); `execution-extend` never clears it;
`execution_service.rs` answers the stored terminal before native selection,
and `dispatch::admit_dispatch` refuses `dispatch-terminal` after that. Phase
34 never met this because its plans were blocked, not complete. Phase 36 is
complete, so its own gap plan, the reason phase 37 exists, would have met
it next.

No plan can carry the fix. The lease is admitted and cannot widen; retiring
the task releases all four checks, and the three that already pass at HEAD
have no red left to give a new owner. This is D-156 by its first clause:
the binary could not record its own repair.

10. `2231fa71` test(execution): a plan admitted after the phase completed
    must dispatch. `tests/execution_terminal_reopen.rs` drives the real
    binary over stdio: plan 1 completes through red, green, attestation,
    close, suite, risk and plan completion; the phase answers complete (the
    control); an artifact-only plan 2 is published, extended at set version
    2 and authorized; `execute-next` must dispatch plan 2 and plan 1 must
    still read complete.
11. `2bacd2a3` fix(execution): a later admission reopens a completed phase.
    `execute-next` consults `history::phase_complete` before answering a
    stored Complete terminal; `admit_dispatch` treats a Complete terminal as
    history for a candidate plan with no retained outcome and clears it as
    the dispatch goes active. Judgment stops are unchanged.

The two commits sit between the executor's red and its completion by
rebase: nothing was pushed. The executor's own check-shape correction
(`698f3e06`) was rebased under them as well, so all four red runs bind to
that commit and all four green runs to the rebased completion `8e8d5e82`.
The close checks the lease only on the evidence commits and the completion,
so the strike between them is what the ancestry rule allows and nothing
more. The executor's earlier runs at commit ids the branch no longer
carries stay in history as observed runs.

Phase 37 then blocked twice on its own suite (plan 1: releasing on the
rejection alone hid the rejection from phase 13; plan 2: eager saved-map
resolution broke an admission unit fixture) and repaired each through a
D-120 gap plan; plan 3 completed and the phase derives executed. The suite
at `f2f9daae` (execution-suite `phase37-plan3-suite`): exit 0, observed by
the binary at plan completion.
