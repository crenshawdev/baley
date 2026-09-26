#!/usr/bin/env bash
#
# Structurizr for this C4 model, on Java alone (no Docker).
#
# Usage:  ./run.sh view                 # interactive viewer on http://localhost:8080
#         ./run.sh validate             # check the model parses (use in CI)
#         ./run.sh export [md files...] # export every view to Mermaid and refresh the
#                                       # marked diagram blocks in the given Markdown files
#
# Needs java on PATH. The Structurizr application (structurizr.war) is downloaded once to
# ${XDG_CACHE_HOME:-~/.cache}/baley-design/, or taken from $STRUCTURIZR_WAR.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL="${DIR}/workspace.dsl"
OUT="${DIR}/diagrams"
WAR="${STRUCTURIZR_WAR:-${XDG_CACHE_HOME:-$HOME/.cache}/baley-design/structurizr.war}"

need_war() {
  command -v java >/dev/null || { echo "java is not on PATH." >&2; exit 1; }
  if [ ! -f "${WAR}" ]; then
    mkdir -p "$(dirname "${WAR}")"
    echo "Downloading structurizr.war to ${WAR} ..."
    curl -sSfL -o "${WAR}" https://download.structurizr.com/structurizr.war
  fi
}

# The Mermaid exporter hard-codes white fills for the diagram and boundary boxes; clear them
# so the diagrams follow the viewer's light or dark background.
clear_white_fills() {
  sed -i \
    -e 's/^\(\s*style diagram \)fill:#ffffff,stroke:#ffffff/\1fill:none,stroke:none/' \
    -e 's/^\(\s*style [A-Za-z0-9_]* \)fill:#ffffff,/\1fill:none,/' \
    -e 's/^\(\s*linkStyle default \)fill:#ffffff/\1fill:none/' \
    "$1"
}

# Replace everything between <!-- c4:KEY --> and <!-- /c4:KEY --> with the fenced Mermaid
# for view KEY (the view key in workspace.dsl).
refresh_blocks() {
  local md="$1" tmp
  tmp="$(mktemp)"
  awk -v out="${OUT}" '
    match($0, /<!-- c4:[A-Za-z0-9_-]+ -->/) {
      print
      key = substr($0, RSTART + 8, RLENGTH - 12)
      file = out "/structurizr-" key ".mmd"
      if ((getline line < file) <= 0) { print "c4: no exported view \"" key "\"" > "/dev/stderr"; exit 2 }
      print "```mermaid"; print line
      while ((getline line < file) > 0) print line
      close(file); print "```"
      skipping = key; next
    }
    skipping != "" && $0 ~ ("<!-- /c4:" skipping " -->") { skipping = ""; print; next }
    skipping == "" { print }
  ' "${md}" > "${tmp}"
  mv "${tmp}" "${md}"
}

case "${1:-}" in
  view)
    need_war
    exec java -jar "${WAR}" local "${DIR}"
    ;;
  validate)
    need_war
    java -jar "${WAR}" validate -workspace "${MODEL}"
    ;;
  export)
    shift
    need_war
    rm -rf "${OUT}"
    java -jar "${WAR}" export -workspace "${MODEL}" -format mermaid -output "${OUT}"
    for f in "${OUT}"/*.mmd; do clear_white_fills "$f"; done
    for md in "$@"; do refresh_blocks "$md"; done
    ;;
  *)
    sed -n '4,11p' "$0"
    exit 1
    ;;
esac
