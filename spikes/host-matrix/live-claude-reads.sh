#!/bin/sh
# Post-run reads for the live Claude Code qualification. It reads what live-claude.sh prepared and the
# sessions left behind, opens the ledger read-only and changes nothing but a scratch folder it removes.
# The output is pasted into the post-run rows of the observation sheet:
#   sh live-claude-reads.sh > <root>/results/reads.txt
set -u

case "${HOME:-}" in
  /?*) ;;
  *) echo "HOME must be an absolute path" >&2; exit 1 ;;
esac

# BALEY_HOME would make home and config one folder, so a shell that sets it (an empty value included)
# is refused, as live-claude.sh refuses it. The disposable folders are the only ones these reads use.
if [ -n "${BALEY_HOME+set}" ]; then
  echo "BALEY_HOME is set. Unset it: it makes home and config one folder, and these reads use two." >&2
  exit 1
fi

ROOT="$HOME/.local/share/baley-live"
DATA="$ROOT/data"
CONF="$ROOT/config"
HOMEF="$DATA/crenshawdev/baley"
BIN="$ROOT/bin/baley"
OUT="$ROOT/results"
DB="$HOMEF/baley.db"

if [ ! -d "$ROOT" ]; then
  echo "missing $ROOT: run live-claude.sh first, and run these reads before running it again, since it clears the root" >&2
  exit 1
fi
if [ ! -f "$DB" ]; then
  echo "missing the ledger $DB: no session has written to it" >&2
  exit 1
fi
for TOOL in sqlite3 jq sha256sum xxd; do
  command -v "$TOOL" >/dev/null 2>&1 || { echo "$TOOL is required and is not on PATH" >&2; exit 1; }
done

# Where Claude Code keeps its installed versions, resolved from the environment this script was started in,
# the same way live-claude.sh resolved it.
case "${XDG_DATA_HOME:-}" in /*) REAL_VERSIONS="$XDG_DATA_HOME/claude/versions" ;; *) REAL_VERSIONS="$HOME/.local/share/claude/versions" ;; esac

# From here on the baley commands see the disposable folders only.
XDG_DATA_HOME="$DATA"
XDG_CONFIG_HOME="$CONF"
export XDG_DATA_HOME XDG_CONFIG_HOME

# Scratch files live in a folder this run creates and owns, so a file or link left at a predictable name
# in results can never be written through.
WORK=$(mktemp -d "$OUT/.reads.XXXXXX") || { echo "could not create a scratch folder in $OUT" >&2; exit 1; }
trap 'rm -rf "$WORK"' EXIT

pin() {
  sed -n "s/^$1: //p" "$OUT/pins.txt" 2>/dev/null
}
ONE=$(pin project-one-id)
TWO=$(pin project-two-id)
LARGE="$ROOT/projects/one/fixtures/large.txt"

# The ledger is opened read-only, so nothing here can change what it shows.
q() {
  sqlite3 -readonly -header -column "$DB" "$1"
}
heading() {
  printf '\n== %s ==\n' "$1"
}
# Runs a baley command and prints its exit status after it, spelled as the binary spells it.
status_of() {
  heading "$*"
  ( cd "$ROOT" && "$BIN" "$@" ) 2>&1
  echo "exit status: $?"
}

heading "pins"
cat "$OUT/pins.txt"

for ID in "$ONE" "$TWO" user; do
  status_of verify --local-only "$ID"
  status_of verify --views "$ID"
done
status_of doctor

heading "streams: events per project and stream, lowest and highest stream_version"
q "select project_id, stream, count(*) as events, min(stream_version) as lowest, max(stream_version) as highest from event group by project_id, stream order by project_id, stream"
heading "streams that do not start at 1 or whose count differs from the span"
q "select project_id, stream, count(*) as events, min(stream_version) as lowest, max(stream_version) as highest from event group by project_id, stream having min(stream_version) <> 1 or count(*) <> max(stream_version) - min(stream_version) + 1"
echo "(no rows above means every stream is complete)"

heading "server callers: each session with the project ids of the events it wrote"
q "select json_extract(caller, '\$.baley_session') as baley_session, json_extract(caller, '\$.project_directory') as project_directory, json_extract(caller, '\$.working_directory') as working_directory, json_extract(caller, '\$.host_session') as host_session, json_extract(caller, '\$.client_version') as client_version, project_id, min(seq) as first_seq, max(seq) as last_seq, count(*) as events from event where json_extract(caller, '\$.form') = 'server' group by 1, 2, 3, 4, 5, project_id order by min(recorded_at)"
echo "project ids in pins.txt: one=$ONE two=$TWO fork=$(pin project-fork-id)"

heading "hook callers in seq order"
q "select project_id, seq, json_extract(caller, '\$.host_session') as host_session, json_extract(caller, '\$.working_directory') as working_directory, json_extract(caller, '\$.project_directory') as project_directory, json_extract(caller, '\$.call.text') as call_id from event where json_extract(caller, '\$.form') = 'hook' order by project_id, seq"

heading "captures: capture.recorded events per request_id with the caller's session and instruction evidence"
q "select project_id, seq, request_id, count(*) over (partition by project_id, request_id) as events_for_request, json_extract(caller, '\$.baley_session') as baley_session, json_extract(caller, '\$.instructions') as instructions, json_extract(payload_json, '\$.kind') as kind, json_extract(payload_json, '\$.bytes') as bytes, json_extract(payload_json, '\$.text') as text from event where type = 'capture.recorded' order by project_id, seq"

heading "stored capture bodies (text above 4,096 bytes): byte count and SHA-256 after zstd -d"
LARGE_SUM=$(sha256sum "$LARGE" 2>/dev/null | cut -d' ' -f1)
echo "large.txt on disk: $(wc -c < "$LARGE" 2>/dev/null) bytes, sha256 $LARGE_SUM"
# The store keeps a body zstd-compressed, so hashing the stored bytes would never match the text sent.
sqlite3 -readonly "$DB" "select project_id, seq, json_extract(payload_json, '\$.body.payload') from event where type = 'capture.recorded' and json_extract(payload_json, '\$.body.payload') is not null order by project_id, seq" | while IFS='|' read -r PROJECT SEQ HASH; do
  BYTES=$(sqlite3 -readonly "$DB" "select bytes from payload where hash = x'$HASH'")
  ENCODING=$(sqlite3 -readonly "$DB" "select encoding from payload where hash = x'$HASH'")
  STORED=$(sqlite3 -readonly "$DB" "select length(body) from payload where hash = x'$HASH'")
  if [ "$ENCODING" = zstd ] && command -v zstd >/dev/null 2>&1; then
    SUM=$(sqlite3 -readonly "$DB" "select hex(body) from payload where hash = x'$HASH'" | xxd -r -p | zstd -dc | sha256sum | cut -d' ' -f1)
    LEN=$(sqlite3 -readonly "$DB" "select hex(body) from payload where hash = x'$HASH'" | xxd -r -p | zstd -dc | wc -c)
    SAME="different from large.txt"
    [ "$SUM" = "$LARGE_SUM" ] && SAME="same as large.txt"
    echo "$PROJECT seq $SEQ payload $HASH: $BYTES bytes recorded, stored $STORED bytes ($ENCODING), $LEN bytes after zstd -d, sha256 $SUM ($SAME)"
  else
    echo "$PROJECT seq $SEQ payload $HASH: $BYTES bytes recorded, stored $STORED bytes ($ENCODING), sha256 not checked (the stored body is compressed and zstd is not on PATH)"
  fi
done

heading "guard answers by decision, with call ids"
q "select json_extract(payload_json, '\$.outcome') as decision, seq, json_extract(payload_json, '\$.tool') as tool, json_extract(payload_json, '\$.verb') as verb, json_extract(payload_json, '\$.branch') as branch, json_extract(payload_json, '\$.call') as call_id from event where type = 'guard.answered' order by decision, seq"

heading "server stderr: start line, exit checkpoint lines and any abandoned-drain line, per file"
for FILE in "$OUT"/server-stderr/*; do
  [ -f "$FILE" ] || { echo "no server stderr files"; break; }
  echo "-- $FILE"
  sed -n '1p' "$FILE"
  echo "exit checkpoint lines: $(grep -c '^baley: exit checkpoint' "$FILE")"
  grep '^baley: exit checkpoint' "$FILE"
  grep "^baley: work still open after" "$FILE"
done

heading "hook timing: calls per tool with the highest and median elapsed milliseconds (the guard alone), the highest wrapper milliseconds and the highest overhead (wrapper minus guard)"
TIMING="$OUT/hook-timing.jsonl"
STARTS="$OUT/hook-starts.jsonl"
if [ -s "$TIMING" ]; then
  jq -sr 'group_by(.tool_name)[] | (map(.elapsed_ms) | sort) as $e | (map(.wrapper_ms // empty) | sort) as $w | (map(.overhead_ms // empty) | sort) as $o | "\(.[0].tool_name) calls=\(length) highest_ms=\($e[-1]) median_ms=\($e[(length / 2 | floor)]) highest_wrapper_ms=\(if ($w | length) > 0 then $w[-1] else "n/a" end) highest_overhead_ms=\(if ($o | length) > 0 then $o[-1] else "n/a" end)"' "$TIMING"
  heading "hook timing: guard decisions by tool"
  jq -sr 'group_by([.tool_name, (.permissionDecision // "none")])[] | "\(.[0].tool_name) \(.[0].permissionDecision // "none") \(length)"' "$TIMING"
  heading "hook timing: finished calls at or above 10,000 ms (the guard alone, or the whole wrapper)"
  jq -c 'select(.elapsed_ms >= 10000 or (.wrapper_ms // 0) >= 10000)' "$TIMING"
  echo "(no lines above means none)"
  heading "hook timing: tool_use_id seen more than once"
  jq -sr 'group_by(.tool_use_id)[] | select(length > 1 and .[0].tool_use_id != "") | "\(.[0].tool_use_id) seen \(length) times"' "$TIMING"
  echo "(no lines above means none)"
else
  echo "no hook-timing.jsonl: no hook call finished through the wrapper"
fi
# A call the host kills at its timeout never reaches the line above, so it shows only here.
heading "hook timing: calls that started and did not finish (killed at the timeout, or still running)"
if [ -s "$STARTS" ]; then
  FINISHED="$TIMING"
  [ -s "$TIMING" ] || { : > "$WORK/none"; FINISHED="$WORK/none"; }
  jq -c --slurpfile finished "$FINISHED" 'select(.call as $c | ($finished | map(.call) | index($c)) == null)' "$STARTS"
  echo "(no lines above means every started call finished)"
else
  echo "no hook-starts.jsonl: no hook call went through the wrapper"
fi

# The owner's real Baley folders, listed again with the same command live-claude.sh used.
list_real() {
  if [ -d "$1" ]; then
    find "$1" -printf '%p %y %s %TY-%Tm-%Td %TH:%TM:%TS\n' | sort
  else
    echo absent
  fi
}
for KIND in real-home real-config real-claude-versions; do
  heading "$KIND against pins.txt"
  FOLDER=$(sed -n "s/^begin $KIND //p" "$OUT/pins.txt")
  sed -n "/^begin $KIND /,/^end $KIND\$/p" "$OUT/pins.txt" | sed '1d;$d' > "$WORK/before-$KIND"
  list_real "$FOLDER" > "$WORK/after-$KIND"
  if diff "$WORK/before-$KIND" "$WORK/after-$KIND"; then
    echo "$FOLDER: no difference"
  else
    echo "$FOLDER: DIFFERENT (left: pins.txt, right: now)"
  fi
done
# The owner's claude command and launcher, compared with what pins.txt recorded before the run. The last
# lines say where each resolves now. These sections print evidence for the post-run row and judge nothing.
compare() {
  if diff "$1" "$2"; then
    echo "no difference"
  else
    echo "DIFFERENT (left: pins.txt, right: now)"
  fi
}
under_root() {
  case "$1" in
    "$ROOT"|"$ROOT"/*|"$ROOT_RESOLVED"|"$ROOT_RESOLVED"/*) return 0 ;;
  esac
  return 1
}
ROOT_RESOLVED="$(readlink -f "$HOME" 2>/dev/null || echo "$HOME")/.local/share/baley-live"
NOW_FOUND=$(command -v claude 2>/dev/null || true)
NOW_RESOLVED=""
case "$NOW_FOUND" in /*) NOW_RESOLVED=$(readlink -f "$NOW_FOUND" 2>/dev/null || true) ;; esac
LAUNCHER="$HOME/.local/bin/claude"
NOW_LAUNCHER_RESOLVED=""
if [ -L "$LAUNCHER" ]; then
  NOW_LAUNCHER_TEXT="link to $(readlink "$LAUNCHER")"
elif [ -e "$LAUNCHER" ]; then
  NOW_LAUNCHER_TEXT="not a link"
else
  NOW_LAUNCHER_TEXT="absent"
fi
[ -e "$LAUNCHER" ] && NOW_LAUNCHER_RESOLVED=$(readlink -f "$LAUNCHER" 2>/dev/null || true)

heading "claude command against pins.txt"
{ echo "command: $(pin claude-command)"; echo "resolved: $(pin claude-command-resolved)"; } > "$WORK/before-command"
{ echo "command: ${NOW_FOUND:-none}"; echo "resolved: ${NOW_RESOLVED:-none}"; } > "$WORK/after-command"
compare "$WORK/before-command" "$WORK/after-command"

heading "~/.local/bin/claude link against pins.txt"
{ echo "launcher: $(pin claude-launcher)"; echo "resolved: $(pin claude-launcher-resolved)"; } > "$WORK/before-launcher"
{ echo "launcher: $NOW_LAUNCHER_TEXT"; echo "resolved: ${NOW_LAUNCHER_RESOLVED:-none}"; } > "$WORK/after-launcher"
compare "$WORK/before-launcher" "$WORK/after-launcher"

heading "where the claude command and the launcher resolve now"
where() {
  if under_root "$2"; then
    echo "$1 resolves INSIDE the disposable root ($2)"
  else
    echo "$1 resolves outside the disposable root ($2)"
  fi
}
where "the claude command" "$NOW_RESOLVED"
where "the launcher" "$NOW_LAUNCHER_RESOLVED"

heading "entries other programs wrote under the exported variables (top level of the disposable data and config roots, other than crenshawdev; evidence only)"
for TREE in "$DATA" "$CONF"; do
  echo "-- $TREE"
  OTHERS=$(find "$TREE" -mindepth 1 -maxdepth 1 ! -name crenshawdev -printf '%f %y\n' 2>/dev/null | sort)
  if [ -n "$OTHERS" ]; then echo "$OTHERS"; else echo "(none)"; fi
done
exit 0
