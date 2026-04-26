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
# Resolve REPO_ROOT to the parent repo (not a feature worktree). All
# worktrees share .git via the common-dir; dirname of that is the parent.
REPO_ROOT="$(dirname "$(git -C "$SCRIPT_DIR" rev-parse --path-format=absolute --git-common-dir)")"
QUEUE="$REPO_ROOT/tools/night-pump/queue.txt"
LOG_DIR="$REPO_ROOT/tools/night-pump/log"
CYCLE_LOG="$LOG_DIR/run-cycle.log"
CONTRIB="$REPO_ROOT/tools/night-pump/contributions.jsonl"
LOCK="$REPO_ROOT/tools/night-pump/run-cycle.lock"
SHARED_TARGET="$REPO_ROOT/.claude/qwen-pump-target"
# Find whichever worktree currently has main checked out (do not hard-code).
MAIN_WT="$(git -C "$REPO_ROOT" worktree list --porcelain \
            | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt; exit}')"

OLLAMA_HOST="${OLLAMA_HOST:-http://127.0.0.1:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3-coder-next:Q4_K_M}"
OLLAMA_KEEP_ALIVE="${OLLAMA_KEEP_ALIVE:-24h}"

mkdir -p "$LOG_DIR" "$SHARED_TARGET"

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*" | tee -a "$CYCLE_LOG" >&2; }

# Single-flight lock (non-blocking).
exec 9>"$LOCK"
if ! flock -n 9; then
  log "skip: another cycle already running"
  exit 0
fi

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

# Crate must exist.
if [ ! -d "$REPO_ROOT/crates/$crate" ]; then
  log "drop: crates/$crate dir missing"
  exit 0
fi

# Skip if already pumped.
if [ -f "$REPO_ROOT/crates/$crate/tests/qwen_drafted.rs" ]; then
  log "drop: $crate already has tests/qwen_drafted.rs"
  exit 0
fi

# Discover module surface.
modules="$(ls "$REPO_ROOT/crates/$crate/src/" 2>/dev/null | awk -F. '/\.rs$/ && $1!="lib"{print $1}' | tr '\n' ',' | sed 's/,$//')"
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

# Build prompt.
crate_underscore="$(echo "$crate" | tr - _)"
PROMPT="$(cat <<EOF
You are writing Rust integration tests for the cave-runtime crate \`$crate\`.

Available modules (relative to the crate root): $modules

Hard requirements:
1. Output a SINGLE Rust source file. No prose. No markdown fences. No backticks.
2. The very first lines of the file must be:
   //! Qwen drafted tests for $crate.
   //! Generated $(date -u +%Y-%m-%d) via local Ollama.
   //! All tests are #[ignore = "impl pending"].
   #![allow(unused, unused_imports, unused_variables, unused_mut, dead_code)]
3. Wrap every test in: #[cfg(test)] mod tests { ... }
4. Each test:
   - is annotated #[test] AND #[ignore = "impl pending"]
   - has a one-line "// upstream: <product> v<ver>/<path>" comment immediately before the #[test] attribute
   - exercises a plausible function/struct from the listed modules using the path \`$crate_underscore::module::Item\`
   - threads a tenant_id String through the assertion (multi-tenant invariant)
5. Generate between 25 and 50 tests covering: happy path, edge cases, invariants, error paths, tenant isolation.
6. Imports go inside the mod block. Do not import items you don't use.
7. Use only \`unimplemented!()\` for any value you cannot compute deterministically.

Begin writing the file now.
EOF
)"

RESPONSE_FILE="$LOG_DIR/${crate}-response.json"
log "calling ollama model=$OLLAMA_MODEL"
HTTP_STATUS=$(curl -sS -o "$RESPONSE_FILE" -w "%{http_code}" \
  --max-time 600 \
  -X POST "$OLLAMA_HOST/api/generate" \
  -H 'Content-Type: application/json' \
  -d "$(jq -nc --arg m "$OLLAMA_MODEL" --arg p "$PROMPT" --arg ka "$OLLAMA_KEEP_ALIVE" \
        '{model:$m, prompt:$p, stream:false, keep_alive:$ka, options:{num_predict:6144, temperature:0.2}}')" \
  2>>"$CYCLE_LOG") || {
    cleanup_failure "ollama curl exit"; exit 0;
  }
if [ "$HTTP_STATUS" != "200" ] || ! jq -e .response "$RESPONSE_FILE" >/dev/null 2>&1; then
  log "ollama bad response (status=$HTTP_STATUS)"
  cleanup_failure "ollama bad response"
  exit 0
fi

# Extract Rust source. Strip any leading/trailing markdown fences.
RUST_OUT="$WT_DIR/crates/$crate/tests/qwen_drafted.rs"
mkdir -p "$(dirname "$RUST_OUT")"
jq -r .response "$RESPONSE_FILE" \
  | sed -E '/^[[:space:]]*```[a-zA-Z]*[[:space:]]*$/d' \
  > "$RUST_OUT"

if [ ! -s "$RUST_OUT" ]; then
  cleanup_failure "empty rust output"
  exit 0
fi

# Compile gate.
log "cargo check -p $crate --tests (target=$SHARED_TARGET)"
if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --tests --quiet ) >>"$CYCLE_LOG" 2>&1; then
  cleanup_failure "compile-gate red"
  exit 0
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
if ! git -C "$MAIN_WT" merge --no-ff "$WT_BR" \
       -m "Merge $WT_BR: qwen-pump scaffold for $crate (+$N tests)" >>"$CYCLE_LOG" 2>&1; then
  log "fail: main merge — branch $WT_BR retained for manual cleanup"
  git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
  exit 0
fi

# Append contribution row (machine-readable).
jq -nc \
  --arg ts "$(ts)" \
  --arg crate "$crate" \
  --arg sha "$SHA" \
  --arg br "$WT_BR" \
  --arg model "$OLLAMA_MODEL" \
  --argjson n "$N" \
  '{ts:$ts, worker_id:"qwen-pump-cron", batch_id:("pump-"+$crate),
    test_delta:$n, commit_sha:$sha, model:$model, crate:$crate,
    branch:$br, merged_to:"main"}' \
  >> "$CONTRIB"

# Drop the per-cycle worktree (branch already merged via --no-ff).
git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true

log "OK $crate +$N tests sha=$SHA"
exit 0
