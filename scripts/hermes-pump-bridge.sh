#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# hermes-pump-bridge.sh
#
# Bridges the cave-runtime qwen-pump (worktree dispatch) with the Hermes Agent
# orchestrator running on the host. Read-only against pump state by default;
# all write actions (Hermes invocations, pump nudges) are gated behind explicit
# subcommands and dry-run flags until Burak signs off.
#
# Created:    2026-05-19 (A+C path — Hermes Agent adoption)
# Owner:      Burak (btartan@gmail.com)
# ADR:        docs/adr/ADR-026_Hermes_Agent_Adoption.md
# launchd:    ~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist
#             (Disabled=true on disk — Burak flips it manually)
#
# PERMISSIONS: this file ships as chmod 644 (NOT executable). Burak runs it
# explicitly via `bash scripts/hermes-pump-bridge.sh <cmd>` until he chmod +x
# and enables the launchd unit. This avoids accidental cron / hook invocations.
#
# RED LINES (do not relax without ADR amendment):
#   * NEVER write to pump dispatch queues — read-only inspection only.
#   * NEVER touch ANTHROPIC_API_KEY — Hermes reads it from ~/.hermes/.env.
#   * NEVER kill / unload the existing watchd / qwen-pump / ollama units.
#   * NEVER drop or rewrite worktrees under cave-runtime / cave-hermes.
#   * NEVER hit /Library/Caches or ~/Library/Caches.
# -----------------------------------------------------------------------------

set -euo pipefail

# -----------------------------------------------------------------------------
# Config (overridable via env)
# -----------------------------------------------------------------------------

HERMES_BIN="${HERMES_BIN:-$HOME/.local/bin/hermes}"
HERMES_HOME_DIR="${HERMES_HOME:-$HOME/.hermes}"
PUMP_STATE_DIR="${PUMP_STATE_DIR:-$HOME/Library/Application Support/cave-runtime}"
LOG_DIR="${LOG_DIR:-$HOME/Library/Application Support/cave-runtime/hermes-orchestrator}"
STUCK_THRESHOLD_SECONDS="${STUCK_THRESHOLD_SECONDS:-900}"   # 15 min default
ORCHESTRATE_INTERVAL_SECONDS="${ORCHESTRATE_INTERVAL_SECONDS:-120}"
DRY_RUN="${DRY_RUN:-1}"   # default ON — explicit DRY_RUN=0 to act

# Cave-runtime worktrees we will *observe* but never write to.
PUMP_LABELS=(
  "com.btartan.cave-upstream-watchd"
  "com.cave.upstream-watchd-poller"
  "com.caveruntime.local-llm-daemon"
  "com.caveruntime.qwen-pump"
  "com.caveruntime.qwen-pump-refiller"
  "homebrew.mxcl.ollama"
)

# -----------------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------------

mkdir -p "$LOG_DIR"

log()  { printf '[%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_DIR/orch.log" >&2; }
warn() { printf '[%s] WARN: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_DIR/orch.err" >&2; }
die()  { warn "$@"; exit 1; }

require_hermes() {
  [[ -x "$HERMES_BIN" ]] || die "hermes binary not found at $HERMES_BIN (run install.sh first)"
}

# Read-only: enumerate pump units and last-modified-time of their log files
# as a proxy for liveness. Does not invoke launchctl write paths.
inspect_pump() {
  local now ts age last_log
  now="$(date +%s)"
  for label in "${PUMP_LABELS[@]}"; do
    last_log="$(/bin/ls -1t "$PUMP_STATE_DIR"/*/"$label".log 2>/dev/null \
                | head -n1 || true)"
    if [[ -z "$last_log" ]]; then
      # Fall back to the canonical watchd location for the known unit.
      if [[ "$label" == "com.btartan.cave-upstream-watchd" ]]; then
        last_log="$PUMP_STATE_DIR/watchd/watchd.log"
      fi
    fi
    if [[ -n "${last_log:-}" && -f "$last_log" ]]; then
      ts=$(stat -f %m "$last_log" 2>/dev/null || echo "0")
      age=$(( now - ts ))
      printf '  %-44s last_log_age=%ds  (%s)\n' "$label" "$age" "$last_log"
    else
      printf '  %-44s [no log file located]\n' "$label"
    fi
  done
}

# Identifies pump units whose log hasn't ticked in > STUCK_THRESHOLD_SECONDS.
# Output: one label per line. Does not act.
list_stuck() {
  local now ts age last_log
  now="$(date +%s)"
  for label in "${PUMP_LABELS[@]}"; do
    last_log="$(/bin/ls -1t "$PUMP_STATE_DIR"/*/"$label".log 2>/dev/null \
                | head -n1 || true)"
    [[ -z "${last_log:-}" || ! -f "$last_log" ]] && continue
    ts=$(stat -f %m "$last_log" 2>/dev/null || echo "0")
    age=$(( now - ts ))
    if (( age > STUCK_THRESHOLD_SECONDS )); then
      printf '%s\t%d\n' "$label" "$age"
    fi
  done
}

# Routes a hypothetical task to the right tier. Today this only LOGS the
# decision (DRY_RUN=1 by default). When Burak flips DRY_RUN=0 + has
# ANTHROPIC_API_KEY wired, this will shell into `hermes -z` with the chosen
# model. Tier rules:
#   refactor / small-edit / smoke      -> Tier-1 (Qwen local)
#   feature / multi-file / new-module  -> Tier-2 Sonnet 4.6
#   architectural / cross-cutting      -> Tier-2 Opus 4.7
route_task() {
  local kind="${1:?usage: route_task <kind> <prompt>}"
  local prompt="${2:?usage: route_task <kind> <prompt>}"
  local model

  case "$kind" in
    refactor|small-edit|smoke|format)
      model="qwen3.6:35b-a3b-coding-mxfp8"
      ;;
    feature|multi-file|new-module)
      model="claude-sonnet-4.6"   # requires Tier-2 active
      ;;
    architectural|cross-cutting|design)
      model="claude-opus-4.7"     # requires Tier-2 active
      ;;
    *)
      die "route_task: unknown kind '$kind' (refactor|feature|architectural|...)"
      ;;
  esac

  log "route_task kind=$kind model=$model dry_run=$DRY_RUN"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "  DRY_RUN — would invoke: hermes -z [redacted prompt] -m $model"
    return 0
  fi

  require_hermes
  "$HERMES_BIN" -z "$prompt" -m "$model"
}

# Append a dispatch record to Hermes's persistent memories store. Hermes's
# own memory layer takes over from here. Format mirrors the conversation
# memory schema (markdown + simple key/value frontmatter).
record_dispatch() {
  local task_id="${1:?usage: record_dispatch <task_id> <worktree> <commit>}"
  local worktree="${2:?}"
  local commit="${3:?}"
  local mem_dir="$HERMES_HOME_DIR/memories/pump-dispatch"
  local mem_file="$mem_dir/${task_id}.md"

  mkdir -p "$mem_dir"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "DRY_RUN — would write $mem_file"
    return 0
  fi

  cat > "$mem_file" <<MEM
---
task_id: $task_id
worktree: $worktree
commit: $commit
recorded_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)
source: hermes-pump-bridge
---

Dispatch record for $task_id → $worktree → $commit.
MEM
  log "recorded dispatch $task_id"
}

# Long-running loop used by the launchd unit. Inspects pump state, logs
# stuck units, never restarts anything until Burak flips DRY_RUN.
orchestrate_loop() {
  log "hermes-pump-bridge orchestrate loop start (interval=${ORCHESTRATE_INTERVAL_SECONDS}s, dry_run=$DRY_RUN)"
  log "  stuck threshold = ${STUCK_THRESHOLD_SECONDS}s"
  log "  watching: ${PUMP_LABELS[*]}"

  while :; do
    local stuck_lines
    stuck_lines="$(list_stuck || true)"
    if [[ -n "$stuck_lines" ]]; then
      while IFS=$'\t' read -r label age; do
        warn "STUCK $label age=${age}s — would request Hermes recovery (dry_run=$DRY_RUN)"
        if [[ "$DRY_RUN" != "1" ]]; then
          # When live: ask Hermes to analyse + suggest a fix. NEVER force-
          # unload the existing unit; let Burak's watchd/refiller cycle.
          require_hermes
          "$HERMES_BIN" -z \
            "Pump unit $label has been idle for ${age}s on cave-runtime host. \
Summarise the likely cause from its logs at $PUMP_STATE_DIR and propose \
a minimal recovery action that does NOT unload the unit." \
            -m "qwen3.6:35b-a3b-coding-mxfp8" \
            >> "$LOG_DIR/recovery.log" 2>&1 || warn "hermes recovery call failed"
        fi
      done <<< "$stuck_lines"
    else
      log "all pump units within liveness window"
    fi
    sleep "$ORCHESTRATE_INTERVAL_SECONDS"
  done
}

# -----------------------------------------------------------------------------
# Subcommand dispatch
# -----------------------------------------------------------------------------

usage() {
  cat <<USAGE
hermes-pump-bridge.sh — Hermes Agent ↔ cave-runtime pump bridge

USAGE:
  bash scripts/hermes-pump-bridge.sh <command> [args]

COMMANDS:
  status                          Show pump unit liveness (read-only)
  stuck                           List units exceeding STUCK_THRESHOLD_SECONDS
  orchestrate                     Long-running loop (used by launchd plist)
  route <kind> <prompt>           Route a task: refactor|feature|architectural
  record <task_id> <wt> <commit>  Record dispatch into Hermes memories
  smoke                           One-shot Hermes call (tier-1, dry-run safe)
  help                            Show this message

ENV:
  HERMES_BIN                      Default: ~/.local/bin/hermes
  HERMES_HOME                     Default: ~/.hermes
  PUMP_STATE_DIR                  Default: ~/Library/Application Support/cave-runtime
  STUCK_THRESHOLD_SECONDS         Default: 900
  ORCHESTRATE_INTERVAL_SECONDS    Default: 120
  DRY_RUN                         Default: 1 (set to 0 to act; do this only
                                  after Burak has wired ANTHROPIC_API_KEY)

USAGE
}

cmd="${1:-status}"
shift || true

case "$cmd" in
  status)
    log "hermes-pump-bridge status"
    inspect_pump
    ;;
  stuck)
    list_stuck
    ;;
  orchestrate)
    orchestrate_loop
    ;;
  route)
    route_task "$@"
    ;;
  record)
    record_dispatch "$@"
    ;;
  smoke)
    require_hermes
    log "smoke: invoking tier-1 via $HERMES_BIN"
    "$HERMES_BIN" -z "Reply with exactly: HERMES_BRIDGE_SMOKE_OK. Nothing else."
    ;;
  help|-h|--help)
    usage
    ;;
  *)
    warn "unknown command: $cmd"
    usage
    exit 2
    ;;
esac
