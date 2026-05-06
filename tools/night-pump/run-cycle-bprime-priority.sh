#!/usr/bin/env bash
# run-cycle-bprime-priority.sh — autonomous Cave Runtime pump cycle.
#
# Replaces the previous multimode dispatcher (E/H/I-only) which kept returning
# DONE on every cosmetically-mature crate, so 0 commits/h. This script:
#
#   1. tries Mode B-prime FIRST (real test scaffolds against parity manifest)
#   2. falls through to I → H → E only if B-prime is not applicable
#   3. exits DONE only if B-prime + E + H + I all have nothing to add
#
# Mode B-prime is the proven legacy logic (was inline in run-cycle.sh up to
# 2026-05-05): scaffold tests/qwen_drafted.rs with #[ignore]-marked behavioural
# tests against ground-truth pub-symbol allowlist, 3-attempt local-Ollama
# retry with compile-error feedback, optional gpt-4o + gemini fallbacks
# (skipped if no token), cargo check + clippy + manifest [[tests]] update.
#
# When this is the LaunchAgent target, the production location is:
#   ~/Library/Application Support/cave-qwen-pump/run-cycle-bprime-priority.sh
# (install via tools/night-pump/install-bprime-priority.sh; original
# run-cycle.sh is preserved alongside as the fallback.)
#
# State files are under SCRIPT_DIR (the install location), not the repo, so
# the daemon survives worktree rebases.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
QUEUE="$SCRIPT_DIR/queue.txt"
LOG_DIR="$SCRIPT_DIR/log"
CYCLE_LOG="$LOG_DIR/run-cycle.log"
CONTRIB="$SCRIPT_DIR/contributions.jsonl"
LOCK_DIR="$SCRIPT_DIR/run-cycle.lockdir"
PHANTOM_TSV="$SCRIPT_DIR/phantom-counter.tsv"
BPRIME_SHELF="$SCRIPT_DIR/bprime-shelf.tsv"  # crate \t consecutive_failures \t last_error
REPO_ROOT="${QWEN_PUMP_REPO_ROOT:-/Users/gnomish/Code/cave-runtime}"

# Mode B-prime tuning knobs (env-overridable for ad-hoc testing).
BPRIME_TARGET_TESTS_DEFAULT="${BPRIME_TARGET_TESTS_DEFAULT:-30}"
BPRIME_MIN_GAP="${BPRIME_MIN_GAP:-5}"           # only run B-prime if gap ≥ 5
BPRIME_MAX_RETRY="${BPRIME_MAX_RETRY:-3}"        # local Qwen attempts before shelf
BPRIME_SHELF_THRESHOLD="${BPRIME_SHELF_THRESHOLD:-3}"  # consecutive shelf hits → mode-fallthrough

if [ ! -d "$REPO_ROOT/.git" ] && [ ! -f "$REPO_ROOT/.git" ]; then
  echo "FATAL: REPO_ROOT '$REPO_ROOT' is not a git repo" >&2; exit 1
fi
MAIN_WT="$(git -C "$REPO_ROOT" worktree list --porcelain \
            | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt; exit}')"
if [ -z "$MAIN_WT" ]; then
  echo "FATAL: no worktree currently has main checked out" >&2; exit 1
fi
SHARED_TARGET="$REPO_ROOT/.claude/qwen-pump-target"

OLLAMA_HOST="${OLLAMA_HOST:-http://127.0.0.1:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3.6:35b-a3b-coding-mxfp8}"
OLLAMA_KEEP_ALIVE="${OLLAMA_KEEP_ALIVE:-24h}"

mkdir -p "$LOG_DIR" "$SHARED_TARGET"
[ -f "$PHANTOM_TSV" ] || : > "$PHANTOM_TSV"
[ -f "$BPRIME_SHELF" ] || : > "$BPRIME_SHELF"

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*" | tee -a "$CYCLE_LOG" >&2; }

# ── Phantom + shelf counters ────────────────────────────────────────────────
get_phantom_count() { grep -E "^$1[[:space:]]" "$PHANTOM_TSV" 2>/dev/null | head -1 | awk '{print $2}'; }
inc_phantom_count() {
  local c="$1" n; n=$(get_phantom_count "$c"); n=${n:-0}; n=$((n+1))
  awk -v c="$c" -v n="$n" '$1!=c{print} END{print c"\t"n}' "$PHANTOM_TSV" > "$PHANTOM_TSV.tmp" && mv "$PHANTOM_TSV.tmp" "$PHANTOM_TSV"
  echo "$n"
}
clear_phantom_count() {
  awk -v c="$1" '$1!=c{print}' "$PHANTOM_TSV" > "$PHANTOM_TSV.tmp" && mv "$PHANTOM_TSV.tmp" "$PHANTOM_TSV"
}
get_shelf_count() { grep -E "^$1\t" "$BPRIME_SHELF" 2>/dev/null | head -1 | awk -F'\t' '{print $2}'; }
inc_shelf_count() {
  local c="$1" err="${2:-}" n; n=$(get_shelf_count "$c"); n=${n:-0}; n=$((n+1))
  awk -F'\t' -v c="$c" -v n="$n" -v e="$err" 'BEGIN{OFS="\t"} $1!=c{print} END{print c, n, e}' "$BPRIME_SHELF" > "$BPRIME_SHELF.tmp" && mv "$BPRIME_SHELF.tmp" "$BPRIME_SHELF"
  echo "$n"
}
clear_shelf_count() {
  awk -F'\t' -v c="$1" '$1!=c{print}' "$BPRIME_SHELF" > "$BPRIME_SHELF.tmp" && mv "$BPRIME_SHELF.tmp" "$BPRIME_SHELF"
}
get_shelf_err() { grep -E "^$1\t" "$BPRIME_SHELF" 2>/dev/null | head -1 | awk -F'\t' '{print $3}'; }

# ── Lock + queue pop ─────────────────────────────────────────────────────────
if [ -d "$LOCK_DIR" ]; then
  if [ -n "$(find "$LOCK_DIR" -maxdepth 0 -mmin +60 2>/dev/null)" ]; then
    log "reclaim: stale lock dir > 60m"
    rmdir "$LOCK_DIR" 2>/dev/null || rm -rf "$LOCK_DIR"
  fi
fi
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  log "skip: another cycle already running"
  exit 0
fi
trap 'rmdir "$LOCK_DIR" 2>/dev/null || rm -rf "$LOCK_DIR"' EXIT

if [ ! -s "$QUEUE" ]; then
  log "queue empty — exit"
  exit 0
fi

# Priority sort: HIGH crates pop first.
sort_queue_by_priority() {
  local q="$1" tmp="$1.tmp" main="$2" line k p m
  : > "$tmp"
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    m="$main/crates/$line/parity.manifest.toml"
    p="LOW"
    if [ -f "$m" ]; then
      p=$(grep -E '^[[:space:]]*pump_priority[[:space:]]*=' "$m" \
          | head -1 | sed -E 's/.*=[[:space:]]*"([A-Z]+)".*/\1/')
      [ -z "$p" ] && p="LOW"
    fi
    case "$p" in HIGH) k=0 ;; MEDIUM) k=1 ;; *) k=2 ;; esac
    printf '%d\t%s\n' "$k" "$line" >> "$tmp"
  done < "$q"
  sort -s -k1,1n "$tmp" | cut -f2- > "$q"
  rm -f "$tmp"
}
sort_queue_by_priority "$QUEUE" "$MAIN_WT"

crate="$(awk 'NF{print; exit}' "$QUEUE")"
if [ -z "${crate:-}" ]; then log "queue head empty"; exit 0; fi
awk -v target="$crate" 'BEGIN{dropped=0} { if (!dropped && $0==target) {dropped=1; next} print }' "$QUEUE" > "$QUEUE.tmp" && mv "$QUEUE.tmp" "$QUEUE"
log "picked: $crate"

# Crate must exist on main.
if [ ! -d "$MAIN_WT/crates/$crate" ]; then
  log "drop: crates/$crate dir missing on main (fs-orphan phantom)"
  PCOUNT=$(inc_phantom_count "$crate")
  if [ "$PCOUNT" -ge 3 ]; then
    log "PHANTOM AUTO-DROP: $crate hit 3 fs-missing strikes"
    clear_phantom_count "$crate"
  else
    log "phantom strike $PCOUNT/3 for $crate"
  fi
  exit 0
fi

CRATE_DIR="$MAIN_WT/crates/$crate"
CARGO_TOML="$CRATE_DIR/Cargo.toml"
LIB_RS="$CRATE_DIR/src/lib.rs"
MANIFEST="$CRATE_DIR/parity.manifest.toml"
DRAFTED="$CRATE_DIR/tests/qwen_drafted.rs"

PKG_NAME="$(awk -F'"' '/^[[:space:]]*name[[:space:]]*=[[:space:]]*"/{print $2; exit}' "$CARGO_TOML" 2>/dev/null)"
[ -z "$PKG_NAME" ] && PKG_NAME="$crate"

# Sync MAIN_WT to current main HEAD before mode discovery.
git -C "$MAIN_WT" reset --hard main >>"$CYCLE_LOG" 2>&1 || \
  log "warn: pre-discover sync of MAIN_WT failed; mode may be stale"

# ── B-prime applicability ───────────────────────────────────────────────────
# Returns 0 if Mode B-prime is the right thing to run for this crate, else 1.
#
# Applicable when:
#   - crate has src/ (any *.rs)
#   - parity.manifest.toml exists
#   - NOT bin-only (must have lib.rs OR [lib] section for integration tests
#     to reach the surface)
#   - test-count gap (target_tests - current) >= BPRIME_MIN_GAP
#   - shelf count for this crate < BPRIME_SHELF_THRESHOLD (else fall through)
#
# target_tests source: parity.manifest.toml `target_tests = N` field, else
#   BPRIME_TARGET_TESTS_DEFAULT.
bprime_applicable() {
  [ -f "$MANIFEST" ] || { log "B-prime skip: no parity.manifest.toml"; return 1; }
  [ -d "$CRATE_DIR/src" ] || { log "B-prime skip: no src/"; return 1; }
  local has_lib=0
  if [ -f "$LIB_RS" ] || grep -qE '^\[lib\]' "$CARGO_TOML" 2>/dev/null; then
    has_lib=1
  fi
  if [ "$has_lib" -eq 0 ]; then
    log "B-prime skip: bin-only ($crate has no src/lib.rs and no [lib])"
    return 1
  fi
  local shelf; shelf=$(get_shelf_count "$crate"); shelf=${shelf:-0}
  if [ "$shelf" -ge "$BPRIME_SHELF_THRESHOLD" ]; then
    log "B-prime skip: shelf count $shelf ≥ $BPRIME_SHELF_THRESHOLD (falling through to E/H/I)"
    return 1
  fi
  local target; target=$(grep -E '^[[:space:]]*target_tests[[:space:]]*=' "$MANIFEST" \
                          | head -1 | sed -E 's/.*=[[:space:]]*([0-9]+).*/\1/')
  [ -z "$target" ] && target="$BPRIME_TARGET_TESTS_DEFAULT"
  local current=0
  if [ -f "$DRAFTED" ]; then
    current=$(grep -cE '^[[:space:]]*#\[(test|tokio::test)\]' "$DRAFTED" 2>/dev/null || echo 0)
  fi
  local gap=$((target - current))
  if [ "$gap" -lt "$BPRIME_MIN_GAP" ]; then
    log "B-prime done: $crate at $current/$target tests (gap $gap < $BPRIME_MIN_GAP)"
    return 1
  fi
  log "B-prime applicable: $crate at $current/$target tests (gap=$gap, shelf=$shelf)"
  BPRIME_TARGET="$target"
  BPRIME_CURRENT="$current"
  return 0
}

# ── E/H/I discovery (unchanged from multimode) ──────────────────────────────
discover_ehi_mode() {
  if [ -f "$CARGO_TOML" ] && ! grep -qE '^[[:space:]]*keywords[[:space:]]*=' "$CARGO_TOML"; then
    echo "I"; return
  fi
  if [ ! -f "$CRATE_DIR/README.md" ]; then
    echo "H"; return
  fi
  local f pubs docs
  for f in "$CRATE_DIR/src"/*.rs; do
    [ -f "$f" ] || continue
    pubs=$(grep -cE '^pub (fn|struct|enum|trait|use|mod|const|static|type)' "$f" 2>/dev/null)
    docs=$(grep -cE '^[[:space:]]*///' "$f" 2>/dev/null)
    if [ "$pubs" -ge 3 ] && [ "$docs" -lt $((pubs * 2 / 3)) ]; then
      echo "E"; return
    fi
  done
  echo "DONE"
}

# ── Branch + worktree setup ─────────────────────────────────────────────────
make_worktree() {
  local mode="$1"
  WT_BR="qwen/pump-${crate}-mode${mode}-$(date -u +%Y%m%d-%H%M%S)"
  WT_DIR="$REPO_ROOT/.claude/worktrees/qwen-pump-${crate}-mode${mode}-$(date -u +%H%M%S)"
  git -C "$REPO_ROOT" worktree add "$WT_DIR" -b "$WT_BR" main >>"$CYCLE_LOG" 2>&1 || {
    log "fail: could not create worktree (re-queue $crate)"
    echo "$crate" >> "$QUEUE"; return 1
  }
  return 0
}

cleanup_worktree() {
  git -C "$REPO_ROOT" worktree remove --force "$WT_DIR" 2>/dev/null || true
  git -C "$REPO_ROOT" branch -D "$WT_BR" 2>/dev/null || true
}

cleanup_failure() {
  local reason="$1" mode="${2:-?}"
  if tail -50 "$CYCLE_LOG" 2>/dev/null | grep -q "package ID specification \`$crate\` did not match\|package ID specification \`$PKG_NAME\` did not match"; then
    PCOUNT=$(inc_phantom_count "$crate")
    log "cargo-phantom strike $PCOUNT/3 for $crate"
    if [ "$PCOUNT" -ge 3 ]; then
      log "PHANTOM AUTO-DROP: $crate hit 3 cargo-phantom strikes — NOT re-queuing"
      clear_phantom_count "$crate"
      cleanup_worktree
      return
    fi
  fi
  log "FAIL ($reason, mode=$mode) — re-queue $crate at tail, drop worktree+branch"
  echo "$crate" >> "$QUEUE"
  cleanup_worktree
}

# ── Ollama wrapper ──────────────────────────────────────────────────────────
call_ollama() {
  local prompt_file="$1" out_file="$2" num_predict="${3:-4096}"
  perl -e 'alarm shift; exec @ARGV or die' 660 \
    curl -sS -o "$out_file" -w "%{http_code}" --max-time 600 --connect-timeout 30 \
    -X POST "$OLLAMA_HOST/api/generate" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg m "$OLLAMA_MODEL" --rawfile p "$prompt_file" --arg ka "$OLLAMA_KEEP_ALIVE" --argjson np "$num_predict" \
          '{model:$m, prompt:$p, stream:false, think:false, keep_alive:$ka,
            options:{num_ctx:32768, num_predict:$np, temperature:0.2}}')" \
    2>>"$CYCLE_LOG"
}

call_ollama_inline() {
  local prompt="$1" out_file="$2" num_predict="${3:-4096}"
  perl -e 'alarm shift; exec @ARGV or die' 660 \
    curl -sS -o "$out_file" -w "%{http_code}" --max-time 600 --connect-timeout 30 \
    -X POST "$OLLAMA_HOST/api/generate" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg m "$OLLAMA_MODEL" --arg p "$prompt" --arg ka "$OLLAMA_KEEP_ALIVE" --argjson np "$num_predict" \
          '{model:$m, prompt:$p, stream:false, think:false, keep_alive:$ka,
            options:{num_ctx:32768, num_predict:$np, temperature:0.2}}')" \
    2>>"$CYCLE_LOG"
}

crate_description() {
  grep -E "^description" "$CARGO_TOML" | head -1 | sed -E 's/^description[[:space:]]*=[[:space:]]*"(.*)"/\1/'
}

# ── Mode B-prime (test scaffold gen, the real-value mode) ───────────────────
# Generates 10–15 #[ignore]-marked behavioural tests with a 3-attempt local
# Qwen retry loop using compile-error feedback. On success, appends a cycle
# block to tests/qwen_drafted.rs and adds [[tests]] entries to manifest.
run_mode_BP() {
  local target="$BPRIME_TARGET" current="$BPRIME_CURRENT"
  local gap=$((target - current))
  log "Mode B-prime START $crate gap=$gap (target=$target, current=$current)"

  local crate_underscore; crate_underscore="$(echo "$crate" | tr - _)"
  local TODAY; TODAY="$(date -u +%Y-%m-%d)"
  local CYCLE_TS; CYCLE_TS="$(date -u +%s)"
  local FENCE_RE
  FENCE_RE=$'^[[:space:]]*\x60\x60\x60[a-zA-Z]*[[:space:]]*$'

  local modules
  modules="$(ls "$CRATE_DIR/src/" 2>/dev/null \
              | awk -F. '/\.rs$/ && $1!="lib"{print $1}' \
              | tr '\n' ',' | sed 's/,$//')"
  if [ -z "$modules" ]; then
    log "Mode B-prime: no src/*.rs modules — fall through"
    return 2
  fi
  local modules_short
  modules_short="$(echo "$modules" | tr ',' '\n' | head -5 | tr '\n' ',' | sed 's/,$//')"

  local SYMBOLS
  SYMBOLS="$(find "$CRATE_DIR/src" -name '*.rs' 2>/dev/null \
    | xargs grep -hE "^pub (fn|struct|enum|trait|use|mod|const|static|type)" 2>/dev/null \
    | head -80 \
    | sed -E 's/[[:space:]]*\{.*$//; s/[[:space:]]*\(.*$//; s/^pub //' \
    | sort -u \
    | head -50)"
  local GROUND_TRUTH_BLOCK="GROUND TRUTH — only these symbols exist in ${crate}. DO NOT invent imports.
${SYMBOLS}

Use ONLY these symbols. Any other path is a hallucination."

  local PRIOR_ERR=""
  local SHELF_HIT
  SHELF_HIT=$(get_shelf_count "$crate"); SHELF_HIT=${SHELF_HIT:-0}
  if [ "$SHELF_HIT" -gt 0 ]; then
    PRIOR_ERR="$(get_shelf_err "$crate")"
    if [ -n "$PRIOR_ERR" ]; then
      PRIOR_ERR="

(Previous attempt for this crate failed with: ${PRIOR_ERR}. Do NOT repeat that mistake.)"
    fi
  fi

  local PROMPT="${GROUND_TRUTH_BLOCK}${PRIOR_ERR}

Write Rust integration tests for the cave-runtime crate ${crate}.

Available modules (relative to the crate root, first few only): ${modules_short}
(Use only these — do not invent module names.)

CRITICAL PATH RULES (these tests live in tests/qwen_drafted.rs, an integration test file — they compile as a separate crate):
- Use absolute crate paths only: \`${crate_underscore}::module::Item\` or \`${crate_underscore}::Item\`.
- DO NOT use \`super::\` (refers to the test file root, NOT the lib crate — integration tests cannot reach lib internals via super).
- DO NOT use \`crate::\` (refers to the integration test crate, not ${crate_underscore}).
- NO inner doc comments (\`//!\`) inside the mod block — only outer (\`///\`) or regular (\`//\`) comments are valid there.

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

  local RUST_OUT="$WT_DIR/crates/$crate/tests/qwen_drafted.rs"
  mkdir -p "$(dirname "$RUST_OUT")"
  local SNAPSHOT="$LOG_DIR/${crate}-snapshot-${CYCLE_TS}.rs"
  if [ -f "$RUST_OUT" ]; then cp "$RUST_OUT" "$SNAPSHOT"; else : > "$SNAPSHOT"; fi

  local SUCCESS_AT_RETRY=-1
  local TOTAL_OLLAMA_CALLS=0
  local TOTAL_OLLAMA_SECS=0
  local WINNING_DRAFT=""
  local CURRENT_PROMPT="$PROMPT"
  local last_error=""

  for ATTEMPT in $(seq 1 "$BPRIME_MAX_RETRY"); do
    local RESPONSE_FILE="$LOG_DIR/${crate}-bp-attempt${ATTEMPT}.json"
    log "B-prime retry_${ATTEMPT}: calling $OLLAMA_MODEL"
    local T0; T0=$(date +%s)

    local HTTP_STATUS
    HTTP_STATUS=$(call_ollama_inline "$CURRENT_PROMPT" "$RESPONSE_FILE" 4096) || HTTP_STATUS="curl_fail"
    local T1; T1=$(date +%s)
    TOTAL_OLLAMA_CALLS=$((TOTAL_OLLAMA_CALLS + 1))
    TOTAL_OLLAMA_SECS=$((TOTAL_OLLAMA_SECS + (T1 - T0)))

    if [ "$HTTP_STATUS" != "200" ]; then
      log "B-prime retry_${ATTEMPT}: bad_response status=$HTTP_STATUS — try next"
      last_error="ollama HTTP $HTTP_STATUS"
      continue
    fi
    if ! jq -e .response "$RESPONSE_FILE" >/dev/null 2>&1; then
      log "B-prime retry_${ATTEMPT}: ollama_no_response — try next"
      last_error="ollama empty response"
      continue
    fi

    local ATTEMPT_RAW="$LOG_DIR/${crate}-bp-attempt${ATTEMPT}.rs"
    jq -r '.response' "$RESPONSE_FILE" \
      | sed -E "/${FENCE_RE}/d" \
      | sed -E '/^[[:space:]]*\/\/!/d' \
      | sed -E "s/^[[:space:]]*mod[[:space:]]+tests[[:space:]]*\{/mod cycle_${CYCLE_TS}_a${ATTEMPT} {/" \
      | sed -E "s/use[[:space:]]+super::/use ${crate_underscore}::/g" \
      | sed -E "s/use[[:space:]]+crate::/use ${crate_underscore}::/g" \
      > "$ATTEMPT_RAW"
    if [ ! -s "$ATTEMPT_RAW" ]; then
      log "B-prime retry_${ATTEMPT}: empty extraction — try next"
      last_error="empty extraction after fence strip"
      continue
    fi

    cp "$SNAPSHOT" "$RUST_OUT"
    {
      echo ""
      echo "// === cycle ${CYCLE_TS} attempt ${ATTEMPT} (${OLLAMA_MODEL}) ==="
      cat "$ATTEMPT_RAW"
    } >> "$RUST_OUT"

    local ERR_FILE="$LOG_DIR/${crate}-bp-attempt${ATTEMPT}.err"
    log "B-prime retry_${ATTEMPT}: cargo check --tests"
    if ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$PKG_NAME" --tests --quiet ) >"$ERR_FILE" 2>&1; then
      cat "$ERR_FILE" >>"$CYCLE_LOG"
      SUCCESS_AT_RETRY=$ATTEMPT
      WINNING_DRAFT="$ATTEMPT_RAW"
      log "B-prime retry_${ATTEMPT}: SUCCESS"
      break
    fi
    cat "$ERR_FILE" >>"$CYCLE_LOG"
    log "B-prime retry_${ATTEMPT}: still_failing"

    if grep -q "did not match any packages" "$ERR_FILE"; then
      PCOUNT=$(inc_phantom_count "$crate")
      log "B-prime phantom strike $PCOUNT/3 for $crate"
      if [ "$PCOUNT" -ge 3 ]; then
        log "PHANTOM AUTO-DROP: $crate hit 3 strikes (B-prime cargo)"
        clear_phantom_count "$crate"
        cleanup_worktree
        return 3   # phantom — no re-queue
      fi
      last_error="cargo cannot resolve package id"
      break
    fi

    local ERROR_EXCERPT; ERROR_EXCERPT=$(grep -E "^error" "$ERR_FILE" | head -8 | head -c 500)
    last_error="$ERROR_EXCERPT"
    CURRENT_PROMPT="${GROUND_TRUTH_BLOCK}

Your previous test draft for ${crate} had compile errors:
${ERROR_EXCERPT}

Fix these. Use ONLY symbols from the GROUND TRUTH list above — every other path is a hallucination.
Do NOT invent module names. Do NOT use items you have not verified exist in the GROUND TRUTH.
Output ONLY the corrected Rust test file. Same format rules:
- 4 mandatory header lines
- Wrap tests in: #[cfg(test)] mod tests { ... }
- 10-15 #[test] #[ignore = \"impl pending\"] functions with timestamped suffix names
- Imports inside the mod block, only what you use
- Use unimplemented!() for anything you cannot compute deterministically"
  done

  cp "$SNAPSHOT" "$RUST_OUT"
  if [ "$SUCCESS_AT_RETRY" -gt 0 ]; then
    {
      echo ""
      echo "// === cycle ${CYCLE_TS} (qwen success at retry ${SUCCESS_AT_RETRY}; ollama_calls=${TOTAL_OLLAMA_CALLS}; ollama_secs=${TOTAL_OLLAMA_SECS}) ==="
      cat "$WINNING_DRAFT"
    } >> "$RUST_OUT"
    if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$PKG_NAME" --tests --quiet ) >>"$CYCLE_LOG" 2>&1; then
      log "B-prime post-restore compile failed unexpectedly"
      SUCCESS_AT_RETRY=-1
    fi
  fi

  if [ "$SUCCESS_AT_RETRY" -lt 1 ]; then
    log "B-prime: all $BPRIME_MAX_RETRY local Qwen attempts failed (last_error=${last_error:0:120})"
    SHELF=$(inc_shelf_count "$crate" "${last_error:0:120}")
    log "B-prime shelf $crate count=$SHELF (threshold=$BPRIME_SHELF_THRESHOLD)"
    cleanup_worktree
    # Re-queue at tail so the next cycle can either retry or fall through to E/H/I.
    echo "$crate" >> "$QUEUE"
    return 1
  fi

  # Count tests + run cargo test --no-run as additional gate.
  local N
  N=$(grep -cE '^[[:space:]]*#\[(test|tokio::test)\]' "$RUST_OUT" || true)
  if [ "$N" -lt 5 ]; then
    log "B-prime gate FAIL: too few tests ($N < 5)"
    SHELF=$(inc_shelf_count "$crate" "too few tests after retries: $N")
    cleanup_worktree
    echo "$crate" >> "$QUEUE"
    return 1
  fi
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo test -p "$PKG_NAME" --no-run --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "B-prime gate FAIL: cargo test --no-run"
    SHELF=$(inc_shelf_count "$crate" "cargo test --no-run failed")
    cleanup_worktree
    echo "$crate" >> "$QUEUE"
    return 1
  fi
  log "B-prime compile-gate GREEN ($N tests)"
  clear_shelf_count "$crate"
  clear_phantom_count "$crate"

  # Manifest [[tests]] update — only for THIS cycle's block.
  local MANIFEST_DELTA=0
  if [ -f "$WT_DIR/crates/$crate/parity.manifest.toml" ]; then
    local CYCLE_FNS
    CYCLE_FNS=$(awk -v marker="cycle ${CYCLE_TS}" '
        $0 ~ marker {in_block=1}
        in_block {print}
      ' "$RUST_OUT" \
      | grep -E '^[[:space:]]*fn [a-zA-Z0-9_]+' \
      | sed -E 's/^[[:space:]]*fn ([a-zA-Z0-9_]+).*/\1/')
    if [ -n "$CYCLE_FNS" ]; then
      {
        echo ""
        echo "# Qwen-pump B-prime cycle ${CYCLE_TS} (model=${OLLAMA_MODEL}, success_at_retry=${SUCCESS_AT_RETRY})"
        while IFS= read -r fn; do
          [ -z "$fn" ] && continue
          echo ""
          echo "[[tests]]"
          echo "upstream_test = \"qwen-scaffold pending review\""
          echo "local_test    = \"$fn\""
        done <<< "$CYCLE_FNS"
      } >> "$WT_DIR/crates/$crate/parity.manifest.toml"
      MANIFEST_DELTA=$(echo "$CYCLE_FNS" | grep -cE '^[a-zA-Z0-9_]+$')
      log "B-prime manifest +$MANIFEST_DELTA [[tests]] entries"
    fi
  fi

  COMMIT_FILES=("crates/$crate/tests/qwen_drafted.rs" "crates/$crate/parity.manifest.toml")
  COMMIT_TYPE="feat"
  COMMIT_MSG="Mode B-prime: +$N red test scaffold (impl pending; manifest +$MANIFEST_DELTA)"
  return 0
}

# ── Mode I: Cargo.toml metadata (unchanged from multimode) ─────────────────
run_mode_I() {
  local desc; desc=$(crate_description)
  local prompt="$LOG_DIR/${crate}-mode-I-prompt.txt"
  cat > "$prompt" <<PEOF
You are Cave Runtime's Cargo.toml metadata agent. Add publish-readiness fields. Output the COMPLETE FILE.

PRESERVATION RULES (HARD):
A. Every line of input MUST appear in output VERBATIM. Output is a STRICT SUPERSET — only new fields added inside [package].
B. New fields go AFTER the description line.
C. Workspace defines: version, edition, license, authors, repository, homepage, rust-version. Use .workspace = true for these.
D. NO duplicate fields. NO changes to [dependencies] or other sections.

CRATE: $crate
DESCRIPTION: $desc

REQUIRED FIELDS TO ADD (only those not already present):
  authors.workspace = true
  repository.workspace = true
  homepage.workspace = true
  documentation = "https://docs.rs/$crate"
  readme = "README.md"
  rust-version.workspace = true
  keywords = [<infer 4-5 single-word keywords from description>]
  categories = [<2-3 from: api-bindings, asynchronous, authentication, command-line-utilities, cryptography, database-implementations, development-tools, encoding, network-programming, parser-implementations, web-programming>]

INPUT FILE (verbatim — output is this with new fields added):

PEOF
  cat "$CARGO_TOML" >> "$prompt"
  local resp="$LOG_DIR/${crate}-mode-I.json"
  call_ollama "$prompt" "$resp" 3072 >/dev/null
  local out_toml="$LOG_DIR/${crate}-mode-I.toml"
  jq -r '.response' "$resp" > "$out_toml"
  local sz; sz=$(wc -c < "$out_toml")
  if [ "$sz" -lt 200 ]; then return 1; fi

  local last_dep_line; last_dep_line=$(awk '/^\[dependencies\]/,/^\[/{if (/^[a-z][a-zA-Z0-9_-]*[[:space:]]*=/) print NR}' "$CARGO_TOML" | tail -1)
  if [ -z "$last_dep_line" ]; then
    cp "$out_toml" "$WT_DIR/crates/$crate/Cargo.toml"
  else
    local anchor; anchor=$(awk -v ln="$last_dep_line" 'NR==ln{print $1; exit}' "$CARGO_TOML")
    local orig_anchor; orig_anchor=$(grep -n "^${anchor}" "$CARGO_TOML" | head -1 | cut -d: -f1)
    local qwen_anchor; qwen_anchor=$(grep -n "^${anchor}" "$out_toml" | head -1 | cut -d: -f1)
    if [ -z "$qwen_anchor" ] || [ -z "$orig_anchor" ]; then return 1; fi
    {
      head -n "$qwen_anchor" "$out_toml"
      tail -n "+$((orig_anchor+1))" "$CARGO_TOML"
    } > "$WT_DIR/crates/$crate/Cargo.toml"
  fi

  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$PKG_NAME" --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode I gate FAIL (cargo check)"; return 1
  fi
  COMMIT_FILES=("crates/$crate/Cargo.toml")
  COMMIT_TYPE="build"
  COMMIT_MSG="Mode I: enrich Cargo.toml metadata (keywords, categories, repository, docs)"
  return 0
}

# ── Mode H: per-crate README ───────────────────────────────────────────────
run_mode_H() {
  if [ -f "$CRATE_DIR/README.md" ]; then return 1; fi
  local desc; desc=$(crate_description)
  local prompt="$LOG_DIR/${crate}-mode-H-prompt.txt"
  cat > "$prompt" <<PEOF
Generate a per-crate README.md for cave-runtime crate.

CONSTRAINTS:
- Output ONLY markdown content, no fences wrapping the whole output.
- 50-90 lines, GitHub-flavored markdown.
- NO inline HTML, NO emoji.
- Use [text](url) only for the upstream link.
- Tone: technical, no marketing.

CRATE: $crate
DESCRIPTION: $desc
WORKSPACE: cave-runtime — sovereign Cloud OS in Rust, Hetzner OSS stack on Linux 7.1

REQUIRED SECTIONS (8 headers total: 1 # + 7 ##):
# $crate  (one-line tagline)
## Status  (1-2 sentences: pre-OSS-launch, parity tracked)
## Upstream  (single bullet with the upstream link if known, else "(internal — no external upstream)")
## Surface ported  (5-10 bullets describing major capabilities)
## Public API  (3-6 bullets pointing at top-level pub fn/structs)
## Tests  (1-2 sentences about test coverage)
## License  (Apache-2.0)
## See also  (2-3 adjacent crates with ../crate-X links)

Begin output now.
PEOF
  local resp="$LOG_DIR/${crate}-mode-H.json"
  call_ollama "$prompt" "$resp" 3072 >/dev/null
  local out_md="$LOG_DIR/${crate}-mode-H.md"
  jq -r '.response' "$resp" > "$out_md"
  local sz; sz=$(wc -c < "$out_md")
  if [ "$sz" -lt 500 ]; then return 1; fi
  local hdrs; hdrs=$(grep -cE '^# |^## ' "$out_md")
  if [ "$hdrs" -lt 6 ]; then log "Mode H gate FAIL (only $hdrs headers)"; return 1; fi

  cp "$out_md" "$WT_DIR/crates/$crate/README.md"
  COMMIT_FILES=("crates/$crate/README.md")
  COMMIT_TYPE="docs"
  COMMIT_MSG="Mode H: add per-crate README.md ($hdrs headers, $(wc -l < "$out_md") lines)"
  return 0
}

# ── Mode E: rustdoc gen on a chosen src file ───────────────────────────────
run_mode_E() {
  local target=""
  local max_gap=0
  for f in "$CRATE_DIR/src"/*.rs; do
    [ -f "$f" ] || continue
    local pubs; pubs=$(grep -cE '^pub (fn|struct|enum|trait|use|mod|const|static|type)' "$f" 2>/dev/null)
    local docs; docs=$(grep -cE '^[[:space:]]*///' "$f" 2>/dev/null)
    local gap=$((pubs - docs))
    if [ "$gap" -gt "$max_gap" ] && [ "$pubs" -ge 3 ]; then
      max_gap=$gap
      target="$f"
    fi
  done
  if [ -z "$target" ]; then log "Mode E: no suitable target file"; return 1; fi
  local rel; rel=${target#$CRATE_DIR/}
  log "Mode E target: $rel (gap=$max_gap)"

  local has_test_mod=0
  grep -q '^#\[cfg(test)\]' "$target" && has_test_mod=1

  local prompt="$LOG_DIR/${crate}-mode-E-prompt.txt"
  cat > "$prompt" <<PEOF
You are Cave Runtime's documentation agent. Add /// rustdoc comments before each pub item and a //! module header. Output the COMPLETE FILE.

ABSOLUTE PRESERVATION RULES (HARD):
A. Every line of input MUST appear in output VERBATIM, byte-for-byte. Output is a STRICT SUPERSET — only /// comments and one //! header are added.
B. ALL input use lines MUST appear in output, in original order, immediately after the //! header.
C. The #[cfg(test)] mod tests block, if present, is OMITTED from your output (the caller will splice it back).
D. NO doctests. NO triple-backtick fences of any kind. Doc comments are PROSE ONLY (4-8 lines per ///).
E. NO new use statements. NO new code. NO signature changes.

CRATE: $crate
FILE: $rel

INPUT FILE (verbatim — output is this with /// added):

PEOF
  if [ "$has_test_mod" -eq 1 ]; then
    awk '/^#\[cfg\(test\)\]/{exit} {print}' "$target" >> "$prompt"
  else
    cat "$target" >> "$prompt"
  fi
  echo "" >> "$prompt"
  echo "End at the closing } of the last impl/fn (before any #[cfg(test)]). Begin output now." >> "$prompt"

  local resp="$LOG_DIR/${crate}-mode-E.json"
  call_ollama "$prompt" "$resp" 6144 >/dev/null
  local out_rs="$LOG_DIR/${crate}-mode-E.rs"
  jq -r '.response' "$resp" > "$out_rs"
  local sz; sz=$(wc -c < "$out_rs")
  if [ "$sz" -lt 200 ]; then return 1; fi
  if grep -qE '^```' "$out_rs"; then log "Mode E gate FAIL: backtick fences"; return 1; fi
  local use_count; use_count=$(grep -c '^use ' "$out_rs")
  local orig_uses; orig_uses=$(grep -c '^use ' "$target")
  if [ "$use_count" -lt "$orig_uses" ]; then log "Mode E gate FAIL: dropped $((orig_uses-use_count)) use lines"; return 1; fi

  local target_in_wt="$WT_DIR/crates/$crate/$rel"
  cat "$out_rs" > "$target_in_wt"
  if [ "$has_test_mod" -eq 1 ]; then
    echo "" >> "$target_in_wt"
    awk '/^#\[cfg\(test\)\]/{found=1} found{print}' "$target" >> "$target_in_wt"
  fi

  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$PKG_NAME" --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo check)"; return 1
  fi
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo doc -p "$PKG_NAME" --no-deps --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo doc)"; return 1
  fi
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo test -p "$PKG_NAME" --doc --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo test --doc)"; return 1
  fi
  COMMIT_FILES=("crates/$crate/$rel")
  COMMIT_TYPE="docs"
  COMMIT_MSG="Mode E: rustdoc gen on $rel (gap=$max_gap → 0)"
  return 0
}

# ── Top-level mode dispatch ────────────────────────────────────────────────
COMMIT_FILES=()
COMMIT_TYPE=""
COMMIT_MSG=""
MODE=""

# Try Mode B-prime first.
if bprime_applicable; then
  MODE="BP"
  if ! make_worktree "BP"; then exit 0; fi
  RC=0
  run_mode_BP || RC=$?
  case "$RC" in
    0) ;;                      # success — continue to commit/merge
    1) exit 0 ;;               # B-prime fail; already shelf-counted, queue updated
    2)                         # B-prime not applicable mid-run — fall through
       cleanup_worktree
       MODE=""
       ;;
    3) exit 0 ;;               # phantom auto-drop, no re-queue
    *) cleanup_failure "B-prime returned $RC" "BP"; exit 0 ;;
  esac
fi

# Fallback to E/H/I.
if [ -z "$MODE" ] || [ "$MODE" = "" ]; then
  EHI="$(discover_ehi_mode)"
  log "E/H/I discovered: $EHI"
  case "$EHI" in
    DONE)
      log "DONE: $crate has Cargo.toml metadata + README + rustdoc; B-prime also at target"
      clear_phantom_count "$crate"
      exit 0
      ;;
    I|H|E)
      MODE="$EHI"
      if ! make_worktree "$MODE"; then exit 0; fi
      RC=0
      case "$MODE" in
        I) run_mode_I || RC=$? ;;
        H) run_mode_H || RC=$? ;;
        E) run_mode_E || RC=$? ;;
      esac
      if [ "$RC" -ne 0 ] || [ -z "$COMMIT_TYPE" ]; then
        cleanup_failure "Mode $MODE returned $RC" "$MODE"
        exit 0
      fi
      ;;
    *)
      log "unknown E/H/I mode '$EHI' — exit"
      exit 0
      ;;
  esac
fi

if [ -z "$COMMIT_TYPE" ] || [ "${#COMMIT_FILES[@]}" -eq 0 ]; then
  log "no-op: COMMIT_TYPE empty after dispatch — bailing"
  cleanup_worktree
  exit 0
fi

# ── Commit + merge ──────────────────────────────────────────────────────────
clear_phantom_count "$crate"
( cd "$WT_DIR" \
  && git add "${COMMIT_FILES[@]}" \
  && git commit -m "$COMMIT_TYPE($crate): $COMMIT_MSG

Generated by $OLLAMA_MODEL via local Ollama (Mode $MODE).
Posted by tools/night-pump/run-cycle-bprime-priority.sh." ) >>"$CYCLE_LOG" 2>&1 || {
  cleanup_failure "commit failed" "$MODE"; exit 0
}
SHA="$(git -C "$WT_DIR" rev-parse HEAD)"

git -C "$MAIN_WT" reset --hard main >>"$CYCLE_LOG" 2>&1 || true
if ! git -C "$MAIN_WT" merge --no-ff "$WT_BR" \
       -m "Merge $WT_BR: Mode $MODE for $crate" >>"$CYCLE_LOG" 2>&1; then
  log "fail: main merge — branch $WT_BR retained for manual cleanup"
  git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
  exit 0
fi

jq -nc \
  --arg ts "$(ts)" \
  --arg crate "$crate" \
  --arg sha "$SHA" \
  --arg br "$WT_BR" \
  --arg model "$OLLAMA_MODEL" \
  --arg mode "$MODE" \
  '{ts:$ts, worker_id:"qwen-pump-bprime-priority", crate:$crate, mode:$mode,
    commit_sha:$sha, branch:$br, merged_to:"main", model:$model}' \
  >> "$CONTRIB"

git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
log "OK $crate Mode $MODE sha=$SHA"
exit 0
