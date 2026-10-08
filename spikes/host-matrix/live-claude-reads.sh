#!/bin/sh
# Post-run reads for the live Claude Code qualification. It reads what live-claude.sh prepared and the
# sessions left behind, opens the ledger read-only and changes nothing. The output is pasted into the
# post-run rows of the observation sheet: sh live-claude-reads.sh > <root>/results/reads.txt
set -u

case "${HOME:-}" in
  /?*) ;;
  *) echo "HOME must be an absolute path" >&2; exit 1 ;;
esac

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

# The disposable folders only, never BALEY_HOME.
unset BALEY_HOME
XDG_DATA_HOME="$DATA"
XDG_CONFIG_HOME="$CONF"
export XDG_DATA_HOME XDG_CONFIG_HOME

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

heading "stored capture bodies (text above 4,096 bytes): byte count and SHA-256 of the stored body"
echo "large.txt on disk: $(wc -c < "$LARGE" 2>/dev/null) bytes, sha256 $(sha256sum "$LARGE" 2>/dev/null | cut -d' ' -f1)"
sqlite3 -readonly "$DB" "select project_id, seq, json_extract(payload_json, '\$.body.payload') from event where type = 'capture.recorded' and json_extract(payload_json, '\$.body.payload') is not null order by project_id, seq" | while IFS='|' read -r PROJECT SEQ HASH; do
  BYTES=$(sqlite3 -readonly "$DB" "select bytes from payload where hash = x'$HASH'")
  SUM=$(sqlite3 -readonly "$DB" "select hex(body) from payload where hash = x'$HASH'" | xxd -r -p | sha256sum | cut -d' ' -f1)
  echo "$PROJECT seq $SEQ payload $HASH: $BYTES bytes, body sha256 $SUM"
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

heading "hook timing: calls per tool with the highest and median elapsed milliseconds"
TIMING="$OUT/hook-timing.jsonl"
if [ -s "$TIMING" ]; then
  jq -sr 'group_by(.tool_name)[] | (map(.elapsed_ms) | sort) as $e | "\(.[0].tool_name) calls=\(length) highest_ms=\($e[-1]) median_ms=\($e[(length / 2 | floor)])"' "$TIMING"
  heading "hook timing: guard decisions by tool"
  jq -sr 'group_by([.tool_name, (.permissionDecision // "none")])[] | "\(.[0].tool_name) \(.[0].permissionDecision // "none") \(length)"' "$TIMING"
  heading "hook timing: calls at or above 10,000 ms"
  jq -c 'select(.elapsed_ms >= 10000)' "$TIMING"
  echo "(no lines above means none)"
  heading "hook timing: tool_use_id seen more than once"
  jq -sr 'group_by(.tool_use_id)[] | select(length > 1 and .[0].tool_use_id != "") | "\(.[0].tool_use_id) seen \(length) times"' "$TIMING"
  echo "(no lines above means none)"
else
  echo "no hook-timing.jsonl: no hook call went through the wrapper"
fi

# The owner's real Baley folders, listed again with the same command live-claude.sh used.
list_real() {
  if [ -d "$1" ]; then
    find "$1" -printf '%p %y %s %TY-%Tm-%Td %TH:%TM:%TS\n' | sort
  else
    echo absent
  fi
}
for KIND in real-home real-config; do
  heading "$KIND against pins.txt"
  FOLDER=$(sed -n "s/^begin $KIND //p" "$OUT/pins.txt")
  sed -n "/^begin $KIND /,/^end $KIND\$/p" "$OUT/pins.txt" | sed '1d;$d' > "$OUT/.before-$KIND"
  list_real "$FOLDER" > "$OUT/.after-$KIND"
  if diff "$OUT/.before-$KIND" "$OUT/.after-$KIND"; then
    echo "$FOLDER: no difference"
  else
    echo "$FOLDER: DIFFERENT (left: pins.txt, right: now)"
  fi
  rm -f "$OUT/.before-$KIND" "$OUT/.after-$KIND"
done
exit 0
