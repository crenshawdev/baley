#!/bin/sh
# Live Claude Code qualification, preparation side. Given a built baley, it builds a disposable
# tree under ~/.local/share/baley-live, renders the real binary's artifacts into launch files, writes
# the observation sheet and prints the exact launch of every session in live-claude.md. It never
# starts Claude Code: the owner runs every session by hand.
set -u

if [ $# -ne 1 ]; then
  echo "usage: sh live-claude.sh <absolute path of a built baley>" >&2
  exit 1
fi
SUPPLIED=$1
case "$SUPPLIED" in
  /*) ;;
  *) echo "the baley path must be absolute: $SUPPLIED" >&2; exit 1 ;;
esac
if [ ! -f "$SUPPLIED" ] || [ ! -x "$SUPPLIED" ]; then
  echo "not an executable file: $SUPPLIED" >&2; exit 1
fi

# BALEY_HOME collapses home and config into one folder, so the two-folder run cannot use it.
if [ -n "${BALEY_HOME+set}" ]; then
  echo "BALEY_HOME is set. Unset it: it makes home and config one folder, and this run needs two." >&2
  exit 1
fi

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

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)

ROOT="$HOME/.local/share/baley-live"
DATA="$ROOT/data"
CONF="$ROOT/config"
HOMEF="$DATA/crenshawdev/baley"
CONFF="$CONF/crenshawdev/baley"
BIN="$ROOT/bin/baley"
CC="$ROOT/claude-config"
PROJ="$ROOT/projects"
REM="$ROOT/remotes"
OUT="$ROOT/results"
REN="$OUT/rendered"

# The only deletion in this script: the tree has to be exactly the root named above, with no
# symbolic link from HOME down to it, so the delete cannot land somewhere else, and owned by this user.
fresh() {
  case "$1" in
    "$HOME/.local/share/baley-live") ;;
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

# The owner's real Baley folders as the binary resolves them (folders.rs), listed now and again
# by live-claude-reads.sh, so a hook or server that missed the exported variables shows up as a change.
case "${XDG_DATA_HOME:-}" in /*) REAL_HOME="$XDG_DATA_HOME/crenshawdev/baley" ;; *) REAL_HOME="$HOME/.local/share/crenshawdev/baley" ;; esac
case "${XDG_CONFIG_HOME:-}" in /*) REAL_CONF="$XDG_CONFIG_HOME/crenshawdev/baley" ;; *) REAL_CONF="$HOME/.config/crenshawdev/baley" ;; esac
list_real() {
  if [ -d "$1" ]; then
    find "$1" -printf '%p %y %s %TY-%Tm-%Td %TH:%TM:%TS\n' | sort
  else
    echo absent
  fi
}

umask 077
fresh "$ROOT"
mkdir -p "$HOMEF" "$CONFF" "$ROOT/bin" "$CC" "$PROJ" "$REM" "$OUT" "$REN" \
  "$OUT/server-stderr" "$OUT/hook-calls" "$ROOT/bin-nosandbox" "$ROOT/bin-nogit"

# Harmless fixtures in both folders. The key value is fake on purpose, and no real credential enters
# either folder.
for D in "$HOMEF" "$CONFF"; do
  echo seed > "$D/seed.txt"
  echo "FAKE_KEY=not-a-real-key-0000" > "$D/keys.env"
  printf '%s\n' '{"cells":[{"cell_type":"code","execution_count":null,"id":"c1","metadata":{},"outputs":[],"source":["x = 1"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}' > "$D/notebook.ipynb"
  chmod 600 "$D/seed.txt" "$D/keys.env" "$D/notebook.ipynb"
done
# A script for the steps that check a command's child process.
cat > "$ROOT/child.sh" <<'EOF'
#!/bin/sh
case "$1" in
  read) cat "$2" ;;
  write) echo child > "$2" ;;
esac
EOF

# Every rendered artifact names this copy, so no step ever writes toward a build folder.
cp "$SUPPLIED" "$BIN" && chmod 700 "$BIN"

# The two folders of commands for the sessions that run with a narrowed PATH. git runs its own
# subcommands without GIT_EXEC_PATH, so removing the exec folder would not make it fail: the
# fallback session has no git at all instead.
link_cmds() {
  DIR=$1; shift
  for NAME in "$@"; do
    FOUND=$(command -v "$NAME" 2>/dev/null || true)
    case "$FOUND" in
      /*) ln -s "$FOUND" "$DIR/$NAME" ;;
      *) echo "WARNING: $NAME is not on PATH, so $DIR has no $NAME." ;;
    esac
  done
  if [ -n "${SHELL:-}" ] && [ -x "$SHELL" ]; then ln -sf "$SHELL" "$DIR/$(basename "$SHELL")"; fi
}
link_cmds "$ROOT/bin-nosandbox" sh env git cat ls jq claude
link_cmds "$ROOT/bin-nogit" sh env cat ls jq claude bwrap socat date

cat > "$OUT/pins.txt" <<EOF
date: $(date -u +%Y-%m-%dT%H:%M:%SZ)
platform: $(uname -sr)
EOF
for TOOL in bwrap socat sqlite3 jq pwsh git; do
  echo "$TOOL: $(command -v "$TOOL" 2>/dev/null || echo absent)" >> "$OUT/pins.txt"
done
if [ -n "${DISABLE_TELEMETRY:-}" ] || [ -n "${CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC:-}" ]; then
  echo "telemetry-switch: set (Monitor is unavailable)" >> "$OUT/pins.txt"
else
  echo "telemetry-switch: unset" >> "$OUT/pins.txt"
fi
{
  echo "checkout-commit: $(git -C "$SCRIPT_DIR" rev-parse HEAD)"
  DIRTY=$(git -C "$SCRIPT_DIR" status --porcelain)
  if [ -n "$DIRTY" ]; then
    echo "checkout-status: DIRTY, the binary may not match the commit"
    echo "$DIRTY"
  else
    echo "checkout-status: clean"
  fi
  echo "binary: $("$BIN" --version)"
  echo "binary-sha256: $(sha256sum "$BIN" | cut -d' ' -f1)"
  echo "begin real-home $REAL_HOME"
  list_real "$REAL_HOME"
  echo "end real-home"
  echo "begin real-config $REAL_CONF"
  list_real "$REAL_CONF"
  echo "end real-config"
} >> "$OUT/pins.txt"
grep -q '^checkout-status: DIRTY' "$OUT/pins.txt" \
  && echo "WARNING: the checkout is dirty, so the binary may not match the commit pins.txt names."

# From here on this script's own baley and git commands see the disposable folders only.
XDG_DATA_HOME="$DATA"
XDG_CONFIG_HOME="$CONF"
export XDG_DATA_HOME XDG_CONFIG_HOME

# The disposable repositories. Each carries its own identity and no signing, so the commits the agent
# is allowed to make depend on neither a global identity nor the owner's signing agent. Nothing here
# touches the global git config or this repository's.
mkrepo() {
  git init -q -b main "$1" || exit 1
  git -C "$1" config user.name "Baley live run"
  git -C "$1" config user.email "live-run@example.invalid"
  git -C "$1" config commit.gpgsign false
}
project_id() {
  sed -n 's/^id = "\(.*\)"$/\1/p' "$1/baley.toml"
}
# Ties a repository to a ledger project, keeps the command's output and exit status for the
# owner-init row, then commits the setting the guard reads: a commit on main is refused.
init_project() {
  NAME=$1
  ( cd "$PROJ/$NAME" && "$BIN" init ) > "$OUT/init-$NAME.txt" 2>&1
  STATUS=$?
  echo "$STATUS" > "$OUT/init-$NAME.status"
  if [ "$STATUS" -ne 0 ]; then
    echo "baley init failed in $PROJ/$NAME (exit $STATUS):" >&2
    cat "$OUT/init-$NAME.txt" >&2
    exit 1
  fi
  printf '\n[git]\non_protected = "refuse"\n' >> "$PROJ/$NAME/baley.toml"
  git -C "$PROJ/$NAME" add baley.toml
  git -C "$PROJ/$NAME" commit -q -m "refuse commits on a protected branch"
}

git init -q --bare -b main "$REM/one.git" || exit 1
git init -q --bare -b main "$REM/fork.git" || exit 1

# Project one: the start folder of the user-scope session is its tracked sub folder, so the project and
# the working directory differ. fixtures/large.txt is plain ASCII of 33,000 bytes, a capture whose
# document read arrives in parts, the first a full 24,576 bytes.
mkrepo "$PROJ/one"
mkdir -p "$PROJ/one/sub" "$PROJ/one/fixtures"
echo "a tracked file in a sub folder" > "$PROJ/one/sub/note.txt"
echo "project one" > "$PROJ/one/README.md"
awk 'BEGIN { for (i = 1; i <= 600; i++) printf "line %04d: the quick brown fox jumps over the lazy dog\n", i }' > "$PROJ/one/fixtures/large.txt"
git -C "$PROJ/one" add -A
git -C "$PROJ/one" commit -q -m "initial"
git -C "$PROJ/one" remote add origin "$REM/one.git"
init_project one

# Project two is the /cd target: initialised the same way, with no remote.
mkrepo "$PROJ/two"
echo "project two" > "$PROJ/two/README.md"
git -C "$PROJ/two" add -A
git -C "$PROJ/two" commit -q -m "initial"
init_project two

# The fork is a clone of project one whose origin is another remote and which is not initialised
# again, so it shares project one's id under a different remote.
git clone -q "$PROJ/one" "$PROJ/fork" || exit 1
git -C "$PROJ/fork" config user.name "Baley live run"
git -C "$PROJ/fork" config user.email "live-run@example.invalid"
git -C "$PROJ/fork" config commit.gpgsign false
git -C "$PROJ/fork" remote set-url origin "$REM/fork.git"

ID_ONE=$(project_id "$PROJ/one")
ID_TWO=$(project_id "$PROJ/two")
{
  echo "project-one-id: $ID_ONE"
  echo "project-two-id: $ID_TWO"
  echo "project-fork-id: $(project_id "$PROJ/fork")"
} >> "$OUT/pins.txt"
