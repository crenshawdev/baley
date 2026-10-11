#!/bin/sh
# Installed qualification, preparation side. It builds a disposable tree under ~/.local/share/baley-install-live:
# a disposable HOME, an isolated Claude Code folder holding seeded owner settings, two git projects and a
# results folder. It prints the environment prefix and the launch of every session in live-install.md. It
# starts neither claude nor baley: the coordinator drives the sessions through tmux and the owner types the login.
set -u

if [ $# -ne 0 ]; then
  echo "usage: sh live-install.sh (no arguments)" >&2
  exit 1
fi

# BALEY_HOME collapses Baley's home and config into one folder and moves them away from the disposable HOME.
if [ -n "${BALEY_HOME+set}" ]; then
  echo "BALEY_HOME is set. Unset it: it makes home and config one folder and moves them off the disposable HOME." >&2
  exit 1
fi

# An inherited GIT_ variable can pick another repository, work tree, index or object store, or point the
# config writes at the owner's files, so git would act outside the disposable repositories. None of them
# reaches this script's git commands. Only GIT_EXEC_PATH stays, because it says where git lives.
for NAME in $(env | sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p'); do
  [ "$NAME" = GIT_EXEC_PATH ] || unset "$NAME"
done

# The paths go into JSON and shell commands unquoted, so only plain path characters are allowed.
case "${HOME:-}" in
  ""|/) echo "HOME must be a real directory" >&2; exit 1 ;;
  /*) ;;
  *) echo "HOME must be absolute" >&2; exit 1 ;;
esac
case "$HOME" in
  *[!A-Za-z0-9._/-]*|*crenshawdev*) echo "HOME holds a character or name this script refuses" >&2; exit 1 ;;
esac

for TOOL in jq git; do
  command -v "$TOOL" >/dev/null 2>&1 || { echo "$TOOL is required and is not on PATH" >&2; exit 1; }
done
if command -v sha256sum >/dev/null 2>&1; then
  sum256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sum256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  sum256() { echo "no sha256sum or shasum"; }
fi

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || echo "$SCRIPT_DIR")
GUIDE="$SCRIPT_DIR/live-install.md"

OWNER_HOME=$HOME
ROOT="$OWNER_HOME/.local/share/baley-install-live"
DH="$ROOT/home"
CC="$ROOT/claude-config"
PROJ="$ROOT/projects"
OUT="$ROOT/results"
ARTIFACTS="$OWNER_HOME/.local/share/baley-install-artifacts/artifacts.txt"

# Everything Baley derives from the disposable HOME: its home and config folders, the stable path and the
# versions folder.
BHOME="$DH/.local/share/crenshawdev/baley"
BCONF="$DH/.config/crenshawdev/baley"
STABLE="$DH/.local/bin/baley"
VERSIONS="$DH/.local/lib/crenshawdev/baley/versions"

# Clearing the root deletes everything under it, so a claude command that lives there would be deleted
# with it. Both ways the owner's claude command is found are resolved first, and the script stops before it
# clears or creates anything when either one lies under the root. It never relinks the launcher.
under_root() {
  case "$1" in
    "$ROOT"|"$ROOT"/*|"$ROOT_RESOLVED"|"$ROOT_RESOLVED"/*) return 0 ;;
  esac
  return 1
}
ROOT_RESOLVED="$(readlink -f "$OWNER_HOME" 2>/dev/null || echo "$OWNER_HOME")/.local/share/baley-install-live"
CLAUDE_FOUND=$(command -v claude 2>/dev/null || true)
CLAUDE_RESOLVED=""
case "$CLAUDE_FOUND" in /*) CLAUDE_RESOLVED=$(readlink -f "$CLAUDE_FOUND" 2>/dev/null || true) ;; esac
LAUNCHER="$OWNER_HOME/.local/bin/claude"
LAUNCHER_RESOLVED=""
if [ -e "$LAUNCHER" ] || [ -L "$LAUNCHER" ]; then
  LAUNCHER_RESOLVED=$(readlink -f "$LAUNCHER" 2>/dev/null || true)
fi
if under_root "$CLAUDE_RESOLVED" || under_root "$LAUNCHER_RESOLVED"; then
  echo "your claude command resolves inside the disposable root, so clearing it would delete the binary your claude runs:" >&2
  echo "  command -v claude: ${CLAUDE_FOUND:-none}, resolved to ${CLAUDE_RESOLVED:-nothing}" >&2
  echo "  $LAUNCHER resolved to ${LAUNCHER_RESOLVED:-nothing}" >&2
  echo "  root: $ROOT" >&2
  echo "Point your claude command at a file outside the root first. This script repairs nothing and relinks nothing." >&2
  exit 1
fi

# The only deletion in this script: the tree has to be exactly the root named above, with no
# symbolic link from HOME down to it, so the delete cannot land somewhere else, and owned by this user.
fresh() {
  case "$1" in
    "$OWNER_HOME/.local/share/baley-install-live") ;;
    *) echo "refusing to clear $1" >&2; exit 1 ;;
  esac
  P="$OWNER_HOME"
  for PART in $(printf '%s\n' "${1#"$OWNER_HOME"/}" | tr '/' ' '); do
    P="$P/$PART"
    [ -L "$P" ] && { echo "refusing to clear $1: $P is a symbolic link" >&2; exit 1; }
  done
  if [ -e "$1" ] && [ -z "$(find "$1" -prune -user "$(id -u)")" ]; then
    echo "refusing to clear $1: it is not owned by $(id -un)" >&2; exit 1
  fi
  rm -rf "$1" && mkdir -p "$1"
}

# The owner's real Baley folders as the binary resolves them (folders.rs) from the environment this script
# started in, listed now and again by live-install-reads.sh. The disposable run never uses these, so any
# change shows that something missed the prefix.
case "${XDG_DATA_HOME:-}" in /*) REAL_HOME="$XDG_DATA_HOME/crenshawdev/baley" ;; *) REAL_HOME="$OWNER_HOME/.local/share/crenshawdev/baley" ;; esac
case "${XDG_CONFIG_HOME:-}" in /*) REAL_CONF="$XDG_CONFIG_HOME/crenshawdev/baley" ;; *) REAL_CONF="$OWNER_HOME/.config/crenshawdev/baley" ;; esac
list_real() {
  if [ -d "$1" ]; then
    find "$1" -printf '%p %y %s %TY-%Tm-%Td %TH:%TM:%TS\n' | sort
  else
    echo absent
  fi
}

umask 077
# A clear that fails part way can leave a link behind that the writes below would follow out of the
# tree, so a failed clear stops the script before any write.
fresh "$ROOT" || { echo "could not clear $ROOT, so nothing was written" >&2; exit 1; }
mkdir -p "$DH" "$CC" "$PROJ" "$OUT" || exit 1
chmod 700 "$ROOT" "$DH" "$CC" "$PROJ" "$OUT"

# The sessions' git reads this file and nothing of the owner's: no identity and no signing.
GITCONFIG="$DH/.gitconfig"
git config --file "$GITCONFIG" user.name "Baley install run" || exit 1
git config --file "$GITCONFIG" user.email "install-run@example.invalid" || exit 1
git config --file "$GITCONFIG" commit.gpgsign false || exit 1
# The script's own git commands read the disposable HOME as well.
g() {
  env -u XDG_CONFIG_HOME HOME="$DH" git "$@"
}

# The seeded owner settings. One key Baley does not own, one owner deny rule over a path inside the root,
# one owner PreToolUse item with its own matcher (Bash is also in Baley's matcher) and a harmless command,
# and one owner host that is no provider's. The install must keep every one of them.
SEED="$CC/settings.json"
jq -n --arg deny "Read(/$ROOT/owner-private/**)" '{
  cleanupPeriodDays: 30,
  permissions: {deny: [$deny]},
  hooks: {PreToolUse: [{matcher: "Bash|WebFetch", hooks: [{type: "command", command: "true"}]}]},
  sandbox: {network: {allowedDomains: ["example.com"]}}
}' > "$SEED" || exit 1
chmod 600 "$SEED"
cp "$SEED" "$OUT/owner-settings.json" && chmod 600 "$OUT/owner-settings.json"

# Two projects on main with an initial commit and a local identity, neither initialised for Baley. The
# procedure initialises project one with the base binary and project two with the older one.
mkrepo() {
  g init -q -b main "$1" || exit 1
  g -C "$1" config --local user.name "Baley install run"
  g -C "$1" config --local user.email "install-run@example.invalid"
  g -C "$1" config --local commit.gpgsign false
  echo "$2" > "$1/README.md"
  g -C "$1" add -A
  g -C "$1" commit -q -m "initial" || exit 1
}
mkrepo "$PROJ/one" "project one"
mkrepo "$PROJ/two" "project two"

# What was true before the run: tools, platform, this checkout, the artifacts, and the owner's real
# installation, to compare after the run.
{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "platform: $(uname -srm)"
  for TOOL in bwrap socat sqlite3 jq curl sha256sum shasum git claude; do
    echo "$TOOL: $(command -v "$TOOL" 2>/dev/null || echo absent)"
  done
  echo "checkout-commit: $(git -C "$SCRIPT_DIR" rev-parse HEAD)"
  DIRTY=$(git -C "$SCRIPT_DIR" status --porcelain)
  if [ -n "$DIRTY" ]; then
    echo "checkout-status: DIRTY, the binaries may not match the commit"
    echo "$DIRTY"
  else
    echo "checkout-status: clean"
  fi
  if [ -f "$ARTIFACTS" ]; then
    echo "begin artifacts $ARTIFACTS"
    cat "$ARTIFACTS"
    echo "end artifacts"
  else
    echo "artifacts: absent"
  fi
  REAL_BALEY="$OWNER_HOME/.local/bin/baley"
  if [ -L "$REAL_BALEY" ]; then
    echo "real-baley: link to $(readlink "$REAL_BALEY")"
    echo "real-baley-sha256: $(sum256 "$REAL_BALEY" 2>/dev/null || echo unreadable)"
  elif [ -e "$REAL_BALEY" ]; then
    echo "real-baley: not a link"
    echo "real-baley-sha256: $(sum256 "$REAL_BALEY" 2>/dev/null || echo unreadable)"
  else
    echo "real-baley: absent"
    echo "real-baley-sha256: absent"
  fi
  if [ -f "$OWNER_HOME/.claude/settings.json" ]; then
    echo "real-claude-settings-sha256: $(sum256 "$OWNER_HOME/.claude/settings.json")"
  else
    echo "real-claude-settings-sha256: absent"
  fi
  if [ -f "$OWNER_HOME/.claude.json" ]; then
    echo "real-claude-registration-sha256: $(jq -c '.mcpServers.baley // "absent"' "$OWNER_HOME/.claude.json" 2>/dev/null | sha256sum | cut -d' ' -f1)"
  else
    echo "real-claude-registration-sha256: absent"
  fi
  if [ -L "$LAUNCHER" ]; then
    echo "claude-launcher: link to $(readlink "$LAUNCHER")"
  elif [ -e "$LAUNCHER" ]; then
    echo "claude-launcher: not a link"
  else
    echo "claude-launcher: absent"
  fi
  echo "begin real-home $REAL_HOME"
  list_real "$REAL_HOME"
  echo "end real-home"
  echo "begin real-config $REAL_CONF"
  list_real "$REAL_CONF"
  echo "end real-config"
} > "$OUT/pins.txt"
grep -q '^checkout-status: DIRTY' "$OUT/pins.txt" \
  && echo "WARNING: the checkout is dirty, so the binaries may not match the commit pins.txt names."

# The prefix every later command and session starts with. Baley's home and config folders, the stable path
# and the versions folder all derive from the disposable HOME, Claude Code's own lookups and updater stay
# inside it, and a bare baley resolves to the disposable stable path first.
PREFIX="env -u BALEY_HOME -u XDG_DATA_HOME -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME HOME=$DH CLAUDE_CONFIG_DIR=$CC DISABLE_AUTOUPDATER=1 PATH=$DH/.local/bin:$PATH"

# END OF PREPARATION

# Where a session starts. $1 is the label, $2 the start folder, $3 any extra claude flags. The permission
# mode that asks is set at launch; a part that needs another mode says so in its flags.
BALEY_TOOLS="mcp__baley__baley_version,mcp__baley__baley_query,mcp__baley__baley_apply"
launch() {
  printf '  %s\n    cd %s && %s claude --debug-file %s/debug-%s.log%s\n\n' \
    "$1" "$2" "$PREFIX" "$OUT" "$1" "$3"
}
print_launches() {
  cat <<TEXT

Sessions to launch (the coordinator starts each in its own tmux window; nothing below has been run for you)

TEXT
  launch before-install "$PROJ/one" " --permission-mode manual"
  launch after-install "$PROJ/one" " --permission-mode manual"
  launch protect "$PROJ/one" " --permission-mode manual"
  launch search-tools "$PROJ/one" " --permission-mode manual --allowedTools Grep,Glob"
  launch bypass "$PROJ/one" " --permission-mode bypassPermissions"
  launch network-project "$PROJ/one" " --permission-mode manual"
  launch opt-in "$PROJ/one" " --permission-mode manual --allowedTools $BALEY_TOOLS"
  launch manual-s1 "$PROJ/one" " --permission-mode manual --allowedTools $BALEY_TOOLS"
  launch manual-s2 "$PROJ/one" " --permission-mode manual --allowedTools $BALEY_TOOLS"
  launch older-s3 "$PROJ/two" " --permission-mode manual --allowedTools $BALEY_TOOLS"
  launch inflight-1 "$PROJ/one" " --permission-mode manual --allowedTools $BALEY_TOOLS"
  launch inflight-2 "$PROJ/one" " --permission-mode manual --allowedTools $BALEY_TOOLS"
}

cat <<TEXT
Installed qualification. This script prepared the disposable tree and ran no claude and no baley command.

Root:         $ROOT
Disposable HOME:  $DH
Claude folder:    $CC (CLAUDE_CONFIG_DIR)
Baley home:       $BHOME
Baley config:     $BCONF
Stable path:      $STABLE
Versions folder:  $VERSIONS
Projects:         $PROJ/one, $PROJ/two
Results:          $OUT
Procedure:        $GUIDE

Environment prefix (every command and session below starts with it):

  $PREFIX

Login into the isolated configuration (the owner types it; a coding agent stops here and asks):

  cd $DH && $PREFIX claude

TEXT
for TOOL in bwrap socat; do
  command -v "$TOOL" >/dev/null 2>&1 || echo "WARNING: $TOOL is not on PATH, so the sandbox cannot start on Linux."
done
print_launches
echo "Setup finished. Next: follow live-install.md Part 0."
