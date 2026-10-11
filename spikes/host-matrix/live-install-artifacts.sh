#!/bin/sh
# Builds the three unsigned development artifact trees the installed qualification uses and lays them out
# the way install.sh reads them. Nothing is published: pushing the trees to a throwaway branch of this repository
# is the owner's step, in the procedure's Part 0.
#
#   older  the T14 merge, version 0.0.1. It has no install command and predates the view set raised to 8.
#   base   the commit being qualified, version 0.0.2.
#   newer  the commit being qualified at its own version, with two local edits so an install rerun has a
#          stub to upgrade and a stub the binary no longer ships.
#
# Every build runs in its own git worktree under the build folder, so this checkout is left as it was.
set -u

if [ $# -ne 2 ]; then
  echo "usage: sh live-install-artifacts.sh <commit to qualify> <absolute path of a build folder that does not exist yet>" >&2
  exit 1
fi
COMMIT_ARG=$1
BUILD=$2
case "$BUILD" in
  /?*) ;;
  *) echo "the build folder must be an absolute path: $BUILD" >&2; exit 1 ;;
esac
if [ -e "$BUILD" ] || [ -L "$BUILD" ]; then
  echo "the build folder exists already: $BUILD. Name one that does not." >&2
  exit 1
fi
case "$(dirname "$BUILD")" in
  /) echo "the build folder may not sit directly under /: $BUILD" >&2; exit 1 ;;
esac
[ -d "$(dirname "$BUILD")" ] || { echo "the folder that would hold the build folder is missing: $(dirname "$BUILD")" >&2; exit 1; }

# An inherited GIT_ variable can pick another repository, work tree or index, so git would act outside
# this checkout. None of them reaches this script's git commands. Only GIT_EXEC_PATH stays.
for NAME in $(env | sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p'); do
  [ "$NAME" = GIT_EXEC_PATH ] || unset "$NAME"
done

case "${HOME:-}" in
  ""|/) echo "HOME must be a real directory" >&2; exit 1 ;;
  /*) ;;
  *) echo "HOME must be absolute" >&2; exit 1 ;;
esac
for TOOL in git cargo jq; do
  command -v "$TOOL" >/dev/null 2>&1 || { echo "$TOOL is required and is not on PATH" >&2; exit 1; }
done
if command -v sha256sum >/dev/null 2>&1; then
  sum256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sum256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  echo "sha256sum or shasum is required and neither is on PATH" >&2
  exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel) || { echo "this script must run from a checkout of the repository" >&2; exit 1; }
QUALIFIED=$(git -C "$REPO" rev-parse --verify "$COMMIT_ARG^{commit}" 2>/dev/null) \
  || { echo "the commit to qualify does not resolve in this checkout: $COMMIT_ARG" >&2; exit 1; }
# The merge of pull request 250, the last commit before the install command and the view set 8 existed.
OLDER_COMMIT=$(git -C "$REPO" rev-parse --verify "beba9141^{commit}" 2>/dev/null) \
  || { echo "the older commit beba9141 does not resolve in this checkout" >&2; exit 1; }
QUALIFIED_VERSION=$(git -C "$REPO" show "$QUALIFIED:crates/baley/Cargo.toml" | awk -F'"' '/^version = "/ { print $2; exit }')
[ -n "$QUALIFIED_VERSION" ] || { echo "no version line in crates/baley/Cargo.toml at $QUALIFIED" >&2; exit 1; }

# install.sh spells the platform folder this way.
case $(uname -s) in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *) echo "unsupported operating system; Linux or Darwin is required" >&2; exit 1 ;;
esac
case $(uname -m) in
  x86_64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture; x86_64 or aarch64 is required" >&2; exit 1 ;;
esac
PLATFORM="$OS-$ARCH"

ART="$HOME/.local/share/baley-install-artifacts"
# The only deletions in this script are the three label folders and artifacts.txt of the artifact folder,
# and the build folder this run creates. The artifact folder must be exactly this path, with no symbolic
# link from HOME down to it, and owned by this user.
check_art() {
  P="$HOME"
  for PART in .local share baley-install-artifacts; do
    P="$P/$PART"
    [ -L "$P" ] && { echo "refusing to use $ART: $P is a symbolic link" >&2; exit 1; }
  done
  if [ -e "$ART" ] && [ -z "$(find "$ART" -prune -user "$(id -u)")" ]; then
    echo "refusing to use $ART: it is not owned by $(id -un)" >&2
    exit 1
  fi
}
check_art

LABELS="older base newer"
CREATED=""
cleanup() {
  if [ -n "$CREATED" ]; then
    for LABEL in $LABELS; do
      [ -e "$BUILD/$LABEL" ] && git -C "$REPO" worktree remove --force "$BUILD/$LABEL" >/dev/null 2>&1
    done
    git -C "$REPO" worktree prune >/dev/null 2>&1
    rm -rf "$BUILD"
  fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# The marker sentence changes the bal-capture stub's text, so the install rerun meets a stub it has to upgrade.
MARKER="This stub was changed for the install rerun check."

mkdir "$BUILD" || exit 1
CREATED=yes
umask 022
mkdir -p "$ART" || exit 1
check_art
# A cleanup that fails can leave a link behind that the copies and the manifest write below would follow
# out of the folder, so a failed cleanup stops the script before any write.
for LABEL in $LABELS; do
  rm -rf "${ART:?}/$LABEL" || { echo "could not clear $ART/$LABEL, so nothing was written" >&2; exit 1; }
done
rm -f "$ART/artifacts.txt" || { echo "could not clear $ART/artifacts.txt, so nothing was written" >&2; exit 1; }

# set_version <worktree> <version>: the first version line of the baley crate, the one in [package].
set_version() {
  F="$1/crates/baley/Cargo.toml"
  awk -v v="$2" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' "$F" > "$F.new" \
    && mv "$F.new" "$F" || return 1
  grep -q "^version = \"$2\"\$" "$F"
}

# edit_newer <worktree>: the two local edits of the newer artifact, and nothing else.
edit_newer() {
  TABLE="$1/crates/baley/src/help/table.rs"
  STUBS="$1/crates/baley/src/host_artifacts/stubs.rs"
  OLD='description: "Record a note, or a story candidate for the backlog, in the ledger."'
  NEW="description: \"Record a note, or a story candidate for the backlog, in the ledger. $MARKER\""
  awk -v old="$OLD" -v new="$NEW" '
    index($0, old) { i = index($0, old); $0 = substr($0, 1, i - 1) new substr($0, i + length(old)); n++ }
    { print }
    END { if (n != 1) exit 1 }
  ' "$TABLE" > "$TABLE.new" && mv "$TABLE.new" "$TABLE" || { echo "the bal-capture row of the help table moved, so the marker sentence was not added" >&2; return 1; }
  awk '
    { print }
    !done && index($0, "Lookup::Served { .. }))") { print "        .filter(|command| command.name != \"bal-help\")"; done = 1 }
    END { if (!done) exit 1 }
  ' "$STUBS" > "$STUBS.new" && mv "$STUBS.new" "$STUBS" || { echo "front_doors moved, so bal-help was not dropped from the manifest" >&2; return 1; }
}

NOTES="$BUILD/notes"
mkdir "$NOTES" || exit 1

# build_one <label> <commit> <version or ""> <locked or "">: one worktree, one target folder, one binary
# laid out for publishing. The target folder goes as soon as the binary is copied out, since a release
# target is large and the build disk is small.
build_one() {
  LABEL=$1; SRC=$2; VERSION=$3; LOCKED=$4
  W="$BUILD/$LABEL"
  T="$BUILD/target-$LABEL"
  echo "== building $LABEL from $SRC"
  git -C "$REPO" worktree add --detach "$W" "$SRC" >/dev/null || { echo "could not add a worktree for $LABEL" >&2; return 1; }
  if [ -n "$VERSION" ]; then
    set_version "$W" "$VERSION" || { echo "could not set the version of $LABEL to $VERSION" >&2; return 1; }
  else
    edit_newer "$W" || return 1
  fi
  ( cd "$W" && CARGO_TARGET_DIR="$T" cargo build --release -p baley -j 6 ${LOCKED:+--locked} ) \
    || { echo "the build of $LABEL failed" >&2; return 1; }
  BIN="$T/release/baley"
  WANT_VERSION=${VERSION:-$QUALIFIED_VERSION}
  GOT=$("$BIN" --version) || return 1
  [ "$GOT" = "baley $WANT_VERSION" ] || { echo "$LABEL prints '$GOT', not 'baley $WANT_VERSION'" >&2; return 1; }
  DIR="$ART/$LABEL/$PLATFORM"
  mkdir -p "$DIR" || return 1
  cp "$BIN" "$DIR/baley" && chmod 755 "$DIR/baley" || return 1
  DIGEST=$(sum256 "$DIR/baley")
  printf 'version %s\nsha256 %s\n' "$WANT_VERSION" "$DIGEST" > "$DIR/manifest" || return 1
  # What changed in the worktree, taken before the worktree goes. Cargo.lock is part of it when the version moved.
  git -C "$W" diff > "$NOTES/$LABEL.diff"
  git -C "$W" diff --name-only > "$NOTES/$LABEL.files"
  printf '%s %s %s %s\n' "$LABEL" "$WANT_VERSION" "$DIGEST" "$SRC" > "$NOTES/$LABEL.line"
  rm -rf "$T"
  git -C "$REPO" worktree remove --force "$W" || return 1
}

# The two lowered builds change the baley line of Cargo.lock, so they cannot be --locked. The newer build
# touches neither Cargo.toml nor Cargo.lock and stays --locked.
build_one older "$OLDER_COMMIT" 0.0.1 "" || exit 1
build_one base "$QUALIFIED" 0.0.2 "" || exit 1
build_one newer "$QUALIFIED" "" locked || exit 1

{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "platform: $PLATFORM"
  echo "commit to qualify: $QUALIFIED"
  echo "older source commit: $OLDER_COMMIT"
  echo "marker sentence added to the bal-capture stub of newer: $MARKER"
  echo "bal-help is left out of newer's manifest by dropping it from front_doors in crates/baley/src/host_artifacts/stubs.rs"
  for LABEL in $LABELS; do
    read -r NAME VERSION DIGEST SRC < "$NOTES/$LABEL.line"
    echo "artifact $NAME: version $VERSION sha256 $DIGEST commit $SRC edits $(tr '\n' ' ' < "$NOTES/$LABEL.files")"
  done
  for LABEL in base newer; do
    echo "begin manifest $LABEL"
    "$ART/$LABEL/$PLATFORM/baley" artifact manifest
    echo "end manifest $LABEL"
    "$ART/$LABEL/$PLATFORM/baley" artifact manifest | jq -r --arg label "$LABEL" '.[] | "stub \($label) \(.identity) \(.digest)"'
  done
  for LABEL in $LABELS; do
    echo "begin edits $LABEL"
    cat "$NOTES/$LABEL.diff"
    echo "end edits $LABEL"
  done
} > "$ART/artifacts.txt" || exit 1

echo
echo "Built at $QUALIFIED, left unpublished. Push each tree to a throwaway branch of this repository for this run only, as the procedure's Part 0 says:"
for LABEL in $LABELS; do
  echo "  $ART/$LABEL/$PLATFORM/  (manifest and baley)  ->  <branch>:$LABEL/$PLATFORM/"
done
echo "The sources the procedure uses are https://raw.githubusercontent.com/<owner>/<repository>/<branch>/older, /base and /newer."
echo "The versions, digests, commits and local edits are in $ART/artifacts.txt."
