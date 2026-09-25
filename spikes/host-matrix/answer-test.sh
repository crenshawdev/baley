#!/bin/sh
# Which PreToolUse hook answers does Codex honour? One throwaway session per answer.
set -u
OUT="$HOME/.local/share/baley-matrix/codex-answers"
rm -rf "$OUT"; mkdir -p "$OUT"
cd "$(dirname "$0")/../../crates" || exit 1

for D in deny ask; do
  cat > "$OUT/hook-$D.sh" <<EOF
#!/bin/sh
cat >/dev/null
printf %s '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"$D","permissionDecisionReason":"matrix-$D"}}'
EOF
  chmod +x "$OUT/hook-$D.sh"
done
cat > "$OUT/hook-exit2.sh" <<'EOF'
#!/bin/sh
cat >/dev/null
echo matrix-exit2 >&2
exit 2
EOF
chmod +x "$OUT/hook-exit2.sh"

for D in deny ask exit2; do
  codex exec --skip-git-repo-check --ephemeral -s workspace-write --dangerously-bypass-hook-trust \
    -c "hooks.PreToolUse=[{matcher=\"Bash\",hooks=[{type=\"command\",command=\"$OUT/hook-$D.sh\",timeout=5}]}]" \
    -o "$OUT/report-$D.md" \
    'Run exactly this shell command once: echo probe-ran ; then report its exact output and exit code, and if the host refused to run it, quote the refusal verbatim. Do nothing else.' \
    > "$OUT/log-$D.txt" 2>&1 </dev/null
  echo "== $D"; cat "$OUT/report-$D.md"; echo
  grep "^hook:" "$OUT/log-$D.txt" | tail -1
done
