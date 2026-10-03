#!/bin/sh
# Host matrix probe, Claude Code side. It prepares disposable stand-ins for Baley's home and
# config folders, renders the settings that should keep agents out of both, and prints an
# interactive procedure for the owner. It never starts Claude Code: the owner runs the session.
set -u

# The paths go into JSON and shell commands unquoted, so only plain path characters are allowed.
case "${HOME:-}" in
  ""|/) echo "HOME must be a real directory" >&2; exit 1 ;;
  /*) ;;
  *) echo "HOME must be absolute" >&2; exit 1 ;;
esac
case "$HOME" in
  *[!A-Za-z0-9._/-]*|*crenshawdev*) echo "HOME holds a character or name this probe refuses" >&2; exit 1 ;;
esac

# One tree per stand-in, under a claude folder so the Codex probes' files beside it stay alone.
DATA_TREE="$HOME/.local/share/baley-matrix/claude"
CONF_TREE="$HOME/.config/baley-matrix/claude"
STAND_HOME="$DATA_TREE/home"
STAND_CONF="$CONF_TREE/config"
WORK="$DATA_TREE/work/session"
OUT="$DATA_TREE/results"
SETTINGS_FILE="$OUT/settings.json"
MCP_FILE="$OUT/mcp.json"
SHEET="$OUT/observations.md"

# The only deletion in this script: a tree has to be exactly one of the two named above, with no
# symbolic link from HOME down to it, so the delete cannot land somewhere else, and owned by this user.
fresh() {
  case "$1" in
    "$HOME/.local/share/baley-matrix/claude"|"$HOME/.config/baley-matrix/claude") ;;
    *) echo "refusing to clear $1" >&2; exit 1 ;;
  esac
  P="$HOME"
  for PART in $(printf '%s\n' "${1#"$HOME"/}" | tr '/' ' '); do
    P="$P/$PART"
    [ -L "$P" ] && { echo "refusing to clear $1: $P is a symbolic link" >&2; exit 1; }
  done
  if [ -e "$1" ] && [ -z "$(find "$1" -prune -user "$(id -u)")" ]; then
    echo "refusing to clear $1: it is not owned by $(id -un)" >&2; exit 1
  fi
  rm -rf "$1" && mkdir -p "$1"
}
fresh "$DATA_TREE"
fresh "$CONF_TREE"
mkdir -p "$STAND_HOME" "$STAND_CONF" "$WORK" "$OUT"

# Harmless fixtures. The key value is fake on purpose.
for D in "$STAND_HOME" "$STAND_CONF"; do
  echo seed > "$D/seed.txt"
  echo "FAKE_KEY=not-a-real-key-0000" > "$D/keys.env"
  printf '%s\n' '{"cells":[{"cell_type":"code","execution_count":null,"id":"c1","metadata":{},"outputs":[],"source":["x = 1"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}' > "$D/notebook.ipynb"
done
# A script for the steps that check a command's child process.
cat > "$WORK/child.sh" <<'EOF'
#!/bin/sh
case "$1" in
  read) cat "$2" ;;
  write) echo child > "$2" ;;
esac
EOF

# allowWrite widens the default write boundary over both trees, so the session and results folders
# stay writable and a refused write to a stand-in comes from its denyWrite, not the default boundary.
# Hooks and local MCP servers run outside the sandbox. Both stand-ins record what they were given.
# The hook neither allows nor denies, so the deny rules and the sandbox decide.
cat > "$SETTINGS_FILE" <<EOF
{
  "env": {"CLAUDE_CODE_USE_POWERSHELL_TOOL": "1"},
  "sandbox": {
    "enabled": true,
    "failIfUnavailable": true,
    "allowUnsandboxedCommands": false,
    "autoAllowBashIfSandboxed": true,
    "filesystem": {
      "allowWrite": ["$DATA_TREE", "$CONF_TREE"],
      "denyRead": ["$STAND_HOME", "$STAND_CONF"],
      "denyWrite": ["$STAND_HOME", "$STAND_CONF"]
    }
  },
  "permissions": {
    "deny": [
      "Read(/$STAND_HOME/**)",
      "Edit(/$STAND_HOME/**)",
      "Read(/$STAND_CONF/**)",
      "Edit(/$STAND_CONF/**)"
    ]
  },
  "hooks": {
    "PreToolUse": [{"matcher": "Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit", "hooks": [{"type": "command", "timeout": 5,
      "command": "tee -a $OUT/hook-stdin.jsonl >/dev/null; echo hook-wrote >> $STAND_HOME/hook.txt"}]}]
  }
}
EOF
cat > "$MCP_FILE" <<EOF
{"mcpServers":{"probe":{"command":"sh","args":["-c","pwd >> $OUT/server-cwd.txt; echo server-wrote >> $STAND_HOME/server.txt; exec cat"]}}}
EOF

# One numbered step with blank fields for the owner. Reading and writing are separate steps.
N=0
step() {
  N=$((N + 1))
  printf '%s. %s\n   Prompt appeared (yes/no):\n   Result:\n\n' "$N" "$1"
}

# The steps that repeat for each stand-in. $1 names it, $2 is its folder, $3 a folder that contains it.
steps_for() {
  L="$1"; D="$2"; P="$3"
  step "[$L] Bash read: cat $D/seed.txt"
  step "[$L] Bash write: echo bash > $D/agent-bash.txt"
  step "[$L] Bash child, read: sh -c 'cat $D/seed.txt'"
  step "[$L] Bash child, write: sh -c 'echo child > $D/agent-sh.txt'"
  step "[$L] Bash script, read: sh $WORK/child.sh read $D/seed.txt"
  step "[$L] Bash script, write: sh $WORK/child.sh write $D/agent-script.txt"
  step "[$L] Monitor command, read: cat $D/seed.txt"
  step "[$L] Monitor command, write: echo monitor > $D/agent-monitor.txt"
  step "[$L] Monitor command child, read: sh $WORK/child.sh read $D/seed.txt"
  step "[$L] Monitor command child, write: sh $WORK/child.sh write $D/agent-monitor-child.txt"
  step "[$L] PowerShell read: cat $D/seed.txt (record 'unavailable' without pwsh)"
  step "[$L] PowerShell write: 'ps' | Out-File $D/agent-ps.txt (record 'unavailable' without pwsh)"
  step "[$L] PowerShell child, read: sh $WORK/child.sh read $D/seed.txt (record 'unavailable' without pwsh)"
  step "[$L] PowerShell child, write: sh $WORK/child.sh write $D/agent-ps-child.txt (record 'unavailable' without pwsh)"
  step "[$L] Read tool: $D/seed.txt"
  step "[$L] Grep over the folder itself: pattern FAKE_KEY, path $D"
  step "[$L] Grep over a folder that contains it: pattern FAKE_KEY, path $P"
  step "[$L] Glob over the folder itself: pattern * , path $D"
  step "[$L] Glob over a folder that contains it: pattern **/seed.txt, path $P"
  step "[$L] Write tool: create $D/agent-write.txt"
  step "[$L] Edit tool: change 'seed' to 'edited' in $D/seed.txt"
  step "[$L] NotebookEdit tool: change cell c1 of $D/notebook.ipynb to 'x = 2'"
}

{
  cat <<EOF
# Claude Code host matrix observations

Claude Code version (claude --version):
Platform:
Date:

Fill the Result of every step with what the tool returned, in the owner's words and with exact
error text. Say in Prompt appeared whether Claude Code asked for permission. Approve every
prompt, so that each result shows the deny rules and the sandbox and not a refusal.

The sandbox allows writes under both trees, so a refused write to a stand-in comes from its denyWrite
entry and not from Claude Code's default boundary.

EOF
  steps_for home "$STAND_HOME" "$DATA_TREE"
  steps_for config "$STAND_CONF" "$CONF_TREE"
  step "Monitor with a WebSocket watch (no command): watch any ws:// address, a refused connection is fine"
  cat <<EOF
## After the session

Hook log (tool_name and tool_input for each call):
  jq -c '{tool_name, tool_input}' $OUT/hook-stdin.jsonl
Entry seen for each of the nine tools (Bash, Monitor, PowerShell, Read, Grep, Glob, Write, Edit, NotebookEdit), with the tool_input fields it carried:
  Bash:
  Monitor:
  PowerShell:
  Read:
  Grep:
  Glob:
  Write:
  Edit:
  NotebookEdit:

Hook and server markers, and the server's working directory:
  cat $STAND_HOME/hook.txt $STAND_HOME/server.txt $OUT/server-cwd.txt
Result:

What landed in each stand-in:
  ls -l $STAND_HOME $STAND_CONF
  cat $STAND_HOME/seed.txt $STAND_CONF/seed.txt
Result:

## What the documentation leaves unclear

Hook for Monitor, command watch: fired (yes/no), fields that arrived:
Hook for Monitor, WebSocket watch: fired (yes/no), fields that arrived:
Hook for NotebookEdit: fired (yes/no), fields that arrived:
Read deny rules stop Grep and Glob over the folder itself, home and config:
Read deny rules stop Grep and Glob over a folder that contains it, home and config:
A rule written as <folder>/** covers a Grep or Glob whose path is the folder itself:
Children of sandboxed Monitor commands are denied, read and write:
Children of sandboxed PowerShell commands are denied, read and write:
Owner's own user settings, as shown by /sandbox (Config tab) and /permissions:
EOF
} > "$SHEET"

cat <<EOF
Claude Code host matrix probe. Nothing below has been run for you.

Stand-ins (disposable, never the real Baley folders):
  home   $STAND_HOME
  config $STAND_CONF
Session folder: $WORK
Results folder: $OUT

Before you start
  Linux needs bubblewrap and socat for the sandbox.
  Record the output of claude --version on the sheet. This script never runs it.
EOF
for TOOL in bwrap socat; do
  command -v "$TOOL" >/dev/null 2>&1 || echo "  WARNING: $TOOL is not on PATH, so the sandbox cannot start on Linux."
done
if command -v pwsh >/dev/null 2>&1; then
  echo "  pwsh found: the PowerShell steps can run."
else
  echo "  pwsh not found: record the PowerShell steps as unavailable (they need PowerShell 7)."
fi
[ -z "${DISABLE_TELEMETRY:-}" ] && [ -z "${CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC:-}" ] \
  || echo "  WARNING: DISABLE_TELEMETRY or CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC is set, so Monitor is unavailable."

cat <<EOF

Start the interactive session from the session folder, with no permission-mode or allowed-tools flag
  \$ cd $WORK
  \$ claude --settings $SETTINGS_FILE --mcp-config $MCP_FILE --strict-mcp-config
Approve any permission prompt and note on the sheet that it appeared.

The sheet is $SHEET. Its steps follow.

EOF
cat "$SHEET"
