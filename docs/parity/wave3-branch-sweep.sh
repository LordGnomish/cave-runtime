#!/usr/bin/env bash
# Branch sweep — categorize 276 branches: dormant+merged → safe-to-delete.
# Analysis-only by default. Pass --apply to actually delete.
set -uo pipefail

REPO="$HOME/Code/cave-runtime"
NOW=$(date +%s)
THIRTY_DAYS=$((30 * 86400))
APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

mkdir -p /tmp/sweep
OUT=/tmp/sweep/branch_audit.tsv
SAFE=/tmp/sweep/branch_safe_to_delete.tsv
> "$OUT"; > "$SAFE"

# Branches that have a worktree checked out — never touch.
WT_BRANCHES=$(git -C "$REPO" worktree list --porcelain | awk '/^branch /{sub("refs/heads/", "", $2); print $2}')

# Walk every local branch
git -C "$REPO" for-each-ref --format='%(refname:short)|%(committerdate:unix)|%(committerdate:short)|%(authorname)' refs/heads/ \
| while IFS='|' read -r br cdate cdate_human author; do
  age=$((NOW - cdate))
  has_wt="no"
  if echo "$WT_BRANCHES" | grep -qx "$br"; then has_wt="yes"; fi

  # Merged into main?
  merged="no"
  if git -C "$REPO" merge-base --is-ancestor "$br" main 2>/dev/null; then merged="yes"; fi

  # Categorize
  cat=""
  if [ "$br" = "main" ]; then
    cat="main"
  elif [ "$has_wt" = "yes" ]; then
    cat="active-worktree"
  elif [ "$merged" = "yes" ] && [ "$age" -gt "$THIRTY_DAYS" ]; then
    cat="dormant-merged"
    echo "$br|$cdate_human|$author" >> "$SAFE"
  elif [ "$merged" = "yes" ]; then
    cat="merged-recent"
  elif [ "$age" -gt "$THIRTY_DAYS" ]; then
    cat="dormant-unmerged"
  else
    cat="active-unmerged"
  fi

  printf '%s\t%s\t%s\t%s\t%s\n' "$br" "$cdate_human" "$cat" "$has_wt" "$author" >> "$OUT"
done

echo "=== Branch audit summary ==="
cut -f3 "$OUT" | sort | uniq -c | sort -rn
echo ""
echo "=== Safe-to-delete (dormant + merged + no worktree): $(wc -l < "$SAFE") branches ==="
head -20 "$SAFE"
echo ""
echo "Full report: $OUT"
echo "Safe-to-delete list: $SAFE"

if [ "$APPLY" = "1" ]; then
  echo ""
  echo "=== APPLY: deleting safe branches ==="
  count=0
  while IFS='|' read -r br _date _author; do
    if git -C "$REPO" branch -D "$br" 2>/dev/null; then
      count=$((count + 1))
    fi
  done < "$SAFE"
  echo "Deleted $count branches."
fi
