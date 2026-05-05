#!/usr/bin/env bash
# run-cycle-multimode.sh — autonomous multi-mode pump cycle.
#
# When LaunchAgent fires this script (no args), one cycle runs:
#   1. flock single-flight lock
#   2. pop head of queue.txt
#   3. discover mode for the crate (state-based heuristic)
#   4. dispatch mode-specific prompt → Ollama → apply → gate
#   5. on success: commit on feature branch, merge to main, contributions.jsonl
#   6. on failure: re-queue (or auto-drop on phantom strike-3)
#
# Modes handled:
#   E  rustdoc gen on src/lib.rs (or first undocumented .rs file)
#   H  per-crate README.md generation
#   I  Cargo.toml metadata enrichment
#   A  scaffold tests (legacy fallback — same surface as old run-cycle.sh)
#
# Mode B-prime is NOT yet autonomous (needs per-crate behavior cases from
# manifest schema extension; until then, B-prime is manual via run-mode-cycle.sh).
#
# Backup of pre-multi-mode run-cycle.sh lives at:
#   ~/Library/Application Support/cave-qwen-pump/run-cycle.sh.bak.<ts>-pre-multimode

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
QUEUE="$SCRIPT_DIR/queue.txt"
LOG_DIR="$SCRIPT_DIR/log"
CYCLE_LOG="$LOG_DIR/run-cycle.log"
CONTRIB="$SCRIPT_DIR/contributions.jsonl"
LOCK_DIR="$SCRIPT_DIR/run-cycle.lockdir"
PHANTOM_TSV="$SCRIPT_DIR/phantom-counter.tsv"
REPO_ROOT="${QWEN_PUMP_REPO_ROOT:-/Users/gnomish/Code/cave-runtime}"

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

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*" | tee -a "$CYCLE_LOG" >&2; }

# ── Phantom counter ──────────────────────────────────────────────────────────
get_phantom_count() { grep -E "^$1[[:space:]]" "$PHANTOM_TSV" 2>/dev/null | head -1 | awk '{print $2}'; }
inc_phantom_count() {
  local c="$1" n; n=$(get_phantom_count "$c"); n=${n:-0}; n=$((n+1))
  awk -v c="$c" -v n="$n" '$1!=c{print} END{print c"\t"n}' "$PHANTOM_TSV" > "$PHANTOM_TSV.tmp" && mv "$PHANTOM_TSV.tmp" "$PHANTOM_TSV"
  echo "$n"
}
clear_phantom_count() {
  awk -v c="$1" '$1!=c{print}' "$PHANTOM_TSV" > "$PHANTOM_TSV.tmp" && mv "$PHANTOM_TSV.tmp" "$PHANTOM_TSV"
}

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

# Priority sort by manifest pump_priority (HIGH | MEDIUM | LOW; default LOW).
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

# ── Mode discovery ───────────────────────────────────────────────────────────
# Order of precedence (highest leverage first):
#   I  if Cargo.toml has no `keywords` field
#   H  if no README.md
#   E  if src/lib.rs exists AND fewer than 30% of pub items have ///
#   A  fallback (legacy scaffold)
discover_mode() {
  # I — metadata
  if [ -f "$CARGO_TOML" ] && ! grep -qE '^[[:space:]]*keywords[[:space:]]*=' "$CARGO_TOML"; then
    echo "I"; return
  fi
  # H — README
  if [ ! -f "$CRATE_DIR/README.md" ]; then
    echo "H"; return
  fi
  # E — rustdoc on lib.rs
  if [ -f "$LIB_RS" ]; then
    local pubs; pubs=$(grep -cE '^pub (fn|struct|enum|trait|use|mod|const|static|type)' "$LIB_RS" 2>/dev/null)
    local docs; docs=$(grep -cE '^[[:space:]]*///' "$LIB_RS" 2>/dev/null)
    if [ "$pubs" -gt 5 ] && [ "$docs" -lt $((pubs / 3 + 1)) ]; then
      echo "E"; return
    fi
  fi
  # All E/H/I satisfied — crate is "mature" wrt this script's scope.
  # Don't spin Mode A scaffold; just signal DONE so we exit cleanly without
  # re-queuing.
  echo "DONE"
}

MODE="$(discover_mode)"
log "mode: $MODE"

# Crate is mature wrt E/H/I — exit cleanly without re-queuing.
if [ "$MODE" = "DONE" ]; then
  log "DONE: $crate has Cargo.toml metadata + README + rustdoc; nothing to add"
  clear_phantom_count "$crate"
  exit 0
fi

# ── Worktree auto-sync (ensure MAIN_WT is at main HEAD) ─────────────────────
git -C "$MAIN_WT" reset --hard main >>"$CYCLE_LOG" 2>&1 || \
  log "warn: auto-sync of main worktree failed; merge may collide"

# ── Branch + worktree for this cycle ────────────────────────────────────────
WT_BR="qwen/pump-${crate}-mode${MODE}-$(date -u +%Y%m%d-%H%M%S)"
WT_DIR="$REPO_ROOT/.claude/worktrees/qwen-pump-${crate}-mode${MODE}-$(date -u +%H%M%S)"
git -C "$REPO_ROOT" worktree add "$WT_DIR" -b "$WT_BR" main >>"$CYCLE_LOG" 2>&1 || {
  log "fail: could not create worktree (re-queue $crate)"
  echo "$crate" >> "$QUEUE"; exit 0
}

cleanup_failure() {
  local reason="$1"
  log "FAIL ($reason) — re-queue $crate at tail, drop worktree+branch"
  echo "$crate" >> "$QUEUE"
  git -C "$REPO_ROOT" worktree remove --force "$WT_DIR" 2>/dev/null || true
  git -C "$REPO_ROOT" branch -D "$WT_BR" 2>/dev/null || true
}

# ── Ollama call wrapper ─────────────────────────────────────────────────────
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

# Helper: extract `description = "..."` from Cargo.toml.
crate_description() {
  grep -E "^description" "$CARGO_TOML" | head -1 | sed -E 's/^description[[:space:]]*=[[:space:]]*"(.*)"/\1/'
}

# ── Mode I: Cargo.toml metadata ─────────────────────────────────────────────
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

  # Splice: Qwen output's [package] head + original [dependencies] tail.
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

  # Gate: cargo check inside worktree
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode I gate FAIL (cargo check)"; return 1
  fi
  COMMIT_FILES=("crates/$crate/Cargo.toml")
  COMMIT_TYPE="build"
  COMMIT_MSG="Mode I: enrich Cargo.toml metadata (keywords, categories, repository, docs)"
  return 0
}

# ── Mode H: per-crate README ───────────────────────────────────────────────
run_mode_H() {
  if [ -f "$CRATE_DIR/README.md" ]; then return 1; fi  # already has one
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
  # Pick the src file with the biggest pub-vs-/// gap
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

  # Detect test mod presence
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

  # Splice with test mod
  local target_in_wt="$WT_DIR/crates/$crate/$rel"
  cat "$out_rs" > "$target_in_wt"
  if [ "$has_test_mod" -eq 1 ]; then
    echo "" >> "$target_in_wt"
    awk '/^#\[cfg\(test\)\]/{found=1} found{print}' "$target" >> "$target_in_wt"
  fi

  # Gate: check + doc + doctest
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo check -p "$crate" --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo check)"; return 1
  fi
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo doc -p "$crate" --no-deps --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo doc)"; return 1
  fi
  if ! ( cd "$WT_DIR" && CARGO_TARGET_DIR="$SHARED_TARGET" cargo test -p "$crate" --doc --quiet ) >>"$CYCLE_LOG" 2>&1; then
    log "Mode E gate FAIL (cargo test --doc)"; return 1
  fi
  COMMIT_FILES=("crates/$crate/$rel")
  COMMIT_TYPE="docs"
  COMMIT_MSG="Mode E: rustdoc gen on $rel (gap=$max_gap → 0)"
  return 0
}

# ── Mode A: scaffold (legacy fallback) ─────────────────────────────────────
# Skip Mode A in this multimode script; mode discovery only routes to A
# when E/H/I have nothing useful to do, which means the crate already has
# README + metadata + rustdoc — a near-mature crate. In that case we
# explicitly EXIT (queue moves on to next crate) rather than spinning Mode A
# scaffold. If Burak wants Mode A coverage, the legacy run-cycle.sh still
# exists at the same path as a `.bak.*-pre-multimode` backup.
run_mode_A() {
  log "Mode A: skipping (legacy scaffold; crate already has E/H/I work done)"
  return 1
}

# ── Dispatch ────────────────────────────────────────────────────────────────
COMMIT_FILES=()
COMMIT_TYPE=""
COMMIT_MSG=""
case "$MODE" in
  I) run_mode_I ;;
  H) run_mode_H ;;
  E) run_mode_E ;;
  A) run_mode_A ;;
  *) log "unknown mode $MODE"; cleanup_failure "unknown mode $MODE"; exit 0 ;;
esac
RC=$?

if [ "$RC" -ne 0 ] || [ -z "$COMMIT_TYPE" ]; then
  cleanup_failure "Mode $MODE returned $RC"
  exit 0
fi

# ── Commit + merge ──────────────────────────────────────────────────────────
clear_phantom_count "$crate"
( cd "$WT_DIR" \
  && git add "${COMMIT_FILES[@]}" \
  && git commit -m "$COMMIT_TYPE($crate): $COMMIT_MSG

Generated by $OLLAMA_MODEL via local Ollama (Mode $MODE).
Posted by tools/night-pump/run-cycle-multimode.sh." ) >>"$CYCLE_LOG" 2>&1 || {
  cleanup_failure "commit failed"; exit 0
}
SHA="$(git -C "$WT_DIR" rev-parse HEAD)"

git -C "$MAIN_WT" reset --hard main >>"$CYCLE_LOG" 2>&1 || true
if ! git -C "$MAIN_WT" merge --no-ff "$WT_BR" \
       -m "Merge $WT_BR: Mode $MODE for $crate" >>"$CYCLE_LOG" 2>&1; then
  log "fail: main merge — branch $WT_BR retained for manual cleanup"
  git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
  exit 0
fi

# Contributions log
jq -nc \
  --arg ts "$(ts)" \
  --arg crate "$crate" \
  --arg sha "$SHA" \
  --arg br "$WT_BR" \
  --arg model "$OLLAMA_MODEL" \
  --arg mode "$MODE" \
  '{ts:$ts, worker_id:"qwen-pump-multimode", crate:$crate, mode:$mode,
    commit_sha:$sha, branch:$br, merged_to:"main", model:$model}' \
  >> "$CONTRIB"

git -C "$REPO_ROOT" worktree remove "$WT_DIR" 2>/dev/null || true
log "OK $crate Mode $MODE sha=$SHA"
exit 0
