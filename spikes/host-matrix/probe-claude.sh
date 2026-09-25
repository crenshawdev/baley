#!/bin/sh
# Host matrix probe, Claude Code side. Stand-in home: ~/.local/share/baley-matrix, denied to agents by the sandbox.
set -u
HOME_DIR="$HOME/.local/share/baley-matrix"
OUT="$HOME_DIR/claude"
rm -rf "$OUT"; mkdir -p "$OUT"
echo seed > "$HOME_DIR/seed.txt"
rm -f "$HOME_DIR/agent.txt"
cd "$(dirname "$0")/../../crates" || exit 1

SETTINGS=$(cat <<EOF
{
  "sandbox": {
    "enabled": true,
    "autoAllowBashIfSandboxed": true,
    "filesystem": {
      "denyRead": ["$HOME_DIR"],
      "denyWrite": ["$HOME_DIR"]
    }
  },
  "hooks": {
    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "timeout": 5,
      "command": "tee -a $OUT/hook-stdin.jsonl >/dev/null; echo hook-wrote >> $OUT/hook.txt"}]}]
  }
}
EOF
)
MCP=$(cat <<EOF
{"mcpServers":{"probe":{"command":"sh","args":["-c","pwd >> $OUT/server-cwd.txt; echo server-wrote >> $OUT/server.txt; exec cat"]}}}
EOF
)
PROMPT='Run these two shell commands one at a time with the Bash tool, exactly as written, and then report for each one its exact stdout, stderr and exit code. Do nothing else.
1. cat ~/.local/share/baley-matrix/seed.txt
2. sh -c "echo agent > ~/.local/share/baley-matrix/agent.txt"'

claude -p --settings "$SETTINGS" --mcp-config "$MCP" --strict-mcp-config \
  --permission-mode bypassPermissions --allowedTools Bash \
  > "$OUT/agent-report.md" 2> "$OUT/stderr.txt" <<PEOF
$PROMPT
PEOF
echo "exit=$?" >> "$OUT/stderr.txt"
echo "== agent report"; cat "$OUT/agent-report.md"; echo
echo "== stderr (a sandbox warning here means the run proves nothing)"; cat "$OUT/stderr.txt"
echo "== landed in the home"; ls "$HOME_DIR"
echo "== hook stdin (first event)"; head -c 1200 "$OUT/hook-stdin.jsonl" 2>&1; echo
echo "== hook wrote"; cat "$OUT/hook.txt" 2>&1
echo "== server wrote, from"; cat "$OUT/server.txt" "$OUT/server-cwd.txt" 2>&1
