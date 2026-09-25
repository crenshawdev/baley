#!/bin/sh
# Host matrix probe, Codex side. Stand-in home: ~/.local/share/baley-matrix, outside the workspace.
set -u
HOME_DIR="$HOME/.local/share/baley-matrix"
OUT="$HOME_DIR/codex"
rm -rf "$OUT"; mkdir -p "$OUT"
echo seed > "$HOME_DIR/seed.txt"
rm -f "$HOME_DIR/agent.txt"
cd "$(dirname "$0")/../../crates" || exit 1

HOOK="sh -c 'tee -a $OUT/hook-stdin.jsonl >/dev/null; echo hook-wrote >> $OUT/hook.txt'"
PROMPT='Run these two shell commands one at a time, exactly as written, and then report for each one its exact stdout, stderr and exit code. Do nothing else.
1. cat ~/.local/share/baley-matrix/seed.txt
2. sh -c "echo agent > ~/.local/share/baley-matrix/agent.txt"'

codex exec --skip-git-repo-check --ephemeral -s workspace-write \
  --dangerously-bypass-hook-trust \
  -c "hooks.PreToolUse=[{matcher=\"Bash\",hooks=[{type=\"command\",command=\"$HOOK\",timeout=5}]}]" \
  -c "mcp_servers.probe.command=\"sh\"" \
  -c "mcp_servers.probe.args=[\"-c\",\"pwd >> $OUT/server-cwd.txt; echo server-wrote >> $OUT/server.txt; exec cat\"]" \
  -c "mcp_servers.probe.startup_timeout_sec=5" \
  -o "$OUT/agent-report.md" \
  "$PROMPT" > "$OUT/exec-log.txt" 2>&1 </dev/null
echo "exit=$?" >> "$OUT/exec-log.txt"
echo "== agent report"; cat "$OUT/agent-report.md"; echo
echo "== landed in the home"; ls "$HOME_DIR"
echo "== hook stdin (first event)"; head -c 1200 "$OUT/hook-stdin.jsonl"; echo
echo "== hook wrote"; cat "$OUT/hook.txt" 2>&1
echo "== server wrote, from"; cat "$OUT/server.txt" "$OUT/server-cwd.txt" 2>&1
