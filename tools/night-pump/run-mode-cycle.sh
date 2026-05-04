#!/usr/bin/env bash
# run-mode-cycle.sh — multi-mode dispatcher for the qwen-pump workforce.
#
# Companion to run-cycle.sh (Mode A scaffold). This script handles four
# additional modes that emit FULL-FILE output instead of unified diffs
# (the diff-mode path failed in Mode B real-impl spike — see capability
# matrix at docs/qwen-pump/capability-matrix-2026-05-04.md):
#
#   B-prime — replace tests/qwen_drafted.rs with real behavior tests
#   E       — add /// rustdoc to a target src/<file>.rs (single file)
#   H       — generate crates/<crate>/README.md
#   I       — enrich crates/<crate>/Cargo.toml metadata
#
# Manual invocation:
#   ./run-mode-cycle.sh <mode> <crate> [target_file]
#
# Examples:
#   ./run-mode-cycle.sh E cave-trace src/comparison.rs
#   ./run-mode-cycle.sh H cave-vault
#   ./run-mode-cycle.sh I cave-streams
#   ./run-mode-cycle.sh B-prime cave-status
#
# This script does NOT integrate with the pump LaunchAgent yet. Pump
# remains on Mode A under run-cycle.sh until the multi-mode workflow is
# validated end-to-end and Burak greenlights the LaunchAgent swap.

set -uo pipefail

MODE="${1:-}"
CRATE="${2:-}"
TARGET_FILE="${3:-}"   # optional — required for Mode E

if [ -z "$MODE" ] || [ -z "$CRATE" ]; then
  cat >&2 <<USAGE
Usage: $0 <mode> <crate> [target_file]
Modes: B-prime | E | H | I
For Mode E, target_file is required (path relative to crate root, e.g. src/lib.rs).
USAGE
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${QWEN_PUMP_REPO_ROOT:-$HOME/Code/cave-runtime}"
MAIN_WT="$(git -C "$REPO_ROOT" worktree list --porcelain \
            | awk '/^worktree /{wt=$2} /^branch refs\/heads\/main$/{print wt; exit}')"
if [ -z "$MAIN_WT" ]; then
  echo "FATAL: no worktree currently has main checked out" >&2
  exit 1
fi

CRATE_DIR="$MAIN_WT/crates/$CRATE"
if [ ! -d "$CRATE_DIR" ]; then
  echo "FATAL: crates/$CRATE does not exist on main" >&2
  exit 1
fi

OLLAMA_HOST="${OLLAMA_HOST:-http://127.0.0.1:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3.6:35b-a3b-coding-mxfp8}"
LOG_DIR="$SCRIPT_DIR/log"
mkdir -p "$LOG_DIR"
ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] [$MODE/$CRATE] $*" >&2; }

call_ollama() {
  # $1 = prompt file path
  # writes JSON response to /tmp/mode-cycle-resp.json, prints just .response
  local prompt_file="$1"
  local resp="/tmp/mode-cycle-resp-$$.json"
  curl -sS -o "$resp" --max-time 300 \
    -X POST "$OLLAMA_HOST/api/generate" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg m "$OLLAMA_MODEL" --rawfile p "$prompt_file" \
          '{model:$m, prompt:$p, stream:false, think:false, keep_alive:"24h",
            options:{num_ctx:32768, num_predict:6144, temperature:0.2}}')" \
    > /dev/null
  jq -r '.response' "$resp"
  rm -f "$resp"
}

# Splice helper: when Qwen emits the leading body of a Rust source file
# (without the #[cfg(test)] mod tests block), splice in the original
# tests block from the backup so the file stays whole.
splice_with_test_mod() {
  # $1 = qwen output file
  # $2 = original (backup) file
  # $3 = output target (will be overwritten)
  local qwen_out="$1" orig="$2" target="$3"
  local tmp; tmp=$(mktemp)
  cat "$qwen_out" > "$tmp"
  echo "" >> "$tmp"
  awk '/^#\[cfg\(test\)\]/{found=1} found{print}' "$orig" >> "$tmp"
  mv "$tmp" "$target"
}

# Splice helper for Cargo.toml: take Qwen output's [package] section,
# concatenate with everything from input AFTER the first non-[package]
# line. Less generic — assumes Qwen produced everything up to and including
# the LAST line of [dependencies] starting with the same anchor word.
splice_cargo_toml() {
  # $1 = qwen output, $2 = original, $3 = anchor (last word of leading section)
  # $4 = target
  local qwen_out="$1" orig="$2" anchor="$3" target="$4"
  local orig_anchor; orig_anchor=$(grep -n "^${anchor}" "$orig" | head -1 | cut -d: -f1)
  local qwen_anchor; qwen_anchor=$(grep -n "^${anchor}" "$qwen_out" | head -1 | cut -d: -f1)
  if [ -z "$orig_anchor" ] || [ -z "$qwen_anchor" ]; then
    echo "splice_cargo_toml: anchor '$anchor' not found in both files" >&2
    return 1
  fi
  local tmp; tmp=$(mktemp)
  head -n "$qwen_anchor" "$qwen_out" > "$tmp"
  tail -n "+$((orig_anchor+1))" "$orig" >> "$tmp"
  mv "$tmp" "$target"
}

# ── Mode dispatchers ──────────────────────────────────────────────────────────

run_mode_e() {
  # Mode E rustdoc on a single source file
  if [ -z "$TARGET_FILE" ]; then
    echo "Mode E requires target_file (e.g. src/lib.rs)" >&2; return 2
  fi
  local src="$CRATE_DIR/$TARGET_FILE"
  if [ ! -f "$src" ]; then
    echo "FATAL: $src does not exist" >&2; return 1
  fi

  log "Mode E: capturing $TARGET_FILE for rustdoc generation"
  local backup="/tmp/run-mode-cycle-${CRATE}-$(basename "$TARGET_FILE")-$(date -u +%s).bak"
  cp "$src" "$backup"

  # Detect if file has a #[cfg(test)] mod tests block — if so, omit from prompt.
  local has_test_mod=0
  grep -q '^#\[cfg(test)\]' "$src" && has_test_mod=1

  local prompt="/tmp/run-mode-cycle-prompt-$$.txt"
  cat > "$prompt" <<EOF
You are Cave Runtime's documentation agent. Add /// rustdoc comments before each pub item and a //! module header. Output the COMPLETE FILE.

ABSOLUTE PRESERVATION RULES (HARD):
A. Every line of input MUST appear in output VERBATIM, byte-for-byte. Output is a STRICT SUPERSET of input — only /// comments and one //! header are added.
B. ALL input use lines MUST appear in the output, in their original order, immediately after the //! header.
C. The #[cfg(test)] mod tests { ... } block, if present, is OMITTED from your output. Stop output at the closing } of the last impl block (before #[cfg(test)]).
D. NO doctests. NO triple-backtick fences of any kind. Doc comments are PROSE ONLY (4-8 lines per ///).
E. NO new use statements. NO new code. NO signature changes.

CONTEXT:
- Crate: $CRATE
- File: $TARGET_FILE
- Workspace: cave-runtime (sovereign Cloud OS in Rust, Hetzner OSS stack)

INPUT FILE (verbatim — your output is this file with /// added):

EOF
  # Append source (if has test mod, only up to but not including it)
  if [ "$has_test_mod" -eq 1 ]; then
    awk '/^#\[cfg\(test\)\]/{exit} {print}' "$src" >> "$prompt"
  else
    cat "$src" >> "$prompt"
  fi
  cat >> "$prompt" <<'EOF'

YOUR OUTPUT FORMAT:
- //! module header (3-5 lines)
- One blank line
- All use lines verbatim, in order
- One blank line
- For each pub item: /// doc block (4-8 lines), then the item verbatim
- End at the closing } of the last impl/fn (before any #[cfg(test)])

Begin output now.
EOF

  log "calling Ollama ($OLLAMA_MODEL, think:false)"
  local out="/tmp/run-mode-cycle-out-$$.rs"
  call_ollama "$prompt" > "$out"
  local sz; sz=$(wc -c < "$out")
  log "Ollama response: $sz bytes"
  if [ "$sz" -lt 200 ]; then
    log "FAIL: response too short ($sz bytes)"
    return 1
  fi

  # Splice + apply
  if [ "$has_test_mod" -eq 1 ]; then
    splice_with_test_mod "$out" "$backup" "$src"
  else
    cp "$out" "$src"
  fi
  log "applied — running gate"

  # Gate sequence
  if ! ( cd "$MAIN_WT" && cargo check -p "$CRATE" --quiet 2>&1 | tail -3 ); then :; fi
  ( cd "$MAIN_WT" && cargo check -p "$CRATE" --quiet ) >/dev/null 2>&1 || { log "GATE check FAIL — reverting"; cp "$backup" "$src"; return 1; }
  ( cd "$MAIN_WT" && cargo doc -p "$CRATE" --no-deps --quiet ) >/dev/null 2>&1 || { log "GATE doc FAIL — reverting"; cp "$backup" "$src"; return 1; }
  ( cd "$MAIN_WT" && cargo test -p "$CRATE" --doc --quiet ) >/dev/null 2>&1 || { log "GATE doctest FAIL — reverting"; cp "$backup" "$src"; return 1; }
  log "GATE PASS — file modified, ready for review/commit"
  rm -f "$prompt" "$out"
  return 0
}

run_mode_h() {
  # Mode H per-crate README
  local readme="$CRATE_DIR/README.md"
  if [ -f "$readme" ]; then
    log "Mode H: README.md already exists, skipping"
    return 0
  fi

  # Read crate description from Cargo.toml
  local desc
  desc=$(grep -E "^description" "$CRATE_DIR/Cargo.toml" | head -1 | sed -E 's/^description[[:space:]]*=[[:space:]]*"(.*)"/\1/')
  log "Mode H: generating README for $CRATE — desc: $desc"

  local prompt="/tmp/run-mode-cycle-prompt-$$.txt"
  cat > "$prompt" <<EOF
Generate a per-crate README.md for cave-runtime crate.

CONSTRAINTS:
- Output ONLY markdown content, no fences wrapping the whole output.
- 50-90 lines, GitHub-flavored markdown.
- NO inline HTML, NO emoji.
- One [text](url) link only (the upstream).
- Tone: technical, no marketing.

CRATE: $CRATE
DESCRIPTION: $desc
WORKSPACE: cave-runtime — sovereign Cloud OS in Rust, Hetzner OSS stack on Linux 7.1

REQUIRED SECTIONS (use # / ## headers, total 8 headers: 1 # + 7 ##):
# $CRATE  (one-line tagline)
## Status  (1-2 sentences: pre-OSS-launch, parity tracked)
## Upstream  (single bullet with the upstream link if known, else "(internal — no external upstream)")
## Surface ported  (5-10 bullets describing major capabilities)
## Public API  (3-6 bullets pointing at top-level pub fn/structs)
## Tests  (1-2 sentences about test coverage)
## License  (Apache-2.0 unless otherwise specified)
## See also  (2-3 adjacent crates with ../crate-X links)

Begin output now.
EOF

  log "calling Ollama"
  local out="/tmp/run-mode-cycle-out-$$.md"
  call_ollama "$prompt" > "$out"
  local sz; sz=$(wc -c < "$out")
  log "response: $sz bytes"
  if [ "$sz" -lt 500 ]; then log "FAIL: response too short"; return 1; fi

  # Quick structure check
  local hdrs; hdrs=$(grep -cE '^# |^## ' "$out")
  if [ "$hdrs" -lt 6 ]; then log "FAIL: only $hdrs headers (need 8)"; return 1; fi

  cp "$out" "$readme"
  log "GATE PASS — README written ($(wc -l < "$readme") lines, $hdrs headers)"
  rm -f "$prompt" "$out"
  return 0
}

run_mode_i() {
  # Mode I Cargo.toml metadata enrichment
  local cargo_toml="$CRATE_DIR/Cargo.toml"
  local backup="/tmp/run-mode-cycle-${CRATE}-cargo-$(date -u +%s).bak"
  cp "$cargo_toml" "$backup"

  # Already has keywords? skip
  if grep -q '^keywords[[:space:]]*=' "$cargo_toml"; then
    log "Mode I: $CRATE already has keywords, skipping"
    return 0
  fi

  local desc
  desc=$(grep -E "^description" "$cargo_toml" | head -1 | sed -E 's/^description[[:space:]]*=[[:space:]]*"(.*)"/\1/')
  log "Mode I: enriching Cargo.toml for $CRATE — desc: $desc"

  local prompt="/tmp/run-mode-cycle-prompt-$$.txt"
  cat > "$prompt" <<EOF
You are Cave Runtime's Cargo.toml metadata agent. Add publish-readiness fields. Output the COMPLETE FILE.

PRESERVATION RULES (HARD):
A. Every line of input MUST appear in output VERBATIM. Output is a STRICT SUPERSET — only new fields added inside [package].
B. New fields go AFTER the description line.
C. Workspace defines: version, edition, license, authors, repository, homepage, rust-version. Use .workspace = true for these.
D. NO duplicate fields. NO changes to [dependencies] or other sections.

CRATE: $CRATE
DESCRIPTION: $desc

REQUIRED FIELDS TO ADD (only those not already in input):
  authors.workspace = true
  repository.workspace = true
  homepage.workspace = true
  documentation = "https://docs.rs/$CRATE"
  readme = "README.md"
  rust-version.workspace = true
  keywords = [<infer 4-5 single-word keywords from description>]
  categories = [<pick 2-3 from: api-bindings, asynchronous, authentication, command-line-utilities, cryptography, database-implementations, development-tools, encoding, network-programming, parser-implementations, web-programming, web-programming::http-server, web-programming::websocket>]

INPUT FILE (verbatim — output is this with new fields added in [package]):

EOF
  cat "$cargo_toml" >> "$prompt"

  log "calling Ollama"
  local out="/tmp/run-mode-cycle-out-$$.toml"
  call_ollama "$prompt" > "$out"
  local sz; sz=$(wc -c < "$out")
  log "response: $sz bytes"

  # Splice: Qwen output's [package] section + original [dependencies] tail.
  # Find anchor: last line of [dependencies] in input (use a robust marker).
  local last_dep_line
  last_dep_line=$(awk '/^\[dependencies\]/,/^\[/{if (/^[a-z][a-z0-9_-]*[[:space:]]*=/) print NR}' "$cargo_toml" | tail -1)
  if [ -z "$last_dep_line" ]; then
    cp "$out" "$cargo_toml"  # whole-file replace
  else
    local anchor; anchor=$(awk -v ln="$last_dep_line" 'NR==ln{print $1; exit}' "$cargo_toml")
    splice_cargo_toml "$out" "$backup" "$anchor" "$cargo_toml" || { log "splice failed — reverting"; cp "$backup" "$cargo_toml"; return 1; }
  fi

  # Gate: cargo check
  ( cd "$MAIN_WT" && cargo check -p "$CRATE" --quiet ) >/dev/null 2>&1 || { log "GATE check FAIL — reverting"; cp "$backup" "$cargo_toml"; return 1; }
  log "GATE PASS — Cargo.toml enriched"
  rm -f "$prompt" "$out"
  return 0
}

run_mode_b_prime() {
  # Mode B-prime: real behavior tests against existing impl
  # Requires per-crate behavior cases — for autonomous use, this should
  # come from a manifest section. For manual invocation, fail informatively.
  log "Mode B-prime requires per-crate behavior cases that the current"
  log "manifest schema does not yet carry. Use the validated session"
  log "pattern: hand-write a prompt at /tmp/mode-b-prime-prompt-<CRATE>.txt"
  log "specifying 5 specific behavior tests against the existing pub API,"
  log "then call this script with that prompt path as TARGET_FILE."
  if [ -n "$TARGET_FILE" ] && [ -f "$TARGET_FILE" ]; then
    log "found custom prompt at $TARGET_FILE — using it"
    local out="/tmp/run-mode-cycle-out-$$.rs"
    call_ollama "$TARGET_FILE" > "$out"
    local sz; sz=$(wc -c < "$out")
    log "response: $sz bytes"
    if [ "$sz" -lt 500 ]; then log "FAIL: response too short"; return 1; fi
    local target_test="$CRATE_DIR/tests/qwen_drafted.rs"
    cp "$target_test" "/tmp/run-mode-cycle-${CRATE}-test-$(date -u +%s).bak" 2>/dev/null || true
    cp "$out" "$target_test"
    ( cd "$MAIN_WT" && cargo check -p "$CRATE" --tests --quiet ) >/dev/null 2>&1 || { log "GATE check FAIL"; return 1; }
    ( cd "$MAIN_WT" && cargo test -p "$CRATE" --test qwen_drafted --quiet ) >/dev/null 2>&1 || { log "GATE test FAIL"; return 1; }
    log "GATE PASS — Mode B-prime tests written"
    rm -f "$out"
    return 0
  fi
  return 2
}

# ── Dispatch ──────────────────────────────────────────────────────────────────

case "$MODE" in
  E|e)         run_mode_e ;;
  H|h)         run_mode_h ;;
  I|i)         run_mode_i ;;
  B-prime|b-prime|B|b) run_mode_b_prime ;;
  *)           echo "Unknown mode: $MODE (valid: B-prime, E, H, I)" >&2; exit 2 ;;
esac
RC=$?
log "exit $RC"
exit $RC
