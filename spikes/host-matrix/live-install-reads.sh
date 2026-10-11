#!/bin/sh
# Post-run reads for the installed qualification. It reads what live-install.sh prepared and the sessions left
# behind, opens the ledger read-only and changes nothing but a scratch folder it removes. Its baley commands
# run through the disposable stable path with the same environment prefix live-install.sh printed. The output
# is pasted into the post-run rows of the observation sheet:
#   sh live-install-reads.sh > <root>/results/reads.txt
set -u

case "${HOME:-}" in
  /?*) ;;
  *) echo "HOME must be an absolute path" >&2; exit 1 ;;
esac

# BALEY_HOME would move Baley's home and config off the disposable HOME, so a shell that sets it (an empty
# value included) is refused, as live-install.sh refuses it.
if [ -n "${BALEY_HOME+set}" ]; then
  echo "BALEY_HOME is set. Unset it: it makes home and config one folder and moves them off the disposable HOME." >&2
  exit 1
fi

# An inherited GIT_ variable can point git at another repository, so none reaches a command here.
for NAME in $(env | sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p'); do
  [ "$NAME" = GIT_EXEC_PATH ] || unset "$NAME"
done

OWNER_HOME=$HOME
ROOT="$OWNER_HOME/.local/share/baley-install-live"
DH="$ROOT/home"
CC="$ROOT/claude-config"
PROJ="$ROOT/projects"
OUT="$ROOT/results"
STABLE="$DH/.local/bin/baley"
VERSIONS="$DH/.local/lib/crenshawdev/baley/versions"
DB="$DH/.local/share/crenshawdev/baley/baley.db"

if [ ! -d "$ROOT" ]; then
  echo "missing $ROOT: run live-install.sh first, and run these reads before running it again, since it clears the root" >&2
  exit 1
fi
if [ ! -f "$DB" ]; then
  echo "missing the disposable ledger $DB: nothing has written to it yet" >&2
  exit 1
fi
if [ ! -L "$STABLE" ] && [ ! -e "$STABLE" ]; then
  echo "missing the disposable stable path $STABLE: no install has placed a baley there" >&2
  exit 1
fi
for TOOL in sqlite3 jq sha256sum; do
  command -v "$TOOL" >/dev/null 2>&1 || { echo "$TOOL is required and is not on PATH" >&2; exit 1; }
done

# Scratch files live in a folder this run creates and owns, so a file or link left at a predictable name
# in results can never be written through.
WORK=$(mktemp -d "$OUT/.reads.XXXXXX") || { echo "could not create a scratch folder in $OUT" >&2; exit 1; }
trap 'rm -rf "$WORK"' EXIT

# The ledger is opened read-only, so nothing here can change what it shows.
q() {
  sqlite3 -readonly -header -column "$DB" "$1"
}
heading() {
  printf '\n== %s ==\n' "$1"
}
# Runs a command with the environment prefix live-install.sh printed, so Baley's home and config folders, the
# stable path and the versions folder all derive from the disposable HOME.
run() {
  env -u BALEY_HOME -u XDG_DATA_HOME -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME \
    HOME="$DH" CLAUDE_CONFIG_DIR="$CC" DISABLE_AUTOUPDATER=1 PATH="$DH/.local/bin:$PATH" "$@"
}
# Runs a baley command through the stable path and prints its exit status after it, spelled as the binary spells it.
status_of() {
  heading "baley $*"
  ( cd "$ROOT" && run "$STABLE" "$@" ) 2>&1
  echo "exit status: $?"
}
project_id() {
  sed -n 's/^id = "\(.*\)"$/\1/p' "$1/baley.toml" 2>/dev/null
}
ONE=$(project_id "$PROJ/one")
TWO=$(project_id "$PROJ/two")

heading "pins"
cat "$OUT/pins.txt"
echo "project ids: one=${ONE:-none} two=${TWO:-none}"

for ID in user "$ONE" "$TWO"; do
  if [ -z "$ID" ]; then
    heading "verify for a project with no baley.toml"
    echo "skipped: that project was not initialised"
    continue
  fi
  status_of verify --local-only "$ID"
  status_of verify --views "$ID"
done
status_of doctor

heading "install.recorded in sequence order"
q "select seq, recorded_at, json_extract(payload_json, '\$.binary_version') as binary_version, json_extract(payload_json, '\$.complete') as complete, json_extract(payload_json, '\$.sandbox.held_back') as held_back, json_extract(payload_json, '\$.registered.hook') is null as hook_is_null from event where project_id = 'user' and type = 'install.recorded' order by seq"

heading "update.checked and update.failed"
q "select seq, type, actor, recorded_at, json_extract(payload_json, '\$.day') as day, coalesce(json_extract(payload_json, '\$.outcome'), json_extract(payload_json, '\$.code')) as outcome_or_code, json_extract(payload_json, '\$.active_version') as active_version, json_extract(payload_json, '\$.staged_version') as staged_version from event where project_id = 'user' and type in ('update.checked', 'update.failed') order by seq"

heading "update.check claims"
q "select seq, actor, recorded_at, request_id from event where project_id = 'user' and stream = 'command/update.check' and type = 'command.claimed' order by seq"

heading "captures per project: recorded_at, request id and the caller's baley_session"
q "select project_id, seq, recorded_at, request_id, json_extract(caller, '\$.baley_session') as baley_session from event where type = 'capture.recorded' order by project_id, seq"

heading "guard answers in user by decision"
q "select json_extract(payload_json, '\$.outcome') as decision, seq, recorded_at, json_extract(payload_json, '\$.tool') as tool, json_extract(payload_json, '\$.verb') as verb, json_extract(payload_json, '\$.call') as call_id from event where project_id = 'user' and type = 'guard.answered' order by decision, seq"

heading "the stable link's stored target"
if [ -L "$STABLE" ]; then
  readlink "$STABLE"
else
  echo "$STABLE is not a link"
fi

heading "versions folder: each child with its baley's modification time in UTC (when its download finished)"
for CHILD in "$VERSIONS"/*; do
  [ -e "$CHILD" ] || { echo "(the versions folder is empty or absent)"; break; }
  if [ -f "$CHILD/baley" ]; then
    TZ=UTC find "$CHILD/baley" -printf '%h %TY-%Tm-%TdT%TH:%TM:%TSZ %s bytes\n'
  else
    echo "$CHILD holds no baley"
  fi
done

SETTINGS="$CC/settings.json"
heading "settings: Baley PreToolUse items that run guard at the disposable stable path"
if [ -f "$SETTINGS" ]; then
  jq --arg stable "$STABLE" '[.hooks.PreToolUse[]? | select(any(.hooks[]?; (.command | contains($stable)) and (.command | endswith("guard"))))] | length' "$SETTINGS"
else
  echo "missing $SETTINGS"
fi

heading "settings: the seeded owner entries against results/owner-settings.json"
if [ -f "$SETTINGS" ] && [ -f "$OUT/owner-settings.json" ]; then
  jq -n -r --slurpfile owner "$OUT/owner-settings.json" --slurpfile now "$SETTINGS" '
    def mark(held): if held then "kept" else "changed" end;
    "owner key cleanupPeriodDays: \(mark($now[0].cleanupPeriodDays == $owner[0].cleanupPeriodDays))",
    "owner permissions.deny rule: \(mark(($owner[0].permissions.deny // []) - ($now[0].permissions.deny // []) == []))",
    "owner PreToolUse item: \(mark(($owner[0].hooks.PreToolUse // []) | all(. as $item | ($now[0].hooks.PreToolUse // []) | any(. == $item))))",
    "owner sandbox.network.allowedDomains host: \(mark(($owner[0].sandbox.network.allowedDomains // []) - ($now[0].sandbox.network.allowedDomains // []) == []))"'
  heading "settings: sandbox.network.allowedDomains"
  jq -c '.sandbox.network.allowedDomains' "$SETTINGS"
else
  echo "missing $SETTINGS or results/owner-settings.json"
fi

heading "registration: the keys of mcpServers in $CC/.claude.json"
if [ -f "$CC/.claude.json" ]; then
  jq -c '.mcpServers | keys' "$CC/.claude.json"
else
  echo "absent"
fi

heading "stubs under $CC/skills with their SHA-256"
for STUB in "$CC"/skills/*/SKILL.md; do
  [ -f "$STUB" ] || { echo "(no stub)"; break; }
  echo "$(sha256sum "$STUB" | cut -d' ' -f1)  $STUB"
done

# The owner's real installation, read again the way live-install.sh read it, line for line.
case "${XDG_DATA_HOME:-}" in /*) REAL_HOME="$XDG_DATA_HOME/crenshawdev/baley" ;; *) REAL_HOME="$OWNER_HOME/.local/share/crenshawdev/baley" ;; esac
case "${XDG_CONFIG_HOME:-}" in /*) REAL_CONF="$XDG_CONFIG_HOME/crenshawdev/baley" ;; *) REAL_CONF="$OWNER_HOME/.config/crenshawdev/baley" ;; esac
list_real() {
  if [ -d "$1" ]; then
    find "$1" -printf '%p %y %s %TY-%Tm-%Td %TH:%TM:%TS\n' | sort
  else
    echo absent
  fi
}
sum256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
LAUNCHER="$OWNER_HOME/.local/bin/claude"
real_lines() {
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
}
heading "the owner's real installation against pins.txt (a difference is shown, left: pins.txt, right: now)"
sed -n '/^real-baley: /p; /^real-baley-sha256: /p; /^real-claude-settings-sha256: /p; /^real-claude-registration-sha256: /p; /^claude-launcher: /p' "$OUT/pins.txt" > "$WORK/before-lines"
real_lines > "$WORK/after-lines"
if diff "$WORK/before-lines" "$WORK/after-lines"; then
  echo "~/.local/bin/baley, ~/.claude/settings.json, the mcpServers.baley entry and the claude launcher: no difference"
fi
for KIND in real-home real-config; do
  FOLDER=$(sed -n "s/^begin $KIND //p" "$OUT/pins.txt")
  sed -n "/^begin $KIND /,/^end $KIND\$/p" "$OUT/pins.txt" | sed '1d;$d' > "$WORK/before-$KIND"
  list_real "$FOLDER" > "$WORK/after-$KIND"
  if diff "$WORK/before-$KIND" "$WORK/after-$KIND"; then
    echo "$FOLDER: no difference"
  else
    echo "$FOLDER: DIFFERENT (left: pins.txt, right: now)"
  fi
done

heading "where the claude command and the launcher resolve now"
ROOT_RESOLVED="$(readlink -f "$OWNER_HOME" 2>/dev/null || echo "$OWNER_HOME")/.local/share/baley-install-live"
NOW_FOUND=$(command -v claude 2>/dev/null || true)
NOW_RESOLVED=""
case "$NOW_FOUND" in /*) NOW_RESOLVED=$(readlink -f "$NOW_FOUND" 2>/dev/null || true) ;; esac
LAUNCHER_RESOLVED=""
[ -e "$LAUNCHER" ] && LAUNCHER_RESOLVED=$(readlink -f "$LAUNCHER" 2>/dev/null || true)
where() {
  case "$2" in
    "$ROOT"|"$ROOT"/*|"$ROOT_RESOLVED"|"$ROOT_RESOLVED"/*|"$OWNER_HOME/.local/share/baley-install-artifacts"|"$OWNER_HOME/.local/share/baley-install-artifacts"/*)
      echo "$1 resolves INSIDE a disposable root ($2)" ;;
    *) echo "$1 resolves outside both disposable roots (${2:-nothing})" ;;
  esac
}
where "the claude command" "$NOW_RESOLVED"
where "the launcher" "$LAUNCHER_RESOLVED"
exit 0
