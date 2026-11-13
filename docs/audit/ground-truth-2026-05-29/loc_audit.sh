#!/usr/bin/env bash
# Ground-truth LOC ratio audit (bash 3.2 compatible)
cd /Users/gnomish/Code/cave-runtime/.claude/worktrees/ground-truth-audit-1780247099 || exit 1
IDX="docs/parity/parity-index.json"
DATA="/tmp/audit-rows.tsv"
: > "$DATA"

# ---- Step A: clone unique upstream repos in parallel (cap 10, 60s timeout) ----
REPOS=$(jq -r '.crates | to_entries[] | .value.upstream // empty' "$IDX" | sort -u)
echo "[loc_audit] unique upstream repos: $(echo "$REPOS" | grep -c .)"

active=0
for repo in $REPOS; do
  name=$(basename "$repo")
  snap="/tmp/${name}-baseline"
  if [ -d "$snap" ]; then continue; fi
  (
    perl -e 'alarm shift; exec @ARGV' 90 git clone --depth 1 "https://github.com/$repo" "$snap" >/dev/null 2>&1 || rm -rf "$snap"
  ) &
  active=$((active+1))
  if [ "$active" -ge 10 ]; then wait; active=0; fi
done
wait
echo "[loc_audit] clones done. cached baselines: $(ls -d /tmp/*-baseline 2>/dev/null | wc -l | tr -d ' ')"

# ---- Step B: per-crate LOC ----
jq -r '.crates | to_entries[] | "\(.key)\t\(.value.crate_dir)\t\(.value.upstream // "null")\t\(.value.honest_ratio // 0)\t\(.value.fill_ratio // 0)"' "$IDX" | \
while IFS=$'\t' read -r CRATE CRATE_DIR UPSTREAM HONEST FILL; do
  if [ -d "$CRATE_DIR/src" ]; then
    CAVE_LOC=$(find "$CRATE_DIR/src" -name "*.rs" -not -name "*test*" -exec cat {} \; 2>/dev/null | wc -l | tr -d ' ')
  else
    CAVE_LOC=0
  fi
  UPSTREAM_LOC=0
  if [ -n "$UPSTREAM" ] && [ "$UPSTREAM" != "null" ]; then
    NAME=$(basename "$UPSTREAM")
    SNAP="/tmp/${NAME}-baseline"
    if [ -d "$SNAP" ]; then
      UPSTREAM_LOC=$(find "$SNAP" \( -name "*.go" -o -name "*.py" -o -name "*.ts" -o -name "*.tsx" -o -name "*.js" -o -name "*.java" -o -name "*.cpp" -o -name "*.cc" -o -name "*.c" -o -name "*.h" -o -name "*.rs" -o -name "*.scala" -o -name "*.kt" \) -not -path "*/vendor/*" -not -path "*/node_modules/*" -not -path "*/.git/*" -not -name "*_test.go" -not -name "*.test.ts" -not -name "*.test.js" -not -name "*_test.py" -not -name "*.spec.ts" -exec cat {} \; 2>/dev/null | wc -l | tr -d ' ')
    fi
  fi
  if [ "${UPSTREAM_LOC:-0}" -gt 0 ] 2>/dev/null; then
    LOC_RATIO=$(python3 -c "print(round($CAVE_LOC/$UPSTREAM_LOC, 4))")
  else
    LOC_RATIO="N/A"
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$CRATE" "$CAVE_LOC" "$UPSTREAM_LOC" "$LOC_RATIO" "$HONEST" "$FILL" "$CRATE_DIR" >> "$DATA"
done

echo "[loc_audit] rows: $(wc -l < "$DATA" | tr -d ' ')"
echo "[loc_audit] DONE"
