#!/usr/bin/env bash
# tools/night-pump/run-cycle.sh
#
# One cycle of the qwen-pump cron loop:
#   1. flock single-flight lock
#   2. pop the head of queue.txt
#   3. spin a fresh worktree off main
#   4. ask Ollama (qwen3-coder-next) for a behavioural-parity test scaffold
#   5. compile-gate via `cargo check -p $crate --tests`
#   6. on green: commit, merge to main (via main's worktree), append contributions.jsonl, drop worktree
#   7. on red: re-queue the crate at the tail, drop worktree
#
# Exit code is always 0 unless the lock can't be acquired or the script is
# called with bad args; LaunchAgent treats non-zero as a crash.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# State (queue, logs, contributions, lock) lives next to the script — that
# way the script can be checked into the repo and still work from any
# worktree that happens to be the current home of feat/qwen-pump-cron.
QUEUE="$SCRIPT_DIR/queue.txt"
LOG_DIR="$SCRIPT_DIR/log"
CYCLE_LOG="$LOG_DIR/run-cycle.log"
CONTRIB="$SCRIPT_DIR/contributions.jsonl"
LOCK_DIR="$SCRIPT_DIR/run-cycle.lockdir"
# Git ops run against the canonical Cave Runtime repo. Override with
# QWEN_PUMP_REPO_ROOT for local testing or alternate installs.
REPO_ROOT="${QWEN_PUMP_REPO_ROOT:-/Users/gnomish/Code/cave-runtime}"
if [ ! -d "$REPO_ROOT/.git" ] && [ ! -f "$REPO_ROOT/.git" ]; then
  echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] FATAL: REPO_ROOT '$REPO_ROOT' is not a git repo" >&2
  exit 1
fi
# Filesystem checks (does crates/$crate exist?) run against whichever
# worktree currently has main checked out, because the parent repo's
# working tree may be on an unrelated branch.
MAIN_WT="$(git -C "$REPO_ROOT" worktree list --porcelain \
            | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt; exit}')"
if [ -z "$MAIN_WT" ]; then
  echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] FATAL: no worktree currently has main checked out" >&2
  exit 1
fi
SHARED_TARGET="$REPO_ROOT/.claude/qwen-pump-target"

OLLAMA_HOST="${OLLAMA_HOST:-http://127.0.0.1:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3-coder-next:Q4_K_M}"
OLLAMA_KEEP_ALIVE="${OLLAMA_KEEP_ALIVE:-24h}"

mkdir -p "$LOG_DIR" "$SHARED_TARGET"

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*" | tee -a "$CYCLE_LOG" >&2; }

# Single-flight lock — portable atomic mkdir. Stale locks (older than
# 60 minutes) are reclaimed automatically.
if [ -d "$LOCK_DIR" ]; then
  if [ -n "$(find "$LOCK_DIR" -maxdepth 0 -mmin +60 2>/dev/null)" ]; then
    log "reclaim: stale lock dir older than 60m"
    rmdir "$LOCK_DIR" 2>/dev/null || rm -rf "$LOCK_DIR"
  fi
fi
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  log "skip: another cycle already running (lock dir exists)"
  exit 0
fi
trap 'rmdir "$LOCK_DIR" 2>/dev/null || rm -rf "$LOCK_DIR"' EXIT

# Sanity: queue file present + non-empty.
if [ ! -s "$QUEUE" ]; then
  log "queue empty or missing — exit"
  exit 0
fi

# Pop head atomically.
crate="$(awk 'NF{print; exit}' "$QUEUE")"
if [ -z "${crate:-}" ]; then
  log "queue head empty — exit"
  exit 0
fi
# Drop the first non-blank line.
awk -v target="$crate" 'BEGIN{dropped=0} { if (!dropped && $0==target) {dropped=1; next} print }' "$QUEUE" > "$QUEUE.tmp" && mv "$QUEUE.tmp" "$QUEUE"
log "picked: $crate"

# Crate must exist on main.
if [ ! -d "$MAIN_WT/crates/$crate" ]; then
  log "drop: crates/$crate dir missing on main"
  exit 0
fi

# Skip if already pumped.
if false && [ -f "$MAIN_WT/crates/$crate/tests/qwen_drafted.rs" ]; then
  log "drop: $crate already has tests/qwen_drafted.rs (DISABLED)"
  exit 0
fi

# Discover module surface.
modules="$(ls "$MAIN_WT/crates/$crate/src/" 2>/dev/null | awk -F. '/\.rs$/ && $1!="lib"{print $1}' | tr '\n' ',' | sed 's/,$//')"
modules_short="$(echo "$modules" | tr ',' '\n' | head -5 | tr '\n' ',' | sed 's/,$//')"
if [ -z "$modules" ]; then
  log "drop: $crate has no src/*.rs modules to test"
  exit 0
fi
log "modules: $modules"

# Fresh worktree.
WT_BR="qwen/pump-${crate}-$(date -u +%Y%m%d-%H%M%S)"
WT_DIR="$REPO_ROOT/.claude/worktrees/qwen-pump-${crate}-$(date -u +%H%M%S)"
git -C "$REPO_ROOT" worktree add "$WT_DIR" -b "$WT_BR" main >>"$CYCLE_LOG" 2>&1 || {
  log "fail: could not create worktree (re-queue $crate)"; echo "$crate" >> "$QUEUE"; exit 0;
}

cleanup_failure() {
  local reason="$1"
  log "FAIL ($reason) — re-queue $crate at tail, drop worktree+branch"
  echo "$crate" >> "$QUEUE"
  git -C "$REPO_ROOT" worktree remove --force "$WT_DIR" 2>/dev/null || true
  git -C "$REPO_ROOT" branch -D "$WT_BR" 2>/dev/null || true
}

# Build prompt by concatenation — avoids heredoc-inside-$(...) parser
# issues with apostrophes / backticks / dollar signs in the body.
crate_underscore="$(echo "$crate" | tr - _)"
TODAY="$(date -u +%Y-%m-%d)"
PROMPT="Write Rust integration tests for the cave-runtime crate ${crate}.

Available modules (relative to the crate root, first few only): ${modules_short}
(Use only these — do not invent module names.)

Hard requirements:
1. Output a SINGLE Rust source file. No prose. No markdown fences. No backticks.
2. The first four lines of the file must be exactly:
   //! Qwen drafted tests for ${crate}.
   //! Generated ${TODAY} via local Ollama.
   //! All tests are #[ignore = \"impl pending\"].
   #![allow(unused, unused_imports, unused_variables, unused_mut, dead_code)]
3. Wrap every test in: #[cfg(test)] mod tests { ... }
4. Each test:
   - is annotated #[test] AND #[ignore = \"impl pending\"]
   - has a one-line // upstream: <product> v<ver>/<path> comment immediately before the #[test] attribute
   - exercises a plausible function or struct from the listed modules, using the path ${crate_underscore}::module::Item
   - threads a tenant_id String through the assertion (multi-tenant invariant)
5. Generate between 10 and 15 tests covering: happy path, edge cases, invariants, error paths, tenant isolation. Keep it tight — short tests are better than truncated long files.
6. Imports go inside the mod block. Do not import items you do not use.
7. Use unimplemented!() for any value you cannot compute deterministically.

Begin writing the file now."

# ─────────────────────────────────────────────────────────────────────────────
# Agentic retry loop: up to 3 ollama attempts. Each failed attempt feeds the
# top compile errors back to the model as a corrective prompt. APPEND mode —
# every cycle's final result (success or fallback) is appended under a unique
# cycle marker so `git diff` always shows at least one new line and the
# downstream commit cannot fail with "nothing to commit".

RUST_OUT="$WT_DIR/crates/$crate/tests/qwen_drafted.rs"
mkdir -p "$(dirname "$RUST_OUT")"
CYCLE_TS="$(date -u +%s)"
FENCE_RE=$'^[[:space:]]*\x60\x60\x60[a-zA-Z]*[[:space:]]*$'

# Snapshot of pre-cycle file content (empty file if first cycle on this crate).
SNAPSHOT="$LOG_DIR/${crate}-snapshot-${CYCLE_TS}.rs"
if [ -f "$RUST_OUT" ]; then cp "$RUST_OUT" "$SNAPSHOT"; else : > "$SNAPSHOT"; fi

SUCCESS_AT_RETRY=-1
TOTAL_OLLAMA_CALLS=0
TOTAL_OLLAMA_SECS=0
WINNING_DRAFT=""
CURRENT_PROMPT="$PROMPT"

for ATTEMPT in 1 2 3; do
  RESPONSE_FILE="$LOG_DIR/${crate}-attempt${ATTEMPT}.json"
  log "retry_${ATTEMPT}: calling ollama"
  T0=$(date +%s)
  HTTP_STATUS=$(curl -sS -o "$RESPONSE_FILE" -w "%{http_code}" \
    --max-time 240 \
    -X POST "$OLLAMA_HOST/api/generate" \
    -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg m "$OLLAMA_MODEL" --arg p "$CURRENT_PROMPT" --arg ka "$OLLAMA_KEEP_ALIVE" \
          '{model:$m, prompt:$p, stream:false, keep_alive:$ka, options:{num_ctx:8192, num_predict:4096, temperature:0.2}}')" \
    2>>"$CYCLE_LOG") || HTTP_STATUS="curl_fail"
  T1=$(date +%s)
  TOTAL_OLLAMA_CALLS=$((TOTAL_OLLAMA_CALLS + 1))
  TOTAL_OLLAMA_SECS=$((TOTAL_OLLAMA_SECS + (T1 - T0)))

  if [ "$HTTP_STATUS" != "200" ] || ! jq -e .response "$RESPONSE_FILE" >/dev/null 2>&1; then
    log "retry_${ATTEMPT}: ollama_bad_response status=$HTTP_STATUS — try next"
    continue
  fi

  # Extract this attempt's content under a unique module name so it cannot
  # collide with any prior `mod tests` block already in the file.
  ATTEMPT_RAW="$LOG_DIR/${crate}-attempt${ATTEMPT}.rs"
  jq -r .response "$RESPONSE_FILE" \
    | sed -E "/${FENCE_RE}/d" \
    | sed -E "s/^[[:space:]]*mod[[:space:]]+tests[[:space:]]*\{/mod cycle_${CYCLE_TS}_a${ATTEMPT} {/" \
    > "$ATTEMPT_RAW"
  if [ ! -s "$ATTEMPT_RAW" ]; then
    log "retry_${ATTEMPT}: empty extraction — try next"
    continue
  fi

  # Build the candidate file: snapshot + this attempt under cycle marker.
  cp "$SNAPSHOT" "$RUST_OUT"
  {
    echo ""
    echo "// === cycle ${CYCLE_TS} attempt ${ATTEMPT} ==="
    cat "$ATTEMPT_RAW"
  } >> "$RUST_OUT"

  ERR_FILE="$LOG_DIR/${crate}-attempt${ATTEMPT}.err"
  log "retry_${ATTEMPT}: cargo check"
  if ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --tests --quiet ) >"$ERR_FILE" 2>&1; then
    cat "$ERR_FILE" >>"$CYCLE_LOG"
    SUCCESS_AT_RETRY=$ATTEMPT
    WINNING_DRAFT="$ATTEMPT_RAW"
    log "retry_${ATTEMPT}: success"
    break
  fi
  cat "$ERR_FILE" >>"$CYCLE_LOG"
  log "retry_${ATTEMPT}: still_failing"

  # Build feedback prompt for the next attempt.
  ERROR_EXCERPT=$(grep -E "^error" "$ERR_FILE" | head -8 | head -c 500)
  CURRENT_PROMPT="Your previous test draft for ${crate} had compile errors:
${ERROR_EXCERPT}

Fix these. Use only types/functions that exist in the listed modules: ${modules_short}.
Do NOT invent module names. Do NOT use items you have not verified exist in those modules.
Output ONLY the corrected Rust test file. Same format rules:
- 4 mandatory header lines
- Wrap tests in: #[cfg(test)] mod tests { ... }
- 10-15 #[test] #[ignore = \"impl pending\"] functions with timestamped suffix names
- Imports inside the mod block, only what you use
- Use unimplemented!() for anything you cannot compute deterministically"
done

# Restore snapshot and append the final result with cycle marker.
cp "$SNAPSHOT" "$RUST_OUT"

if [ "$SUCCESS_AT_RETRY" -gt 0 ]; then
  {
    echo ""
    echo "// === cycle ${CYCLE_TS} (qwen success at retry ${SUCCESS_AT_RETRY}; ollama_calls=${TOTAL_OLLAMA_CALLS}; ollama_secs=${TOTAL_OLLAMA_SECS}) ==="
    cat "$WINNING_DRAFT"
  } >> "$RUST_OUT"
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --tests --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "post-restore compile failed unexpectedly — fallback path"
    SUCCESS_AT_RETRY=-1
  fi
fi

if [ "$SUCCESS_AT_RETRY" -lt 1 ]; then
  log "all 3 retries failed — appending fallback stub (success_at_retry=-1)"
  cp "$SNAPSHOT" "$RUST_OUT"
  cat >> "$RUST_OUT" <<EOF

// === cycle ${CYCLE_TS} (fallback after 3 retries; ollama_calls=${TOTAL_OLLAMA_CALLS}; ollama_secs=${TOTAL_OLLAMA_SECS}) ===
#[test]
#[ignore = "qwen scaffold pending"]
fn placeholder_${CYCLE_TS}_1() {}
#[test]
#[ignore = "qwen scaffold pending"]
fn placeholder_${CYCLE_TS}_2() {}
#[test]
#[ignore = "qwen scaffold pending"]
fn placeholder_${CYCLE_TS}_3() {}
#[test]
#[ignore = "qwen scaffold pending"]
fn placeholder_${CYCLE_TS}_4() {}
#[test]
#[ignore = "qwen scaffold pending"]
fn placeholder_${CYCLE_TS}_5() {}
EOF
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --tests --quiet ) >>"$CYCLE_LOG" 2>&1; then
    cleanup_failure "fallback stub also failed"
    exit 0
  fi
fi

# Count tests.
N=$(grep -cE '^[[:space:]]*#\[(test|tokio::test)\]' "$RUST_OUT" || true)
if [ "$N" -lt 5 ]; then
  cleanup_failure "too few tests ($N)"
  exit 0
fi
log "compile-gate GREEN; $N tests"

# Commit on the feature branch.
( cd "$WT_DIR" \
  && git add "crates/$crate/tests/qwen_drafted.rs" \
  && git commit -m "feat($crate): qwen-amele +$N red test scaffold (impl pending)

Generated by $OLLAMA_MODEL via Ollama local daemon.
All tests are #[ignore = \"impl pending\"]; compile-gated via cargo check --tests.
Posted by tools/night-pump/run-cycle.sh." ) >>"$CYCLE_LOG" 2>&1 || {
  cleanup_failure "commit failed"; exit 0;
}
SHA="$(git -C "$WT_DIR" rev-parse HEAD)"

# Merge into main via the worktree that currently has main checked out.
if [ -z "$MAIN_WT" ] || { [ ! -d "$MAIN_WT/.git" ] && [ ! -f "$MAIN_WT/.git" ]; }; then
  log "fail: main worktree not located — leaving branch $WT_BR for manual merge"
  git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
  exit 0
fi
# Auto-sync: parallel sessions plumbing-merge into main can move HEAD ahead
# while this worktree's checkout stays behind, blocking subsequent merges
# with "local changes would be overwritten". Reset the worktree to main HEAD
# before each merge attempt so the daemon never stalls on stale checkouts.
git -C "$MAIN_WT" reset --hard main >>"$CYCLE_LOG" 2>&1 || \
  log "warn: auto-sync of main worktree failed; merge may collide"
if ! git -C "$MAIN_WT" merge --no-ff "$WT_BR" \
       -m "Merge $WT_BR: qwen-pump scaffold for $crate (+$N tests)" >>"$CYCLE_LOG" 2>&1; then
  log "fail: main merge — branch $WT_BR retained for manual cleanup"
  git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
  exit 0
fi

# Append contribution row (machine-readable). Includes agentic retry stats:
# success_at_retry = 1/2/3 if a qwen draft compiled, -1 if all retries failed
# and the fallback placeholder was committed.
jq -nc \
  --arg ts "$(ts)" \
  --arg crate "$crate" \
  --arg sha "$SHA" \
  --arg br "$WT_BR" \
  --arg model "$OLLAMA_MODEL" \
  --argjson n "$N" \
  --argjson success_at_retry "$SUCCESS_AT_RETRY" \
  --argjson ollama_calls "$TOTAL_OLLAMA_CALLS" \
  --argjson ollama_secs "$TOTAL_OLLAMA_SECS" \
  '{ts:$ts, worker_id:"qwen-pump-cron", batch_id:("pump-"+$crate),
    test_delta:$n, commit_sha:$sha, model:$model, crate:$crate,
    branch:$br, merged_to:"main",
    success_at_retry:$success_at_retry, ollama_calls:$ollama_calls, ollama_secs:$ollama_secs}' \
  >> "$CONTRIB"

# Drop the per-cycle worktree (branch already merged via --no-ff).
git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true

log "OK $crate +$N tests sha=$SHA"
exit 0
