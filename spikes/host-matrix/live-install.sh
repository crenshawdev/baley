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

# The observation sheet. Every row has an id that never changes, so the procedure, the sheet and
# live-install-reads.sh can name it. Marks are pass, fail or unavailable, or observed for a row that
# records without an expectation. Each row's expected text names the design text it comes from.
SHEET="$OUT/observations.md"
PINS_COMMIT=$(sed -n 's/^checkout-commit: //p' "$OUT/pins.txt" | head -n 1)
PINS_PLATFORM=$(sed -n 's/^platform: //p' "$OUT/pins.txt" | head -n 1)
art_line() {
  sed -n "s/^artifact $1: //p" "$ARTIFACTS" 2>/dev/null | head -n 1
}
OLDER_V=0.0.1
BASE_V=0.0.2
NEWER_V=$(art_line newer | sed -n 's/^version \([^ ]*\) .*/\1/p')
[ -n "$NEWER_V" ] || NEWER_V="the newer version"
# A request id is 17170000-0000-4000-8000-0000000000NN, written below by its last two digits.
RID() {
  printf '17170000-0000-4000-8000-0000000000%s' "$1"
}
SETTINGS="$CC/settings.json"
REGISTRY="$CC/.claude.json"

# row ID STEP EXPECTED [MARK]: one table row, with the columns the owner fills left empty. A cell never
# holds a pipe character, because the table would split there.
row() {
  printf '| %s | %s | %s | %s |  |  |  |\n' "$1" "$2" "$3" "${4:-}"
}
section() {
  printf '\n## %s\n\n%s\n\n| id | step | expected | mark | actual outcome | evidence | class |\n|---|---|---|---|---|---|---|\n' "$1" "$2"
}

# The seven rows for one protected place: Bash read, Bash write, then the Read, Write, Edit, Grep and Glob
# tools. $1 names the place, $2 is the shape of its protection: full (read and write refused) or write-only
# (writes refused, reads allowed).
SANDBOX_TEXT="design 0012 section 5, Sandbox and provider access"
protect_rows() {
  KEY=$1; MODE=$2; D=$3
  case "$KEY" in
    stable)
      S_BR="Bash read: $STABLE --version, which runs the staged binary through the link"
      S_BW="Bash write: echo x >> $STABLE (host question 2)"
      S_RD="Read tool: $STABLE"
      S_WR="Write tool: replace $STABLE with one line (host question 2)"
      S_ED="Edit tool: change 'a' to 'b' in $STABLE (host question 2)"
      S_GR="Grep tool (search-tools session): pattern baley, path $STABLE"
      S_GL="Glob tool (search-tools session): pattern baley, path $DH/.local/bin"
      ;;
    versions)
      S_BR="Bash read: cat $D/seed.txt"
      S_BW="Bash write: echo bash > $D/agent-bash.txt"
      S_RD="Read tool: $D/seed.txt"
      S_WR="Write tool: create $D/agent-write.txt"
      S_ED="Edit tool: change 'FAKE_KEY' to 'EDITED_KEY' in $D/seed.txt"
      S_GR="Grep tool (search-tools session): pattern FAKE_KEY, path $D"
      S_GL="Glob tool (search-tools session): pattern *, path $D"
      ;;
    *)
      S_BR="Bash read: cat $D/seed.txt"
      S_BW="Bash write: echo bash > $D/agent-bash.txt"
      S_RD="Read tool: $D/seed.txt"
      S_WR="Write tool: create $D/agent-write.txt"
      S_ED="Edit tool: change 'FAKE_KEY' to 'EDITED_KEY' in $D/seed.txt"
      S_GR="Grep tool (search-tools session): pattern FAKE_KEY, path $D"
      S_GL="Glob tool (search-tools session): pattern *, path $D"
      ;;
  esac
  if [ "$MODE" = full ]; then
    row "prot.$KEY.bash-read" "$S_BR" "Refused by the sandbox (denyRead) and the file appears missing ($SANDBOX_TEXT, and design 0010 GRD-R13)"
    row "prot.$KEY.bash-write" "$S_BW" "Refused by the sandbox (denyWrite) and nothing lands in the folder ($SANDBOX_TEXT, and design 0010 GRD-R13)"
    row "prot.$KEY.read" "$S_RD" "Refused by the Read deny rule (design 0010 GRD-R13)"
    row "prot.$KEY.write" "$S_WR" "Refused by the Edit deny rule (design 0010 GRD-R13)"
    row "prot.$KEY.edit" "$S_ED" "Refused by the Edit deny rule (design 0010 GRD-R13)"
    row "prot.$KEY.grep" "$S_GR" "Refused by the guard, which denies a Grep whose target lies inside the folder or holds it. Claude Code applies the Read rule to Grep only on a best-effort basis (design 0010 GRD-R13)"
    row "prot.$KEY.glob" "$S_GL" "Refused by the guard, which denies a Glob whose target lies inside the folder or holds it. Claude Code applies the Read rule to Glob only on a best-effort basis (design 0010 GRD-R13)"
  else
    case "$KEY" in
      stable) WHAT="the stable path links into the versions folder, which has no denyRead entry"; THROUGH=" Host question 2: the staged file behind the link is not changed" ;;
      *) WHAT="the proposal gives the versions folder no denyRead entry and no Read rule, so sandboxed commands can run the binaries inside it"; THROUGH="" ;;
    esac
    row "prot.$KEY.bash-read" "$S_BR" "Allowed: $WHAT ($SANDBOX_TEXT)"
    row "prot.$KEY.bash-write" "$S_BW" "Refused by the sandbox (denyWrite names the folder and the stable path) and nothing lands ($SANDBOX_TEXT).$THROUGH"
    row "prot.$KEY.read" "$S_RD" "Allowed: no Read rule covers it ($SANDBOX_TEXT)"
    row "prot.$KEY.write" "$S_WR" "Refused by the Edit rule for the folder or the stable path ($SANDBOX_TEXT).$THROUGH"
    row "prot.$KEY.edit" "$S_ED" "Refused by the Edit rule for the folder or the stable path ($SANDBOX_TEXT).$THROUGH"
    row "prot.$KEY.grep" "$S_GR" "Allowed: the guard refuses a Grep only for Baley's home and config folders (design 0010 GRD-R13)"
    row "prot.$KEY.glob" "$S_GL" "Allowed: the guard refuses a Glob only for Baley's home and config folders (design 0010 GRD-R13)"
  fi
}

write_sheet() {
  cat <<EOF
# Installed qualification observations

Claude Code version (claude --version):
Platform: $PINS_PLATFORM
Date of the run:
Baley commit (pins.txt): $PINS_COMMIT
Artifact older (artifacts.txt): $(art_line older)
Artifact base (artifacts.txt): $(art_line base)
Artifact newer (artifacts.txt): $(art_line newer)
Artifact source, base address (https://<owner domain>/<path>):

The procedure is $GUIDE. Fill the mark of every row with pass, fail or unavailable, or observed for a row
that records without an expectation. Actual outcome is what a file, the ledger or a receipt showed
(ls -l, cat, jq, sqlite3 with -readonly, the installer's or baley's printed lines), never only what the model
reported. Evidence is a results file and line, or the pasted output. Class stays blank for the record.
Approve every permission prompt unless a part says otherwise, and note on the row that a prompt appeared.
A host question in a step is one of the eight in the procedure's last section.
EOF
  section "Before the run" "The artifact source, the version of Claude Code and the login into the isolated configuration."
  row pre.source-older "curl -q --proto '=https' --tlsv1.2 -fsSL <older source>/<os>-<arch>/manifest (host question 8)" "Two lines, version $OLDER_V and a 64-digit sha256, each ending in a newline, served over a publicly trusted certificate (design 0012 section 5, Installer and baley install). Host question 8: a domain the owner controls can serve the artifacts"
  row pre.source-base "The same request for <base source> (host question 8)" "Two lines, version $BASE_V and a 64-digit sha256 (design 0012 section 5, Installer and baley install). Host question 8"
  row pre.source-newer "The same request for <newer source> (host question 8)" "Two lines, version $NEWER_V and a 64-digit sha256 (design 0012 section 5, Installer and baley install). Host question 8"
  row pre.claude-version "claude --version, before any session" "Prints a version; recorded in the header" observed
  row pre.login "The login into the isolated configuration, typed by the owner, then test -f $CC/.credentials.json (presence only)" "The file is present, and a session started in the isolated configuration shows no login screen afterwards"

  section "Install" "The development install script into the disposable HOME, and what a session sees before and after it."
  row inst.before.server "Session before-install, started before the install: ask which MCP servers it has, and whether a skill named bal-capture or bal-help is listed" "No server named baley and no Baley stub: nothing is installed yet (design 0012 section 5, Installer and baley install)"
  row inst.script.receipt "sh install.sh <base source>, with the prefix: the installer's own lines" "baley: placed version $BASE_V at $STABLE, then baley: versions present in $VERSIONS: with $BASE_V listed (design 0012 section 5, Installer and baley install)"
  row inst.script.install-receipt "The receipt baley install prints as the installer's last step" "install outcome: complete, baley $BASE_V at $STABLE, one line per artifact (bal-capture, bal-help, the registration, the hook, the settings) and no gap line (design 0012 section 5, Receipt)"
  row inst.script.exit "The installer's exit status, from echo \$? right after it" "0: the installer ends with baley install's status, and 0 is complete (design 0012 section 5, Installer and baley install)"
  row inst.register.stored "jq .mcpServers.baley $REGISTRY (host question 3)" "command $STABLE and args [\"serve\"], with no alwaysLoad, env or type key (design 0012 section 5, Where each artifact goes). Host question 3: claude mcp add-json --scope user honours CLAUDE_CONFIG_DIR, so this file holds the entry"
  row inst.register.real-unchanged "The owner's real ~/.claude.json: the digest of its mcpServers.baley entry, against real-claude-registration-sha256 in pins.txt (host question 3)" "Equal: the registration went into the isolated folder and not into the owner's file. Host question 3"
  row inst.settings.written "jq on $SETTINGS: the PreToolUse items, permissions.deny and sandbox" "A Baley PreToolUse item with the nine-tool matcher and a 10-second timeout whose command runs the single-quoted stable path and guard, Baley's Read and Edit deny rules, and its sandbox block with denyRead and denyWrite for the folders (design 0012 section 5, Where each artifact goes and Sandbox and provider access)"
  row inst.stubs.written "$CC/skills/bal-capture/SKILL.md and $CC/skills/bal-help/SKILL.md against baley artifact stub <identity> from the base binary" "Both exist and equal the base binary's stub bytes (design 0012 section 5, Where each artifact goes)"
  row inst.old-session.server "Session before-install, after the install: ask again which MCP servers it has" "Still no baley server: a new Claude Code session loads the wiring (design 0012 section 5, Installer and baley install)"
  row inst.old-session.stub "Session before-install, after the install: ask again which skills it lists (host question 7)" "Record whether bal-capture and bal-help appear. Host question 7 asks whether Claude Code finds new stubs only in new sessions" observed
  row inst.init "baley init in project one, through the stable path" "Exit status 0 and a baley.toml in project one (baley init --help: it writes baley.toml and records project.initialized)"
  row inst.after.mcp "Session after-install: /mcp" "baley is listed from the user scope with its three tools baley_version, baley_query and baley_apply (design 0012 section 5, baley_version, baley_query and baley_apply)"
  row inst.after.version "Session after-install: call baley_version" "The answer names version $BASE_V (design 0012 section 5, baley_version)"
  row inst.after.hook "Session after-install: ask for Bash git push origin main, and answer no at the prompt" "The installed hook answers: the guard asks (design 0010 GRD-R4), and the user ledger holds a guard.answered with outcome ask for tool Bash whose caller is a hook"
  row inst.after.stubs "Session after-install: ask which skills it lists (host question 7)" "bal-capture and bal-help are listed: a new session loads the stubs (design 0012 section 5, Installer and baley install). Host question 7"
  row inst.doctor "baley doctor through the stable path, with its exit status" "A stable path line naming staged version $BASE_V, the install record read, every installed location writable and no finding; exit status 0 (design 0012 section 5, baley doctor)"

  section "Protection" "Four protected places and seven tools each, then the stable path's staged file, a bypass-permissions session and the two network requests. Run in the order of the procedure's Part 2."
  protect_rows home full "$BHOME"
  protect_rows config full "$BCONF"
  protect_rows versions write-only "$VERSIONS"
  protect_rows stable write-only "$STABLE"
  row prot.stable.staged-intact "sha256sum $VERSIONS/$BASE_V/baley after the rows above, against the sha256 line of the base manifest (host question 2)" "Equal: nothing wrote the staged binary through the link (design 0012 section 5, Sandbox and provider access). Host question 2"
  row prot.versions.no-stray "ls -l $VERSIONS after the rows above" "No agent-bash.txt or agent-write.txt and no change to seed.txt: nothing landed (design 0012 section 5, Sandbox and provider access)"
  row prot.bypass.read-home "Session bypass (bypassPermissions): Read tool on $BHOME/seed.txt (host question 1)" "Refused by the Read deny rule, which still applies in this mode (design 0010 GRD-R13). Host question 1"
  row prot.bypass.bash-write-home "Session bypass: Bash write: echo bash > $BHOME/agent-bypass.txt (host question 1)" "Refused by the sandbox, which still applies in this mode (design 0010 GRD-R13). Host question 1"
  row prot.net.user-listed "Session protect: Bash curl -sSI https://example.com, a host the user settings file lists" "Record whether a prompt appeared and whether the request went through" observed
  row prot.net.unlisted "Session protect: Bash curl -sSI https://example.org, a host no settings file lists, while the user file lists one owner host (host question 4)" "Record whether the request is cut off with no prompt, asks, or goes through. Design 0012 section 5 (Sandbox and provider access) says install keeps every host the file holds and adds none; host question 4 asks whether a present list only pre-approves its hosts" observed
  row prot.net.project-owner-host "Session network-project, with project one's own .claude/settings.json listing www.iana.org: Bash curl -sSI https://example.com, the owner host the user file lists (host question 5)" "Record whether a prompt appeared. Host question 5 asks whether a project's list adds to the user file's list" observed
  row prot.net.project-own-host "Session network-project: Bash curl -sSI https://www.iana.org, the host only project one's file lists (host question 5)" "Record whether a prompt appeared. Host question 5" observed

  section "Opt-in update" "updates.auto on, a new session, and a newer version staged without the session waiting."
  row optin.set "baley config set --global updates.source=<newer source>, then baley config set --global updates.auto=true, then baley config show" "Both commands exit 0 and config show lists updates.source and updates.auto true as global settings (design 0012 section 5, baley update)"
  row optin.session "Session opt-in, started after both settings: call baley_version" "The answer names version $BASE_V: the server started on the version the stable path ran then, and activation does not touch a running server (design 0012 section 5, baley update)"
  row optin.staged "The user ledger's update.checked events and the stable link, after the session started" "One update.checked with actor baley, outcome staged and active_version and staged_version $NEWER_V, and the stable link now targets $VERSIONS/$NEWER_V/baley (design 0012 section 6, Update checks)"
  row optin.no-wait "Session opt-in's debug log: the time of the baley connection and of its first answer, against the update.checked recorded_at and the staged binary's modification time" "The connection and the first answer are logged before the staged binary's modification time, so the session did not wait for the download (design 0012 section 5, baley update: the server starts a detached process without a wait)"

  section "Manual update" "updates.auto off, the base version back behind the stable path, and baley update while a session runs."
  row man.setting-off "baley config set --global updates.auto=false" "Exit 0 (design 0012 section 5, baley update)"
  row man.relink "sh install.sh <base source>" "The installer places $BASE_V again, prints versions present listing $BASE_V and $NEWER_V, and the stable link targets $VERSIONS/$BASE_V/baley (design 0012 section 5, Installer and baley install: it moves the link and removes no version)"
  row man.s1.version "Session manual-s1: call baley_version" "The answer names version $BASE_V"
  row man.update.receipt "baley update while manual-s1 runs, with its exit status" "update outcome: staged, staged version: $NEWER_V, $NEWER_V is now the active version at $STABLE, new Claude Code sessions start $NEWER_V; running sessions keep the version they started with, the catalog seed line and a kept versions line; exit status 0 (design 0012 section 5, baley update, Receipt)"
  row man.s1.keeps "Session manual-s1 again: call baley_version" "Still $BASE_V: a running session keeps the version it started with (design 0012 section 5, baley update)"
  row man.s2.new "Session manual-s2, started after the update: call baley_version" "The answer names version $NEWER_V (design 0012 section 5, baley update)"
  row man.ledger "The user ledger's update.checked events" "A second update.checked, with actor owner and outcome staged, after the opt-in one with actor baley. A manual check never refuses a day already claimed (design 0012 section 6, Daily claim)"

  section "Install rerun" "baley install through the stable path at the newer version. The newer artifact changes the bal-capture stub's text and drops bal-help from its manifest."
  row rerun.doctor-stale "Before the rerun, baley doctor through the stable path at $NEWER_V, with its exit status" "A line naming bal-help as stale with baley install as its fix, and exit status 1 (design 0012 section 5, baley doctor, and HST-R12: the doctor checks the stubs against the version they serve)"
  row rerun.doctor-capture "The same doctor output: its line for bal-capture" "Recorded as seen: the placed stub carries the base text and the newer manifest differs (design 0012 section 5, baley doctor, which compares each placed stub with the manifest)" observed
  row rerun.stub-sums-before "sha256sum of $CC/skills/bal-capture/SKILL.md and $CC/skills/bal-help/SKILL.md, before the rerun" "The base manifest's digests for both" observed
  row rerun.receipt "baley install through the stable path, with its exit status" "install outcome: complete and exit status 0, with the bal-capture line reading replaced and the hook, the registration and the settings lines reading unchanged (design 0012 section 5, Receipt, and Ownership: a stub whose SHA-256 is the hash the latest record holds is Baley's and is replaced)"
  row rerun.record "The latest install.recorded in the user ledger" "binary_version $NEWER_V and complete true (design 0012 section 6, install.recorded)"
  row rerun.capture-upgraded "sha256sum of $CC/skills/bal-capture/SKILL.md after the rerun, against the bal-capture digest of newer's manifest in artifacts.txt" "Changed from the digest taken before the rerun and equal to newer's manifest digest (design 0012 section 5, Ownership)"
  row rerun.dropped-stub-file "Whether $CC/skills/bal-help/SKILL.md is still on disk after baley install" "Recorded as seen: the design states nothing about removing a stub file the manifest no longer ships" observed
  row rerun.one-hook "The number of Baley PreToolUse items in $SETTINGS" "Exactly one (design 0012 section 5, Installer and baley install: running install again adds only entries that are absent)"
  row rerun.one-registration "jq on $REGISTRY: the keys of mcpServers" "One baley entry, equal to the one stored before (design 0012 section 5, Installer and baley install)"
  row rerun.owner-key "The seeded owner key cleanupPeriodDays in $SETTINGS" "Kept with its value 30 (design 0012 section 5, Installer and baley install: existing owner settings are preserved)"
  row rerun.owner-deny "The seeded owner permissions.deny rule over $ROOT/owner-private" "Kept beside Baley's rules"
  row rerun.owner-hook "The seeded owner PreToolUse item with its own matcher" "Kept beside Baley's item"
  row rerun.owner-host "The seeded owner host example.com in sandbox.network.allowedDomains" "Kept (design 0012 section 5, Sandbox and provider access: install keeps every host the file already holds)"

  section "Older server" "A server from the T14 merge keeps answering after the stable path moves to the newer version. Project two is written by nothing newer before this part."
  row old.installer "sh install.sh <older source>" "The installer places $OLDER_V and prints its two receipt lines, then the older binary has no install command and refuses it, so the installer ends non-zero (design 0012 section 5, Installer and baley install: it ends with the status of the last command)"
  row old.init "baley init in project two through $VERSIONS/$OLDER_V/baley" "Exit status 0 and a baley.toml in project two: no binary at view set 8 has written there (design 0012 section 5, baley update)"
  row old.s3.version "Session older-s3, started in project two: call baley_version" "The answer names version $OLDER_V"
  row old.s3.capture-1 "Session older-s3: baley_apply capture with request id $(RID 21), kind note, text older first" "Recorded in project two, with the older server's baley_session in its caller (design 0012 section 6, The caller on every recorded event)"
  row old.relink "sh install.sh <newer source> while older-s3 runs" "The installer places $NEWER_V, lists versions present, moves the stable link to $VERSIONS/$NEWER_V/baley and ends with install's status 0 (design 0012 section 5, Installer and baley install)"
  row old.s3.keeps "Session older-s3, one request after another: baley_version, then baley_apply capture with request id $(RID 22), kind note, text older second, then baley_version" "Both baley_version answers name $OLDER_V and the capture is recorded: the older server keeps answering beside the newer stable path (design 0012 section 5, baley update)"
  row old.s3.same-server "The baley_session of captures $(RID 21) and $(RID 22) in project two" "Equal: one server answered both, since the caller records no server version"
  row old.s3.hook "Session older-s3: Bash git push origin main, and answer no at the prompt" "The newer hook through the stable path answers: a guard.answered with outcome ask is recorded in user after the relink, which a binary that cannot write user could not do (design 0012 section 5, baley update: the hook follows the stable path)"
  row old.user-refusal "The older binary's own guard, run by hand with a Bash git push hook call on its standard input (the procedure gives the command)" "Refused: user holds a type the older binary cannot read, so it is read-only for it (design 0012 section 5, baley update: binaries built before T15 cannot write user once it holds install.recorded)"

  section "In-flight exit" "Claude Code's process ended while Baley calls run, with the three tools pre-allowed so no prompt holds a call back."
  row exit.r1.signal "Round 1 (session inflight-1): the signal sent to Claude Code's process, its time, and the request ids sent" "Recorded as seen" observed
  row exit.r1.in-flight "Round 1: the debug log's call lines for the request ids, against the signal's time (host question 6)" "At least one call started and not completed when the signal came. Mark unavailable with the reason when none was (host question 6)"
  row exit.r1.answered "Round 1: the user and project ledgers for each request id the server read" "Each request id the server read is answered in its record (a capture.recorded, or a recorded refusal), none lost and none twice (design 0012 section 7, exit, and ADR 0034)"
  row exit.r1.server-end "Round 1: how the debug log shows the server ending (the signal Claude Code sent it, or its input closing) (host question 6)" "Recorded as seen. Host question 6 asks whether Claude Code closes a stdio server's input while a call runs when a session ends" observed
  row exit.r1.stderr "Round 1: where the server's exit checkpoint line and any drain line appear without a redirect" "Recorded as seen: the earlier live runs never ran the stderr control, so this is not yet known" observed
  row exit.r2.signal "Round 2 (session inflight-2, the other of SIGTERM and SIGKILL), run only when round 1 showed no call in flight: the signal, its time and the request ids" "Recorded as seen" observed
  row exit.r2.in-flight "Round 2: the debug log's call lines for the request ids, against the signal's time" "At least one call in flight, else unavailable with the reason (host question 6)"
  row exit.r2.answered "Round 2: the ledgers for each request id the server read" "Each request id the server read is answered in its record, none lost and none twice (design 0012 section 7, exit, and ADR 0034)"
  row exit.condition "Whether any round put a call inside the server when Claude Code's process ended (host question 6)" "Recorded as seen: when none did, the exit rows read unavailable and the reason is written here" observed

  section "Post-run" "After every session has exited: sh live-install-reads.sh > $OUT/reads.txt."
  row post.verify-user "baley verify --local-only user" "Exit status 0"
  row post.views-user "baley verify --views user" "Exit status 0"
  row post.verify-one "baley verify --local-only for project one" "Exit status 0"
  row post.views-one "baley verify --views for project one" "Exit status 0"
  row post.verify-two "baley verify --local-only for project two" "Exit status 0"
  row post.views-two "baley verify --views for project two" "Exit status 0"
  row post.doctor "baley doctor through the stable path, with its exit status" "Recorded as seen: the rerun left bal-help's file in place, and whether the record still lists it decides the exit status" observed
  row post.owner-settings "The seeded owner key, deny rule, PreToolUse item and host against results/owner-settings.json" "Each marked kept"
  row post.real-installation "The owner's real ~/.local/bin/baley, ~/.claude/settings.json, mcpServers.baley digest and the real Baley folders against pins.txt" "No difference"
  row post.real-claude "The owner's claude command and ~/.local/bin/claude link against pins.txt" "No difference, and neither resolves inside either disposable root"
}
write_sheet > "$SHEET"

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
