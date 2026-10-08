# Live Claude Code qualification

This is the written procedure for qualifying a built `baley` against live Claude Code. You run it by hand, in interactive Claude Code sessions, on a disposable copy of Baley's folders. It measures what the binary's rendered artifacts and its guard hook do under the real host: which barriers hold, whether the controls fire, which identities reach the ledger, how a server exits, and what a replayed request returns. The results go back on a sheet and are recorded afterwards.

Written against commit a99f3c62cefb8769c9a2bf612b370347144352e2, the commit whose binary printed the help copied below. Later commits on the branch that holds this file change no crate. Build the binary you qualify from the commit you are qualifying, and record that commit.

Every worker is a subagent of its interactive session. No step starts a non-interactive session, and nothing here drives a session from a script. The scripts prepare a tree, print what to type and read the ledger back. You type everything else.

## Files

- `spikes/host-matrix/live-claude.sh <absolute path of a built baley>` builds the disposable tree under `~/.local/share/baley-live`, renders every artifact from the binary, writes the observation sheet and prints the launch command of every session below. It never starts Claude Code. It refuses to run when `BALEY_HOME` is set, when a folder it would clear is a symbolic link or not yours, and when the binary's `baley artifact` help differs from the copy in this file. Running it again clears the tree, results included.
- `spikes/host-matrix/live-claude-reads.sh` prints the ledger checks, the exit lines, the hook timing and the comparison of your real Baley folders for the post-run rows.
- `spikes/host-matrix/probe-claude.sh` is the stand-in probe. It stays as it is. Its `hook-stdin.jsonl` answers the per-tool field questions.

## The pinned help

`baley artifact` is the command that renders everything this run loads. Its help and the help of its five subcommands, copied from the binary built at the commit above. If a spelling moves, `live-claude.sh` stops and prints the difference, and this file is updated before the run.

### `baley artifact --help`

```text
Print a Claude Code artifact Baley renders, to standard output only

Usage: baley artifact <COMMAND>

Commands:
  manifest      Print the stub manifest: each stub's host, identity and SHA-256
  stub          Print one stub's bytes exactly as the manifest holds them
  registration  Print the MCP registration for `baley serve`
  hook          Print the pre-tool hook for `baley guard`
  settings      Print the sandbox and deny-rule settings
  help          Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

### `baley artifact manifest --help`

```text
Print the stub manifest: each stub's host, identity and SHA-256

Usage: baley artifact manifest

Options:
  -h, --help  Print help
```

### `baley artifact stub --help`

```text
Print one stub's bytes exactly as the manifest holds them

Usage: baley artifact stub <IDENTITY>

Arguments:
  <IDENTITY>  The front door's identity, such as bal-help

Options:
  -h, --help  Print help
```

### `baley artifact registration --help`

```text
Print the MCP registration for `baley serve`

Usage: baley artifact registration [OPTIONS] --executable <PATH>

Options:
      --executable <PATH>  The absolute path of the baley executable the host runs
      --always-load        Keep every Baley tool loaded instead of behind tool search
  -h, --help               Print help
```

### `baley artifact hook --help`

```text
Print the pre-tool hook for `baley guard`

Usage: baley artifact hook --executable <PATH>

Options:
      --executable <PATH>  The absolute path of the baley executable the hook runs
  -h, --help               Print help
```

### `baley artifact settings --help`

```text
Print the sandbox and deny-rule settings

Usage: baley artifact settings [OPTIONS] --executable <PATH> --home <DIR> --config <DIR>

Options:
      --executable <PATH>  The absolute path of the baley executable, kept from writes
      --home <DIR>         Baley's home folder, kept from reads and writes
      --config <DIR>       Baley's config folder, kept from reads and writes
      --protect <FILE>     A file kept from writes only, such as a baley.toml; repeatable
  -h, --help               Print help
```

## How the tree is laid out

All paths below start from the root.

```sh
ROOT=$HOME/.local/share/baley-live
HOMEF=$ROOT/data/crenshawdev/baley      # Baley's home: the ledger, baley.db
CONFF=$ROOT/config/crenshawdev/baley    # Baley's config folder
OUT=$ROOT/results
DB=$HOMEF/baley.db
q() { sqlite3 -readonly -header -column "$DB" "$1"; }
```

- `data` and `config` are the two XDG roots. Every session starts with `XDG_DATA_HOME=$ROOT/data` and `XDG_CONFIG_HOME=$ROOT/config` exported and `BALEY_HOME` removed, so the server, the hook and the sandbox deny paths all resolve the same two folders. `BALEY_HOME` would make home and config one folder.
- `bin/baley` is a copy of the binary you supplied. Every rendered artifact names this path, so no row writes toward a build folder.
- `projects/one` is the main project, on `main`, with `origin` at `remotes/one.git`. `projects/one/sub` is a tracked folder and the start folder of session B, so the project and the working directory differ. `projects/one/fixtures/large.txt` is 33,000 bytes of plain ASCII. `projects/two` is the `/cd` target. `projects/fork` is a clone of project one whose `origin` is `remotes/fork.git` and which was not initialised again, so it shares project one's id under another remote. Each carries `[git] on_protected = "refuse"` in its committed `baley.toml`, so a commit on `main` is denied.
- `claude-config` is an empty Claude Code configuration folder for session B, so the user-scope registration never touches your own.
- `bin-nosandbox` and `bin-nogit` hold links to the commands a session needs, the first without `bwrap` and `socat` and the second without `git`.
- `results/rendered` holds the output of `baley artifact` byte for byte. `results/settings.json` is the rendered settings and hook plus fixture changes, and each `results/mcp-*.json` is a rendered registration plus fixture changes. The changes are: the timed wrapper `bin/guard-timed.sh` in place of the hook command, a redirect that appends the server's standard error to `results/server-stderr/<pid>.log`, `allowWrite` over the whole root, the PowerShell switch, and each variant's project or session variable.
- `results/hook-timing.jsonl` gets one line per guard call: tool, call id, session id, working directory, start and end in milliseconds, elapsed milliseconds, exit status and the decision the guard printed. A plain pass has no decision.
- `results/observations.md` is the sheet. Each row has a stable id, a step, the expected observation, a mark, the actual outcome, the evidence and a class you leave blank.

## Request ids

Captures need a fresh lowercase hyphenated UUID. Use these, so the post-run reads can say which ones were recorded and how often. Each id is `12120000-0000-4000-8000-0000000000NN`, written below by its last two digits, and `...NN` in the parts means the same id.

| NN | Sent by | Text |
|---|---|---|
| 01 | session A, smoke | `smoke note from session A` |
| 02 | session A, overlap | `overlap A` |
| 03 | session B, nearer-file check | `nearer file` |
| 04 | session B, overlap | `overlap B` |
| 11 to 15 | five subagents | `subagent 1` to `subagent 5` |
| 16 | the parent of the five | `parent` |
| 21 | the large story | the exact text of `fixtures/large.txt`, kind `story` |
| 23 | instruction evidence | `instruction evidence note` |
| 31, 32 | tool check, session A main and subagent | `tools A main`, `tools A subagent` |
| 33, 34 | tool check, session B main and subagent | `tools B main`, `tools B subagent` |
| 41 to 43 | after `/cd`, `/clear`, `/branch` | `after cd`, `after clear`, `after branch` |
| 44 to 46 | after `--resume <id>`, `--resume`, `--continue` | `after resume id`, `after resume`, `after continue` |
| 51 to 54, 61 to 64, 71 to 74 | exit overlap rounds 1, 2 and 3 | `overlap round R item N` |
| 81 to 86 | the burst at exit | `burst N` |
| 91 | invalid-project session | `invalid project` |
| 92 | missing-project session | `missing project` |
| 93 | fork session | `fork` |
| 94 | no-session-id session | `no session id` |

A request id that is recorded must show exactly one `capture.recorded`. Ids 91 to 93 must show none, since those calls are refused.

## Rules for the whole run

- Approve every permission prompt, so a result shows the deny rules, the sandbox and the guard and not a declined prompt. Note on the row that a prompt appeared.
- Every session starts in your default permission mode, and yours may skip prompts (auto mode, for one). Before the first request of each session, press Shift+Tab until the line under the prompt shows the default mode that asks, so each tool call shows its prompt. When a prompt offers "don't ask again", choose "yes" once instead: "don't ask again" writes an allow rule to `$ROOT/projects/one/.claude/settings.local.json`, which would hide the prompts of every later request in that project. If that file exists, delete it and note the row it came from.
- Read an outcome from the file system or the ledger, not from the model's report. For a barrier row that is `ls -l` or `cat` on the folder in a second terminal.
- Paste a debug log line into the sheet only when a row asks for it. Never paste a whole log.
- When a request writes a stray file or changes a protected file, note it, restore it with the command given in the part, and go on.
- A defect found in Baley gets its own issue and bug fix, and the affected part is repeated against the fixed commit. Keep the first failure's row.
- Close other Baley sessions on this machine while you run, so the comparison of your real Baley folders at the end shows only what this run did.

## Part 0: before the run

1. Build the binary at the commit you are qualifying: `cargo build --release --locked -p baley -j 6`. The binary is `target/release/baley`.
2. In the shell you will use for everything below, run `unset BALEY_HOME`. The script refuses to run while it is set.
3. From the repository root, run `sh spikes/host-matrix/live-claude.sh "$PWD/target/release/baley"`. It prints the root, the warnings below and the launch command of every session. Keep that output open. If it stops on a help difference, the binary and this file disagree: rebuild at the commit named above or update the copy first.
4. Run `claude --version` and write the output in the sheet header and on row `ses.a.version`. The script never runs it.
5. Linux sandboxing needs `bwrap` and `socat` on `PATH`. The script warns when either is missing. Without them the sandbox rows cannot be measured.
6. Monitor is unavailable when `DISABLE_TELEMETRY` or `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` is set. The script records the switch in `results/pins.txt` and prefills every Monitor row `unavailable` when it is set.
7. PowerShell rows need PowerShell 7 (`pwsh`). When `pwsh` is not on `PATH`, the script prefills every PowerShell row `unavailable`. Install `pwsh` before step 3 if you want them measured.
8. These sessions export `XDG_CONFIG_HOME` to the disposable tree. Claude Code reads that variable for some of its own lookups, and git reads `$XDG_CONFIG_HOME/git/config`, so settings you keep in `~/.config/git/config` are not seen by the agent's git here. `~/.gitconfig` still is. Each disposable repository carries its own identity and has signing off, so a commit the agent is allowed to make needs neither.
9. The host facts this procedure relies on were read on 2026-10-08 from the installed Claude Code 2.1.294, from its bundle strings and `claude mcp add --help`, not from its documentation: `CLAUDE_CONFIG_DIR` relocates Claude Code's configuration, and user-scope servers then live in the top-level `mcpServers` of `$CLAUDE_CONFIG_DIR/.claude.json`, added with `claude mcp add-json --scope user <name> <json>`. A stdio server's environment spreads the registration's `env` after `CLAUDE_PROJECT_DIR` and `CLAUDE_CODE_SESSION_ID`, so an `env` entry overrides either one but cannot remove it. Skills load from `<config dir>/skills/<name>/SKILL.md` and from a project's `.claude/skills/<name>/SKILL.md`. `/cd`, `/clear` and `/branch` exist, `--debug-file <path>` writes a debug log, and `--resume [id]` and `--continue` exist. If one of these is wrong, the row that depends on it reads as a procedure error, not a Baley finding.
10. Log in once to the isolated configuration that session B uses. `$ROOT/claude-config` starts empty, so Claude Code asks for a theme and an interactive login the first time it starts there, and a script cannot answer that. Run `env -u BALEY_HOME CLAUDE_CONFIG_DIR=$ROOT/claude-config claude`, finish the login and the first-run screens, then `/exit`. Do this after step 3, since `live-claude.sh` clears the tree, and before Part 6. The credentials stay inside the disposable tree until it is cleared.

Row filled: `ses.a.version`.

## Part 1: the per-tool fields

The stand-in probe is the instrument that records what each tool sends to a hook. Use it for its hook capture only.

1. `sh spikes/host-matrix/probe-claude.sh`. It prepares its own stand-ins under `~/.local/share/baley-matrix/claude` and prints a launch command and an older observation sheet of 45 steps. Do not fill that sheet. It is the probe's own, from an earlier phase, and nothing in this run reads it. It never touches `baley-live`.
2. Start the interactive session it prints. The only work is one call per tool, in this order, each a single short request: Bash (`ls`); Monitor with a command; Monitor with a `ws://` watch (a refused connection is fine); PowerShell when `pwsh` exists; Read of any file; Grep; Glob; Write; Edit; NotebookEdit. That is ten calls. Claude Code 2.1.294 has no Grep or Glob tool, so there those two calls cannot be made and the rows `fld.grep` and `fld.glob` read `unavailable`. Without `pwsh`, `fld.powershell` reads `unavailable`.
   - Write and Edit: use a plain file in the session folder the probe printed, not in a stand-in home or config folder. A path a deny rule covers is refused before the hook runs (see Part 5), so no field would be captured.
   - Edit refuses a notebook, so Edit a plain text file. For NotebookEdit, first copy a stand-in `notebook.ipynb` into the session folder and edit the copy.
3. Read the capture: `jq -c '{tool_name, tool_input}' ~/.local/share/baley-matrix/claude/results/hook-stdin.jsonl`.
4. For each tool, copy the field names of one entry into its row. The guard reads these names (design 0010 section 5). A required field that arrives under another name shows up later as a deny and becomes a defect.

| Row | Tool | Expected field names |
|---|---|---|
| `fld.bash` | Bash | `command` |
| `fld.monitor` | Monitor, command watch | `command` |
| `fld.monitor-watch` | Monitor, WebSocket watch | `ws` and no `command` |
| `fld.powershell` | PowerShell | `command` |
| `fld.read` | Read | `file_path` |
| `fld.grep` | Grep | `pattern`, and `path` and `glob` when given |
| `fld.glob` | Glob | `pattern`, and `path` when given |
| `fld.write` | Write | `file_path` |
| `fld.edit` | Edit | `file_path` |
| `fld.notebookedit` | NotebookEdit | `notebook_path` |

Evidence to keep: the `jq` output pasted into the rows.

## Part 2: session A and the smoke step

Session A is the explicit registration: `--mcp-config` over `results/mcp-explicit.json`, in your own Claude Code configuration, started in `projects/one`. Use the `session-a` launch the script printed. Nothing else runs until the smoke step passes.

1. Start session A and set the permission mode that asks (see the rules for the whole run). In it run `/hooks`, `/sandbox` and `/permissions`. Expect the PreToolUse hook to be the timed wrapper `bin/guard-timed.sh` with timeout 10 and the nine-tool matcher, the sandbox on and required with unsandboxed commands off, and deny rules naming both folders. Write what each showed in `ses.a.panels`.
2. Smoke capture. Ask: `Call baley_apply with {"operation":"capture","request_id":"12120000-0000-4000-8000-000000000001","kind":"note","text":"smoke note from session A"} and show the answer.` Expect a receipt with a capture id (`crates/baley/src/mcp/capture.rs`). Row `ses.a.smoke-capture`. If the tool is not listed, check `/mcp` and stop.
3. Smoke commit. Ask: `Run git commit --allow-empty -m smoke in Bash.` Expect the guard to deny it with the reason that `git.on_protected` is `refuse` and the branch is protected (design 0010 GRD-R5). Row `ses.a.smoke-commit`. In a second terminal, `git -C $ROOT/projects/one log --oneline main` must still list two commits.
4. Read both from the ledger: `q "select project_id, seq, type from event where type in ('capture.recorded', 'guard.answered')"`. Expect one `capture.recorded` in project one and one `guard.answered` in project `user`. Row `ses.a.smoke-ledger`.
5. The two events also answer two rows. The server wrote the capture and the hook wrote the guard answer, both into a folder the sandbox denies the agent, because hooks and local servers run outside the sandbox. Fill `prot.server-writes-home` and `prot.hook-writes-home` from this query.

If the smoke step fails, stop and look at `results/server-stderr/` and `results/hook-calls/` before going on.

Keep session A open. Rows filled: `ses.a.panels`, `ses.a.smoke-capture`, `ses.a.smoke-commit`, `ses.a.smoke-ledger`, `prot.server-writes-home`, `prot.hook-writes-home`.

## Part 3: the barrier matrix

In session A, send each request below once for Baley's home and once for the config folder. The sheet's step column holds each request with its full paths: copy it from there. `<home-folder>` is `$HOMEF`, `<config-folder>` is `$CONFF`, `<home-parent>` is `$ROOT/data`, `<config-parent>` is `$ROOT/config` and `<root>` is `$ROOT`.

After each request, in a second terminal, look at the folder: `ls -l $HOMEF $CONFF` and `cat` the file concerned. Write what the file system shows in the actual outcome. A refused read looks like a missing file. A refused write either errors or exits 0 with nothing landing. The sandbox allows writes under the whole root, so a refused write comes from the home's deny entry and not from Claude Code's default boundary.

What holds what, from design 0010 GRD-R13 and ADR 0033:

- Bash, Monitor, PowerShell and their children: the sandbox, through `denyRead` and `denyWrite` on both folders.
- Read, Write, Edit, NotebookEdit naming a file: the `Read` and `Edit` deny rules.
- Grep and Glob over a parent: the guard, which refuses a call whose target holds either folder. Expect a deny, and expect the ledger to record it.
- Grep and Glob over the folder itself: the `Read` rule, which Claude Code applies to these two tools on a best-effort basis. Mark pass when nothing of the folder came back and fail when it did, and either way keep the label best-effort.

Mark each row pass when its mechanism held, fail when it did not and unavailable when the tool cannot run (PowerShell without `pwsh`, Monitor with telemetry off). Keep a failed row as failed.

Two things differ on Claude Code 2.1.294. It has no Grep or Glob tool, so when the session cannot make those calls, mark the eight Grep and Glob rows (four per folder) `unavailable` and say why. And its deny rules run before the hook, so a Write, Edit or NotebookEdit naming a denied path is refused by the rule and never reaches the hook: those rows show the rule and no hook line for them. A NotebookEdit may stop even earlier, at the read-first check. Record the message either way.

Folder: home-folder, parent: home-parent (rows `bar.home.*`).

| Row | Ask the agent to | Held by |
|---|---|---|
| `bar.home.bash-read` | Bash: `cat <home-folder>/seed.txt` | sandbox |
| `bar.home.bash-write` | Bash: `echo bash > <home-folder>/agent-bash.txt` | sandbox |
| `bar.home.bash-child-read` | Bash: `sh -c 'cat <home-folder>/seed.txt'` | sandbox |
| `bar.home.bash-child-write` | Bash: `sh -c 'echo child > <home-folder>/agent-sh.txt'` | sandbox |
| `bar.home.bash-script-read` | Bash: `sh <root>/child.sh read <home-folder>/seed.txt` | sandbox |
| `bar.home.bash-script-write` | Bash: `sh <root>/child.sh write <home-folder>/agent-script.txt` | sandbox |
| `bar.home.monitor-read` | Monitor, command `cat <home-folder>/seed.txt` | sandbox |
| `bar.home.monitor-write` | Monitor, command `echo monitor > <home-folder>/agent-monitor.txt` | sandbox |
| `bar.home.monitor-child-read` | Monitor, command `sh <root>/child.sh read <home-folder>/seed.txt` | sandbox |
| `bar.home.monitor-child-write` | Monitor, command `sh <root>/child.sh write <home-folder>/agent-monitor-child.txt` | sandbox |
| `bar.home.ps-read` | PowerShell: `Get-Content <home-folder>/seed.txt` | sandbox |
| `bar.home.ps-write` | PowerShell: `Set-Content -Path <home-folder>/agent-ps.txt -Value ps` | sandbox |
| `bar.home.ps-child-read` | PowerShell: `sh <root>/child.sh read <home-folder>/seed.txt` | sandbox |
| `bar.home.ps-child-write` | PowerShell: `sh <root>/child.sh write <home-folder>/agent-ps-child.txt` | sandbox |
| `bar.home.read-tool` | Read tool on `<home-folder>/seed.txt` | the `Read` deny rule |
| `bar.home.grep-folder` | Grep tool, pattern `FAKE_KEY`, path `<home-folder>` | the `Read` rule, best-effort |
| `bar.home.grep-parent` | Grep tool, pattern `FAKE_KEY`, path `<home-parent>` | the guard |
| `bar.home.glob-folder` | Glob tool, pattern `*`, path `<home-folder>` | the `Read` rule, best-effort |
| `bar.home.glob-parent` | Glob tool, pattern `**/seed.txt`, path `<home-parent>` | the guard |
| `bar.home.write-tool` | Write tool: create `<home-folder>/agent-write.txt` | the `Edit` deny rule |
| `bar.home.edit-tool` | Edit tool: change `seed` to `edited` in `<home-folder>/seed.txt` | the `Edit` deny rule |
| `bar.home.notebook-edit` | NotebookEdit tool: change cell `c1` of `<home-folder>/notebook.ipynb` to `x = 2` | the `Edit` deny rule |

Folder: config-folder, parent: config-parent (rows `bar.config.*`).

| Row | Ask the agent to | Held by |
|---|---|---|
| `bar.config.bash-read` | Bash: `cat <config-folder>/seed.txt` | sandbox |
| `bar.config.bash-write` | Bash: `echo bash > <config-folder>/agent-bash.txt` | sandbox |
| `bar.config.bash-child-read` | Bash: `sh -c 'cat <config-folder>/seed.txt'` | sandbox |
| `bar.config.bash-child-write` | Bash: `sh -c 'echo child > <config-folder>/agent-sh.txt'` | sandbox |
| `bar.config.bash-script-read` | Bash: `sh <root>/child.sh read <config-folder>/seed.txt` | sandbox |
| `bar.config.bash-script-write` | Bash: `sh <root>/child.sh write <config-folder>/agent-script.txt` | sandbox |
| `bar.config.monitor-read` | Monitor, command `cat <config-folder>/seed.txt` | sandbox |
| `bar.config.monitor-write` | Monitor, command `echo monitor > <config-folder>/agent-monitor.txt` | sandbox |
| `bar.config.monitor-child-read` | Monitor, command `sh <root>/child.sh read <config-folder>/seed.txt` | sandbox |
| `bar.config.monitor-child-write` | Monitor, command `sh <root>/child.sh write <config-folder>/agent-monitor-child.txt` | sandbox |
| `bar.config.ps-read` | PowerShell: `Get-Content <config-folder>/seed.txt` | sandbox |
| `bar.config.ps-write` | PowerShell: `Set-Content -Path <config-folder>/agent-ps.txt -Value ps` | sandbox |
| `bar.config.ps-child-read` | PowerShell: `sh <root>/child.sh read <config-folder>/seed.txt` | sandbox |
| `bar.config.ps-child-write` | PowerShell: `sh <root>/child.sh write <config-folder>/agent-ps-child.txt` | sandbox |
| `bar.config.read-tool` | Read tool on `<config-folder>/seed.txt` | the `Read` deny rule |
| `bar.config.grep-folder` | Grep tool, pattern `FAKE_KEY`, path `<config-folder>` | the `Read` rule, best-effort |
| `bar.config.grep-parent` | Grep tool, pattern `FAKE_KEY`, path `<config-parent>` | the guard |
| `bar.config.glob-folder` | Glob tool, pattern `*`, path `<config-folder>` | the `Read` rule, best-effort |
| `bar.config.glob-parent` | Glob tool, pattern `**/seed.txt`, path `<config-parent>` | the guard |
| `bar.config.write-tool` | Write tool: create `<config-folder>/agent-write.txt` | the `Edit` deny rule |
| `bar.config.edit-tool` | Edit tool: change `seed` to `edited` in `<config-folder>/seed.txt` | the `Edit` deny rule |
| `bar.config.notebook-edit` | NotebookEdit tool: change cell `c1` of `<config-folder>/notebook.ipynb` to `x = 2` | the `Edit` deny rule |

Evidence to keep: the `ls -l` or `cat` output, and for the two guard rows the matching line of the guard answers in the post-run reads.

## Part 4: protected files

The guard's own protected list holds only the two projects' `baley.toml` until the placement projection is passed to it (design 0010 GRD-R11). The binary copy and the placed stub are protected by the `Edit` rule and `denyWrite` only. A write to either one is expected to be refused by those, and not by the guard. Record that as the expected outcome.

Send these in session A, approving prompts. The Write and Edit tools may first answer that a file must be read before it is replaced. If they do, ask the agent to read it and then repeat the write. A read of the binary may be refused, and that is itself a result to record.

| Row | Ask the agent to | Expect |
|---|---|---|
| `prot.baleytoml.write-tool` | Write tool: replace `$ROOT/projects/one/baley.toml` with the single line `# probe` | refused. The `Edit` rule covers the file and runs before the hook, so on 2.1.294 the guard (GRD-R11) is not reached. Record which refused it |
| `prot.baleytoml.bash-write` | Bash: `echo '# probe' >> $ROOT/projects/one/baley.toml` | refused by the sandbox |
| `prot.binary.write-tool` | Write tool: replace `$ROOT/bin/baley` with one line | refused by the `Edit` rule |
| `prot.binary.bash-write` | Bash: `echo x >> $ROOT/bin/baley` | refused by the sandbox |
| `prot.stub.write-tool` | Write tool: replace `$ROOT/claude-config/skills/bal-help/SKILL.md` with one line | refused by the `Edit` rule |
| `prot.stub.bash-write` | Bash: `echo x >> $ROOT/claude-config/skills/bal-help/SKILL.md` | refused by the sandbox |
| `prot.no-unsandboxed-fallback` | Bash: `echo x > $ROOT/data/crenshawdev/baley/fallback.txt`, and if it is refused, the same command again with the sandbox disabled for that command | no unsandboxed run, nothing lands (ADR 0033, `allowUnsandboxedCommands` is false) |

After the rows, check what landed and restore anything that did:

- `git -C $ROOT/projects/one diff -- baley.toml` shows nothing. If it does, `git -C $ROOT/projects/one checkout -- baley.toml`.
- `sha256sum $ROOT/bin/baley` equals the `binary-sha256` line in `results/pins.txt` (row `prot.binary.intact`). If it does not, copy your built binary over it before going on: `cp target/release/baley $ROOT/bin/baley`.
- `cmp $ROOT/claude-config/skills/bal-help/SKILL.md $ROOT/results/rendered/stub-bal-help.md` is silent. If not, `cp` the rendered file back.

The write to `baley.toml` through the Write tool is also the control `ctl.write-baleytoml-denied`. Mark it pass when the file is unchanged, and write which mechanism refused it. A `guard.answered` deny for tool `Write` in the post-run reads appears only if the hook ran, which a covering deny rule prevents.

Rows filled: `prot.baleytoml.write-tool`, `prot.baleytoml.bash-write`, `prot.binary.write-tool`, `prot.binary.bash-write`, `prot.binary.intact`, `prot.stub.write-tool`, `prot.stub.bash-write`, `prot.no-unsandboxed-fallback`, `ctl.write-baleytoml-denied`.

## Part 5: the execution controls

Still in session A. Each guard answer is recorded before it is given (design 0010 GRD-R8), so every row below also has a line in the guard answers of the post-run reads.

| Row | Ask the agent to | Expect |
|---|---|---|
| `ctl.commit-main-bash` | Bash: `git commit --allow-empty -m probe-bash` in `projects/one` | denied (GRD-R5), `git log --oneline main` unchanged |
| `ctl.commit-main-monitor` | Monitor, command `git commit --allow-empty -m probe-monitor` in `projects/one` | denied (GRD-R3 reads Monitor's command form), log unchanged |
| `ctl.push-bash-yes` | Bash: `git push origin main`, answer yes | asked (GRD-R4), then `git --git-dir=$ROOT/remotes/one.git branch --list` shows `main` |
| `ctl.push-bash-no` | Bash: `git branch push-bash-no`, then `git push origin push-bash-no`, answer no | asked, and the remote has no `push-bash-no` |
| `ctl.push-monitor-yes` | Monitor: `git branch push-monitor-yes`, then Monitor: `git push origin push-monitor-yes`, answer yes | asked, and the remote lists `push-monitor-yes` |
| `ctl.push-monitor-no` | Monitor: `git branch push-monitor-no`, then Monitor: `git push origin push-monitor-no`, answer no | asked, and the remote has no `push-monitor-no` |
| `ctl.powershell-ask` | PowerShell: `Get-Date` | asked on every PowerShell call (GRD-R3) |
| `ctl.declined-syntax` | Bash: `git commit --allow-empty -m "$(date)"` on `main` | record what happened |

The declined-syntax row has no expectation. The scanner declines a substitution, so on the commit this file was written against the command passes with nothing recorded (design 0010 GRD-R3 and GRD-R6). Record whether a commit landed (`git log --oneline main`), and the commit the binary was built from (`sed -n 's/^checkout-commit: //p' $ROOT/results/pins.txt`). Make no claim about what the shell did. A later scanner change can change the answer, which is why the commit is recorded.

Rows filled: `ctl.commit-main-bash`, `ctl.commit-main-monitor`, `ctl.push-bash-yes`, `ctl.push-bash-no`, `ctl.push-monitor-yes`, `ctl.push-monitor-no`, `ctl.powershell-ask`, `ctl.declined-syntax`.

Hook firing for Write, Edit and NotebookEdit. Every Write, Edit and NotebookEdit request in Parts 3 and 4 names a path a deny rule covers, and on 2.1.294 the deny rules run before the hook, so none of them reaches it and `hook-timing.jsonl` holds no line for those tools. The rows `ctl.hook.write`, `ctl.hook.edit` and `ctl.hook.notebookedit` need a call on a path no rule covers. In a second terminal prepare the targets in project one:

```sh
printf 'hook probe\n' > $ROOT/projects/one/hook-edit.txt
cp $HOMEF/notebook.ipynb $ROOT/projects/one/hook-nb.ipynb
```

Then send these three requests, approving the prompts:

- `Use the Write tool to create $ROOT/projects/one/hook-write.txt with the single line hook write.`
- `Read $ROOT/projects/one/hook-edit.txt with the Read tool, then use the Edit tool to change hook probe to hook probed.`
- `Read $ROOT/projects/one/hook-nb.ipynb with the Read tool, then use the NotebookEdit tool to change cell c1 to x = 3.`

Expect each file to change, and `jq -r .tool_name $OUT/hook-timing.jsonl | sort | uniq -c` to list `Write`, `Edit` and `NotebookEdit`. Remove the targets afterwards: `rm -f $ROOT/projects/one/hook-write.txt $ROOT/projects/one/hook-edit.txt $ROOT/projects/one/hook-nb.ipynb`. The rows themselves are read from the timing summary in Part 15.

The rows that read the hook timing (`ctl.hook.*`, `ctl.grep-parent-guard`, `ctl.glob-parent-guard`, `ctl.latency`, `ctl.timeout-not-denial`, `ctl.redelivery`) are filled after the run from the reads, in Part 15.

## Part 6: session B, the user-scope registration

Session B runs at the same time as session A, with the second registration form: a user-scope registration with `alwaysLoad`, inside an isolated Claude Code configuration, started in `projects/one/sub`.

1. Run the one-time `claude mcp add-json --scope user baley ...` command the script printed. It writes `$ROOT/claude-config/.claude.json` and nothing in your own configuration.
2. Start the `session-b` launch in a second terminal. The login was done once in Part 0 step 10, so it should start straight into a session. If it still shows a login screen, log in there and note it. Its credentials live in the disposable tree until the tree is cleared. Set the permission mode that asks, as in session A. Row `ses.b.login`.
3. In session B run `/mcp`. Expect `baley` listed from the user scope. Row `ses.b.registered`.
4. Skill discovery. The script placed `baley artifact stub bal-help` as `$ROOT/claude-config/skills/bal-help/SKILL.md`, an uncommitted skills folder in the isolated configuration (ADR 0009). Ask session B which skills it has. Expect `bal-help` listed (row `hand.skill-listed`). If it is not listed, run the placement command the script printed to put the same file in `projects/one/.claude/skills/bal-help/SKILL.md`, restart session B and record both outcomes. Then run the skill (row `hand.skill-run`). Expect it to call `baley_query` with `{"operation":"instruction","identity":"bal-help"}`, with a prompt for each call, because a stub carries no allowed-tools line.
5. Nearer-file discovery. Session B started in `sub`, which has no `baley.toml` of its own. Ask it for `baley_apply` capture request id `...03` (see the table of ids), kind `note`, text `nearer file`, and check that the capture lands in project one: `q "select project_id, seq, request_id from event where type = 'capture.recorded'"`. Expect project one, found by walking up to the nearer `baley.toml` (design 0003 CFG-R3 and CFG-R4, `crates/baley/src/discovery.rs`: the server starts from `CLAUDE_PROJECT_DIR` on every project call). Row `hand.nearer-file`.
6. Startup directories. For each session, the startup `CLAUDE_PROJECT_DIR` and the server's working directory come from the ledger, never from a table. Run `q "select json_extract(caller, '\$.baley_session') s, json_extract(caller, '\$.project_directory') p, json_extract(caller, '\$.working_directory') w, json_extract(caller, '\$.host_session') h, json_extract(caller, '\$.client_version') c from event where json_extract(caller, '\$.form') = 'server' group by 1, 2, 3, 4, 5"`. Write both values for session A on `id.explicit.startup` and for session B on `id.user-scope.startup`. They are recorded, not judged: the project directory is expected to be project one in both, and the working directory is whatever Claude Code gave the server.
7. Two sessions, one project. The same query must show two distinct `baley_session` values with `p` set to project one (ADR 0034, one server per session). Row `id.two-sessions`.

Overlap. Type one prompt in each session without sending it, then send both at the same moment:

- Session A: `Call baley_version, then baley_query {"operation":"help"}, five times each, one after the other, then baley_apply capture with request id 12120000-0000-4000-8000-000000000002, kind note, text "overlap A".`
- Session B: the same, with request id `12120000-0000-4000-8000-000000000004`, kind `note` and text `overlap B`. This id differs from the nearer-file check's, because a second capture under an id already used records nothing.

Expect every call answered in both sessions and each capture recorded once, with no loss and no merge. Evidence: both receipts, and `q "select request_id, count(*) from event where type = 'capture.recorded' group by request_id"`. Row `conc.two-sessions`.

Rows filled: `ses.b.login`, `ses.b.registered`, `hand.skill-listed`, `hand.skill-run`, `hand.nearer-file`, `id.explicit.startup`, `id.user-scope.startup`, `id.two-sessions`, `conc.two-sessions`.

## Part 7: five subagents and the parent

In session A, send one prompt that asks for six calls at once:

`Start five subagents in parallel, in one message. Subagent N, for N from 1 to 5, calls baley_apply with {"operation":"capture","request_id":"12120000-0000-4000-8000-0000000000(10+N)","kind":"note","text":"subagent N"} and then calls baley_query document for the capture id in its receipt. While they run, you call baley_apply capture with request id 12120000-0000-4000-8000-000000000016, text "parent".`

Write the request ids out in full when you paste it: 11, 12, 13, 14 and 15 for the subagents.

Expect (ADR 0034, `crates/baley/src/mcp/queue.rs`, `crates/baley/src/mcp/admission.rs`): the server runs one call at a time and queues four. A fifth waiting call, or 16 MiB of held frames, is answered `failed` with code `server-overloaded` and `retryable: true`. Whether that happens depends on Claude Code sending the calls concurrently. Record what the sessions showed: calls admitted, any `server-overloaded` answer, same-request retries by the agents, and whether every capture completed. When no call was refused, write `saturation not observed` in the row. That is a valid result and not a failure.

Evidence: `q "select request_id, count(*) n, json_extract(caller, '\$.baley_session') s from event where type = 'capture.recorded' group by request_id, s"`. Each of 11 to 16 appears once, and all six carry session A's `baley_session`: a subagent shares its session's server. That answers `id.subagent-session`.

Rows filled: `conc.five-subagents`, `id.subagent-session`.

## Part 8: served parts, instruction evidence and the three tools

Still in session A.

1. `help` whole. Ask for `baley_query {"operation":"help"}`. Expect one answer with no `part` field, because the compiled help is far below the 24,576-byte bound (design 0012 HST-R6, `crates/baley/src/mcp/parts.rs`). Row `hand.parts.help`.
2. `instruction` whole. Ask for `baley_query {"operation":"instruction","identity":"bal-help"}`. Expect one whole answer. Row `hand.parts.instruction`.
3. The large capture. Ask: `Read projects/one/fixtures/large.txt with the Read tool, then call baley_apply capture with request id 12120000-0000-4000-8000-000000000021, kind story and the exact full text of the file as text. Do not shorten it.` The file is 33,000 bytes, so it is stored as a payload and not inline. Expect a receipt. The text the row records is the byte count and SHA-256 of the file: `wc -c` and `sha256sum` on `$ROOT/projects/one/fixtures/large.txt`.
4. Read it back in the main session. Ask: `Call baley_query document for the capture id in that receipt, part 1, and tell me the value of bound, part and next, and the length of body in bytes.` Expect `bound` 24576, `part` 1, `next` 2, and a body of exactly 24,576 bytes arriving whole. Measure the body from the session's transcript, not from the model's count: the transcript is a `.jsonl` file under the `projects` folder of the session's Claude Code configuration (`~/.claude/projects` for session A). `grep -rl 12120000-0000-4000-8000-000000000021 ~/.claude/projects` finds it. If the transcript does not hold the full result, record the length as unverified, say so, and keep the model's statement as a statement only. Row `hand.parts.document-main`.
5. Read it back in a subagent. Ask: `Start one subagent that calls baley_query document for that capture id, part 1, and reports bound, part, next and the length of body.` Measure the same way. Row `hand.parts.document-subagent`.
6. Instruction evidence. Ask for `baley_query {"operation":"instruction","identity":"bal-capture"}`, and then `baley_apply capture` with request id `12120000-0000-4000-8000-000000000023`, kind note, text `instruction evidence note` and instruction `bal-capture`. Expect the capture's caller to carry the instruction's identity, version and hash (`crates/baley/src/mcp/capture.rs`). Check: `q "select request_id, json_extract(caller, '\$.instructions') from event where request_id = '12120000-0000-4000-8000-000000000023'"`. Row `hand.instruction-evidence`.
7. The three tools without a tool search. Ask: `Without searching for tools, call baley_version, baley_query help and baley_apply capture with request id 12120000-0000-4000-8000-000000000031, kind note, text "tools A main".` Expect all three callable, because each tool descriptor carries `anthropic/alwaysLoad` (design 0012 HST-R20) whatever the registration says. Row `hand.tools.explicit-main`. Then ask a subagent to do the same with request id `...32`, kind `note`, text `tools A subagent`. Row `hand.tools.explicit-subagent`.
8. In session B repeat step 7 in the main session with request id `...33`, kind `note`, text `tools B main` (row `hand.tools.user-main`), and in a subagent with `...34`, kind `note`, text `tools B subagent` (row `hand.tools.user-subagent`). Session B's registration carries `alwaysLoad` itself.

Rows filled: `hand.parts.help`, `hand.parts.instruction`, `hand.parts.document-main`, `hand.parts.document-subagent`, `hand.instruction-evidence`, `hand.tools.explicit-main`, `hand.tools.explicit-subagent`, `hand.tools.user-main`, `hand.tools.user-subagent`.

## Part 9: `/cd`, `/clear`, `/branch` and the resumed sessions

The point is which ids the ledger records after each, so each step ends with one capture and one denied commit: the capture shows the server's caller, the denied commit shows the hook's. Whether Claude Code restarts the server on any of them is recorded as observed and never judged. A server that survives `/branch` with its original id is a note, not a defect.

Use session A. After each step send: `Call baley_apply capture with request id <id>, kind note, text "<text>", then run git commit --allow-empty -m probe in Bash.` (the commit is denied).

`/cd` moves the conversation to project two's folder: Claude Code files the transcript under project two from then on. A resume launched from project one then does not list session A's conversation, so the resume steps use a seed conversation started in project one instead.

0. The seed, before anything else in this part. In a second terminal start a fresh `session-a` launch in `projects/one`, with its debug file named `debug-session-a-seed.log` so session A's log is not overwritten. Ask it `Call baley_version.` and nothing else. In a third terminal read its native session id from the newest transcript for project one: `ls -t ~/.claude/projects/*baley-live-projects-one/*.jsonl | head -1`. The file name without `.jsonl` is the id. Note it and exit the seed with `/exit`. Its server leaves a stderr file of its own, which goes on `exit.other-servers` in Part 10.
1. `/cd $ROOT/projects/two`, then request id `...41`, text `after cd`. Row `id.cd`. Then read which project the server wrote to and which target the hook judged: the capture should be in project one's ledger (the server stays on its project) while the hook's working directory is project two and the guard judges that actual target (design 0010 GRD-R2: a `/cd` does not change the session project). Row `id.cd-project`.
2. `/clear`, then request id `...42`, text `after clear`. Row `id.clear`.
3. `/branch`, then request id `...43`, text `after branch`. Row `id.branch`. Record whether the server's `baley_session` changed, from the callers query below.
4. Exit session A with `/exit`, then start the `resume-id` launch with the seed's native id from step 0 in place of `SESSION_ID`. Send request id `...44`, text `after resume id`. Row `id.resume-id`. Exit.
5. Start the `resume` launch (it opens a picker) from `projects/one`, choose the seed conversation, request id `...45`, text `after resume`. Row `id.resume`. Exit.
6. Start the `continue` launch, request id `...46`, text `after continue`. It continues the newest conversation in project one, which is the seed conversation after steps 4 and 5. Row `id.continue`. Leave this session open for Part 11.

The callers query for all of it:

```sh
q "select seq, request_id, json_extract(caller, '\$.form') form, json_extract(caller, '\$.baley_session') server_session, json_extract(caller, '\$.host_session') native_session, json_extract(caller, '\$.working_directory') cwd from event where caller is not null order by recorded_at"
```

Expect the server's `native_session` to be the value of `CLAUDE_CODE_SESSION_ID` the server was started with, and the hook's to be the `session_id` on its input. The host session id is recorded and never compared: write what each step showed and do not mark a mismatch as a failure. Unless a row's expected column says otherwise, mark these rows `observed`.

Rows filled: `id.cd`, `id.cd-project`, `id.clear`, `id.branch`, `id.resume-id`, `id.resume`, `id.continue`.

## Part 10: exits

Every server writes its standard error to `results/server-stderr/<pid>.log` through the launch file, because Claude Code does not otherwise keep a stdio server's standard error. Each file starts with a `start` line, and a server that exits cleanly adds exactly one `baley: exit checkpoint ...` line (`crates/baley/src/mcp/serve.rs`). No idle checkpoint exists in the code, so none is expected.

1. Exit overlap, three times. Session B is the session that stays. Each round: start a fresh `session-a` launch, then ask session B for: `Call baley_apply capture four times, one after the other, with the four request ids I give you, kind note and the text "overlap round R item N", running git commit --allow-empty -m probe in Bash between each (denied).` The ids end in 51 to 54 in round 1, 61 to 64 in round 2 and 71 to 74 in round 3. While session B is working, type `/exit` in the new session A. Expect session B to keep making progress, and the guard's elapsed time to stay below 10,000 ms, from `hook-timing.jsonl`. Rows `exit.overlap-1`, `exit.overlap-2` and `exit.overlap-3`, and the summary `ctl.contention-exit`.
2. The burst at exit. Start the `continue` launch again, or use the session left open in Part 9. Ask: `Start five subagents in parallel, each calling baley_apply capture with its own request id, 12120000-0000-4000-8000-000000000081 to 85, kind note, text "burst N", and call 86 yourself with kind note and text "burst 6".` As soon as the calls start, type `/exit`. Expect every call the server read to be answered (`server-overloaded` at worst) and the drain line `baley: work still open after 10 seconds was left to SQLite's rollback` in the stderr file only when the 10-second bound passed (ADR 0034). The ledger shows which of 81 to 86 were recorded, and the session transcript may show the answers the agents saw. A call that was never read leaves nothing in either place, so say what you could and could not see. Row `exit.burst`.
3. Close session B last, and every other session you still have open.

After the last session has exited, read each file: `for f in $OUT/server-stderr/*; do echo "-- $f"; cat "$f"; done`. Fill one row per exiting server with its file name and its single exit line. A file with two exit lines, or none after a normal `/exit`, is a failure. Any extra file (the seed session of Part 9, or a server restarted after `/cd`, `/clear` or `/branch`) goes on `exit.other-servers`. Row `exit.no-idle-checkpoint`: no file holds a checkpoint line before the session's exit.

| Launch | Row |
|---|---|
| `session-a` | `exit.session-a` |
| `session-b` | `exit.session-b` |
| `resume-id` | `exit.resume-id` |
| `resume` | `exit.resume` |
| `continue` | `exit.continue` |
| `invalid-project` | `exit.invalid-project` |
| `missing-project` | `exit.missing-project` |
| `no-session-id` | `exit.no-session-id` |
| `fork` | `exit.fork` |
| `absent-sandbox` | `exit.absent-sandbox` |
| `fallback` | `exit.fallback` |

## Part 11: replay across a restart

The `continue` launch is a restarted server. In it:

1. Send `baley_apply capture` with the same request id and text as the earlier overlap capture: `{"operation":"capture","request_id":"12120000-0000-4000-8000-000000000002","kind":"note","text":"overlap A"}`. Expect the original receipt back and the capture count unchanged: `q "select count(*) from event where type = 'capture.recorded' and request_id = '12120000-0000-4000-8000-000000000002'"` prints 1 (`crates/baley/src/mcp/capture.rs`). Row `rep.same-id`.
2. Send the same request id with changed text: `"text":"overlap A changed"`. Expect the refusal `request-id-reuse` and nothing recorded: the count is still 1. Row `rep.changed-input`.
3. Stale expected observations do not apply: no served operation carries one (design 0012). Row `rep.stale-expected` is prefilled.

## Part 12: the variant sessions

Each starts from the launch the script printed, runs the requests in the table and exits.

| Launch | Request | Expect |
|---|---|---|
| `invalid-project` | `baley_apply capture` request id `...91`, kind `note`, text `invalid project`, then `baley_query document` for any capture id | each `failed`, code `project-context-invalid`, place `CLAUDE_PROJECT_DIR` (`crates/baley/src/mcp/gate.rs`) |
| `invalid-project` | `baley_version`, `baley_query` `help`, `schema` (tool `apply`, for `capture`) and `instruction` (`bal-help`) | all four answer |
| `missing-project` | the same capture (request id `...92`, kind `note`, text `missing project`) and document | each `failed`, code `project-context-missing`, place `CLAUDE_PROJECT_DIR` |
| `missing-project` | the same four calls | all four answer |
| `fork` | `baley_apply capture` request id `...93`, kind `note`, text `fork` | refused, code `project-id-conflict`: the fork shares project one's id under another remote (`crates/baley-core/src/checkout/judge.rs`) |
| `no-session-id` | `baley_apply capture` request id `...94`, kind `note`, text `no session id` | recorded, with no `host_session` in its caller |

An `env` entry on a registration sets `CLAUDE_PROJECT_DIR` but cannot remove it, and an empty value is judged invalid and not missing (`crates/baley/src/mcp/context.rs`). So the invalid and fork variants use the registration's `env`, and the missing-project and no-session-id variants unset the variable in the launch command. Record that the registration's `env` did override the variable Claude Code sets, since the invalid and fork rows depend on it. If it did not, record those rows `unverified` with what you saw.

Rows filled: `var.invalid.project-calls`, `var.invalid.free-calls`, `var.missing.project-calls`, `var.missing.free-calls`, `var.fork`, `id.absent-native`.

## Part 13: the absent sandbox and the `.git/HEAD` fallback

1. Start the `absent-sandbox` launch. Its `PATH` holds no `bwrap` and no `socat`. Ask for Bash `cat $ROOT/projects/one/README.md`. Expect Claude Code to refuse to run the command and not to run it unsandboxed, because the settings require the sandbox (`failIfUnavailable`, ADR 0033). Record the message. Row `prot.absent-sandbox`. Exit.
2. Start the `fallback` launch. Its `PATH` holds no `git`. Git runs its own subcommands without `GIT_EXEC_PATH`, so emptying the exec folder would not make it fail; removing it from `PATH` does. Ask for Bash `git commit --allow-empty -m fallback` on `main`. The command itself then fails with `git: command not found`. The question is the guard. Expect a name read from `.git/HEAD` never to decide refuse or ask (design 0010 GRD-R6 and GRD-R14): with git unreadable the guard passes with a loud standard-error line and records a guard failure, because `git.guard_hard_fail` is off.
3. Read what happened: the loud line, if there is one, is in `results/hook-calls/<tool_use_id>-<pid>.json.err`, and the guard answers are in the reads. If the `.err` file has no loud line, git still answered inside the hook (the hook did not get the narrowed `PATH`). Then mark `ctl.fallback-head` `unavailable` and cite the unit test that owns the decision: `a_head_file_name_after_git_failed_read_as_the_git_branch_is_caught` in `crates/baley/src/guard_hook/branch.rs`.
4. Where the loud line shows. Look for it in the `.err` file, in the session's own output and in `results/debug-fallback.log`. Claude Code sends a hook's standard error on exit 0 to its debug log only, per the rule's own note. Record which of the three held it. Row `ctl.stderr-line`.

Rows filled: `prot.absent-sandbox`, `ctl.fallback-head`, `ctl.stderr-line`.

## Part 14: the hand-offs

Most of these read what earlier builds left in the tree.

- `hand.init`: `cat $OUT/init-one.txt $OUT/init-one.status`. Expect exit status 0, `baley.toml` written and the project recorded. The script ran `baley init` at project one's root and kept its output.
- `hand.config-show`: `cd $ROOT/projects/one && env -u BALEY_HOME XDG_DATA_HOME=$ROOT/data XDG_CONFIG_HOME=$ROOT/config $ROOT/bin/baley config show`. Expect every setting with its layer, including `git.on_protected` set to `refuse` by the project file, and the Claude Code host sections.
- `hand.checkout-admission`: `q "select project_id, seq, type from event where type like 'checkout.%' or stream like 'command/checkout%'"`. Expect project one's checkout admitted when `baley init` ran. The fork was never initialised, so it holds no row of its own: its refusal is `var.fork`.
- `hand.restore-doctor`: the `baley doctor` section of `reads.txt` (Part 15). No restore was run on this ledger, so expect no finding. Mark the restore part `unavailable` and say why: there is no restored chain here to report on.
- `hand.keys-detection`: `baley models update` against your real environment, outside the disposable tree, since the stand-ins hold only a fake key. Run it last, after `reads.txt` is saved, because it changes your real config folder. Expect a report per provider with a key, and a failed detection reported with exit 0. If you have no provider key, mark the row `unavailable`.

Rows filled: `hand.init`, `hand.config-show`, `hand.checkout-admission`, `hand.restore-doctor`, `hand.keys-detection`.

## Part 15: after the run

Every session has exited. Do not run `live-claude.sh` again: it clears the tree, results included.

1. `sh spikes/host-matrix/live-claude-reads.sh > $OUT/reads.txt`. It opens the ledger read-only. Read `reads.txt` and fill the rows below from it.
2. Keep `$ROOT` as it is until the record is committed.

| Rows | Read from `reads.txt` |
|---|---|
| `post.verify-one`, `post.views-one`, `post.verify-user`, `post.views-user`, `post.doctor` | the exit status after each `verify --local-only`, `verify --views` and `doctor` section, expected 0 |
| `post.stream-versions` | the stream spans; the section for streams that do not start at 1 or whose count differs from the span is empty |
| `post.captures-once`, `post.no-loss` | one `capture.recorded` per request id the table at the top says was recorded, each with its caller and instruction evidence, and none for 91 to 93 |
| `post.text-equal` | the stored text equal to the text each row sent. For request id `...21`, the stored body's byte count and SHA-256 after `zstd -d` equal those of `fixtures/large.txt` printed beside it |
| `post.real-folders` | the last two sections say `no difference` |
| `ctl.hook.bash`, `ctl.hook.monitor`, `ctl.hook.powershell`, `ctl.hook.read`, `ctl.hook.grep`, `ctl.hook.glob`, `ctl.hook.write`, `ctl.hook.edit`, `ctl.hook.notebookedit` | the hook timing section: a call count for each tool, 9 tools, with Monitor and PowerShell unavailable when the pins say so and Grep and Glob unavailable where the session has no such tool. Write, Edit and NotebookEdit count only the calls from the hook-firing requests in Part 5 |
| `ctl.grep-parent-guard`, `ctl.glob-parent-guard`, `ctl.write-baleytoml-denied` | the guard answers: a `deny` for tool Grep and for tool Glob, `unavailable` where the session has no such tool. For `ctl.write-baleytoml-denied` a `deny` for tool Write appears only if the hook ran (see Part 4) |
| `ctl.latency` | the highest elapsed milliseconds, below 10,000 |
| `ctl.timeout-not-denial` | any call at or above 10,000 ms, and what Claude Code did. `unavailable` when none |
| `ctl.redelivery` | any `tool_use_id` seen more than once. `unavailable` when none |
| `id.mcp-revision` | not in the ledger or in `reads.txt`. Look in the debug logs: `grep -n -i 'protocolVersion\|protocol version' $OUT/debug-*.log`. Record the revision and the log it came from. When no log shows it, write `not observed` and name the logs you searched. The server supports `2025-11-25` and `2026-07-28` (`crates/baley/src/mcp/tools.rs`) |

Last, run the `baley models update` hand-off of Part 14.

Return the filled `$OUT/observations.md`, `$OUT/reads.txt` and the results folder, and say the date of the run. The sheet is checked for a mark on every row before it is recorded.

## What the run settles about the host

These are the questions the documentation leaves open. The sheet's rows answer them, and the record should state each plainly:

- Which MCP revision Claude Code negotiated, and where it was read (`id.mcp-revision`).
- Whether a stdio server's standard error is visible anywhere without the redirect in the launch file. Look in `/mcp`, in the debug log and in the terminal. The redirect is a fixture, not a launcher.
- The skills layout: whether a `SKILL.md` in an uncommitted folder of the isolated configuration is listed and runnable (`hand.skill-listed`, `hand.skill-run`).
- What `/cd`, `/clear`, `/branch`, `--resume <id>`, `--resume` and `--continue` do to the server and its native session id (the `id.*` rows).
- Whether an `env` entry on a registration overrides `CLAUDE_PROJECT_DIR` (the invalid-project and fork rows).
- Whether a hook's loud standard-error line reaches the owner (`ctl.stderr-line`).
