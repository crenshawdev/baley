# Host matrix

The matrix measures whether Claude Code keeps agents from reading or writing Baley's home and its config folder, through shell commands and their children and through its built-in file tools. It also measures whether Baley's own hook and server still write the home, and whether the host hands the hook and the server what the design assumes. These are the probes behind acceptance gate 2 of [design 0001](../../docs/design/0001-evidence-ledger.md). It is a spike. Nothing here ships. Claude Code's probe is run before each release under ADR 0033 (`../../docs/adr/0033-host-security-bar.md`). The Codex probes stay as the instruments that measure Codex against the bar, should it return.

The Claude Code probe uses two disposable stand-in folders, one for home (`~/.local/share/baley-matrix/claude/home`) and one for config (`~/.config/baley-matrix/claude/config`), so it never touches a real ledger or real settings. Its session is interactive and run by the owner. A hook and a stand-in MCP server record where they ran and write a marker into the home stand-in, and the owner asks the agent to read and write both stand-ins with each tool. The Codex probes keep their single stand-in, `~/.local/share/baley-matrix`, and each runs one throwaway non-interactive session from a subdirectory of this repository.

## Run it

Codex, all three probes:

```
sh probe-codex.sh
sh answer-test.sh
```

Claude Code (its sandbox needs `bubblewrap` and `socat` on Linux):

```
sh probe-claude.sh
```

The script clears and recreates its own folders under `baley-matrix/claude`, seeds both stand-ins with harmless fixtures and renders the settings: the sandbox on and required, write access over the two trees that hold the stand-ins (so a refused write comes from the stand-in's deny entry and not the default boundary), sandbox and permission deny rules over both stand-ins, and a hook matching Bash, Monitor, PowerShell, Read, Grep, Glob, Write, Edit and NotebookEdit. It then prints the procedure and writes an observation sheet, `observations.md`, into the results folder. It never starts `claude`.

The owner starts the printed interactive session from the printed folder, runs each step, approves any permission prompt so the result shows the deny rules and the sandbox, and fills in the sheet. The sheet also has the version, platform and date, and a field for each behaviour the Claude Code documentation leaves unclear. The PowerShell steps need `pwsh` 7 and are recorded as unavailable without it. No step starts a non-interactive session.

The Codex scripts print the agent's own report, the hook's stdin, the server's working directory and which files landed in the stand-in home. `answer-test.sh` checks which hook answers Codex honours.

## What was found on 2026-09-25

Codex CLI 0.156.1, Claude Code 2.1.282, Linux.

| Concern | Claude Code | Codex |
|---|---|---|
| MCP server working directory | The session's directory, a subdirectory | The session's directory, a subdirectory |
| Hook and server can write the home | Yes | Yes |
| Agent reads the home | Denied; the file appears missing | Allowed: every Codex sandbox policy grants full disk read |
| Agent writes the home | Dropped: the command exits 0 and nothing lands | Refused: "Read-only file system" |
| Hook stdin | `session_id`, `cwd`, `hook_event_name`, `tool_name` `Bash`, `tool_input.command`, `tool_use_id` | The same fields |
| Hook answers honoured | `permissionDecision` `allow`, `deny`, `ask` | `deny` (camelCase, with a reason) and exit code 2; `ask` is rejected as unsupported and the command runs |
