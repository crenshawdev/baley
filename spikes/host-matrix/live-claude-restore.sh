#!/bin/sh
# Restore report for the live Claude Code qualification. It copies the disposable ledger, anchors project
# one on its disposable remote, restores the copy behind that anchor and prints each baley report that
# follows, every one with its exit status. It judges nothing: live-claude.md says what each section is
# expected to show, from design 0001. Run it once, after every session has exited and after the post-run
# reads are saved, because it changes the ledger:
#   sh live-claude-restore.sh > <root>/results/restore.txt
set -u

if [ $# -ne 0 ]; then
  echo "usage: sh live-claude-restore.sh (no arguments)" >&2
  exit 1
fi
case "${HOME:-}" in
  /?*) ;;
  *) echo "HOME must be an absolute path" >&2; exit 1 ;;
esac

# BALEY_HOME would make home and config one folder, so a shell that sets it (an empty value included)
# is refused, as live-claude.sh refuses it.
if [ -n "${BALEY_HOME+set}" ]; then
  echo "BALEY_HOME is set. Unset it: it makes home and config one folder, and this run needs two." >&2
  exit 1
fi

# Git's own configuration variables would send the commit below to the owner's files instead of the
# disposable repository, so none of them reaches this script's git or baley commands.
for NAME in $(env | sed -n 's/^\(GIT_CONFIG_[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$NAME"; done
unset GIT_CONFIG

ROOT="$HOME/.local/share/baley-live"
DATA="$ROOT/data"
CONF="$ROOT/config"
HOMEF="$DATA/crenshawdev/baley"
BIN="$ROOT/bin/baley"
OUT="$ROOT/results"
DB="$HOMEF/baley.db"
ONE_DIR="$ROOT/projects/one"
REMOTE_DIR="$ROOT/remotes/one.git"
COPY="$OUT/baley-copy.db"

if [ ! -d "$ROOT" ]; then
  echo "missing $ROOT: run live-claude.sh and the sessions first" >&2
  exit 1
fi
if [ ! -f "$DB" ]; then
  echo "missing the ledger $DB: no session has written to it" >&2
  exit 1
fi
# The reads come first, because this script changes the ledger and they describe it as the sessions left it.
if [ ! -f "$OUT/reads.txt" ]; then
  echo "missing $OUT/reads.txt: save the post-run reads first (sh live-claude-reads.sh > $OUT/reads.txt), since this script changes the ledger" >&2
  exit 1
fi
for PATH_NEEDED in "$BIN" "$ONE_DIR/baley.toml" "$REMOTE_DIR"; do
  if [ ! -e "$PATH_NEEDED" ]; then
    echo "missing $PATH_NEEDED" >&2
    exit 1
  fi
done
for TOOL in sqlite3 git awk ps; do
  command -v "$TOOL" >/dev/null 2>&1 || { echo "$TOOL is required and is not on PATH" >&2; exit 1; }
done

umask 077
WORK=$(mktemp -d "$OUT/.restore.XXXXXX") || { echo "could not create a scratch folder in $OUT" >&2; exit 1; }
trap 'rm -rf "$WORK"' EXIT

# The copy and the restore need Baley stopped (design 0001, "Copies of the store"). The process list is
# written to a file first, so this script's own command line is never part of what is searched.
ps -eo pid=,args= > "$WORK/processes" 2>/dev/null
if grep -F -- "$BIN" "$WORK/processes" > "$WORK/running"; then
  echo "a running process names $BIN, so the ledger is not stopped:" >&2
  cat "$WORK/running" >&2
  echo "Close every session and wait for its server to exit, then run this again." >&2
  exit 1
fi

ONE=$(sed -n 's/^project-one-id: //p' "$OUT/pins.txt" 2>/dev/null)
if [ -z "$ONE" ]; then
  echo "pins.txt names no project-one-id" >&2
  exit 1
fi

# From here on the baley commands see the disposable folders only.
XDG_DATA_HOME="$DATA"
XDG_CONFIG_HOME="$CONF"
export XDG_DATA_HOME XDG_CONFIG_HOME

heading() {
  printf '\n== %s ==\n' "$1"
}
# Runs a baley command from project one, as an owner would, and prints its exit status after it.
status_of() {
  heading "$*"
  ( cd "$ONE_DIR" && "$BIN" "$@" ) 2>&1
  echo "exit status: $?"
}
stop() {
  echo "$1" >&2
  exit 1
}
highest() {
  sqlite3 -readonly "$1" "select coalesce(max(seq), 0) from event where project_id = '$ONE'"
}
# The highest sequence any anchor tag on the remote names for project one.
remote_highest() {
  git --git-dir="$REMOTE_DIR" tag -l "baley-anchor/$ONE/*" | sed 's#.*/##' | sort -n | tail -1
}

# The anchor remote is a project setting read from HEAD's copy of baley.toml (design 0001, anchoring), so
# the setting is committed on main, with the repository's own identity, before any anchor.
heading "project one: git.remote set to origin and committed on main"
if awk '/^\[git\]$/ { in_git = 1; next } /^\[/ { in_git = 0 } in_git && /^remote *=/ { found = 1 } END { exit !found }' "$ONE_DIR/baley.toml"; then
  echo "git.remote is already set in baley.toml, so nothing was changed"
else
  awk '{ print } /^\[git\]$/ { print "remote = \"origin\"" }' "$ONE_DIR/baley.toml" > "$WORK/baley.toml" || stop "could not write the new baley.toml"
  cat "$WORK/baley.toml" > "$ONE_DIR/baley.toml" || stop "could not update $ONE_DIR/baley.toml"
  git -C "$ONE_DIR" add baley.toml || stop "git add failed in $ONE_DIR"
  git -C "$ONE_DIR" commit -q -m "name the anchor remote" || stop "git commit failed in $ONE_DIR"
fi
echo "branch: $(git -C "$ONE_DIR" rev-parse --abbrev-ref HEAD), commit: $(git -C "$ONE_DIR" rev-parse --short HEAD)"
echo "git show HEAD:baley.toml"
git -C "$ONE_DIR" show HEAD:baley.toml

# The policy step of this anchor records the new setting, and the remote then holds an anchor that matches
# the chain, before the copy is taken.
echo "first anchor, before the copy"
status_of anchor

heading "copy of baley.db taken with SQLite's backup API, mode 0600"
rm -f "$COPY"
sqlite3 -readonly "$DB" ".backup '$COPY'" || stop "the backup of $DB failed"
chmod 600 "$COPY" || stop "could not set the mode of $COPY"
COPY_HEAD=$(highest "$COPY")
ls -l "$COPY"
echo "project one's highest seq in the copy: $COPY_HEAD"

# An anchor names the sequence the chain had before its own events, and the events it writes come after,
# so an anchor right after the copy names the copy's head and no more. One more anchor then names a
# sequence above it. The restore has to land behind an anchor, so a third anchor is taken only if needed.
echo "second anchor, after the copy"
status_of anchor
REMOTE_TOP=$(remote_highest)
if [ -z "$REMOTE_TOP" ] || [ "$REMOTE_TOP" -le "$COPY_HEAD" ]; then
  echo "no anchor names a sequence above the copy's head yet, so third anchor"
  status_of anchor
fi

heading "the remote's anchor tags for project one, and project one's highest anchor.pushed sequence in the ledger"
git --git-dir="$REMOTE_DIR" tag -l "baley-anchor/$ONE/*"
LEDGER_PUSHED=$(sqlite3 -readonly "$DB" "select coalesce(max(seq), 0) from event where project_id = '$ONE' and type = 'anchor.pushed'")
echo "highest anchor.pushed sequence in the ledger: $LEDGER_PUSHED"
REMOTE_TOP=$(remote_highest)
echo "highest sequence an anchor tag names: $REMOTE_TOP"
echo "copy's head: $COPY_HEAD"
if [ -z "$REMOTE_TOP" ] || [ "$REMOTE_TOP" -le "$COPY_HEAD" ]; then
  stop "no anchor on the remote names a sequence above the copy's head ($COPY_HEAD), so a restore of the copy would not land behind an anchor. The ledger has not been restored. The sections above show why."
fi

heading "restore: baley.db replaced by the copy, its -wal and -shm files removed, mode 0600"
rm -f "$DB-wal" "$DB-shm" || stop "could not remove the write-ahead files of $DB"
cp "$COPY" "$DB" || stop "could not copy $COPY over $DB"
chmod 600 "$DB" || stop "could not set the mode of $DB"
ls -l "$HOMEF"
echo "project one's highest seq in the restored ledger: $(highest "$DB")"

status_of verify
status_of acknowledge-restore
status_of acknowledge-restore
status_of verify
status_of verify --local-only "$ONE"
status_of doctor
exit 0
