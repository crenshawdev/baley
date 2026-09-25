# Host matrix

The probes behind acceptance gate 2 of [design 0001](../../docs/design/0001-evidence-ledger.md): does each host keep agents out of Baley's home while Baley's own hook and server still write it, and does each host hand the hook and the server what the design assumes? It is a spike. Nothing here ships. Run it by hand before acceptance and again before each release, as ADR 0008 says.

The probes use a stand-in home, `~/.local/share/baley-matrix`, so they never touch a real ledger. Each host runs one throwaway non-interactive session from a subdirectory of this repository with a hook and a stand-in MCP server that record where they ran, and an agent that is asked to read and write the stand-in home.

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

Each script prints the agent's own report, the hook's stdin, the server's working directory and which files landed in the stand-in home. `answer-test.sh` checks which hook answers Codex honours.

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
