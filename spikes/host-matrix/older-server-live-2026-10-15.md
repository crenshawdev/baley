# Older-server repeat observations

Claude Code version (claude --version): 2.1.300 (Claude Code)
Platform: Linux 7.2.10-1-cachyos x86_64
Date of the run: 2026-10-15
Newer binary: version 0.1.0 at view set 9, built from the tree `f127e247` of commit `90e02095` (Build 4 T3, which raises the view set to 9)
Older binary: version 0.0.9 at view set 8, built from `main` at `0f023936`, which holds the fix for issue 258 (`a0d66781`), with its version lowered in `crates/baley/Cargo.toml` and `Cargo.lock` so its answers name it
Disposable root: `~/.local/share/baley-scope-live`, with `home` as the disposable `HOME`, `claude-config` as `CLAUDE_CONFIG_DIR` and `projects/two` as the older binary's project

This repeats the older-server rows of `live-install.md`, Parts 1 and 3, at the raise of the view set from 8 to 9, with an older binary that holds the fix for issue 258. It is the repeat the 2026-10-11 run left owed. Three things differ from the procedure:

- The binaries were placed by hand, not with `install.sh`, the way the installer places a download: copied into `versions/<version>/baley` under the disposable `HOME`, the stable path `~/.local/bin/baley` linked to it, then `baley install` run through the stable path. Nothing was downloaded, so no artifact branch was published.
- The older binary is built from `main` and has an `install` command, so its install ends with status 0 instead of the usage error the procedure expects of the first run's older artifact.
- Session `older-s3` was one headless Claude Code process (`claude -p --model haiku --input-format stream-json --output-format stream-json --allowedTools mcp__baley__baley_version,mcp__baley__baley_query,mcp__baley__baley_apply`), fed one request at a time, so one process, and one Baley server, answered every call. With no one to answer a prompt, an `ask` from the hook is not approved.

The same Claude folder had earlier held a version 0.1.0 install for another check. Its Baley home was moved aside before this run, so the ledger below was created by the older binary.

Evidence: `results/older-s3-calls.txt` is every call and answer of the session, taken from its stream output `older-s3/out.jsonl`. `results/ledger-reads.txt` is the read-only `sqlite3` queries of the ledger after the run. Both are under the disposable root.

## Older server, setup

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| old.installer | Place 0.0.9 behind the stable path and run `baley install` through it | Records the per-user records at view set 8 | observed | `install outcome: complete`, `baley 0.0.9 at <stable path>`; the stubs, registration, hook and settings lines read `unchanged`, since the 0.1.0 install had written the same bytes; the view-set catalog held 1, 2 and 8 | the receipt, pasted in the outcome; `results/ledger-reads.txt` == user events, seq 1 to 4 | |
| old.init | `baley init` in project two through `versions/0.0.9/baley` | Exit status 0, a `baley.toml` in project two, `created project <id> in the ledger` and `recorded project.initialized` | pass | exit status 0, `baley.toml` written, created project `b961db5f-da14-4b3f-9bbc-df52f93ade8f`, recorded `project.initialized` | the init output, pasted in the outcome; `results/ledger-reads.txt` == project two events, seq 1 to 2 | |

## Older server, session

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| old.s3.version | Session older-s3, started in project two: call `baley_version` | The answer names version 0.0.9 | pass | `{"status":"ok","version":"0.0.9","os":"linux","arch":"x86_64"}` | `results/older-s3-calls.txt` | |
| old.s3.capture-1 | `baley_apply` capture, request id `17170000-0000-4000-8000-000000000021`, kind note, text `older first` | Recorded in project two (design 0012 section 5, baley update) | pass | status ok, capture id `5ae46461...`, recorded at 13:18:08Z | `results/older-s3-calls.txt`; `results/ledger-reads.txt` == captures, seq 9 | |
| old.relink-base | While older-s3 runs: link the stable path to `versions/0.1.0/baley` and run its `baley install` | Install ends with status 0 and brings `user` to view set 9 | observed | link moved at 13:18:15Z; `install outcome: complete`, exit status 0; the catalog gained 9, `user` moved to generation 1 and project two stayed at 0 | the receipt, pasted in the outcome; `results/ledger-reads.txt` == view_set_catalog, user events seq 5 to 6 | |
| old.s3.keeps | One after another: `baley_version`, capture `17170000-0000-4000-8000-000000000022` with text `older second`, `baley_version` | Both version answers name 0.0.9 and the capture is recorded: the older server keeps answering beside the newer stable path, and install writes only `user`, so the session's own project stays writable (design 0012 section 5, baley update) | pass | 0.0.9 both times; the capture recorded at 13:18:22Z | `results/older-s3-calls.txt`; `results/ledger-reads.txt` == captures, seq 13 | |
| old.s3.hook | In older-s3: Bash `git push origin main` | The newer hook through the stable path answers: a `guard.answered` with outcome `ask` is recorded in `user` after the relink | pass | the call was stopped with the guard's ask text and did not run; `guard.answered` `ask` at 13:18:32Z, after the relink at 13:18:15Z | `results/older-s3-calls.txt`; `results/ledger-reads.txt` == guard.answered in user | |
| old.s3.same-server | The `baley_session` of the two captures | Equal: one server answered both | pass | both `a5d380cd-1aaf-44bc-9f74-3795dc24f477` | `results/ledger-reads.txt` == captures | |
| old.user-refusal | The older binary's own guard, run by hand in project two with a Bash `git push origin main` hook call on its standard input | The guard says it could not record because `user` is read-only for it, and denies the call | pass | `refused: ProjectReadOnly { project: ProjectId("user"), reason: "its live view set version 9 is newer than this binary's 8" }`, then `permissionDecision` `deny`; no new `guard.answered` | the guard output, pasted in the outcome; `results/ledger-reads.txt` == guard.answered in user | |
| old.s3.fenced | `baley scope show` in project two through the stable path (0.1.0), then in older-s3: capture `17170000-0000-4000-8000-000000000023` with text `older third`, then `baley_version` | Once a binary at view set 9 has used project two, the older server can no longer write it: the capture is refused and nothing is recorded (design 0012 section 5, baley update) | pass | project two moved to generation 1; the capture answered `failed`, `ledger-unavailable`, `refused: ProjectReadOnly` on project two, "its live view set version 9 is newer than this binary's 8", `recorded: false`; project two's head stayed at 14; `baley_version` still 0.0.9 | `results/older-s3-calls.txt`; `results/ledger-reads.txt` == project_gen, project two events | |
