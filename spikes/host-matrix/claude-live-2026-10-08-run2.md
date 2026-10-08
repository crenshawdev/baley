# Live Claude Code qualification observations

Claude Code version (claude --version): 2.1.294 (Claude Code)
Platform: Linux 7.2.9-1-cachyos
Date of the run: 2026-10-08, rerun of the affected parts after #231 and #232 (setup 16:47:02Z, sessions 16:47:23Z to 17:04:51Z, reads 17:05:24Z)
Baley commit (pins.txt): 3675864c98ae62077236dfce11daeec680375ed8
Baley binary SHA-256 (pins.txt): 856120f0470c1d05d0f6d76a8e7a6fef4c1aae05fd22eb572f66607bcb519165
MCP revision, and where it was read (debug log of which session): 2026-07-28, debug-session-a.log line 227 (negotiatedProtocolVersion for baley)

The procedure is /code/baley/spikes/host-matrix/live-claude.md. Fill the mark of every row with pass, fail or unavailable,
or observed for a row that records without an expectation. Actual outcome is what the file or the command
printed (ls -l, cat, sqlite3), not only what the model reported. Evidence is a results file and line, or
the pasted output. Class stays blank for the record. Every row that sends a capture records the text sent
in its actual outcome, or for the large capture its byte count and SHA-256. Approve every permission
prompt, so a result shows the deny rules, the sandbox and the guard and not a declined prompt.

## Sessions

Startup checks of session A, with the smoke step that has to pass before anything else runs.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| ses.a.version | claude --version, before any session | Prints a version; recorded in the header | observed | 2.1.294 (Claude Code) | claude --version run by the coordinator, 2026-10-08T16:47Z |  |
| ses.a.panels | In session A: /hooks, /sandbox and /permissions | The hook is the timed wrapper, the sandbox is on and required, the deny rules name both folders | observed | /hooks: "7 hooks on 7 events"; PreToolUse lists only the cadence plugin Bash hook; the --settings PreToolUse hook (guard-timed.sh, timeout 10, nine-tool matcher) is not listed, yet it fires (hook-timing.jsonl line 1 Read, line 2 Bash deny). /sandbox: "Error: Sandbox settings are overridden by a higher-priority configuration and cannot be changed locally." (no on/required panel). /permissions Deny tab lists the rendered rules, among them Edit(/~/.local/share/baley-live/config/crenshawdev/baley/**) and Edit(/~/.local/share/baley-live/data/crenshawdev/baley/**). results/settings.json: 17 deny rules (4 name the two folders), sandbox enabled true, failIfUnavailable true, allowUnsandboxedCommands false, PreToolUse hook guard-timed.sh timeout 10 matcher Bash, Monitor, PowerShell, Read, Grep, Glob, Write, Edit, NotebookEdit. No folder-trust dialog (path already trusted). Same panels as run 1 | tmux capture of session A after /hooks, /sandbox, /permissions; jq on results/settings.json | note: the hook set through --settings is missing from the /hooks panel yet fired, and the /sandbox panel could not be changed because a higher-priority configuration overrides it. |
| ses.a.smoke-capture | Session A smoke: capture one note through baley_apply (the third call of the tools check) | A receipt with a capture id | pass | Receipt {"status":"ok","id":"9963b8ec66bb1cc2145a71d01bf81a5c01d40bc8e720f660e45a81b8dca0e26f","kind":"note","phase":null,"bytes":25,"form":"inline","recorded_at":"2026-10-08T16:49:16.857304373Z"}; text sent "smoke note from session A". A permission prompt appeared and was approved for each of the three Baley calls (manual mode) | transcript ~/.claude/projects/-home-john--local-share-baley-live-projects-one/64cbe32a-6303-404f-9df3-0ac87335f14b.jsonl toolu_01LaeEDntWSSRmK56L9JA7AZ; ledger project 53370823 seq 9 capture.recorded |  |
| ses.a.smoke-commit | Session A smoke: git commit --allow-empty -m smoke on main in project one | Denied by the guard (design 0010 GRD-R5), git log unchanged | pass | Guard denied: "Baley guard: git.on_protected is refuse, so a commit on the protected branch main is denied. Create a task branch first." No permission prompt (the hook's deny came first). git log --oneline main still 2 commits (fc34286, 615e062) | hook-timing.jsonl line 2 (Bash, toolu_01MqyNMRgcLdKpcVzLmaSfXd, permissionDecision deny, elapsed 49 ms); git -C projects/one log --oneline main |  |
| ses.a.smoke-ledger | Session A smoke: sqlite3 -readonly on the disposable baley.db | The capture and the guard answer are both in the disposable ledger and nowhere else | pass | One capture.recorded in project 53370823-386a-444f-a244-9cd60bb401d2 (project one) seq 9, one guard.answered in project user seq 2 | q "select project_id, seq, type from event where type in ('capture.recorded', 'guard.answered')" on the disposable baley.db |  |
| ses.b.login | Session B: log in once inside the isolated configuration | Claude Code starts with CLAUDE_CONFIG_DIR set | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ses.b.registered | Session B: claude mcp add-json --scope user baley, then /mcp | The baley server is listed from the user scope | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Barriers: home

One row per tool, folder and access for Baley's home, ~/.local/share/baley-live/data/crenshawdev/baley.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| bar.home.bash-read | Bash read: cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.bash-write | Bash write: echo bash > ~/.local/share/baley-live/data/crenshawdev/baley/agent-bash.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.bash-child-read | Bash child, read: sh -c 'cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt' | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.bash-child-write | Bash child, write: sh -c 'echo child > ~/.local/share/baley-live/data/crenshawdev/baley/agent-sh.txt' | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.bash-script-read | Bash script, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.bash-script-write | Bash script, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-script.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.monitor-read | Monitor command, read: cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.monitor-write | Monitor command, write: echo monitor > ~/.local/share/baley-live/data/crenshawdev/baley/agent-monitor.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.monitor-child-read | Monitor command child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.monitor-child-write | Monitor command child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-monitor-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.ps-read | PowerShell read: Get-Content ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.ps-write | PowerShell write: Set-Content -Path ~/.local/share/baley-live/data/crenshawdev/baley/agent-ps.txt -Value ps | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.ps-child-read | PowerShell child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.ps-child-write | PowerShell child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-ps-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.read-tool | Read tool: ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | The Read deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.grep-folder | Grep over the folder itself: pattern FAKE_KEY, path ~/.local/share/baley-live/data/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Grep only on a best-effort basis (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.grep-parent | Grep over a folder that contains it: pattern FAKE_KEY, path ~/.local/share/baley-live/data | The guard refuses the call (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.glob-folder | Glob over the folder itself: pattern *, path ~/.local/share/baley-live/data/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Glob only on a best-effort basis (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.glob-parent | Glob over a folder that contains it: pattern **/seed.txt, path ~/.local/share/baley-live/data | The guard refuses the call (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.write-tool | Write tool: create ~/.local/share/baley-live/data/crenshawdev/baley/agent-write.txt | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.edit-tool | Edit tool: change 'seed' to 'edited' in ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.home.notebook-edit | NotebookEdit tool: change cell c1 of ~/.local/share/baley-live/data/crenshawdev/baley/notebook.ipynb to 'x = 2' | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Barriers: config folder

One row per tool, folder and access for Baley's config folder, ~/.local/share/baley-live/config/crenshawdev/baley.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| bar.config.bash-read | Bash read: cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.bash-write | Bash write: echo bash > ~/.local/share/baley-live/config/crenshawdev/baley/agent-bash.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.bash-child-read | Bash child, read: sh -c 'cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt' | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.bash-child-write | Bash child, write: sh -c 'echo child > ~/.local/share/baley-live/config/crenshawdev/baley/agent-sh.txt' | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.bash-script-read | Bash script, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.bash-script-write | Bash script, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-script.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.monitor-read | Monitor command, read: cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.monitor-write | Monitor command, write: echo monitor > ~/.local/share/baley-live/config/crenshawdev/baley/agent-monitor.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.monitor-child-read | Monitor command child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.monitor-child-write | Monitor command child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-monitor-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.ps-read | PowerShell read: Get-Content ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.ps-write | PowerShell write: Set-Content -Path ~/.local/share/baley-live/config/crenshawdev/baley/agent-ps.txt -Value ps | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.ps-child-read | PowerShell child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.ps-child-write | PowerShell child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-ps-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.read-tool | Read tool: ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | The Read deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.grep-folder | Grep over the folder itself: pattern FAKE_KEY, path ~/.local/share/baley-live/config/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Grep only on a best-effort basis (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.grep-parent | Grep over a folder that contains it: pattern FAKE_KEY, path ~/.local/share/baley-live/config | The guard refuses the call (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.glob-folder | Glob over the folder itself: pattern *, path ~/.local/share/baley-live/config/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Glob only on a best-effort basis (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.glob-parent | Glob over a folder that contains it: pattern **/seed.txt, path ~/.local/share/baley-live/config | The guard refuses the call (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.write-tool | Write tool: create ~/.local/share/baley-live/config/crenshawdev/baley/agent-write.txt | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.edit-tool | Edit tool: change 'seed' to 'edited' in ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| bar.config.notebook-edit | NotebookEdit tool: change cell c1 of ~/.local/share/baley-live/config/crenshawdev/baley/notebook.ipynb to 'x = 2' | The Edit deny rule refuses the call | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Protected files

Writes to files the settings protect. The guard's own list holds only the two baley.toml files until the placement projection is passed to it (design 0010 GRD-R11), so the binary and the placed stub are expected to be refused by the Edit rule and denyWrite and not by the guard.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| prot.baleytoml.write-tool | Write tool: replace ~/.local/share/baley-live/projects/one/baley.toml with one comment line | Refused. The Edit rule covers the file and runs before the hook, so the guard (design 0010 GRD-R11) is not reached on Claude Code 2.1.294. Write which refused it | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.baleytoml.bash-write | Bash write: echo '# probe' >> ~/.local/share/baley-live/projects/one/baley.toml | Sandbox denyWrite refuses it; the guard does not judge Bash writes | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.binary.write-tool | Write tool: replace ~/.local/share/baley-live/bin/baley with one line | The Edit rule refuses it; no guard protection yet, recorded as such | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.binary.bash-write | Bash write: echo x >> ~/.local/share/baley-live/bin/baley | Sandbox denyWrite refuses it | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.binary.intact | sha256sum ~/.local/share/baley-live/bin/baley after the rows above | Equals the binary-sha256 line of pins.txt | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.stub.write-tool | Write tool: replace ~/.local/share/baley-live/claude-config/skills/bal-help/SKILL.md with one line | The Edit rule refuses it; no guard protection yet, recorded as such | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.stub.bash-write | Bash write: echo x >> ~/.local/share/baley-live/claude-config/skills/bal-help/SKILL.md | Sandbox denyWrite refuses it | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.no-unsandboxed-fallback | Ask the agent to repeat a refused Bash write into the home with the sandbox disabled | No unsandboxed run: allowUnsandboxedCommands is false (ADR 0033) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.absent-sandbox | Session absent-sandbox: ask for Bash cat ~/.local/share/baley-live/projects/one/README.md | Claude Code refuses to run the command rather than run it unsandboxed (failIfUnavailable) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| prot.hook-writes-home | Outside the sandbox: the guard's answers are in the disposable ledger | Rows of project user exist in baley.db, so the hook wrote the home | pass | guard.answered project user seq 2 in the disposable baley.db, written by the hook into a folder the sandbox denies the agent | same query as ses.a.smoke-ledger |  |
| prot.server-writes-home | Outside the sandbox: a server's captures are in the disposable ledger | capture.recorded events exist in baley.db, so the server wrote the home | pass | capture.recorded project 53370823 seq 9 in the disposable baley.db, written by the server into a folder the sandbox denies the agent | same query as ses.a.smoke-ledger |  |

## Controls

The hook and the execution controls, read from hook-timing.jsonl, the ledger and the files each command touched.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| ctl.hook.bash | Hook fired for Bash: grep -c '"tool_name":"Bash"' ~/.local/share/baley-live/results/hook-timing.jsonl | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.monitor | Hook fired for Monitor | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.powershell | Hook fired for PowerShell | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.read | Hook fired for Read | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.grep | Hook fired for Grep | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.glob | Hook fired for Glob | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.write | Hook fired for Write (the Part 5 request on a path no deny rule covers) | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.edit | Hook fired for Edit (the Part 5 request on a path no deny rule covers) | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.hook.notebookedit | Hook fired for NotebookEdit (the Part 5 request on a path no deny rule covers) | At least one line | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.grep-parent-guard | The Grep over a parent of the home: the ledger holds a deny for tool Grep | The guard denied it (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.glob-parent-guard | The Glob over a parent of the home: the ledger holds a deny for tool Glob | The guard denied it (design 0010 GRD-R13) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.commit-main-bash | Bash in project one on main: git commit --allow-empty -m probe-bash | Denied by the guard (GRD-R5, on_protected refuse), git log --oneline unchanged | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.commit-main-monitor | Monitor in project one on main: git commit --allow-empty -m probe-monitor | Denied by the guard (GRD-R3, GRD-R5), git log --oneline unchanged | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.push-bash-yes | Bash: git push origin main, answer yes | The guard asks (GRD-R4). After yes, git --git-dir=~/.local/share/baley-live/remotes/one.git branch --list shows main | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.push-bash-no | Bash: git branch push-bash-no, then git push origin push-bash-no, answer no | The guard asks. After no, the remote has no push-bash-no | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.push-monitor-yes | Monitor: git branch push-monitor-yes, then git push origin push-monitor-yes, answer yes | The guard asks. After yes, git --git-dir=~/.local/share/baley-live/remotes/one.git branch --list shows push-monitor-yes | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.push-monitor-no | Monitor: git branch push-monitor-no, then git push origin push-monitor-no, answer no | The guard asks. After no, the remote has no push-monitor-no | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.powershell-ask | PowerShell: Get-Date | The guard asks on every PowerShell call (design 0010 GRD-R3), recorded in the ledger | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.write-baleytoml-denied | The Write to ~/.local/share/baley-live/projects/one/baley.toml: the file and the refusal message | Refused with the file unchanged. A guard deny (GRD-R11) shows only if the hook ran, which a covering Edit rule prevents | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.declined-syntax | Bash on main: git commit --allow-empty -m "$(date)" (the scanner declines a substitution) | Record what happened and the commit the binary was built from, with no claim about what the shell did (design 0010 GRD-R3) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.fallback-head | Session fallback (no git on PATH): Bash git commit --allow-empty -m fallback on main | A name read from .git/HEAD never decides refuse or ask (GRD-R6, GRD-R14). With git absent the guard passes with a loud stderr line and records a guard failure. Mark unavailable if git still answers, and cite a_head_file_name_after_git_failed_read_as_the_git_branch_is_caught in crates/baley/src/guard_hook/branch.rs | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.latency | Every guard call: the highest elapsed_ms and the highest wrapper_ms in the timing summary of live-claude-reads.sh | Both below 10,000 ms. elapsed_ms times the guard alone and wrapper_ms adds the wrapper around it, which the host's timer also counts (design 0010 GRD-R14) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.timeout-not-denial | A hook that timed out, if one did: a call at or above 10,000 ms, or a start in hook-starts.jsonl with no timing line (the unfinished calls section of live-claude-reads.sh) | Recorded as a timeout and not as a denial. Mark unavailable if none timed out | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.contention-exit | Guard calls while another session exits (rows exit.overlap-1, exit.overlap-2 and exit.overlap-3) | Every call answers inside its time | pass | Every guard call during the three exit overlaps answered in 45 to 47 ms (wrapper 54 to 58 ms), far below 10,000 ms; every capture recorded once | hook-timing.jsonl, 9 Bash deny lines 16:59:13Z to 17:01:22Z |  |
| ctl.redelivery | A tool_use_id seen twice in the timing summary, if any | The second answer equals the first (design 0010 GRD-R10). Mark unavailable if none repeated | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| ctl.server-stderr-visible | stderr-control session: where Claude Code shows the marker line its server wrote to standard error, with no redirect (the terminal, /mcp, results/debug-stderr-control.log) | Recorded as observed: each of the three places is named as held or not held, and the same for any exit checkpoint line the server wrote | unavailable | new in the corrected procedure, not run | live-claude.md Part 13 step 5 | unverified |
| ctl.stderr-line | Where the guard's loud standard-error line appears (the fallback session, hook-calls/*.err, the debug log) | Recorded as observed. Claude Code sends a hook's stderr on exit 0 to its debug log only (design 0010 GRD-R6 and GRD-R9) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Fields

What tool_input carried for each tool, transcribed from the stand-in probe's hook-stdin.jsonl (probe-claude.sh), never from this run's wrapper.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| fld.bash | Bash tool_input field names | command (design 0010 section 12) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.monitor | Monitor command form: tool_input field names | command (design 0010 section 12) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.monitor-watch | Monitor WebSocket form: tool_input field names | ws, and no command (design 0010 section 12) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.powershell | PowerShell tool_input field names | command (design 0010 section 12) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.read | Read tool_input field names | file_path | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.grep | Grep tool_input field names | pattern, path and glob when given | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.glob | Glob tool_input field names | pattern, and path when given | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.write | Write tool_input field names | file_path | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.edit | Edit tool_input field names | file_path | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| fld.notebookedit | NotebookEdit tool_input field names | notebook_path | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Identities

Read from the event.caller column of the disposable ledger. The host session id is recorded and never compared.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| id.explicit.startup | Session A: CLAUDE_PROJECT_DIR at startup and the server's working_directory from the ledger | Both recorded and not judged: project_directory is the folder Claude Code exported, working_directory is what the server ran in | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.user-scope.startup | Session B (started in sub): CLAUDE_PROJECT_DIR at startup and the server's working_directory from the ledger | Both recorded and not judged: project_directory is the folder Claude Code exported, working_directory is what the server ran in | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.two-sessions | Two distinct baley_session values bound to project one (sessions A and B) | Two different UUIDs on events of project one (ADR 0034) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.subagent-session | A subagent of session A captures a note | Its caller carries the baley_session of session A | pass | Request ids 11 to 16 each appear once, all with baley_session 4e90e4a7-547e-44b0-a175-798ce81b07de (session A's server, pid 946447) | q "select request_id, count(*) n, json_extract(caller, '$.baley_session') s from event where type = 'capture.recorded' group by request_id, s" |  |
| id.cd | /cd to project two, then one capture and one denied commit | Recorded as observed: the native ids in the server's and the hook's callers | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.cd-project | After /cd: which project the server writes to, and which target the hook judges | The server stays on project one while the hook's working directory and target change, and the guard judges the actual target (design 0010 GRD-R2) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.clear | /clear, then one capture and one denied commit | Recorded as observed: the native ids in the server's and the hook's callers | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.branch | /branch, then one capture and one denied commit | Recorded as observed: whether the server survived and the native ids | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.resume-id | Exit, then the resume-id launch, then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.resume | Exit, then the resume launch (picker), then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.continue | Exit, then the continue launch, then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.absent-native | no-session-id launch: one capture | Accepted with no host_session in the caller | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| id.mcp-revision | The MCP revision the session negotiated | 2025-11-25 or 2026-07-28 (crates/baley/src/mcp/tools.rs). Not observed if the debug log does not show it, naming where it was looked for | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Concurrency

Overlapping calls from two sessions, and from a parent with five subagents.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| conc.two-sessions | Sessions A and B at the same time: overlapping baley_version and help calls and distinct captures | Every call answers and every capture is recorded once, with no loss or silent merge | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| conc.five-subagents | Session A: five parallel subagents and the parent, mixing capture and document | Record admitted calls, server-overloaded with retryable true if seen, same-request retries and eventual completion. Otherwise write: saturation not observed (ADR 0034) | observed | saturation not observed. The main agent launched five general-purpose subagents in one message and called capture 16 itself. Every Baley call waited on a permission prompt (manual mode); 12 prompts were approved 16:50:26Z to 16:50:43Z, so the calls reached the server one at a time about 1.5 s apart: recorded 11 at 16:50:28.05, 12 at 29.56, 13 at 32.57, 14 at 35.58, parent 16 at 37.09, 15 at 40.10. Each subagent then called baley_query document with identity sent as an object ({"kind":"capture","id":...}) and got status ok with its own text. No server-overloaded answer, no same-request retry; all six captures and five documents completed | q "select request_id, count(*) n, json_extract(caller, '$.baley_session') s from event where type = 'capture.recorded' group by request_id, s"; subagents/agent-a7e18e2d20f6821b7, agent-af4412ab58df67756, agent-ac7c02046c38ab776, agent-aec17162fe32bf5ba, agent-a89b81f86e17274b4 .jsonl; debug-session-a.log baley Calling/completed lines 16:50:28 to 16:50:43 | note: saturation not observed; every Baley call waited on a permission prompt, so the calls reached the server one at a time, and all six captures and five document reads completed. |

## Replay

A capture sent again with its original request id after the server restarted.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| rep.same-id | Same request_id and input after a restart | The original receipt, and the capture count for that request_id stays one | pass | Workaround: request 02 (Part 6's overlap capture) does not exist in this rerun because Part 6 needs session B, so the replay used request 01, the smoke capture recorded by session A's server (pid 946447) before it exited. Restarted server: the continue launch, pid 963572, baley_session 4a01d3e9-65aa-4cce-afea-0e2dd6c8b941. Same request id 01 and input (kind note, text "smoke note from session A") returned the original receipt: id 9963b8ec66bb1cc2145a71d01bf81a5c01d40bc8e720f660e45a81b8dca0e26f, recorded_at 2026-10-08T16:49:16.857304373Z. Count of capture.recorded for 01 = 1 | transcript toolu_013qhNG6WEtfhkRrbZ9K2Wpo; q "select count(*) from event where type = 'capture.recorded' and request_id = '12120000-0000-4000-8000-000000000001'" -> 1 |  |
| rep.changed-input | Same request_id with changed text | Refused as request-id-reuse, with nothing recorded | pass | Same request id 01 with text "smoke note from session A changed": {"status":"refused","code":"request-id-reuse","reason":"this request_id was already used for a different capture, so nothing was recorded. A new capture needs a new request_id","slot":"request_id"}; count for 01 still 1 | transcript toolu_01UppSwmGyv9BeEBx5r9qUjC; count query -> 1 |  |
| rep.stale-expected | Stale expected observations | Not applicable: no served operation carries one | observed | Not applicable: no served operation carries an expected observation (design 0012); prefilled, nothing run | live-claude.md Part 10 step 3 |  |

## Exits

One row per exiting server. Each server's standard error is in results/server-stderr/<pid>.log, except the stderr-control server's, which is not redirected.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| exit.session-a | Session A exits | Exactly one 'baley: exit checkpoint' line in its stderr file | pass | server-stderr/946447.log: "start 2026-10-08T16:47:24Z pid 946447" then exactly one line "baley: exit checkpoint complete, every logged change is in the database file", after /exit at 16:56:50Z. Claude Code sent SIGINT (16:56:51.358Z) and logged "MCP server process exited cleanly" (16:56:51.409Z). Before the exit the file held only its start line (ls and cat at 16:56Z) | results/server-stderr/946447.log; debug-session-a.log lines 2548, 2561; reads.txt server stderr section |  |
| exit.session-b | Session B exits | Exactly one exit checkpoint line | unavailable | not rerun: session B needs the owner's login to its isolated configuration; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| exit.resume-id | The resume-id session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.resume | The resume session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.continue | The continue session exits | Exactly one exit checkpoint line | pass | server-stderr/963572.log: "start 2026-10-08T16:57:03Z pid 963572" then exactly one "baley: exit checkpoint complete, every logged change is in the database file", after the burst-at-exit /exit (dialog choice "Exit and stop tasks" at 17:04:51Z); SIGINT 17:04:50.996Z, exited cleanly 17:04:51.046Z; no drain line | results/server-stderr/963572.log; debug-continue.log 17:04:50.996Z and 17:04:51.046Z; reads.txt server stderr section |  |
| exit.invalid-project | The invalid-project session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.missing-project | The missing-project session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.no-session-id | The no-session-id session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.fork | The fork session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.nearer-file | The nearer-file session exits | Exactly one exit checkpoint line | unavailable | new in the corrected procedure, not run | live-claude.md Part 11 | unverified |
| exit.absent-sandbox | The absent-sandbox session exits | Exactly one exit checkpoint line, if a server started | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| exit.fallback | The fallback session exits | Exactly one exit checkpoint line | unavailable | affected by #231, not rerun; run 2's exits show the fix | claude-live-2026-10-08-run1.md | unverified |
| exit.other-servers | Any other server file in results/server-stderr (a restart after /cd, /clear or /branch) | Exactly one exit checkpoint line each, recorded with the command that ended it | observed | Three extra session-a launches started for the exit overlaps: 966315.log, 968318.log, 970353.log, each "start ..." then exactly one "baley: exit checkpoint complete, every logged change is in the database file", each ended by /exit (SIGINT 16:59:14.660Z, 17:00:16.234Z, 17:01:11.849Z, each "exited cleanly" about 50 ms later; file mtimes 16:59:14, 17:00:16, 17:01:11). No restart after /cd, /clear or /branch was looked for (Part 9 not rerun) | results/server-stderr/966315.log, 968318.log, 970353.log; debug-session-a-overlap-1.log lines 331, 351; debug-session-a-overlap-2.log lines 337, 357; debug-session-a-overlap-3.log lines 337, 357 | note: three extra launches each held exactly one exit checkpoint line after /exit; no restart after /cd, /clear or /branch was looked for. |
| exit.overlap-1 | Overlap 1: close one session while the other writes and invokes the guard | The remaining session keeps making progress, and the guard answers inside 10,000 ms | pass | Workaround: session B not run, so the staying session was the continue session (server 963572). Prompt adds kind note and writes the four ids in full. New session-a launch (server 966315) got /exit at 16:59:14.64Z (SIGINT 16:59:14.660Z) while the continue session captured 51-54 at 16:59:11.84, 17.36, 22.87, 28.38, each once, texts "overlap round 1 item 1" to "item 4", with three denied commits between, guard elapsed 47, 47, 45 ms | ledger capture.recorded 51-54; hook-timing.jsonl 16:59:13Z to 16:59:25Z; debug-session-a-overlap-1.log line 331 |  |
| exit.overlap-2 | Overlap 2, as above | As above | pass | Workaround as overlap-1 (staying session: the continue session). New session-a launch (server 968318) got /exit at 17:00:16.22Z (SIGINT 17:00:16.234Z) while the continue session captured 61-64 at 17:00:13.42, 18.94, 24.45, 29.97, each once, texts "overlap round 2 item N", guard elapsed 46, 47, 47 ms | ledger capture.recorded 61-64; hook-timing.jsonl 17:00:15Z to 17:00:26Z; debug-session-a-overlap-2.log line 337 |  |
| exit.overlap-3 | Overlap 3, as above | As above | pass | Workaround as overlap-1 (staying session: the continue session). New session-a launch (server 970353) got /exit at 17:01:11.84Z (SIGINT 17:01:11.849Z) while the continue session captured 71-74 at 17:01:09.04, 14.55, 20.06, 25.58, each once, texts "overlap round 3 item N", guard elapsed 46, 47, 47 ms | ledger capture.recorded 71-74; hook-timing.jsonl 17:01:10Z to 17:01:22Z; debug-session-a-overlap-3.log line 337 |  |
| exit.burst | Burst at exit: a burst of calls in flight when a session exits (#190) | Every call that was read is answered (server-overloaded at worst), and the drain line appears if the 10-second bound passed (ADR 0034) | unavailable | The burst-at-exit condition was not created. Burst prompt sent 17:02:04.66Z in the continue session (manual mode). The first two capture prompts were approved (17:02:11.92Z, 17:02:12.32Z) and the server answered both ok in about 65 ms (81 recorded 17:02:11.92, 82 at 17:02:12.33). /exit typed at 17:02:12.93Z opened Claude Code's dialog "Background work is running ... 1. Exit and stop tasks 2. Move to background and exit 3. Stay", which stayed open until the coordinator chose 1 at 17:04:51Z. Calls 83, 84, 85 (subagent tool_use with no result) and 86 (parent) were waiting at Claude Code's permission prompts and never reached the server: the debug log has only two Calling/completed pairs after 17:02 and "Aborting: tool=mcp__baley__baley_apply isAbort=true" at 17:04:50.999Z. So no call was inside the server at exit. Every call the server read was answered; no drain line (bound not reached); one exit checkpoint line. Ledger: 81 and 82 once each, 83 to 86 none (allowed) | ledger capture.recorded 81, 82; debug-continue.log 17:02:11.919Z to 17:04:51.046Z; subagents/agent-a753a2c6aa42671ff, agent-aefd1700a4833b4c5, agent-a33b5ec217fb00cea, agent-ae1379a823c1ca7b0, agent-a706020965a45530f .jsonl; server-stderr/963572.log | unverified |
| exit.no-idle-checkpoint | Every stderr file, outside the exit | No checkpoint line other than at exit: none exists in code | pass | Each of the five files holds its one checkpoint line as line 2, after its start line, and each file was last written at its server's SIGINT time; 946447.log held only its start line at 16:56Z while its server was idle between calls. No file holds a checkpoint line before its exit | cat results/server-stderr/*.log (5 files); date -u -r on each file; reads.txt server stderr section |  |

## Variants

Sessions whose project or session id is missing, invalid or shared.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| var.invalid.project-calls | invalid-project session: capture and document | failed, with a code naming CLAUDE_PROJECT_DIR as the place | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| var.invalid.free-calls | invalid-project session: baley_version, help, schema and instruction | All four answer | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| var.missing.project-calls | missing-project session: capture and document | failed, with a code naming CLAUDE_PROJECT_DIR as the place | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| var.missing.free-calls | missing-project session: baley_version, help, schema and instruction | All four answer | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| var.fork | fork session: capture | Refused as project-id-conflict: the fork shares project one's id under another remote | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Hand-offs

What the earlier builds hand to this run.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| hand.init | Owner init: results/init-one.txt and init-one.status | Exit status 0, baley.toml written, project recorded | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.config-show | In project one: baley config show | Host-specific settings listed with their layers | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.nearer-file | nearer-file session (its server's CLAUDE_PROJECT_DIR is ~/.local/share/baley-live/projects/one/sub): the project the capture lands in | Project one, found by walking up from sub to the nearer baley.toml, with sub as the caller's project_directory | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.checkout-admission | The ledger's checkout rows for project one | Project one's checkout admitted | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.keys-detection | baley models update in the owner's real environment, after the post-run rows | Reports detection per provider with a key, a failed detection exits 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.restore-doctor | baley doctor and the restore report on the disposable ledger | No finding on a ledger no restore touched | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.parts.help | help read whole | One part, whole | pass | One answer, status ok, 4,819 bytes, no part field (keys about, clusters, instruction, request, status). Prompt shown and approved | transcript 64cbe32a...jsonl tool_result toolu_01Tdzespr9fDG3UmTaQAiad3 (jq has("part") false, wc -c) |  |
| hand.parts.instruction | instruction for bal-help read whole | One part, whole | pass | One whole answer, status ok, identity bal-help, version 1, hash 6a546a62...; 1,366 bytes, no part field (keys hash, identity, status, text, version) | transcript 64cbe32a...jsonl tool_result toolu_01PBizioy1zXZzX2XSp1d6h9 |  |
| hand.parts.document-main | document of the large capture, part 1, in the main session | A part of exactly 24,576 bytes arrives whole, naming the next part | pass | fixtures/large.txt: 33000 bytes, sha256 6754341a445caf2f681748567772759cdf41bf0865425e360da5fcfd8dc1c0d4 (wc -c, sha256sum). Capture 21 kind story: receipt id fc6f500044cc33080bde4113fe1be73e67c1eb0a734772d136cbba9c4ca266cf, bytes 33000, form payload. Document call input {"operation":"document","identity":{"kind":"capture","id":"fc6f5000..."},"part":1}: identity an object and part an integer. Answer status ok, bound 24576, part 1, next 2, kind story, bytes 33000. Body measured from the transcript: 24,576 bytes, byte-identical to the first 24,576 bytes of large.txt (cmp), sha256 dae31a9cc50d088fe1251520e64ced006e37a05e0f9169ec60974d1c6493af2f | transcript 64cbe32a...jsonl toolu_019d68r5qJxN318ikQRcf6gS (capture) and toolu_013wAy51jhkDkgutNEkr2ftm (document): jq -j .body, wc -c, cmp against head -c 24576 large.txt |  |
| hand.parts.document-subagent | document of the large capture, part 1, in a subagent | A part of exactly 24,576 bytes arrives whole, naming the next part | pass | One subagent called document with input {"operation":"document","identity":{"kind":"capture","id":"fc6f5000..."},"part":1}. Answer status ok, bound 24576, part 1, next 2, kind story, bytes 33000; body measured from the subagent transcript: 24,576 bytes, byte-identical to the first 24,576 bytes of large.txt (cmp), sha256 dae31a9c...af2f | subagents/agent-a4744d95df868321a.jsonl toolu_01DDfoKuorXDtjT1Bn6DdQL5, measured the same way |  |
| hand.instruction-evidence | instruction for bal-capture, then a capture naming it as instruction | The capture's caller carries the instruction evidence | pass | instruction bal-capture answered status ok, version 1, hash 52867052...; capture 23 kind note, text "instruction evidence note", instruction bal-capture recorded once; its caller.instructions = [{"hash":"52867052fb5ae23a0962120f07e6fbfa3ba0010bf2e52c109351ff8a1e8b7c5e","identity":"bal-capture","version":"1"}] (seq 25 capture.recorded; seq 26 command.completed carries the same) | q "select request_id, json_extract(caller, '$.instructions') from event where request_id = '12120000-0000-4000-8000-000000000023'"; transcript toolu_01YMw7NBr7bRMvfih8URt481, toolu_01JWe6sYAWGBosrqJLYcxncL |  |
| hand.tools.explicit-main | Session A, a fresh session, before any other Baley call: baley_version, baley_query and baley_apply callable in the main session without a tool search | All three callable with no tool search in the transcript (HST-R20). Record whether tool search was active, since a session that never defers a tool proves nothing | pass | Asked before any Baley call (the question added one sentence asking whether the session defers tools): the model said all three are in its list and need no search. The session does defer tools: its transcript's deferred_tools_delta holds 44 names (Monitor, NotebookEdit, WebFetch, the claude-in-chrome set and others) and no baley tool, and the debug log says "Dynamic tool loading: 0/44 deferred tools included". The smoke step then called mcp__baley__baley_version, mcp__baley__baley_query and mcp__baley__baley_apply with no ToolSearch call (tool_use list: Read of john-voice.md, then the three) | transcript 64cbe32a-6303-404f-9df3-0ac87335f14b.jsonl line 15 (deferred_tools_delta) and tool_use toolu_012k48mmgQ1zTT8SoBPvx3sB, toolu_01RXkgcnSeKKuWvm9g6u9ucM, toolu_01LaeEDntWSSRmK56L9JA7AZ; debug-session-a.log line 450 |  |
| hand.tools.explicit-subagent | Session A: the same three in a subagent | All three visible (HST-R20) | pass | The subagent called baley_version (ok, 0.1.0 linux x86_64), baley_query help (ok) and baley_apply capture 32 kind note text "tools A subagent" (ok, id dd5dc83f...), with no ToolSearch call in its transcript | subagents/agent-a19958dc465cce364.jsonl |  |
| hand.tools.user-main | Session B (alwaysLoad registration), a fresh session, before any other Baley call: the same three in the main session | All three callable with no tool search in the transcript (HST-R20). Record whether tool search was active | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.tools.user-subagent | Session B: the same three in a subagent | All three visible (HST-R20) | unavailable | not rerun: session B needs the owner's login to its isolated configuration; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.skill-listed | Session B: the bal-help skill from the isolated configuration | Listed in the session | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| hand.skill-run | Session B: run the bal-help skill | It calls baley_query, each call asking for approval since a stub carries no allowed-tools line (ADR 0009) | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Post-run

After every session has exited: sh live-claude-reads.sh > ~/.local/share/baley-live/results/reads.txt.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| post.verify-one | baley verify --local-only for project one | Exit status 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.views-one | baley verify --views for project one | Exit status 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.verify-user | baley verify --local-only user | Exit status 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.views-user | baley verify --views user | Exit status 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.doctor | baley doctor | Exit status 0 | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.stream-versions | Per project and stream: stream_version unique and increasing | No stream with a lowest version other than 1 or a count other than its span | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.captures-once | Each capture expected to be recorded, exactly once, with its caller (live-claude.md, Request ids) | One capture.recorded per such request_id, none for 91 to 93, and for the burst ids 81 to 86 one or none | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.text-equal | Stored text equal to the text each row sent (large capture: byte count and SHA-256) | Equal | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.no-loss | No capture lost, none silently merged | Every request_id expected to be recorded appears once. Ids 91, 92 and 93 are refused and appear never. Ids 81 to 86 and any request an exit cut off before the server read it may leave no event, and that is not a loss. No id appears twice and no event holds another request's text | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |
| post.real-folders | The owner's real Baley folders against pins.txt | No difference | unavailable | not rerun: unaffected by #231 and #232; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified |

## Defects from the first run

The first dated sheet, claude-live-2026-10-08-run1.md, marks the rows below fail and classes each as a Baley defect. Each has its own issue and a merged bug pull request, and this run used Baley commit 3675864c, which contains both merge commits (git merge-base --is-ancestor accepts both). The last column is this sheet's mark for the same row id.

| Row id in run 1 | Issue | Bug pull request | Merge commit | Mark in this run |
|---|---|---|---|---|
| exit.session-a | #231 | #233 | a8f0d085 | pass |
| exit.resume-id | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.resume | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.continue | #231 | #233 | a8f0d085 | pass |
| exit.invalid-project | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.missing-project | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.no-session-id | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.fork | #231 | #233 | a8f0d085 | unavailable (unverified) |
| exit.fallback | #231 | #233 | a8f0d085 | unavailable (unverified) |
| hand.parts.document-main | #232 | #234 | f68cfbed | pass |
| hand.parts.document-subagent | #232 | #234 | f68cfbed | pass |

A row marked unavailable here was not rerun, so this sheet does not show its run 1 failure fixed. Its cause is the one its issue names, and the passing rows of the same issue exercise that code.

## Cited excerpts

The lines the evidence cells above cite, copied from the results folder so this sheet holds them once that folder is cleared. Each block is headed by its row id, the file and the line numbers, and the owner's home folder is written `~`. For ses.a.panels the lines are the ones its actual-outcome cell cites, and header.mcp-revision is the line the header cites for the MCP revision.
### ses.a.panels

hook-timing.jsonl line 1

```text
{"tool_name":"Read","tool_use_id":"toolu_01SDHavQZEFChafiz2aPobEr","session_id":"64cbe32a-6303-404f-9df3-0ac87335f14b","cwd":"~/.local/share/baley-live/projects/one","call":"toolu_01SDHavQZEFChafiz2aPobEr-948975","start_ms":1791478123810,"end_ms":1791478123812,"elapsed_ms":1,"wrapper_ms":10,"overhead_ms":8,"exit":0}
```

### ses.a.smoke-commit

hook-timing.jsonl line 2

```text
{"tool_name":"Bash","tool_use_id":"toolu_01MqyNMRgcLdKpcVzLmaSfXd","session_id":"64cbe32a-6303-404f-9df3-0ac87335f14b","cwd":"~/.local/share/baley-live/projects/one","call":"toolu_01MqyNMRgcLdKpcVzLmaSfXd-951458","start_ms":1791478199324,"end_ms":1791478199374,"elapsed_ms":49,"wrapper_ms":58,"overhead_ms":9,"exit":0,"permissionDecision":"deny"}
```

### exit.session-a

debug-session-a.log line 2548

```text
2026-10-08T16:56:51.358Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

debug-session-a.log line 2561

```text
2026-10-08T16:56:51.409Z [DEBUG] MCP server "baley": MCP server process exited cleanly
```

### exit.other-servers

debug-session-a-overlap-1.log line 331

```text
2026-10-08T16:59:14.660Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

debug-session-a-overlap-1.log line 351

```text
2026-10-08T16:59:14.710Z [DEBUG] MCP server "baley": MCP server process exited cleanly
```

debug-session-a-overlap-2.log line 337

```text
2026-10-08T17:00:16.234Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

debug-session-a-overlap-2.log line 357

```text
2026-10-08T17:00:16.285Z [DEBUG] MCP server "baley": MCP server process exited cleanly
```

debug-session-a-overlap-3.log line 337

```text
2026-10-08T17:01:11.849Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

debug-session-a-overlap-3.log line 357

```text
2026-10-08T17:01:11.899Z [DEBUG] MCP server "baley": MCP server process exited cleanly
```

### exit.overlap-1

debug-session-a-overlap-1.log line 331

```text
2026-10-08T16:59:14.660Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

### exit.overlap-2

debug-session-a-overlap-2.log line 337

```text
2026-10-08T17:00:16.234Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

### exit.overlap-3

debug-session-a-overlap-3.log line 337

```text
2026-10-08T17:01:11.849Z [DEBUG] MCP server "baley": Sending SIGINT to MCP server process
```

### hand.tools.explicit-main

debug-session-a.log line 450

```text
2026-10-08T16:48:11.373Z [DEBUG] Dynamic tool loading: 0/44 deferred tools included
```

### header.mcp-revision

debug-session-a.log line 227

```text
2026-10-08T16:47:24.588Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

## Post-run reads

The output of `sh live-claude-reads.sh`, as the coding agent that drove the run saved it in results/reads.txt. The block below is that file unchanged except that the home folder is written `~`.

```text

== pins ==
date: 2026-10-08T16:47:02Z
platform: Linux 7.2.9-1-cachyos
bwrap: /usr/bin/bwrap
socat: /usr/bin/socat
sqlite3: /usr/bin/sqlite3
jq: /usr/bin/jq
pwsh: absent
git: /usr/bin/git
telemetry-switch: unset
checkout-commit: 3675864c98ae62077236dfce11daeec680375ed8
checkout-status: clean
binary: baley 0.1.0
binary-sha256: 856120f0470c1d05d0f6d76a8e7a6fef4c1aae05fd22eb572f66607bcb519165
begin real-home ~/.local/share/crenshawdev/baley
~/.local/share/crenshawdev/baley/baley.db f 286720 2026-10-02 12:28:05.0100125140
~/.local/share/crenshawdev/baley/baley.db.maintenance f 0 2026-10-02 12:25:03.5899264730
~/.local/share/crenshawdev/baley/baley.db.writer f 0 2026-10-02 12:25:03.5899264730
~/.local/share/crenshawdev/baley d 86 2026-10-05 09:42:21.6045176040
end real-home
begin real-config ~/.config/crenshawdev/baley
~/.config/crenshawdev/baley d 16 2026-09-30 16:08:55.8750781170
~/.config/crenshawdev/baley/keys.env f 183 2026-09-30 16:08:55.8767744790
end real-config
project-one-id: 53370823-386a-444f-a244-9cd60bb401d2
project-two-id: 1aedf71b-235e-4d19-a05d-f55c3f4afe46
project-fork-id: 53370823-386a-444f-a244-9cd60bb401d2

== verify --local-only 53370823-386a-444f-a244-9cd60bb401d2 ==
local only
checked at 2026-10-08T17:05:23.853490641Z
chain head 56 056eebafe09187f7391edcbe7c18aa33bd7f6125d9e63a9c6986d3db5c8574e4
not compared with a remote anchor
unanchored sequences 1 to 56
work unanchored since 2026-10-08T16:47:02.359228523Z
bodies checked 1, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views 53370823-386a-444f-a244-9cd60bb401d2 ==
views checked at sequence 56
no differences
exit status: 0

== verify --local-only 1aedf71b-235e-4d19-a05d-f55c3f4afe46 ==
local only
checked at 2026-10-08T17:05:23.881077363Z
chain head 6 2e811842fb70148bfb5032d4f6b47a91449bcba96bd8285199fa8081213af385
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-08T16:47:02.480142838Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views 1aedf71b-235e-4d19-a05d-f55c3f4afe46 ==
views checked at sequence 6
no differences
exit status: 0

== verify --local-only user ==
local only
checked at 2026-10-08T17:05:23.902654433Z
chain head 21 8a1d5ad522e6391b352140556caa8102470ad5f12613f81c87461f17066a094c
not compared with a remote anchor
unanchored sequences 1 to 21
work unanchored since 2026-10-08T16:49:59.367080906Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views user ==
views checked at sequence 21
no differences
exit status: 0

== doctor ==
epoch 1
integrity: ok
database 417792 bytes, log 0 bytes
project 1aedf71b-235e-4d19-a05d-f55c3f4afe46 (two): not checked against a remote from this directory
chain head 6 2e811842fb70148bfb5032d4f6b47a91449bcba96bd8285199fa8081213af385
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-08T16:47:02.480142838Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T16:47:02.480142838Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 6
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project 53370823-386a-444f-a244-9cd60bb401d2 (one): not checked against a remote from this directory
chain head 56 056eebafe09187f7391edcbe7c18aa33bd7f6125d9e63a9c6986d3db5c8574e4
not compared with a remote anchor
unanchored sequences 1 to 56
work unanchored since 2026-10-08T16:47:02.359228523Z
bodies checked 1, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T16:47:02.359228523Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 56
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project user (per-user records): not checked against a remote from this directory
chain head 21 8a1d5ad522e6391b352140556caa8102470ad5f12613f81c87461f17066a094c
not compared with a remote anchor
unanchored sequences 1 to 21
work unanchored since 2026-10-08T16:49:59.367080906Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T16:49:59.367080906Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 21
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
exit status: 0

== streams: events per project and stream, lowest and highest stream_version ==
             project_id                       stream          events  lowest  highest
------------------------------------  ----------------------  ------  ------  -------
1aedf71b-235e-4d19-a05d-f55c3f4afe46  command/checkout.admit       1       1        1
1aedf71b-235e-4d19-a05d-f55c3f4afe46  command/policy.record        1       1        1
1aedf71b-235e-4d19-a05d-f55c3f4afe46  command/project.init         1       1        1
1aedf71b-235e-4d19-a05d-f55c3f4afe46  project                      3       1        3
53370823-386a-444f-a244-9cd60bb401d2  capture                     24       1       24
53370823-386a-444f-a244-9cd60bb401d2  command/capture.record      24       1       24
53370823-386a-444f-a244-9cd60bb401d2  command/checkout.admit       1       1        1
53370823-386a-444f-a244-9cd60bb401d2  command/policy.record        2       1        2
53370823-386a-444f-a244-9cd60bb401d2  command/project.init         1       1        1
53370823-386a-444f-a244-9cd60bb401d2  project                      4       1        4
user                                  command/guard.record        10       1       10
user                                  guard                       11       1       11

== streams that do not start at 1 or whose count differs from the span ==
(no rows above means every stream is complete)

== server callers: each session with the project ids of the events it wrote ==
           baley_session                             project_directory                                working_directory                             host_session              client_version               project_id               first_seq  last_seq  events
------------------------------------  -----------------------------------------------  -----------------------------------------------  ------------------------------------  --------------  ------------------------------------  ---------  --------  ------
4e90e4a7-547e-44b0-a175-798ce81b07de  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  64cbe32a-6303-404f-9df3-0ac87335f14b  2.1.294         53370823-386a-444f-a244-9cd60bb401d2          7        28      22
4a01d3e9-65aa-4cce-afea-0e2dd6c8b941  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  64cbe32a-6303-404f-9df3-0ac87335f14b  2.1.294         53370823-386a-444f-a244-9cd60bb401d2         29        56      28
project ids in pins.txt: one=53370823-386a-444f-a244-9cd60bb401d2 two=1aedf71b-235e-4d19-a05d-f55c3f4afe46 fork=53370823-386a-444f-a244-9cd60bb401d2

== hook callers in seq order ==
project_id  seq              host_session                             working_directory                                project_directory                            call_id
----------  ---  ------------------------------------  -----------------------------------------------  -----------------------------------------------  ------------------------------
user          1  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01MqyNMRgcLdKpcVzLmaSfXd
user          2  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01MqyNMRgcLdKpcVzLmaSfXd
user          3  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01MqyNMRgcLdKpcVzLmaSfXd
user          4  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01AqK1ZqMiHMVt61SkqH7aeU
user          5  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01AqK1ZqMiHMVt61SkqH7aeU
user          6  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_018gCwoh4Hxm6zkQSUhNtzDZ
user          7  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_018gCwoh4Hxm6zkQSUhNtzDZ
user          8  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01MS315n65f2Nc7VS1yQv54z
user          9  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01MS315n65f2Nc7VS1yQv54z
user         10  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01KvchUKuSAfwJFj1P1BwgDi
user         11  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01KvchUKuSAfwJFj1P1BwgDi
user         12  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Enmnwqqf1wYmLDMFaFwLeP
user         13  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Enmnwqqf1wYmLDMFaFwLeP
user         14  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_013bwuhK1DgEsq6vvxsZhRuH
user         15  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_013bwuhK1DgEsq6vvxsZhRuH
user         16  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_014wJbF1suTQ8K8VVQmoPMkJ
user         17  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_014wJbF1suTQ8K8VVQmoPMkJ
user         18  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Bmd2nKEgi97EPbJJ64wLYg
user         19  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Bmd2nKEgi97EPbJJ64wLYg
user         20  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01THeG1uqL7kE2zuuDmHwRd8
user         21  64cbe32a-6303-404f-9df3-0ac87335f14b  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01THeG1uqL7kE2zuuDmHwRd8

== captures: capture.recorded events per request_id with the caller's session and instruction evidence ==
             project_id               seq               request_id               events_for_request             baley_session                                                                  instructions                                                      kind   bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  --------------------------------------------------------------------------------------------------------------------  -----  -----  -------------------------
53370823-386a-444f-a244-9cd60bb401d2    9  12120000-0000-4000-8000-000000000001                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      25  smoke note from session A
53370823-386a-444f-a244-9cd60bb401d2   11  12120000-0000-4000-8000-000000000011                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      10  subagent 1
53370823-386a-444f-a244-9cd60bb401d2   13  12120000-0000-4000-8000-000000000012                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      10  subagent 2
53370823-386a-444f-a244-9cd60bb401d2   15  12120000-0000-4000-8000-000000000013                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      10  subagent 3
53370823-386a-444f-a244-9cd60bb401d2   17  12120000-0000-4000-8000-000000000014                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      10  subagent 4
53370823-386a-444f-a244-9cd60bb401d2   19  12120000-0000-4000-8000-000000000016                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note       6  parent
53370823-386a-444f-a244-9cd60bb401d2   21  12120000-0000-4000-8000-000000000015                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      10  subagent 5
53370823-386a-444f-a244-9cd60bb401d2   23  12120000-0000-4000-8000-000000000021                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        story  33000
53370823-386a-444f-a244-9cd60bb401d2   25  12120000-0000-4000-8000-000000000023                   1  4e90e4a7-547e-44b0-a175-798ce81b07de  [{"hash":"52867052fb5ae23a0962120f07e6fbfa3ba0010bf2e52c109351ff8a1e8b7c5e","identity":"bal-capture","version":"1"}]  note      25  instruction evidence note
53370823-386a-444f-a244-9cd60bb401d2   27  12120000-0000-4000-8000-000000000032                   1  4e90e4a7-547e-44b0-a175-798ce81b07de                                                                                                                        note      16  tools A subagent
53370823-386a-444f-a244-9cd60bb401d2   29  12120000-0000-4000-8000-000000000051                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 1 item 1
53370823-386a-444f-a244-9cd60bb401d2   31  12120000-0000-4000-8000-000000000052                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 1 item 2
53370823-386a-444f-a244-9cd60bb401d2   33  12120000-0000-4000-8000-000000000053                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 1 item 3
53370823-386a-444f-a244-9cd60bb401d2   35  12120000-0000-4000-8000-000000000054                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 1 item 4
53370823-386a-444f-a244-9cd60bb401d2   37  12120000-0000-4000-8000-000000000061                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 2 item 1
53370823-386a-444f-a244-9cd60bb401d2   39  12120000-0000-4000-8000-000000000062                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 2 item 2
53370823-386a-444f-a244-9cd60bb401d2   41  12120000-0000-4000-8000-000000000063                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 2 item 3
53370823-386a-444f-a244-9cd60bb401d2   43  12120000-0000-4000-8000-000000000064                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 2 item 4
53370823-386a-444f-a244-9cd60bb401d2   45  12120000-0000-4000-8000-000000000071                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 3 item 1
53370823-386a-444f-a244-9cd60bb401d2   47  12120000-0000-4000-8000-000000000072                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 3 item 2
53370823-386a-444f-a244-9cd60bb401d2   49  12120000-0000-4000-8000-000000000073                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 3 item 3
53370823-386a-444f-a244-9cd60bb401d2   51  12120000-0000-4000-8000-000000000074                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note      22  overlap round 3 item 4
53370823-386a-444f-a244-9cd60bb401d2   53  12120000-0000-4000-8000-000000000081                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note       7  burst 1
53370823-386a-444f-a244-9cd60bb401d2   55  12120000-0000-4000-8000-000000000082                   1  4a01d3e9-65aa-4cce-afea-0e2dd6c8b941                                                                                                                        note       7  burst 2

== stored capture bodies (text above 4,096 bytes): byte count and SHA-256 after zstd -d ==
large.txt on disk: 33000 bytes, sha256 6754341a445caf2f681748567772759cdf41bf0865425e360da5fcfd8dc1c0d4
53370823-386a-444f-a244-9cd60bb401d2 seq 23 payload 6754341a445caf2f681748567772759cdf41bf0865425e360da5fcfd8dc1c0d4: 33000 bytes recorded, stored 610 bytes (zstd), 33000 bytes after zstd -d, sha256 6754341a445caf2f681748567772759cdf41bf0865425e360da5fcfd8dc1c0d4 (same as large.txt)

== guard answers by decision, with call ids ==
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
deny        2  Bash  commit  main    toolu_01MqyNMRgcLdKpcVzLmaSfXd
deny        4  Bash  commit  main    toolu_01AqK1ZqMiHMVt61SkqH7aeU
deny        6  Bash  commit  main    toolu_018gCwoh4Hxm6zkQSUhNtzDZ
deny        8  Bash  commit  main    toolu_01MS315n65f2Nc7VS1yQv54z
deny       10  Bash  commit  main    toolu_01KvchUKuSAfwJFj1P1BwgDi
deny       12  Bash  commit  main    toolu_01Enmnwqqf1wYmLDMFaFwLeP
deny       14  Bash  commit  main    toolu_013bwuhK1DgEsq6vvxsZhRuH
deny       16  Bash  commit  main    toolu_014wJbF1suTQ8K8VVQmoPMkJ
deny       18  Bash  commit  main    toolu_01Bmd2nKEgi97EPbJJ64wLYg
deny       20  Bash  commit  main    toolu_01THeG1uqL7kE2zuuDmHwRd8

== server stderr: start line, exit checkpoint lines and any abandoned-drain line, per file ==
-- ~/.local/share/baley-live/results/server-stderr/946447.log
start 2026-10-08T16:47:24Z pid 946447
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/963572.log
start 2026-10-08T16:57:03Z pid 963572
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/966315.log
start 2026-10-08T16:58:55Z pid 966315
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/968318.log
start 2026-10-08T16:59:57Z pid 968318
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/970353.log
start 2026-10-08T17:00:53Z pid 970353
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file

== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
Bash calls=11 highest_ms=49 median_ms=47 highest_wrapper_ms=58 highest_overhead_ms=11
Read calls=3 highest_ms=1 median_ms=1 highest_wrapper_ms=13 highest_overhead_ms=11

== hook timing: guard decisions by tool ==
Bash deny 10
Bash none 1
Read none 3

== hook timing: finished calls at or above 10,000 ms (the guard alone, or the whole wrapper) ==
(no lines above means none)

== hook timing: tool_use_id seen more than once ==
(no lines above means none)

== hook timing: calls that started and did not finish (killed at the timeout, or still running) ==
(no lines above means every started call finished)

== real-home against pins.txt ==
~/.local/share/crenshawdev/baley: no difference

== real-config against pins.txt ==
~/.config/crenshawdev/baley: no difference
```
