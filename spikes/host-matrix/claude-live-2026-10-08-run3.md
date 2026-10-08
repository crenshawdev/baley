# Live Claude Code qualification observations

Claude Code version (claude --version): 2.1.294 (Claude Code)
Platform: Linux 7.2.9-1-cachyos
Date of the run: 2026-10-08, re-run of the parts Plan 3 fixed (setup 19:03:29Z, sessions 19:07Z to 19:21Z, reads 19:22:27Z, restore 19:23:06Z), driven by the coordinator through tmux, the isolated login typed by the owner
Baley commit (pins.txt): 7c8ca4c39249d8135ba56dab62941d7bc7483964
Baley binary SHA-256 (pins.txt): 856120f0470c1d05d0f6d76a8e7a6fef4c1aae05fd22eb572f66607bcb519165
MCP revision, and where it was read (debug log of which session): 2026-07-28, debug-session-a.log line 226 (negotiatedProtocolVersion for baley)

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
| ses.a.version | claude --version, before any session | Prints a version; recorded in the header | observed | 2.1.294 (Claude Code) | claude --version run by the coordinator after live-claude.sh |  |
| ses.a.panels | In session A: /hooks, /sandbox and /permissions | The hook is the timed wrapper, the sandbox is on and required, the deny rules name both folders | observed | /hooks: "7 hooks on 7 events"; PreToolUse lists only the cadence plugin Bash hook, and the --settings PreToolUse hook (guard-timed.sh) is not listed, yet it fires (hook-timing.jsonl). /sandbox: "Error: Sandbox settings are overridden by a higher-priority configuration and cannot be changed locally." /permissions Deny tab lists Edit(/~/.local/share/baley-live/bin/baley), Edit(//.../claude-config/skills/bal-help/SKILL.md), Edit(//.../config/crenshawdev/baley/**), Edit(//.../data/crenshawdev/baley/**), both baley.toml files and the mcp-*.json files. Same panels as runs 1 and 2 | tmux capture of session A after /hooks, /sandbox, /permissions | note: the hook set through --settings is missing from the /hooks panel yet fired, and the /sandbox panel could not be changed because a higher-priority configuration overrides it. |
| ses.a.smoke-capture | Session A smoke: capture one note through baley_apply (the third call of the tools check) | A receipt with a capture id | pass | Request 01, text "smoke note from session A": {"status":"ok","id":"9963b8ec66bb1cc2145a71d01bf81a5c01d40bc8e720f660e45a81b8dca0e26f","kind":"note","phase":null,"bytes":25,"form":"inline","recorded_at":"2026-10-08T19:10:28.749725438Z"}. A prompt appeared for each Baley tool and was approved | reads.txt captures section seq 9; session A transcript 4c01fab1-79a7-490c-8ce6-4bee52f7f0c1.jsonl |  |
| ses.a.smoke-commit | Session A smoke: git commit --allow-empty -m smoke on main in project one | Denied by the guard (design 0010 GRD-R5), git log unchanged | pass | "Baley guard: git.on_protected is refuse, so a commit on the protected branch main is denied. Create a task branch first." git log --oneline main: af5317d, 04c1e36 (two commits) | reads.txt guard answers seq 2 deny Bash commit main toolu_01SPVurHqetRvWPLZidDdt69 |  |
| ses.a.smoke-ledger | Session A smoke: sqlite3 -readonly on the disposable baley.db | The capture and the guard answer are both in the disposable ledger and nowhere else | pass | af645d0f-a096-41c5-84c3-2da2d85ae4ca seq 9 capture.recorded; user seq 2 guard.answered | sqlite3 query of Part 2 step 5 |  |
| ses.b.login | Session B: starts after the owner's login into the isolated configuration | Session B starts with no login screen, after the owner's login in Part 0; claude-config holds a credentials file (presence only) | pass | The owner typed the login into claude-config before session B (Part 0 step 10). test -f claude-config/.credentials.json printed present. Session B showed a folder trust dialog for projects/one/sub, answered "Yes, I trust this folder", and no login screen, then started straight into a session | tmux capture of session B at start; presence test |  |
| ses.b.registered | Session B: claude mcp add-json --scope user baley, then /mcp | The baley server is listed from the user scope | observed | /mcp: "User MCPs (~/.local/share/baley-live/claude-config/.claude.json)" with "baley 3 tools". The account's claude.ai connectors and built-in plugin MCPs are also listed. The launch carries no --mcp-config | tmux capture of /mcp in session B; claude-config/.claude.json mcpServers keys ["baley"] |  |

## Barriers: home

One row per tool, folder and access for Baley's home, ~/.local/share/baley-live/data/crenshawdev/baley.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| bar.home.bash-read | Bash read: cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.bash-write | Bash write: echo bash > ~/.local/share/baley-live/data/crenshawdev/baley/agent-bash.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.bash-child-read | Bash child, read: sh -c 'cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt' | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.bash-child-write | Bash child, write: sh -c 'echo child > ~/.local/share/baley-live/data/crenshawdev/baley/agent-sh.txt' | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.bash-script-read | Bash script, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.bash-script-write | Bash script, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-script.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.monitor-read | Monitor command, read: cat ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.monitor-write | Monitor command, write: echo monitor > ~/.local/share/baley-live/data/crenshawdev/baley/agent-monitor.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.monitor-child-read | Monitor command child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.monitor-child-write | Monitor command child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-monitor-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.ps-read | PowerShell read: Get-Content ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.home.ps-write | PowerShell write: Set-Content -Path ~/.local/share/baley-live/data/crenshawdev/baley/agent-ps.txt -Value ps | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.home.ps-child-read | PowerShell child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.home.ps-child-write | PowerShell child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/data/crenshawdev/baley/agent-ps-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.home.read-tool | Read tool: ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | The Read deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.grep-folder | Grep over the folder itself (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-live/data/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Grep only on a best-effort basis (design 0010 GRD-R13) | pass | Pattern FAKE_KEY, path data/crenshawdev/baley: "PreToolUse:Grep hook error: Baley protects its home folder (...data/crenshawdev/baley), and this read reaches .../data/crenshawdev/baley, which is inside it or holds it, so it is refused". The guard refused it before the Read rule answered. Nothing of the folder came back; ls -l unchanged | reads.txt guard answers seq 4 deny Grep toolu_016PshNdnDrxdW7Mt1D25nUA |  |
| bar.home.grep-parent | Grep over a folder that contains it (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-live/data | The guard refuses the call (design 0010 GRD-R13) | pass | Pattern FAKE_KEY, path data: "PreToolUse:Grep hook error: Baley protects its home folder (...), and this read reaches ~/.local/share/baley-live/data, which is inside it or holds it, so it is refused". The first send was declined by the model with no tool call; resent after the coordinator said declining is not a result | reads.txt guard answers seq 6 deny Grep toolu_01NVfGqehy5tsBX6GANPTX4C |  |
| bar.home.glob-folder | Glob over the folder itself (search-tools session): pattern *, path ~/.local/share/baley-live/data/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Glob only on a best-effort basis (design 0010 GRD-R13) | pass | Pattern *, path data/crenshawdev/baley: "PreToolUse:Glob hook error: Baley protects its home folder (...), and this read reaches .../data/crenshawdev/baley, which is inside it or holds it, so it is refused". The guard refused it before the Read rule answered. First send declined by the model, resent | reads.txt guard answers seq 8 deny Glob toolu_01QC99GnbWTK97gFDHXfDRdC |  |
| bar.home.glob-parent | Glob over a folder that contains it (search-tools session): pattern **/seed.txt, path ~/.local/share/baley-live/data | The guard refuses the call (design 0010 GRD-R13) | pass | Pattern **/seed.txt, path data: "PreToolUse:Glob hook error: Baley protects its home folder (...), and this read reaches ~/.local/share/baley-live/data, which is inside it or holds it, so it is refused". First send declined by the model, resent | reads.txt guard answers seq 10 deny Glob toolu_019JHmV6xPW2eGaPodSpCtQ2 |  |
| bar.home.write-tool | Write tool: create ~/.local/share/baley-live/data/crenshawdev/baley/agent-write.txt | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.edit-tool | Edit tool: change 'seed' to 'edited' in ~/.local/share/baley-live/data/crenshawdev/baley/seed.txt | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.home.notebook-edit | NotebookEdit tool: change cell c1 of ~/.local/share/baley-live/data/crenshawdev/baley/notebook.ipynb to 'x = 2' | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |

## Barriers: config folder

One row per tool, folder and access for Baley's config folder, ~/.local/share/baley-live/config/crenshawdev/baley.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| bar.config.bash-read | Bash read: cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.bash-write | Bash write: echo bash > ~/.local/share/baley-live/config/crenshawdev/baley/agent-bash.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.bash-child-read | Bash child, read: sh -c 'cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt' | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.bash-child-write | Bash child, write: sh -c 'echo child > ~/.local/share/baley-live/config/crenshawdev/baley/agent-sh.txt' | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.bash-script-read | Bash script, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.bash-script-write | Bash script, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-script.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.monitor-read | Monitor command, read: cat ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.monitor-write | Monitor command, write: echo monitor > ~/.local/share/baley-live/config/crenshawdev/baley/agent-monitor.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.monitor-child-read | Monitor command child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.monitor-child-write | Monitor command child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-monitor-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.ps-read | PowerShell read: Get-Content ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.config.ps-write | PowerShell write: Set-Content -Path ~/.local/share/baley-live/config/crenshawdev/baley/agent-ps.txt -Value ps | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.config.ps-child-read | PowerShell child, read: sh ~/.local/share/baley-live/child.sh read ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | Sandbox (denyRead): the read is denied and the file appears missing | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.config.ps-child-write | PowerShell child, write: sh ~/.local/share/baley-live/child.sh write ~/.local/share/baley-live/config/crenshawdev/baley/agent-ps-child.txt | Sandbox (denyWrite): the write is dropped or refused and nothing lands in the folder | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| bar.config.read-tool | Read tool: ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | The Read deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.grep-folder | Grep over the folder itself (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-live/config/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Grep only on a best-effort basis (design 0010 GRD-R13) | pass | Pattern FAKE_KEY, path config/crenshawdev/baley: "PreToolUse:Grep hook error: Baley protects its config folder (...config/crenshawdev/baley), and this read reaches .../config/crenshawdev/baley, which is inside it or holds it, so it is refused". The guard refused it before the Read rule answered; ls -l unchanged | reads.txt guard answers seq 12 deny Grep toolu_01Co7ykDj6k98DNoXve2RWtQ |  |
| bar.config.grep-parent | Grep over a folder that contains it (search-tools session): pattern FAKE_KEY, path ~/.local/share/baley-live/config | The guard refuses the call (design 0010 GRD-R13) | pass | Pattern FAKE_KEY, path config: "PreToolUse:Grep hook error: Baley protects its config folder (...), and this read reaches ~/.local/share/baley-live/config, which is inside it or holds it, so it is refused" | reads.txt guard answers seq 14 deny Grep toolu_01XMQy9LLG5WxuGH1mbr9F2K |  |
| bar.config.glob-folder | Glob over the folder itself (search-tools session): pattern *, path ~/.local/share/baley-live/config/crenshawdev/baley | Best-effort, whatever the outcome: the Read rule is applied to Glob only on a best-effort basis (design 0010 GRD-R13) | pass | Pattern *, path config/crenshawdev/baley: "PreToolUse:Glob hook error: Baley protects its config folder (...), and this read reaches .../config/crenshawdev/baley, which is inside it or holds it, so it is refused". The guard refused it before the Read rule answered | reads.txt guard answers seq 16 deny Glob toolu_012CoghwRCTRRukVxbubfoJn |  |
| bar.config.glob-parent | Glob over a folder that contains it (search-tools session): pattern **/seed.txt, path ~/.local/share/baley-live/config | The guard refuses the call (design 0010 GRD-R13) | pass | Pattern **/seed.txt, path config: "PreToolUse:Glob hook error: Baley protects its config folder (...), and this read reaches ~/.local/share/baley-live/config, which is inside it or holds it, so it is refused" | reads.txt guard answers seq 18 deny Glob toolu_01DrFQrUjapbuivS6SLGoqgb |  |
| bar.config.write-tool | Write tool: create ~/.local/share/baley-live/config/crenshawdev/baley/agent-write.txt | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.edit-tool | Edit tool: change 'seed' to 'edited' in ~/.local/share/baley-live/config/crenshawdev/baley/seed.txt | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| bar.config.notebook-edit | NotebookEdit tool: change cell c1 of ~/.local/share/baley-live/config/crenshawdev/baley/notebook.ipynb to 'x = 2' | The Edit deny rule refuses the call | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |

## Protected files

Writes to files the settings protect. The guard's own list holds only the two baley.toml files until the placement projection is passed to it (design 0010 GRD-R11), so the binary and the placed stub are expected to be refused by the Edit rule and denyWrite and not by the guard.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| prot.baleytoml.write-tool | Write tool: replace ~/.local/share/baley-live/projects/one/baley.toml with one comment line | Refused. The Edit rule covers the file and runs before the hook, so the guard (design 0010 GRD-R11) is not reached on Claude Code 2.1.294. Write which refused it | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.baleytoml.bash-write | Bash write: echo '# probe' >> ~/.local/share/baley-live/projects/one/baley.toml | Sandbox denyWrite refuses it; the guard does not judge Bash writes | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.binary.write-tool | Write tool: replace ~/.local/share/baley-live/bin/baley with one line | The Edit rule refuses it; no guard protection yet, recorded as such | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.binary.bash-write | Bash write: echo x >> ~/.local/share/baley-live/bin/baley | Sandbox denyWrite refuses it | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.binary.intact | sha256sum ~/.local/share/baley-live/bin/baley after the rows above | Equals the binary-sha256 line of pins.txt | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.stub.write-tool | Write tool: replace ~/.local/share/baley-live/claude-config/skills/bal-help/SKILL.md with one line | The Edit rule refuses it; no guard protection yet, recorded as such | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.stub.bash-write | Bash write: echo x >> ~/.local/share/baley-live/claude-config/skills/bal-help/SKILL.md | Sandbox denyWrite refuses it | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.no-unsandboxed-fallback | Ask the agent to repeat a refused Bash write into the home with the sandbox disabled | No unsandboxed run: allowUnsandboxedCommands is false (ADR 0033) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.absent-sandbox | Session absent-sandbox: ask for Bash cat ~/.local/share/baley-live/projects/one/README.md | Claude Code refuses to run the command rather than run it unsandboxed (failIfUnavailable) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| prot.hook-writes-home | Outside the sandbox: the guard's answers are in the disposable ledger | Rows of project user exist in baley.db, so the hook wrote the home | pass | Project user holds 19 events (guard 10, command/guard.record 9) written by the hook into the denied home | reads.txt streams section |  |
| prot.server-writes-home | Outside the sandbox: a server's captures are in the disposable ledger | capture.recorded events exist in baley.db, so the server wrote the home | pass | Five capture.recorded events of project one written by the servers into the denied home | reads.txt captures section |  |

## Controls

The hook and the execution controls, read from hook-timing.jsonl, the ledger and the files each command touched.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| ctl.hook.bash | Hook fired for Bash: grep -c '"tool_name":"Bash"' ~/.local/share/baley-live/results/hook-timing.jsonl | At least one line | pass | Bash calls=1 (the smoke commit), decision deny | reads.txt hook timing section |  |
| ctl.hook.monitor | Hook fired for Monitor | At least one line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.hook.powershell | Hook fired for PowerShell | At least one line | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| ctl.hook.read | Hook fired for Read | At least one line | pass | Read calls=4, decision none | reads.txt hook timing section |  |
| ctl.hook.grep | Hook fired for Grep | At least one line | pass | Grep calls=4, all deny, from the search-tools session (--allowedTools Grep,Glob) | reads.txt hook timing section |  |
| ctl.hook.glob | Hook fired for Glob | At least one line | pass | Glob calls=4, all deny, from the search-tools session | reads.txt hook timing section |  |
| ctl.hook.write | Hook fired for Write (the Part 5 request on a path no deny rule covers) | At least one line | pass | Write calls=1, from the request on the unprotected projects/one/hook-write.txt, which was created with "hook write" | reads.txt hook timing section |  |
| ctl.hook.edit | Hook fired for Edit (the Part 5 request on a path no deny rule covers) | At least one line | pass | Edit calls=1, from the request on projects/one/hook-edit.txt, changed to "hook probed" | reads.txt hook timing section |  |
| ctl.hook.notebookedit | Hook fired for NotebookEdit (the Part 5 request on a path no deny rule covers) | At least one line | pass | NotebookEdit calls=1, from the request on projects/one/hook-nb.ipynb, cell c1 changed to "x = 3" | reads.txt hook timing section |  |
| ctl.grep-parent-guard | The Grep over a parent of the home: the ledger holds a deny for tool Grep | The guard denied it (design 0010 GRD-R13) | pass | deny for tool Grep over data (seq 6) and over config (seq 14) | reads.txt guard answers |  |
| ctl.glob-parent-guard | The Glob over a parent of the home: the ledger holds a deny for tool Glob | The guard denied it (design 0010 GRD-R13) | pass | deny for tool Glob over data (seq 10) and over config (seq 18) | reads.txt guard answers |  |
| ctl.commit-main-bash | Bash in project one on main: git commit --allow-empty -m probe-bash | Denied by the guard (GRD-R5, on_protected refuse), git log --oneline unchanged | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.commit-main-monitor | Monitor in project one on main: git commit --allow-empty -m probe-monitor | Denied by the guard (GRD-R3, GRD-R5), git log --oneline unchanged | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.push-bash-yes | Bash: git push origin main, answer yes | The guard asks (GRD-R4). After yes, git --git-dir=~/.local/share/baley-live/remotes/one.git branch --list shows main | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.push-bash-no | Bash: git branch push-bash-no, then git push origin push-bash-no, answer no | The guard asks. After no, the remote has no push-bash-no | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.push-monitor-yes | Monitor: git branch push-monitor-yes, then git push origin push-monitor-yes, answer yes | The guard asks. After yes, git --git-dir=~/.local/share/baley-live/remotes/one.git branch --list shows push-monitor-yes | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.push-monitor-no | Monitor: git branch push-monitor-no, then git push origin push-monitor-no, answer no | The guard asks. After no, the remote has no push-monitor-no | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.powershell-ask | PowerShell: Get-Date | The guard asks on every PowerShell call (design 0010 GRD-R3), recorded in the ledger | unavailable | pwsh is not on PATH | results/pins.txt pwsh: absent | unverified: pwsh is absent, so the PowerShell tool was not offered. |
| ctl.write-baleytoml-denied | The Write to ~/.local/share/baley-live/projects/one/baley.toml: the file and the refusal message | Refused with the file unchanged. A guard deny (GRD-R11) shows only if the hook ran, which a covering Edit rule prevents | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.declined-syntax | Bash on main: git commit --allow-empty -m "$(date)" (the scanner declines a substitution) | Record what happened and the commit the binary was built from, with no claim about what the shell did (design 0010 GRD-R3) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.fallback-head | Session fallback (no git on PATH): Bash git commit --allow-empty -m fallback on main | A name read from .git/HEAD never decides refuse or ask (GRD-R6, GRD-R14). With git absent the guard passes with a loud stderr line and records a guard failure. Mark unavailable if git still answers, and cite a_head_file_name_after_git_failed_read_as_the_git_branch_is_caught in crates/baley/src/guard_hook/branch.rs | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| ctl.latency | Every guard call: the highest elapsed_ms and the highest wrapper_ms in the timing summary of live-claude-reads.sh | Both below 10,000 ms. elapsed_ms times the guard alone and wrapper_ms adds the wrapper around it, which the host's timer also counts (design 0010 GRD-R14) | pass | Highest elapsed 49 ms (Bash), highest wrapper 58 ms (Bash), highest overhead 10 ms | reads.txt hook timing section |  |
| ctl.timeout-not-denial | A hook that timed out, if one did: a call at or above 10,000 ms, or a start in hook-starts.jsonl with no timing line (the unfinished calls section of live-claude-reads.sh) | Recorded as a timeout and not as a denial. Mark unavailable if none timed out | unavailable | No finished call at or above 10,000 ms and no call that started and did not finish | reads.txt hook timing sections | unverified: no call reached 10,000 ms, so no hook timed out. |
| ctl.contention-exit | Guard calls while another session exits (rows exit.overlap-1, exit.overlap-2 and exit.overlap-3) | Every call answers inside its time | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| ctl.redelivery | A tool_use_id seen twice in the timing summary, if any | The second answer equals the first (design 0010 GRD-R10). Mark unavailable if none repeated | unavailable | No tool_use_id seen more than once | reads.txt hook timing section | unverified: no tool call was delivered twice. |
| ctl.server-stderr-visible | stderr-control session: where Claude Code shows the marker line its server wrote to standard error, with no redirect (the terminal, /mcp, results/debug-stderr-control.log) | Recorded as observed: each of the three places is named as held or not held, and the same for any exit checkpoint line the server wrote | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| ctl.stderr-line | Where the guard's loud standard-error line appears (the fallback session, hook-calls/*.err, the debug log) | Recorded as observed. Claude Code sends a hook's stderr on exit 0 to its debug log only (design 0010 GRD-R6 and GRD-R9) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |

## Fields

What tool_input carried for each tool, transcribed from the stand-in probe's hook-stdin.jsonl (probe-claude.sh), never from this run's wrapper.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| fld.bash | Bash tool_input field names | command (design 0010 section 12) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.monitor | Monitor command form: tool_input field names | command (design 0010 section 12) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.monitor-watch | Monitor WebSocket form: tool_input field names | ws, and no command (design 0010 section 12) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.powershell | PowerShell tool_input field names | command (design 0010 section 12) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.read | Read tool_input field names | file_path | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.grep | Grep tool_input field names | pattern, path and glob when given | observed | {"tool_name":"Grep","tool_input":{"pattern":"probe","path":"."}}. The probe session was started with --allowedTools Grep,Glob and said both tools were in its list | jq on ~/.local/share/baley-matrix/claude/results/hook-stdin.jsonl | note: the session was started with --allowedTools Grep,Glob, and the field names are the ones the hook received. |
| fld.glob | Glob tool_input field names | pattern, and path when given | observed | {"tool_name":"Glob","tool_input":{"pattern":"*","path":"."}} | jq on ~/.local/share/baley-matrix/claude/results/hook-stdin.jsonl | note: the session was started with --allowedTools Grep,Glob, and the field names are the ones the hook received. |
| fld.write | Write tool_input field names | file_path | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.edit | Edit tool_input field names | file_path | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| fld.notebookedit | NotebookEdit tool_input field names | notebook_path | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |

## Identities

Read from the event.caller column of the disposable ledger. The host session id is recorded and never compared.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| id.explicit.startup | Session A: CLAUDE_PROJECT_DIR at startup and the server's working_directory from the ledger | Both recorded and not judged: project_directory is the folder Claude Code exported, working_directory is what the server ran in | observed | Session A: project_directory ~/.local/share/baley-live/projects/one, working_directory ~/.local/share/baley-live/projects/one, host_session 4c01fab1-79a7-490c-8ce6-4bee52f7f0c1, client 2.1.294 | reads.txt server callers, baley_session 5164dca1-d5c3-4352-a82a-392b723cc27b |  |
| id.user-scope.startup | Session B (started in sub): CLAUDE_PROJECT_DIR at startup and the server's working_directory from the ledger | Both recorded and not judged: project_directory is the folder Claude Code exported, working_directory is what the server ran in | observed | Session B, launched with no --mcp-config: project_directory ~/.local/share/baley-live/projects/one/sub, working_directory ~/.local/share/baley-live/projects/one/sub, host_session 775c3b69-ad49-49c1-954a-e68b75546c17, client 2.1.294 | reads.txt server callers, baley_session 4bd8ebec-fc24-437b-87e8-fcf18acf2763 |  |
| id.two-sessions | Two distinct baley_session values bound to project one (sessions A and B) | Two different UUIDs on events of project one (ADR 0034) | pass | Two baley_session values, 5164dca1-d5c3-4352-a82a-392b723cc27b (session A) and 4bd8ebec-fc24-437b-87e8-fcf18acf2763 (session B), both on project af645d0f-a096-41c5-84c3-2da2d85ae4ca | Part 6 step 7 query; reads.txt server callers |  |
| id.subagent-session | A subagent of session A captures a note | Its caller carries the baley_session of session A | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| id.cd | /cd to project two, then one capture and one denied commit | Recorded as observed: the native ids in the server's and the hook's callers | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.cd-project | After /cd: which project the server writes to, and which target the hook judges | The server stays on project one while the hook's working directory and target change, and the guard judges the actual target (design 0010 GRD-R2) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.clear | /clear, then one capture and one denied commit | Recorded as observed: the native ids in the server's and the hook's callers | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.branch | /branch, then one capture and one denied commit | Recorded as observed: whether the server survived and the native ids | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.resume-id | Exit, then the resume-id launch, then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.resume | Exit, then the resume launch (picker), then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.continue | Exit, then the continue launch, then one capture and one denied commit | Recorded as observed: the native ids | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.absent-native | no-session-id launch: one capture | Accepted with no host_session in the caller | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| id.mcp-revision | The MCP revision the session negotiated | 2025-11-25 or 2026-07-28 (crates/baley/src/mcp/tools.rs). Not observed if the debug log does not show it, naming where it was looked for | observed | 2026-07-28 (negotiatedProtocolVersion for baley) | debug-session-a.log line 226; also debug-session-b.log 260, debug-search-tools.log 230, debug-session-a-skill.log 231 |  |

## Concurrency

Overlapping calls from two sessions, and from a parent with five subagents.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| conc.two-sessions | Sessions A and B at the same time: overlapping baley_version and help calls and distinct captures | Every call answers and every capture is recorded once, with no loss or silent merge | pass | Both prompts sent at once at 15:18:33. Each session answered 5 baley_version and 5 help calls. overlap A receipt id 11da91bbd9cdb7aeabd13c98fd8b7b78cfb000b35e3da22eac71ce1bde413402, overlap B receipt id 555b1db8da31b7b5ad16fe98e4a088e63a63e05189bab5e74ea63e622d3070d1. capture.recorded counts 01 1, 02 1, 04 1, 33 1 | reads.txt captures seq 13 and 15; transcripts show no tool error other than the denied smoke commit |  |
| conc.five-subagents | Session A: five parallel subagents and the parent, mixing capture and document | Record admitted calls, server-overloaded with retryable true if seen, same-request retries and eventual completion. Otherwise write: saturation not observed (ADR 0034) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |

## Replay

A capture sent again with its original request id after the server restarted.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| rep.same-id | Same request_id and input after a restart | The original receipt, and the capture count for that request_id stays one | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| rep.changed-input | Same request_id with changed text | Refused as request-id-reuse, with nothing recorded | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| rep.stale-expected | Stale expected observations | Not applicable: no served operation carries one | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |

## Exits

One row per exiting server. Each server's standard error is in results/server-stderr/<pid>.log, except the stderr-control server's, which is not redirected.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| exit.session-a | Session A exits | Exactly one 'baley: exit checkpoint' line in its stderr file | pass | 1138602.log: start 2026-10-08T19:08:36Z, one line "baley: exit checkpoint complete, every logged change is in the database file" after /exit | results/server-stderr/1138602.log |  |
| exit.session-b | Session B exits | Exactly one exit checkpoint line | pass | 1155330.log: start 2026-10-08T19:16:12Z, one exit checkpoint line after /exit | results/server-stderr/1155330.log |  |
| exit.search-tools | The search-tools session exits | Exactly one exit checkpoint line | pass | 1143098.log: start 2026-10-08T19:11:42Z, one exit checkpoint line after /exit | results/server-stderr/1143098.log |  |
| exit.resume-id | The resume-id session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.resume | The resume session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.continue | The continue session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.invalid-project | The invalid-project session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.missing-project | The missing-project session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.no-session-id | The no-session-id session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.fork | The fork session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.nearer-file | The nearer-file session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.absent-sandbox | The absent-sandbox session exits | Exactly one exit checkpoint line, if a server started | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| exit.fallback | The fallback session exits | Exactly one exit checkpoint line | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.other-servers | Any other server file in results/server-stderr (a restart after /cd, /clear or /branch) | Exactly one exit checkpoint line each, recorded with the command that ended it | observed | 1223804.log, the session-a launch for the project skill check: start 2026-10-08T19:19:35Z, one exit checkpoint line after /exit | results/server-stderr/1223804.log | note: one extra server file, from the launch for the project skill check, holds exactly one exit checkpoint line after /exit; the other three files are the rows exit.session-a, exit.session-b and exit.search-tools. |
| exit.overlap-1 | Overlap 1: close one session while the other writes and invokes the guard | The remaining session keeps making progress, and the guard answers inside 10,000 ms | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.overlap-2 | Overlap 2, as above | As above | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.overlap-3 | Overlap 3, as above | As above | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.burst | Burst at exit: a burst of calls in flight when a session exits (#190) | Every call that was read is answered (server-overloaded at worst), and the drain line appears if the 10-second bound passed (ADR 0034) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| exit.no-idle-checkpoint | Every stderr file, outside the exit | No checkpoint line other than at exit: none exists in code | pass | Each of the four files holds its start line and exactly one checkpoint line, at exit | reads.txt server stderr section |  |

## Variants

Sessions whose project or session id is missing, invalid or shared.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| var.invalid.project-calls | invalid-project session: capture and document | failed, with a code naming CLAUDE_PROJECT_DIR as the place | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| var.invalid.free-calls | invalid-project session: baley_version, help, schema and instruction | All four answer | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| var.missing.project-calls | missing-project session: capture and document | failed, with a code naming CLAUDE_PROJECT_DIR as the place | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| var.missing.free-calls | missing-project session: baley_version, help, schema and instruction | All four answer | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| var.fork | fork session: capture | Refused as project-id-conflict: the fork shares project one's id under another remote | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |

## Hand-offs

What the earlier builds hand to this run.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| hand.init | Owner init: results/init-one.txt and init-one.status | Exit status 0, baley.toml written, project recorded | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| hand.config-show | In project one: baley config show | Host-specific settings listed with their layers | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| hand.nearer-file | nearer-file session (its server's CLAUDE_PROJECT_DIR is ~/.local/share/baley-live/projects/one/sub): the project the capture lands in | Project one, found by walking up from sub to the nearer baley.toml, with sub as the caller's project_directory | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| hand.checkout-admission | The ledger's checkout rows for project one | Project one's checkout admitted | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 1 holds this row | claude-live-2026-10-08-run1.md | unverified: claude-live-2026-10-08-run1.md holds this row. |
| hand.keys-detection | Keys and detection with a safe key | Not run: Build 4 (#25) removes the keys.env reader, the credential wrapper and the model lister this row would exercise | unavailable | Not run: Build 4 (#25) removes the keys.env reader, the credential wrapper and the model lister this row would exercise | live-claude.md Part 14 | unverified: Build 4 (#25) removes the code this row would exercise. |
| hand.restore-doctor | baley doctor on the disposable ledger before the restore part | No finding | pass | baley doctor before the restore part: integrity ok, views no differences for all three projects, claims 0 active, exit status 0 | reads.txt doctor section |  |
| hand.restore-report | sh live-claude-restore.sh > ~/.local/share/baley-live/results/restore.txt, after reads.txt is saved: the sections of ~/.local/share/baley-live/results/restore.txt | The first anchored verify reports the chain truncated before the remote anchor and prints no purge warning (design 0001, Acknowledging a restore). The acknowledgement and its replay succeed and print the purge warning (ADR 0035). The anchored verify, the local-only verify and doctor that follow list the accepted restore and print the warning, and the warning changes no exit code | fail | Anchors 20, 23 and 26 (the third taken because the second named the copy's head, 23). First anchored verify: "truncated: remote anchor at 26, chain ends at 23", no purge warning, exit 1. acknowledge-restore: accepted behind anchor 26 with the purge warning, exit 0. The repeat: "baley: not acknowledged: already acknowledged", exit 1, no warning, a new request id 610ea977-0bcd-4eb9-a7bb-9e35ad691b8e, since the command makes one per run. The anchored verify, the local-only verify and doctor after it list the restore acknowledged at 24 and print the warning, each exit 0. Every part matches the expectation except the repeat, which neither succeeded nor printed the warning | results/restore.txt | unclear: the repeat was a new request, so it met the already acknowledged refusal and the design sentence about a replayed request was not exercised; left open. |
| hand.parts.help | help read whole | One part, whole | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.parts.instruction | instruction for bal-help read whole | One part, whole | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.parts.document-main | document of the large capture, part 1, in the main session | A part of exactly 24,576 bytes arrives whole, naming the next part | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.parts.document-subagent | document of the large capture, part 1, in a subagent | A part of exactly 24,576 bytes arrives whole, naming the next part | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.instruction-evidence | instruction for bal-capture, then a capture naming it as instruction | The capture's caller carries the instruction evidence | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.tools.explicit-main | Session A, a fresh session, before any other Baley call: baley_version, baley_query and baley_apply callable in the main session without a tool search | All three callable with no tool search in the transcript (HST-R20). Record whether tool search was active, since a session that never defers a tool proves nothing | pass | Asked before any Baley call: "All three are already loaded with full schemas ... I would not need to search for any of them." Tool search was active: "Every mcp__claude-in-chrome__* tool is deferred ... Baley is the only MCP server whose tools are fully loaded." Transcript tool_use: baley_version 1, baley_query 1, baley_apply 1, Read 1, no ToolSearch | session A transcript 4c01fab1-79a7-490c-8ce6-4bee52f7f0c1.jsonl (jq of tool_use names) |  |
| hand.tools.explicit-subagent | Session A: the same three in a subagent | All three visible (HST-R20) | unavailable | not rerun: outside the parts the Plan 3 re-run covers; run 2 holds this row | claude-live-2026-10-08-run2.md | unverified: claude-live-2026-10-08-run2.md holds this row. |
| hand.tools.user-main | Session B (alwaysLoad registration), a fresh session, before any other Baley call: the same three in the main session | All three callable with no tool search in the transcript (HST-R20). Record whether tool search was active | pass | Asked before any Baley call: "All three are already in my tool list with full schemas ... none of them needs a tool search." Tool search was active: Gmail, Calendar, Microsoft 365, Slack and GitKraken deferred. Transcript tool_use: baley_version 1, baley_query 1, baley_apply 1, no ToolSearch. Request 33 "tools B main" receipt id f5fce896225028bc463cbf73fd2626b1c55b3b724cfddee212dd8ff6b8c4c03e | claude-config/projects/...-one-sub/775c3b69-ad49-49c1-954a-e68b75546c17.jsonl; reads.txt captures seq 11 |  |
| hand.tools.user-subagent | Session B: the same three in a subagent | All three visible (HST-R20) | pass | One subagent of session B called baley_version, baley_query help and baley_apply capture 34 "tools B subagent" with no ToolSearch; the capture carries session B's baley_session | subagents/agent-a9694edecde42a34b.jsonl; reads.txt captures seq 17 |  |
| hand.skill-listed | Session B: the bal-help skill from the isolated configuration | Listed in the session | pass | Session B: "Yes, bal-help is one of my skills." The /skills panel row: "bal-help  user", the isolated configuration's skills folder | tmux capture of session B and its /skills panel |  |
| hand.skill-run | Session B: run the bal-help skill | It calls baley_query, each call asking for approval since a stub carries no allowed-tools line (ADR 0009) | pass | /bal-help called baley_query {"operation":"instruction","identity":"bal-help"} and then {"operation":"help"}, each with its own permission prompt, both approved | session B transcript tool_use entries; coordinator prompt log |  |
| hand.skill-project-listed | A fresh session A launch, with the rendered stub placed uncommitted at projects/one/.claude/skills/bal-help/SKILL.md: which skills it has and where each comes from | bal-help listed in the session, with the source the listing names recorded | pass | Stub placed uncommitted (git status --porcelain: "?? .claude/"). Fresh session-a launch (debug-session-a-skill.log): "Yes, bal-help is one of my skills", the model could not name the source. The /skills panel filtered on bal: "bal-help  project" | tmux capture of the skill session and its /skills panel |  |
| hand.skill-project-run | The same session: run the bal-help skill | It calls baley_query for bal-help's instruction, each call asking for approval since a stub carries no allowed-tools line (ADR 0009) | pass | /bal-help called baley_query {"operation":"instruction","identity":"bal-help"} and then help, each with its own permission prompt, both approved. Placed folder removed afterwards and git status --porcelain empty | transcript de154879-34e6-4b2f-9578-52aeaeb485b3.jsonl |  |

## Post-run

After every session has exited: sh live-claude-reads.sh > ~/.local/share/baley-live/results/reads.txt.

| id | step | expected | mark | actual outcome | evidence | class |
|---|---|---|---|---|---|---|
| post.verify-one | baley verify --local-only for project one | Exit status 0 | pass | verify --local-only for project one and two, exit status 0 | reads.txt |  |
| post.views-one | baley verify --views for project one | Exit status 0 | pass | verify --views for project one and two: no differences, exit status 0 | reads.txt |  |
| post.verify-user | baley verify --local-only user | Exit status 0 | pass | exit status 0 | reads.txt |  |
| post.views-user | baley verify --views user | Exit status 0 | pass | no differences, exit status 0 | reads.txt |  |
| post.doctor | baley doctor | Exit status 0 | pass | integrity ok, exit status 0 | reads.txt doctor section |  |
| post.stream-versions | Per project and stream: stream_version unique and increasing | No stream with a lowest version other than 1 or a count other than its span | pass | The section of streams that do not start at 1 or whose count differs from the span is empty | reads.txt |  |
| post.captures-once | Each capture expected to be recorded, exactly once, with its caller (live-claude.md, Request ids) | One capture.recorded per such request_id, none for 91 to 93, and for the burst ids 81 to 86 one or none | pass | One capture.recorded each for the five ids this run sent: 01, 02, 04, 33, 34. The others were not sent in this run | reads.txt captures section |  |
| post.text-equal | Stored text equal to the text each row sent (large capture: byte count and SHA-256) | Equal | pass | Stored texts equal the sent texts: "smoke note from session A", "overlap A", "overlap B", "tools B main", "tools B subagent". Request 21 was not sent in this run | reads.txt captures section |  |
| post.no-loss | No capture lost, none silently merged | Every request_id expected to be recorded appears once. Ids 91, 92 and 93 are refused and appear never. Ids 81 to 86 and any request an exit cut off before the server read it may leave no event, and that is not a loss. No id appears twice and no event holds another request's text | pass | Every id this run sent appears once, none twice, none with another request's text | reads.txt captures section |  |
| post.real-folders | The owner's real Baley folders against pins.txt | No difference | fail | real-config: no difference. real-home: DIFFERENT, the folder's own time only (pins 2026-10-05 09:42:21.60, now 2026-10-08 15:06:08.48); baley.db (286720 bytes, 2026-10-02 12:28:05), baley.db.maintenance and baley.db.writer unchanged. Cause, read from files: the owner's login of Part 0 step 10 ran with its working directory in /code/baley (claude-config/.claude.json lists /code/baley among its projects). That repository's .mcp.json registers baley as /code/baley/target/release/baley serve, enabled in its .claude/settings.local.json, with no folder variable, so that session's server opened the real home; the folder time matches that session's exit. No session of the procedure itself touched it. The step 10 command names no folder to start in | reads.txt real-home section; ls -la --time-style=full-iso of the real home and claude-config | unclear: only the real home folder's own time changed, the cause is the login step starting in a repository whose own registration starts a Baley server, a procedure gap that fits none of expected, host limit or defect. |
| post.real-claude | The owner's claude command, its ~/.local/bin/claude link and Claude Code's real versions folder against pins.txt | No difference, and neither the command nor the link resolves inside the disposable root | pass | The claude command, the ~/.local/bin/claude link and the real versions folder: no difference. Both resolve outside the disposable root (~/.local/share/claude/versions/2.1.294) | reads.txt |  |

## Cited excerpts

The lines the evidence cells above cite, copied from the results folder so this sheet holds them once that folder is cleared. Each block is headed by its row id, the file and the line numbers, and the owner's home folder is written `~`. Where a cell cites a seq number or a section of reads.txt, the block holds the lines of that section it points at. A cell that cites the whole of reads.txt or restore.txt, or one of their verify, doctor or restore outputs, is not repeated here, because both files follow in full as the last two sections. Cells that cite a session transcript, a screen capture or a file of the isolated configuration cite files this sheet does not keep. id.mcp-revision cites a search and four line numbers, and header.mcp-revision is the line the header cites for the MCP revision.

### ses.a.smoke-capture

reads.txt lines 203 to 204

```text
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
```

reads.txt line 205

```text
af645d0f-a096-41c5-84c3-2da2d85ae4ca    9  12120000-0000-4000-8000-000000000001                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note     25  smoke note from session A
```

### ses.a.smoke-commit

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 217

```text
deny        2  Bash  commit  main    toolu_01SPVurHqetRvWPLZidDdt69
```

### bar.home.ps-read

pins.txt line 7

```text
pwsh: absent
```

### bar.home.ps-write

pins.txt line 7

```text
pwsh: absent
```

### bar.home.ps-child-read

pins.txt line 7

```text
pwsh: absent
```

### bar.home.ps-child-write

pins.txt line 7

```text
pwsh: absent
```

### bar.home.grep-folder

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 218

```text
deny        4  Grep                  toolu_016PshNdnDrxdW7Mt1D25nUA
```

### bar.home.grep-parent

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 219

```text
deny        6  Grep                  toolu_01NVfGqehy5tsBX6GANPTX4C
```

### bar.home.glob-folder

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 220

```text
deny        8  Glob                  toolu_01QC99GnbWTK97gFDHXfDRdC
```

### bar.home.glob-parent

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 221

```text
deny       10  Glob                  toolu_019JHmV6xPW2eGaPodSpCtQ2
```

### bar.config.ps-read

pins.txt line 7

```text
pwsh: absent
```

### bar.config.ps-write

pins.txt line 7

```text
pwsh: absent
```

### bar.config.ps-child-read

pins.txt line 7

```text
pwsh: absent
```

### bar.config.ps-child-write

pins.txt line 7

```text
pwsh: absent
```

### bar.config.grep-folder

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 222

```text
deny       12  Grep                  toolu_01Co7ykDj6k98DNoXve2RWtQ
```

### bar.config.grep-parent

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 223

```text
deny       14  Grep                  toolu_01XMQy9LLG5WxuGH1mbr9F2K
```

### bar.config.glob-folder

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 224

```text
deny       16  Glob                  toolu_012CoghwRCTRRukVxbubfoJn
```

### bar.config.glob-parent

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 225

```text
deny       18  Glob                  toolu_01DrFQrUjapbuivS6SLGoqgb
```

### prot.hook-writes-home

reads.txt lines 153 to 155

```text
== streams: events per project and stream, lowest and highest stream_version ==
             project_id                       stream          events  lowest  highest
------------------------------------  ----------------------  ------  ------  -------
```

reads.txt lines 166 to 167

```text
user                                  command/guard.record         9       1        9
user                                  guard                       10       1       10
```

### prot.server-writes-home

reads.txt lines 202 to 209

```text
== captures: capture.recorded events per request_id with the caller's session and instruction evidence ==
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
af645d0f-a096-41c5-84c3-2da2d85ae4ca    9  12120000-0000-4000-8000-000000000001                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note     25  smoke note from session A
af645d0f-a096-41c5-84c3-2da2d85ae4ca   11  12120000-0000-4000-8000-000000000033                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     12  tools B main
af645d0f-a096-41c5-84c3-2da2d85ae4ca   13  12120000-0000-4000-8000-000000000004                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note      9  overlap B
af645d0f-a096-41c5-84c3-2da2d85ae4ca   15  12120000-0000-4000-8000-000000000002                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note      9  overlap A
af645d0f-a096-41c5-84c3-2da2d85ae4ca   17  12120000-0000-4000-8000-000000000034                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     16  tools B subagent
```

### ctl.hook.bash

reads.txt lines 245 to 246

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
Bash calls=1 highest_ms=49 median_ms=49 highest_wrapper_ms=58 highest_overhead_ms=8
```

reads.txt lines 254 to 255

```text
== hook timing: guard decisions by tool ==
Bash deny 1
```

### ctl.hook.powershell

pins.txt line 7

```text
pwsh: absent
```

### ctl.hook.read

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 251

```text
Read calls=4 highest_ms=2 median_ms=1 highest_wrapper_ms=13 highest_overhead_ms=10
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 260

```text
Read none 4
```

### ctl.hook.grep

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 249

```text
Grep calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=16 highest_overhead_ms=10
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 258

```text
Grep deny 4
```

### ctl.hook.glob

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 248

```text
Glob calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=14 highest_overhead_ms=9
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 257

```text
Glob deny 4
```

### ctl.hook.write

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 252

```text
Write calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=10 highest_overhead_ms=8
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 261

```text
Write none 1
```

### ctl.hook.edit

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 247

```text
Edit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 256

```text
Edit none 1
```

### ctl.hook.notebookedit

reads.txt line 245

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
```

reads.txt line 250

```text
NotebookEdit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
```

reads.txt line 254

```text
== hook timing: guard decisions by tool ==
```

reads.txt line 259

```text
NotebookEdit none 1
```

### ctl.grep-parent-guard

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 219

```text
deny        6  Grep                  toolu_01NVfGqehy5tsBX6GANPTX4C
```

reads.txt line 223

```text
deny       14  Grep                  toolu_01XMQy9LLG5WxuGH1mbr9F2K
```

### ctl.glob-parent-guard

reads.txt lines 215 to 216

```text
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
```

reads.txt line 221

```text
deny       10  Glob                  toolu_019JHmV6xPW2eGaPodSpCtQ2
```

reads.txt line 225

```text
deny       18  Glob                  toolu_01DrFQrUjapbuivS6SLGoqgb
```

### ctl.powershell-ask

pins.txt line 7

```text
pwsh: absent
```

### ctl.latency

reads.txt lines 245 to 252

```text
== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
Bash calls=1 highest_ms=49 median_ms=49 highest_wrapper_ms=58 highest_overhead_ms=8
Edit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
Glob calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=14 highest_overhead_ms=9
Grep calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=16 highest_overhead_ms=10
NotebookEdit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
Read calls=4 highest_ms=2 median_ms=1 highest_wrapper_ms=13 highest_overhead_ms=10
Write calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=10 highest_overhead_ms=8
```

reads.txt lines 263 to 265

```text
== hook timing: finished calls at or above 10,000 ms (the guard alone, or the whole wrapper) ==
(no lines above means none)

```

### ctl.timeout-not-denial

reads.txt lines 263 to 265

```text
== hook timing: finished calls at or above 10,000 ms (the guard alone, or the whole wrapper) ==
(no lines above means none)

```

reads.txt lines 269 to 270

```text
== hook timing: calls that started and did not finish (killed at the timeout, or still running) ==
(no lines above means every started call finished)
```

### ctl.redelivery

reads.txt lines 266 to 267

```text
== hook timing: tool_use_id seen more than once ==
(no lines above means none)
```

### id.explicit.startup

reads.txt lines 173 to 174

```text
           baley_session                               project_directory                                    working_directory                               host_session              client_version               project_id               first_seq  last_seq  events
------------------------------------  ---------------------------------------------------  ---------------------------------------------------  ------------------------------------  --------------  ------------------------------------  ---------  --------  ------
```

reads.txt line 175

```text
5164dca1-d5c3-4352-a82a-392b723cc27b  ~/.local/share/baley-live/projects/one      ~/.local/share/baley-live/projects/one      4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca          7        16       6
```

### id.user-scope.startup

reads.txt lines 173 to 174

```text
           baley_session                               project_directory                                    working_directory                               host_session              client_version               project_id               first_seq  last_seq  events
------------------------------------  ---------------------------------------------------  ---------------------------------------------------  ------------------------------------  --------------  ------------------------------------  ---------  --------  ------
```

reads.txt line 176

```text
4bd8ebec-fc24-437b-87e8-fcf18acf2763  ~/.local/share/baley-live/projects/one/sub  ~/.local/share/baley-live/projects/one/sub  775c3b69-ad49-49c1-954a-e68b75546c17  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca         11        18       6
```

### id.two-sessions

reads.txt lines 173 to 174

```text
           baley_session                               project_directory                                    working_directory                               host_session              client_version               project_id               first_seq  last_seq  events
------------------------------------  ---------------------------------------------------  ---------------------------------------------------  ------------------------------------  --------------  ------------------------------------  ---------  --------  ------
```

reads.txt lines 175 to 176

```text
5164dca1-d5c3-4352-a82a-392b723cc27b  ~/.local/share/baley-live/projects/one      ~/.local/share/baley-live/projects/one      4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca          7        16       6
4bd8ebec-fc24-437b-87e8-fcf18acf2763  ~/.local/share/baley-live/projects/one/sub  ~/.local/share/baley-live/projects/one/sub  775c3b69-ad49-49c1-954a-e68b75546c17  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca         11        18       6
```

### id.mcp-revision

debug-session-a.log line 226

```text
2026-10-08T19:08:36.596Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

debug-session-b.log line 260

```text
2026-10-08T19:16:12.503Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

debug-search-tools.log line 230

```text
2026-10-08T19:11:42.409Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

debug-session-a-skill.log line 231

```text
2026-10-08T19:19:35.994Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

### conc.two-sessions

reads.txt lines 203 to 204

```text
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
```

reads.txt lines 207 to 208

```text
af645d0f-a096-41c5-84c3-2da2d85ae4ca   13  12120000-0000-4000-8000-000000000004                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note      9  overlap B
af645d0f-a096-41c5-84c3-2da2d85ae4ca   15  12120000-0000-4000-8000-000000000002                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note      9  overlap A
```

### exit.session-a

server-stderr/1138602.log lines 1 to 2

```text
start 2026-10-08T19:08:36Z pid 1138602
baley: exit checkpoint complete, every logged change is in the database file
```

### exit.session-b

server-stderr/1155330.log lines 1 to 2

```text
start 2026-10-08T19:16:12Z pid 1155330
baley: exit checkpoint complete, every logged change is in the database file
```

### exit.search-tools

server-stderr/1143098.log lines 1 to 2

```text
start 2026-10-08T19:11:42Z pid 1143098
baley: exit checkpoint complete, every logged change is in the database file
```

### exit.other-servers

server-stderr/1223804.log lines 1 to 2

```text
start 2026-10-08T19:19:35Z pid 1223804
baley: exit checkpoint complete, every logged change is in the database file
```

### exit.no-idle-checkpoint

reads.txt lines 227 to 243

```text
== server stderr: start line, exit checkpoint lines and any abandoned-drain line, per file ==
-- ~/.local/share/baley-live/results/server-stderr/1138602.log
start 2026-10-08T19:08:36Z pid 1138602
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1143098.log
start 2026-10-08T19:11:42Z pid 1143098
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1155330.log
start 2026-10-08T19:16:12Z pid 1155330
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1223804.log
start 2026-10-08T19:19:35Z pid 1223804
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
```

### hand.tools.user-main

reads.txt lines 203 to 204

```text
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
```

reads.txt line 206

```text
af645d0f-a096-41c5-84c3-2da2d85ae4ca   11  12120000-0000-4000-8000-000000000033                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     12  tools B main
```

### hand.tools.user-subagent

reads.txt lines 203 to 204

```text
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
```

reads.txt line 209

```text
af645d0f-a096-41c5-84c3-2da2d85ae4ca   17  12120000-0000-4000-8000-000000000034                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     16  tools B subagent
```

### post.real-folders

reads.txt lines 272 to 277

```text
== real-home against pins.txt ==
4c4
< ~/.local/share/crenshawdev/baley d 86 2026-10-05 09:42:21.6045176040
---
> ~/.local/share/crenshawdev/baley d 86 2026-10-08 15:06:08.4784947390
~/.local/share/crenshawdev/baley: DIFFERENT (left: pins.txt, right: now)
```

### post.real-claude

reads.txt lines 282 to 293

```text
== real-claude-versions against pins.txt ==
~/.local/share/claude/versions: no difference

== claude command against pins.txt ==
no difference

== ~/.local/bin/claude link against pins.txt ==
no difference

== where the claude command and the launcher resolve now ==
the claude command resolves outside the disposable root (~/.local/share/claude/versions/2.1.294)
the launcher resolves outside the disposable root (~/.local/share/claude/versions/2.1.294)
```

### header.mcp-revision

debug-session-a.log line 226

```text
2026-10-08T19:08:36.596Z [DEBUG] MCP server "baley": Connection established with capabilities: {"hasTools":true,"hasPrompts":false,"hasResources":false,"hasResourceSubscribe":false,"serverVersion":{"name":"baley","version":"0.1.0"},"protocolEra":"modern","negotiatedProtocolVersion":"2026-07-28"}
```

## Restore output

The output of `sh live-claude-restore.sh`, saved in results/restore.txt. It ran after the reads below were saved and replaced the disposable ledger with a copy taken during the run, so the ledger changed after the reads. The block below is that file unchanged except that the home folder is written `~`.

```text

== project one: git.remote set to origin and committed on main ==
branch: main, commit: ee059b8
git show HEAD:baley.toml
[project]
id = "af645d0f-a096-41c5-84c3-2da2d85ae4ca"
name = "one"

[git]
remote = "origin"
on_protected = "refuse"
first anchor, before the copy

== anchor ==
request 3be8b81b-74d8-4335-b3a8-5597230fd9b0
anchored sequence 20 (353ab0b06a7b81ffe5de3e411c2e584bf64e41f38f99ef8023b7e8c4f1ee0c03) as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20 on origin, confirmed at 2026-10-08T19:23:06.607744120Z
exit status: 0

== copy of baley.db taken with SQLite's backup API, mode 0600 ==
-rw------- 1 john john 368640 Oct  8 15:23 ~/.local/share/baley-live/results/baley-copy.db
project one's highest seq in the copy: 23
second anchor, after the copy

== anchor ==
request f1f20798-7070-40de-9550-d2176e5a4f37
anchored sequence 23 (0537dbd0152bdf8ec36a8b755da9ca187598df2277dd8f95e5e7625a1c8a42b2) as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/23 on origin, confirmed at 2026-10-08T19:23:06.805343329Z
exit status: 0
no anchor names a sequence above the copy's head yet, so third anchor

== anchor ==
request a715fba7-2525-4332-b97d-896c15d00455
anchored sequence 26 (799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62) as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/26 on origin, confirmed at 2026-10-08T19:23:06.992872916Z
exit status: 0

== the remote's anchor tags for project one, and project one's highest anchor.pushed sequence in the ledger ==
baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20
baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/23
baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/26
highest anchor.pushed sequence in the ledger: 28
highest sequence an anchor tag names: 26
copy's head: 23

== restore: baley.db replaced by the copy, its -wal and -shm files removed, mode 0600 ==
total 372
-rw------- 1 john john 368640 Oct  8 15:23 baley.db
-rw------- 1 john john      0 Oct  8 15:03 baley.db.maintenance
-rw------- 1 john john      0 Oct  8 15:03 baley.db.writer
-rw------- 1 john john     29 Oct  8 15:03 keys.env
-rw------- 1 john john    158 Oct  8 15:03 notebook.ipynb
-rw------- 1 john john      5 Oct  8 15:03 seed.txt
project one's highest seq in the restored ledger: 23

== verify ==
remote origin: anchor at 26 799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62
checked at 2026-10-08T19:23:07.096504458Z
chain head 23 0537dbd0152bdf8ec36a8b755da9ca187598df2277dd8f95e5e7625a1c8a42b2
truncated: remote anchor at 26, chain ends at 23
bodies checked 0, tombstones 0
local anchor 20 353ab0b06a7b81ffe5de3e411c2e584bf64e41f38f99ef8023b7e8c4f1ee0c03 as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20, confirmed at 2026-10-08T19:23:06.607744120Z
local anchor row: older than the remote anchor
exit status: 1

== acknowledge-restore ==
request 58f0081a-d6d6-433f-8883-7746e4981d8d
acknowledged: the restored chain is accepted behind remote anchor baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/26 (26 799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62); anchoring may resume
warning: purges recorded in history this copy lacks may be missing, whether they came after the copy was taken or on a branch it replaced, and bodies purged there may have reappeared
Every secret that was in such a body must be rotated, as after any purge: a secret that already reached a review provider, an export or any other system must still be rotated.
To repeat the purges you know of, run baley purge <project> <hash>... --reason <text> for each project, from records kept outside the store.
Review first any hash with nothing left to release, because one such hash refuses the whole request, and check purge's "kept ... because another reference still requires it" lines for bodies another reference still holds.
exit status: 0

== acknowledge-restore ==
request 610ea977-0bcd-4eb9-a7bb-9e35ad691b8e
baley: not acknowledged: already acknowledged
exit status: 1

== verify ==
remote origin: anchor at 26 799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62
checked at 2026-10-08T19:23:07.457372903Z
chain head 26 dd6790c0b9f3b39e1e087d99a06fe6f43e561d9bcf9d3cfa81969f4a104d63e1
acknowledged at 24: restore 23 0537dbd0152bdf8ec36a8b755da9ca187598df2277dd8f95e5e7625a1c8a42b2 behind remote anchor 26
unanchored sequences 25 to 26
warning: purges recorded in history this copy lacks may be missing, whether they came after the copy was taken or on a branch it replaced, and bodies purged there may have reappeared
Every secret that was in such a body must be rotated, as after any purge: a secret that already reached a review provider, an export or any other system must still be rotated.
To repeat the purges you know of, run baley purge <project> <hash>... --reason <text> for each project, from records kept outside the store.
Review first any hash with nothing left to release, because one such hash refuses the whole request, and check purge's "kept ... because another reference still requires it" lines for bodies another reference still holds.
bodies checked 0, tombstones 0
local anchor 20 353ab0b06a7b81ffe5de3e411c2e584bf64e41f38f99ef8023b7e8c4f1ee0c03 as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20, confirmed at 2026-10-08T19:23:06.607744120Z
local anchor row: older than the remote anchor
exit status: 0

== verify --local-only af645d0f-a096-41c5-84c3-2da2d85ae4ca ==
local only
checked at 2026-10-08T19:23:07.460645567Z
chain head 26 dd6790c0b9f3b39e1e087d99a06fe6f43e561d9bcf9d3cfa81969f4a104d63e1
not compared with a remote anchor
unanchored sequences 1 to 26
work unanchored since 2026-10-08T19:03:29.868970972Z
acknowledged restore at 24: 23 0537dbd0152bdf8ec36a8b755da9ca187598df2277dd8f95e5e7625a1c8a42b2 behind 26 799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62
warning: purges recorded in history this copy lacks may be missing, whether they came after the copy was taken or on a branch it replaced, and bodies purged there may have reappeared
Every secret that was in such a body must be rotated, as after any purge: a secret that already reached a review provider, an export or any other system must still be rotated.
To repeat the purges you know of, run baley purge <project> <hash>... --reason <text> for each project, from records kept outside the store.
Review first any hash with nothing left to release, because one such hash refuses the whole request, and check purge's "kept ... because another reference still requires it" lines for bodies another reference still holds.
bodies checked 0, tombstones 0
local anchor 20 353ab0b06a7b81ffe5de3e411c2e584bf64e41f38f99ef8023b7e8c4f1ee0c03 as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20, confirmed at 2026-10-08T19:23:06.607744120Z
local anchor row: not compared with a remote anchor
exit status: 0

== doctor ==
epoch 1
integrity: ok
database 368640 bytes, log 0 bytes
project 7173f106-d4b8-4512-9cd0-58e4364f28be (two): not checked against a remote from this directory
chain head 6 12eb0e66ece17b59cc7f06bc382507243fb54de87394489a62538972603f8c13
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-08T19:03:29.948081320Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T19:03:29.948081320Z
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
project af645d0f-a096-41c5-84c3-2da2d85ae4ca (one): anchor at 26 799d92cea973671f226feb7df96ff9de42b8e4428ab049728bd9f1533df05b62
chain head 26 dd6790c0b9f3b39e1e087d99a06fe6f43e561d9bcf9d3cfa81969f4a104d63e1
acknowledged at 24: restore 23 0537dbd0152bdf8ec36a8b755da9ca187598df2277dd8f95e5e7625a1c8a42b2 behind remote anchor 26
unanchored sequences 25 to 26
warning: purges recorded in history this copy lacks may be missing, whether they came after the copy was taken or on a branch it replaced, and bodies purged there may have reappeared
Every secret that was in such a body must be rotated, as after any purge: a secret that already reached a review provider, an export or any other system must still be rotated.
To repeat the purges you know of, run baley purge <project> <hash>... --reason <text> for each project, from records kept outside the store.
Review first any hash with nothing left to release, because one such hash refuses the whole request, and check purge's "kept ... because another reference still requires it" lines for bodies another reference still holds.
bodies checked 0, tombstones 0
local anchor 20 353ab0b06a7b81ffe5de3e411c2e584bf64e41f38f99ef8023b7e8c4f1ee0c03 as baley-anchor/af645d0f-a096-41c5-84c3-2da2d85ae4ca/20, confirmed at 2026-10-08T19:23:06.607744120Z
local anchor row: older than the remote anchor
unanchored age: none
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 26
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project user (per-user records): not checked against a remote from this directory
chain head 19 3afdf9ec9f0aa6d873cea370fcd56284fa432796ae8a608c2e00d20f52c21c3e
not compared with a remote anchor
unanchored sequences 1 to 19
work unanchored since 2026-10-08T19:11:14.983606568Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T19:11:14.983606568Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 19
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
exit status: 0
```

## Post-run reads

The output of `sh live-claude-reads.sh`, as the coding agent that drove the run saved it in results/reads.txt. The block below is that file unchanged except that the home folder is written `~`.

```text

== pins ==
date: 2026-10-08T19:03:29Z
platform: Linux 7.2.9-1-cachyos
bwrap: /usr/bin/bwrap
socat: /usr/bin/socat
sqlite3: /usr/bin/sqlite3
jq: /usr/bin/jq
pwsh: absent
git: /usr/bin/git
telemetry-switch: unset
checkout-commit: 7c8ca4c39249d8135ba56dab62941d7bc7483964
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
claude-command: ~/.local/bin/claude
claude-command-resolved: ~/.local/share/claude/versions/2.1.294
claude-launcher: link to ~/.local/share/claude/versions/2.1.294
claude-launcher-resolved: ~/.local/share/claude/versions/2.1.294
begin real-claude-versions ~/.local/share/claude/versions
~/.local/share/claude/versions/2.1.293 f 252755128 2026-10-07 14:11:00.2344368200
~/.local/share/claude/versions/2.1.294 f 252755128 2026-10-08 07:41:15.2608733970
~/.local/share/claude/versions d 28 2026-10-08 13:33:08.2187588350
end real-claude-versions
project-one-id: af645d0f-a096-41c5-84c3-2da2d85ae4ca
project-two-id: 7173f106-d4b8-4512-9cd0-58e4364f28be
project-fork-id: af645d0f-a096-41c5-84c3-2da2d85ae4ca

== verify --local-only af645d0f-a096-41c5-84c3-2da2d85ae4ca ==
local only
checked at 2026-10-08T19:22:27.617420409Z
chain head 18 753b5ac75010a3b0f6af1ac77aa648460bf30c6c453d46cb1125b64caadd2ffd
not compared with a remote anchor
unanchored sequences 1 to 18
work unanchored since 2026-10-08T19:03:29.868970972Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views af645d0f-a096-41c5-84c3-2da2d85ae4ca ==
views checked at sequence 18
no differences
exit status: 0

== verify --local-only 7173f106-d4b8-4512-9cd0-58e4364f28be ==
local only
checked at 2026-10-08T19:22:27.643138640Z
chain head 6 12eb0e66ece17b59cc7f06bc382507243fb54de87394489a62538972603f8c13
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-08T19:03:29.948081320Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views 7173f106-d4b8-4512-9cd0-58e4364f28be ==
views checked at sequence 6
no differences
exit status: 0

== verify --local-only user ==
local only
checked at 2026-10-08T19:22:27.664847156Z
chain head 19 3afdf9ec9f0aa6d873cea370fcd56284fa432796ae8a608c2e00d20f52c21c3e
not compared with a remote anchor
unanchored sequences 1 to 19
work unanchored since 2026-10-08T19:11:14.983606568Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
exit status: 0

== verify --views user ==
views checked at sequence 19
no differences
exit status: 0

== doctor ==
epoch 1
integrity: ok
database 368640 bytes, log 0 bytes
project 7173f106-d4b8-4512-9cd0-58e4364f28be (two): not checked against a remote from this directory
chain head 6 12eb0e66ece17b59cc7f06bc382507243fb54de87394489a62538972603f8c13
not compared with a remote anchor
unanchored sequences 1 to 6
work unanchored since 2026-10-08T19:03:29.948081320Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T19:03:29.948081320Z
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
project af645d0f-a096-41c5-84c3-2da2d85ae4ca (one): not checked against a remote from this directory
chain head 18 753b5ac75010a3b0f6af1ac77aa648460bf30c6c453d46cb1125b64caadd2ffd
not compared with a remote anchor
unanchored sequences 1 to 18
work unanchored since 2026-10-08T19:03:29.868970972Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T19:03:29.868970972Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 18
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
project user (per-user records): not checked against a remote from this directory
chain head 19 3afdf9ec9f0aa6d873cea370fcd56284fa432796ae8a608c2e00d20f52c21c3e
not compared with a remote anchor
unanchored sequences 1 to 19
work unanchored since 2026-10-08T19:11:14.983606568Z
bodies checked 0, tombstones 0
local anchor row: not compared with a remote anchor
unanchored age: since 2026-10-08T19:11:14.983606568Z
view set 7, binary 7
view capture: live 1, binary 1
view checkout: live 1, binary 1
view claim_scope: live 1, binary 1
view guard: live 1, binary 1
view guard_policy: live 1, binary 1
view model_catalog: live 1, binary 1
view policy: live 1, binary 1
view request: live 2, binary 2
views checked at sequence 19
no differences
claims: 0 active, 0 interrupted, 0 awaiting owner
exit status: 0

== streams: events per project and stream, lowest and highest stream_version ==
             project_id                       stream          events  lowest  highest
------------------------------------  ----------------------  ------  ------  -------
7173f106-d4b8-4512-9cd0-58e4364f28be  command/checkout.admit       1       1        1
7173f106-d4b8-4512-9cd0-58e4364f28be  command/policy.record        1       1        1
7173f106-d4b8-4512-9cd0-58e4364f28be  command/project.init         1       1        1
7173f106-d4b8-4512-9cd0-58e4364f28be  project                      3       1        3
af645d0f-a096-41c5-84c3-2da2d85ae4ca  capture                      5       1        5
af645d0f-a096-41c5-84c3-2da2d85ae4ca  command/capture.record       5       1        5
af645d0f-a096-41c5-84c3-2da2d85ae4ca  command/checkout.admit       1       1        1
af645d0f-a096-41c5-84c3-2da2d85ae4ca  command/policy.record        2       1        2
af645d0f-a096-41c5-84c3-2da2d85ae4ca  command/project.init         1       1        1
af645d0f-a096-41c5-84c3-2da2d85ae4ca  project                      4       1        4
user                                  command/guard.record         9       1        9
user                                  guard                       10       1       10

== streams that do not start at 1 or whose count differs from the span ==
(no rows above means every stream is complete)

== server callers: each session with the project ids of the events it wrote ==
           baley_session                               project_directory                                    working_directory                               host_session              client_version               project_id               first_seq  last_seq  events
------------------------------------  ---------------------------------------------------  ---------------------------------------------------  ------------------------------------  --------------  ------------------------------------  ---------  --------  ------
5164dca1-d5c3-4352-a82a-392b723cc27b  ~/.local/share/baley-live/projects/one      ~/.local/share/baley-live/projects/one      4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca          7        16       6
4bd8ebec-fc24-437b-87e8-fcf18acf2763  ~/.local/share/baley-live/projects/one/sub  ~/.local/share/baley-live/projects/one/sub  775c3b69-ad49-49c1-954a-e68b75546c17  2.1.294         af645d0f-a096-41c5-84c3-2da2d85ae4ca         11        18       6
project ids in pins.txt: one=af645d0f-a096-41c5-84c3-2da2d85ae4ca two=7173f106-d4b8-4512-9cd0-58e4364f28be fork=af645d0f-a096-41c5-84c3-2da2d85ae4ca

== hook callers in seq order ==
project_id  seq              host_session                             working_directory                                project_directory                            call_id
----------  ---  ------------------------------------  -----------------------------------------------  -----------------------------------------------  ------------------------------
user          1  4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01SPVurHqetRvWPLZidDdt69
user          2  4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01SPVurHqetRvWPLZidDdt69
user          3  4c01fab1-79a7-490c-8ce6-4bee52f7f0c1  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01SPVurHqetRvWPLZidDdt69
user          4  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_016PshNdnDrxdW7Mt1D25nUA
user          5  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_016PshNdnDrxdW7Mt1D25nUA
user          6  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01NVfGqehy5tsBX6GANPTX4C
user          7  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01NVfGqehy5tsBX6GANPTX4C
user          8  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01QC99GnbWTK97gFDHXfDRdC
user          9  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01QC99GnbWTK97gFDHXfDRdC
user         10  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_019JHmV6xPW2eGaPodSpCtQ2
user         11  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_019JHmV6xPW2eGaPodSpCtQ2
user         12  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Co7ykDj6k98DNoXve2RWtQ
user         13  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01Co7ykDj6k98DNoXve2RWtQ
user         14  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01XMQy9LLG5WxuGH1mbr9F2K
user         15  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01XMQy9LLG5WxuGH1mbr9F2K
user         16  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_012CoghwRCTRRukVxbubfoJn
user         17  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_012CoghwRCTRRukVxbubfoJn
user         18  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01DrFQrUjapbuivS6SLGoqgb
user         19  2d4b5641-b8d7-4ee7-8658-6e3efa8d3c93  ~/.local/share/baley-live/projects/one  ~/.local/share/baley-live/projects/one  toolu_01DrFQrUjapbuivS6SLGoqgb

== captures: capture.recorded events per request_id with the caller's session and instruction evidence ==
             project_id               seq               request_id               events_for_request             baley_session              instructions  kind  bytes            text
------------------------------------  ---  ------------------------------------  ------------------  ------------------------------------  ------------  ----  -----  -------------------------
af645d0f-a096-41c5-84c3-2da2d85ae4ca    9  12120000-0000-4000-8000-000000000001                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note     25  smoke note from session A
af645d0f-a096-41c5-84c3-2da2d85ae4ca   11  12120000-0000-4000-8000-000000000033                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     12  tools B main
af645d0f-a096-41c5-84c3-2da2d85ae4ca   13  12120000-0000-4000-8000-000000000004                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note      9  overlap B
af645d0f-a096-41c5-84c3-2da2d85ae4ca   15  12120000-0000-4000-8000-000000000002                   1  5164dca1-d5c3-4352-a82a-392b723cc27b                note      9  overlap A
af645d0f-a096-41c5-84c3-2da2d85ae4ca   17  12120000-0000-4000-8000-000000000034                   1  4bd8ebec-fc24-437b-87e8-fcf18acf2763                note     16  tools B subagent

== stored capture bodies (text above 4,096 bytes): byte count and SHA-256 after zstd -d ==
large.txt on disk: 33000 bytes, sha256 6754341a445caf2f681748567772759cdf41bf0865425e360da5fcfd8dc1c0d4

== guard answers by decision, with call ids ==
decision  seq  tool   verb   branch             call_id
--------  ---  ----  ------  ------  ------------------------------
deny        2  Bash  commit  main    toolu_01SPVurHqetRvWPLZidDdt69
deny        4  Grep                  toolu_016PshNdnDrxdW7Mt1D25nUA
deny        6  Grep                  toolu_01NVfGqehy5tsBX6GANPTX4C
deny        8  Glob                  toolu_01QC99GnbWTK97gFDHXfDRdC
deny       10  Glob                  toolu_019JHmV6xPW2eGaPodSpCtQ2
deny       12  Grep                  toolu_01Co7ykDj6k98DNoXve2RWtQ
deny       14  Grep                  toolu_01XMQy9LLG5WxuGH1mbr9F2K
deny       16  Glob                  toolu_012CoghwRCTRRukVxbubfoJn
deny       18  Glob                  toolu_01DrFQrUjapbuivS6SLGoqgb

== server stderr: start line, exit checkpoint lines and any abandoned-drain line, per file ==
-- ~/.local/share/baley-live/results/server-stderr/1138602.log
start 2026-10-08T19:08:36Z pid 1138602
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1143098.log
start 2026-10-08T19:11:42Z pid 1143098
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1155330.log
start 2026-10-08T19:16:12Z pid 1155330
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file
-- ~/.local/share/baley-live/results/server-stderr/1223804.log
start 2026-10-08T19:19:35Z pid 1223804
exit checkpoint lines: 1
baley: exit checkpoint complete, every logged change is in the database file

== hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard) ==
Bash calls=1 highest_ms=49 median_ms=49 highest_wrapper_ms=58 highest_overhead_ms=8
Edit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
Glob calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=14 highest_overhead_ms=9
Grep calls=4 highest_ms=5 median_ms=5 highest_wrapper_ms=16 highest_overhead_ms=10
NotebookEdit calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=12 highest_overhead_ms=10
Read calls=4 highest_ms=2 median_ms=1 highest_wrapper_ms=13 highest_overhead_ms=10
Write calls=1 highest_ms=1 median_ms=1 highest_wrapper_ms=10 highest_overhead_ms=8

== hook timing: guard decisions by tool ==
Bash deny 1
Edit none 1
Glob deny 4
Grep deny 4
NotebookEdit none 1
Read none 4
Write none 1

== hook timing: finished calls at or above 10,000 ms (the guard alone, or the whole wrapper) ==
(no lines above means none)

== hook timing: tool_use_id seen more than once ==
(no lines above means none)

== hook timing: calls that started and did not finish (killed at the timeout, or still running) ==
(no lines above means every started call finished)

== real-home against pins.txt ==
4c4
< ~/.local/share/crenshawdev/baley d 86 2026-10-05 09:42:21.6045176040
---
> ~/.local/share/crenshawdev/baley d 86 2026-10-08 15:06:08.4784947390
~/.local/share/crenshawdev/baley: DIFFERENT (left: pins.txt, right: now)

== real-config against pins.txt ==
~/.config/crenshawdev/baley: no difference

== real-claude-versions against pins.txt ==
~/.local/share/claude/versions: no difference

== claude command against pins.txt ==
no difference

== ~/.local/bin/claude link against pins.txt ==
no difference

== where the claude command and the launcher resolve now ==
the claude command resolves outside the disposable root (~/.local/share/claude/versions/2.1.294)
the launcher resolves outside the disposable root (~/.local/share/claude/versions/2.1.294)

== entries other programs wrote under the exported variables (top level of the disposable data and config roots, other than crenshawdev; evidence only) ==
-- ~/.local/share/baley-live/data
applications d
GitKrakenCLI d
gk d
terminus d
-- ~/.local/share/baley-live/config
anthropic d
mimeapps.list f
```
