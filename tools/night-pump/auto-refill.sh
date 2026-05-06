#!/usr/bin/env bash
# auto-refill.sh — keep the qwen-pump queue fed with crates that still have
# real B-prime work to do (test_count gap to target_tests). Hourly LaunchAgent.
#
# Old behavior (pre-2026-05-06): "any crate with src/ and no tests/qwen_drafted.rs"
# only matched ~10 crates and they all returned DONE-mode immediately, so the
# pump produced 0 commits/h. New behavior: prefer crates whose
# parity.manifest.toml has pump_priority + a real B-prime gap. Falls back
# to legacy filter if no priority crates have a gap.
#
# Production install location:
#   ~/Library/Application Support/cave-qwen-pump/auto-refill.sh
# Repo source-of-truth:
#   tools/night-pump/auto-refill.sh
set -uo pipefail

QUEUE="$HOME/Library/Application Support/cave-qwen-pump/queue.txt"
LOG_DIR="$HOME/Library/Logs/cave-qwen-pump"
LOG="$LOG_DIR/auto-refill.log"
REPO="${QWEN_PUMP_REPO_ROOT:-/Users/gnomish/Code/cave-runtime}"
THRESHOLD=25

mkdir -p "$LOG_DIR"
ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }

DEPTH=$(wc -l < "$QUEUE" 2>/dev/null | tr -d ' ' || echo 0)
[ -z "$DEPTH" ] && DEPTH=0

if [ "$DEPTH" -ge "$THRESHOLD" ]; then
  echo "[$(ts)] queue depth $DEPTH ≥ $THRESHOLD; skip refill" >> "$LOG"
  exit 0
fi

MAIN_WT="$(git -C "$REPO" worktree list --porcelain 2>/dev/null \
  | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt; exit}')"
if [ -z "$MAIN_WT" ]; then
  echo "[$(ts)] FATAL: no worktree on main, cannot refill" >> "$LOG"
  exit 1
fi

# Read pump_priority + target_tests + current test_count for a crate.
# Returns: <priority>\t<gap>  or empty if not a candidate.
crate_state() {
  local c="$1" m="$MAIN_WT/crates/$1/parity.manifest.toml"
  [ -f "$m" ] || return 1
  [ -d "$MAIN_WT/crates/$c/src" ] || return 1

  local p
  p=$(grep -E '^[[:space:]]*pump_priority[[:space:]]*=' "$m" \
      | head -1 | sed -E 's/.*=[[:space:]]*"([A-Z]+)".*/\1/')
  [ -z "$p" ] && p="LOW"

  local target
  target=$(grep -E '^[[:space:]]*target_tests[[:space:]]*=' "$m" \
           | head -1 | sed -E 's/.*=[[:space:]]*([0-9]+).*/\1/')
  [ -z "$target" ] && target=30

  local current=0
  if [ -f "$MAIN_WT/crates/$c/tests/qwen_drafted.rs" ]; then
    current=$(grep -cE '^[[:space:]]*#\[(test|tokio::test)\]' \
              "$MAIN_WT/crates/$c/tests/qwen_drafted.rs" 2>/dev/null || echo 0)
  fi
  local gap=$((target - current))
  printf '%s\t%d\n' "$p" "$gap"
}

ADDED_HIGH=0
ADDED_MEDIUM=0
ADDED_LEGACY=0

# Pass 1: HIGH-priority crates with gap ≥ 5 (always added regardless of depth,
# unless already queued).
for c in "$MAIN_WT"/crates/*/; do
  name=$(basename "$c")
  state=$(crate_state "$name") || continue
  pri=$(printf '%s' "$state" | cut -f1)
  gap=$(printf '%s' "$state" | cut -f2)
  [ "$pri" != "HIGH" ] && continue
  [ "$gap" -lt 5 ] && continue
  grep -qx "$name" "$QUEUE" 2>/dev/null && continue
  echo "$name" >> "$QUEUE"
  ADDED_HIGH=$((ADDED_HIGH + 1))
done

# Pass 2: MEDIUM-priority crates with gap ≥ 5 (only if depth still under threshold).
NEW_DEPTH=$(wc -l < "$QUEUE" 2>/dev/null | tr -d ' ' || echo 0)
if [ "$NEW_DEPTH" -lt "$THRESHOLD" ]; then
  for c in "$MAIN_WT"/crates/*/; do
    name=$(basename "$c")
    state=$(crate_state "$name") || continue
    pri=$(printf '%s' "$state" | cut -f1)
    gap=$(printf '%s' "$state" | cut -f2)
    [ "$pri" != "MEDIUM" ] && continue
    [ "$gap" -lt 5 ] && continue
    grep -qx "$name" "$QUEUE" 2>/dev/null && continue
    echo "$name" >> "$QUEUE"
    ADDED_MEDIUM=$((ADDED_MEDIUM + 1))
  done
fi

# Pass 3 (legacy fallback): if depth STILL under threshold and we added nothing
# from priority lists, add crates without tests/qwen_drafted.rs (the old logic
# — keeps the daemon non-idle for crates whose manifest hasn't been annotated).
NEW_DEPTH=$(wc -l < "$QUEUE" 2>/dev/null | tr -d ' ' || echo 0)
if [ "$NEW_DEPTH" -lt "$THRESHOLD" ] \
   && [ $((ADDED_HIGH + ADDED_MEDIUM)) -eq 0 ]; then
  for c in "$MAIN_WT"/crates/*/; do
    name=$(basename "$c")
    if [ ! -f "$c/tests/qwen_drafted.rs" ] && [ -d "$c/src" ] \
       && ! grep -qx "$name" "$QUEUE" 2>/dev/null; then
      echo "$name" >> "$QUEUE"
      ADDED_LEGACY=$((ADDED_LEGACY + 1))
    fi
  done
fi

NEW_DEPTH=$(wc -l < "$QUEUE" 2>/dev/null | tr -d ' ' || echo 0)
echo "[$(ts)] queue depth $DEPTH → $NEW_DEPTH (+$ADDED_HIGH HIGH, +$ADDED_MEDIUM MEDIUM, +$ADDED_LEGACY legacy)" >> "$LOG"
