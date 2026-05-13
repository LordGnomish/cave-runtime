#!/usr/bin/env bash
# install-git-hooks.sh — copy tracked hook templates into .git/hooks
# (which is per-clone and not version-controlled).
#
# Run once after cloning:
#   ./scripts/install-git-hooks.sh
#
# Re-run any time the hook source is updated (the script is idempotent).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_SRC_DIR="$REPO_ROOT/scripts/git-hooks"

# Ask git itself for the hooks path — this handles plain clones AND worktrees
# (where .git is a pointer file, not a directory) and respects core.hooksPath
# when set. Falls back to .git/hooks if the command fails for any reason.
if HOOK_DST_DIR="$(cd "$REPO_ROOT" && git rev-parse --git-path hooks 2>/dev/null)"; then
    # `--git-path` returns a path relative to the cwd at invocation time; resolve.
    case "$HOOK_DST_DIR" in
        /*) ;;
        *)  HOOK_DST_DIR="$REPO_ROOT/$HOOK_DST_DIR" ;;
    esac
else
    HOOK_DST_DIR="$REPO_ROOT/.git/hooks"
fi
mkdir -p "$HOOK_DST_DIR"

if [ ! -d "$HOOK_SRC_DIR" ]; then
    echo "❌ no hook source dir at $HOOK_SRC_DIR" >&2
    exit 1
fi
if [ ! -d "$HOOK_DST_DIR" ]; then
    echo "❌ no hooks dir at $HOOK_DST_DIR — are you in a git repo?" >&2
    exit 1
fi
echo "hook destination: $HOOK_DST_DIR"

installed=0
for hook in "$HOOK_SRC_DIR"/*; do
    [ -f "$hook" ] || continue
    name="$(basename "$hook")"
    dst="$HOOK_DST_DIR/$name"
    cp "$hook" "$dst"
    chmod +x "$dst"
    echo "✅ installed $name → $dst"
    installed=$((installed + 1))
done

if [ "$installed" -eq 0 ]; then
    echo "no hooks to install"
else
    echo
    echo "$installed hook(s) installed."
fi
