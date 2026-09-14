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

Phase 34 plan 1 is recorded failed on suite `p34-1-suite-20260913`, exit
101, on the four targets commits 3 through 6 fix. Commit 3 is plan 1's own
miss. Phase 34 plan 2 goes through the front door on a green suite.

## The suite

`cargo test --workspace --no-fail-fast` at `7d60cd9c` on 2026-09-13:
exit 0, 52 targets, 941 passed, 0 failed, 1 ignored (the T6 red).

## What it taught

Two gaps, both open, neither decided here.

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

A fix written by hand ran its own tests and never the suite, twice, and it
was the suite at the end that caught it. The gate held because it was kept;
a plan would have run it at close, and the plan is the thing that could not
run.

Candidates for D-158 and D-159 when a phase's context is ready to carry them.
