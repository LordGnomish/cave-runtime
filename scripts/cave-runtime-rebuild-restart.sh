#!/usr/bin/env bash
# cave-runtime-rebuild-restart.sh — atomic rebuild + restart of the local
# cave-runtime daemon on macOS (launchctl-managed).
#
# Why this exists: portal pages (compliance, /upstream tracker, etc.) read
# manifests + code that are baked into the running cave-runtime binary at
# compile time. When the source improves but the binary doesn't restart,
# the portal silently serves a stale view — which is exactly how Burak
# saw "Cilium 0" for hours after the fix landed on main. This script is
# the one-command rebuild-and-restart for the local daemon.
#
# Usage:
#   ./scripts/cave-runtime-rebuild-restart.sh             # release build
#   ./scripts/cave-runtime-rebuild-restart.sh --debug     # debug build (faster)
#   ./scripts/cave-runtime-rebuild-restart.sh --no-restart # rebuild only
#
# Safety:
#   * Rebuild is in a temp file; the install over /usr/local/bin/cave-runtime
#     is atomic (mv after build, never partial).
#   * Will refuse to install if the build failed.
#   * Will print the current launchctl state before/after so you can spot
#     a missing service.
#   * Errors halt the script (set -euo pipefail).

set -euo pipefail

# -- Resolve repo root + profile --------------------------------------------
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="release"
RESTART=true
for arg in "$@"; do
    case "$arg" in
        --debug)      PROFILE="debug" ;;
        --no-restart) RESTART=false ;;
        --help|-h)
            sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//;/^set -euo/d'
            exit 0 ;;
        *)
            echo "❌ unknown arg: $arg (try --help)" >&2
            exit 1 ;;
    esac
done

# -- Build target paths -----------------------------------------------------
case "$PROFILE" in
    release) BUILD_FLAGS="--release"; BIN_PATH="$REPO_ROOT/target/release/cave-runtime" ;;
    debug)   BUILD_FLAGS="";          BIN_PATH="$REPO_ROOT/target/debug/cave-runtime"   ;;
esac

INSTALL_PATH="/usr/local/bin/cave-runtime"
PLIST_NAME="com.cave.runtime"
PLIST_PATH="$HOME/Library/LaunchAgents/${PLIST_NAME}.plist"
LOG_PATH="$HOME/Library/Logs/cave-runtime.log"

echo "── cave-runtime rebuild + restart ──────────────────────────────────────"
echo "repo:    $REPO_ROOT"
echo "profile: $PROFILE"
echo "binary:  $BIN_PATH"
echo "install: $INSTALL_PATH"
echo "plist:   $PLIST_PATH"
echo

# -- Pre-flight: report current daemon state --------------------------------
echo "── Pre-flight: current daemon state ────────────────────────────────────"
if [ -x "$INSTALL_PATH" ]; then
    echo "installed binary: $(stat -f '%Sm  %z bytes' "$INSTALL_PATH")"
else
    echo "installed binary: (none at $INSTALL_PATH)"
fi
if launchctl print "gui/$(id -u)/${PLIST_NAME}" >/dev/null 2>&1; then
    pid=$(launchctl print "gui/$(id -u)/${PLIST_NAME}" 2>/dev/null | awk '/^[[:space:]]*pid =/ {print $3; exit}')
    if [ -n "${pid:-}" ] && [ "$pid" != "0" ]; then
        echo "launchctl state:  running (pid=$pid)"
    else
        echo "launchctl state:  loaded but not running"
    fi
else
    echo "launchctl state:  not loaded"
fi
echo

# -- Build ------------------------------------------------------------------
echo "── Building cave-runtime ($PROFILE) ────────────────────────────────────"
( cd "$REPO_ROOT" && cargo build -p cave-runtime --bin cave-runtime $BUILD_FLAGS )
if [ ! -x "$BIN_PATH" ]; then
    echo "❌ build did not produce $BIN_PATH" >&2
    exit 1
fi
echo "✅ build OK: $(stat -f '%z bytes' "$BIN_PATH")"
echo

# -- Install (atomic) -------------------------------------------------------
echo "── Installing binary to $INSTALL_PATH ──────────────────────────────────"
# Use mv from same filesystem so the swap is atomic on macOS.
TMP_DEST="${INSTALL_PATH}.new.$$"
if [ ! -w "$(dirname "$INSTALL_PATH")" ]; then
    echo "⚠️  $(dirname "$INSTALL_PATH") needs sudo to write; using sudo for the copy step."
    sudo cp "$BIN_PATH" "$TMP_DEST"
    sudo mv "$TMP_DEST" "$INSTALL_PATH"
else
    cp "$BIN_PATH" "$TMP_DEST"
    mv "$TMP_DEST" "$INSTALL_PATH"
fi
echo "✅ installed: $(stat -f '%z bytes' "$INSTALL_PATH")"
echo

if [ "$RESTART" = false ]; then
    echo "── --no-restart specified; leaving daemon as-is ────────────────────────"
    exit 0
fi

# -- Plist sanity (don't auto-create — explicit user setup) -----------------
if [ ! -f "$PLIST_PATH" ]; then
    echo "⚠️  No launchctl plist at $PLIST_PATH"
    echo "   Run:"
    echo "     mkdir -p \"$HOME/Library/LaunchAgents\""
    echo "     cp scripts/com.cave.runtime.plist.template \"$PLIST_PATH\""
    echo "     # then edit DATA-DIR / PORT / JWT-SECRET inside that file"
    echo "     launchctl bootstrap gui/\$(id -u) \"$PLIST_PATH\""
    echo
    echo "✅ binary installed; daemon-restart step skipped (no plist)."
    exit 0
fi

# -- Restart --------------------------------------------------------------
echo "── Restarting daemon via launchctl ────────────────────────────────────"
# `kickstart -k` restarts an already-loaded service if it exists; otherwise
# bootstrap the plist from scratch. Both are safe — kickstart no-ops if
# the service isn't loaded, and bootstrap fails cleanly if already loaded.
if launchctl print "gui/$(id -u)/${PLIST_NAME}" >/dev/null 2>&1; then
    echo "service already loaded — kickstart -k (graceful restart)"
    launchctl kickstart -k "gui/$(id -u)/${PLIST_NAME}"
else
    echo "service not loaded — bootstrap"
    launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
fi
sleep 1

# -- Post-flight: confirm restart ------------------------------------------
echo
echo "── Post-flight: daemon state ──────────────────────────────────────────"
if launchctl print "gui/$(id -u)/${PLIST_NAME}" >/dev/null 2>&1; then
    pid=$(launchctl print "gui/$(id -u)/${PLIST_NAME}" 2>/dev/null | awk '/^[[:space:]]*pid =/ {print $3; exit}')
    if [ -n "${pid:-}" ] && [ "$pid" != "0" ]; then
        echo "✅ daemon running (pid=$pid)"
    else
        echo "⚠️  service loaded but not running — check $LOG_PATH"
        tail -5 "$LOG_PATH" 2>/dev/null || true
        exit 1
    fi
else
    echo "❌ service is no longer loaded — check $LOG_PATH" >&2
    tail -5 "$LOG_PATH" 2>/dev/null || true
    exit 1
fi

echo
echo "✅ all done — portal at the configured port should now be serving the new binary."
