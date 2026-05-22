#!/usr/bin/env bash
# install-bprime-priority.sh — install run-cycle-bprime-priority.sh as the
# active LaunchAgent target, preserving the existing run-cycle.sh as the
# previous-version backup.
#
# Reversible: restore-original.sh (printed at end) swaps back to run-cycle.sh.
#
# Usage:
#   ./tools/night-pump/install-bprime-priority.sh           # install + swap plist + reload
#   ./tools/night-pump/install-bprime-priority.sh --dry-run # show what would happen
#   ./tools/night-pump/install-bprime-priority.sh --revert  # restore previous run-cycle.sh

set -euo pipefail

PROD_DIR="$HOME/Library/Application Support/cave-qwen-pump"
PLIST="$HOME/Library/LaunchAgents/com.caveruntime.qwen-pump.plist"
NEW_SCRIPT_NAME="run-cycle-bprime-priority.sh"
SRC="$(cd "$(dirname "$0")" && pwd)/$NEW_SCRIPT_NAME"
PROD_NEW="$PROD_DIR/$NEW_SCRIPT_NAME"
PROD_OLD="$PROD_DIR/run-cycle.sh"
LABEL="com.caveruntime.qwen-pump"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
DRY=0
REVERT=0

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    --revert)  REVERT=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

run() {
  if [ "$DRY" -eq 1 ]; then
    echo "DRY: $*"
  else
    eval "$@"
  fi
}

if [ "$REVERT" -eq 1 ]; then
  echo "[revert] stopping LaunchAgent..."
  run "launchctl unload \"$PLIST\" 2>/dev/null || true"
  echo "[revert] restoring plist ProgramArguments to run-cycle.sh..."
  if [ -f "$PLIST.bak.bprime-priority" ]; then
    run "cp \"$PLIST.bak.bprime-priority\" \"$PLIST\""
    echo "[revert] plist restored from .bak.bprime-priority"
  else
    echo "[revert] WARNING: $PLIST.bak.bprime-priority not found; manual edit needed"
  fi
  echo "[revert] reloading LaunchAgent..."
  run "launchctl load \"$PLIST\""
  echo "[revert] done. Active script: $(grep -A1 ProgramArguments \"$PLIST\" | tail -1 | sed -E 's/.*<string>(.*)<\/string>.*/\1/')"
  exit 0
fi

if [ ! -x "$SRC" ]; then
  echo "FATAL: $SRC not executable" >&2; exit 1
fi
if [ ! -f "$PLIST" ]; then
  echo "FATAL: $PLIST not found" >&2; exit 1
fi

echo "[install] copying $NEW_SCRIPT_NAME to production..."
run "cp \"$SRC\" \"$PROD_NEW\""
run "chmod +x \"$PROD_NEW\""

echo "[install] backing up plist to $PLIST.bak.bprime-priority..."
run "cp \"$PLIST\" \"$PLIST.bak.bprime-priority\""

echo "[install] swapping plist ProgramArguments..."
# macOS sed needs `-i ''` for inplace edit; use POSIX-portable form.
run "sed -i .bprime-tmp \"s|/run-cycle.sh|/$NEW_SCRIPT_NAME|g\" \"$PLIST\""
run "rm -f \"$PLIST.bprime-tmp\""

echo "[install] stopping then reloading LaunchAgent..."
run "launchctl unload \"$PLIST\" 2>/dev/null || true"
run "launchctl load \"$PLIST\""

echo
echo "Active script after install:"
grep -A1 ProgramArguments "$PLIST" | tail -1 | sed -E 's/.*<string>(.*)<\/string>.*/  → \1/'
echo
echo "To revert: $0 --revert"
echo "Logs:      tail -f $HOME/Library/Logs/qwen-pump.log"
