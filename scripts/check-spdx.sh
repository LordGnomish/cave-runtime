#!/usr/bin/env bash
# check-spdx.sh — every .rs file under crates/ and tools/ must start with
# the AGPL-3.0-or-later SPDX header.
#
# Used by:
#   - .github/workflows/license.yml (CI gate)
#   - scripts/git-hooks/pre-commit  (local hook, installed by
#     scripts/install-git-hooks.sh)
#
# Exits 0 if every file has the header, 1 otherwise (with a list of
# offending files printed to stderr).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

EXPECTED="// SPDX-License-Identifier: AGPL-3.0-or-later"

FILES=$(find crates tools -name '*.rs' -not -path '*/target/*' 2>/dev/null)

bad_count=0
bad_files=()
total=0
for f in $FILES; do
    total=$((total + 1))
    first=$(head -n 1 "$f" 2>/dev/null)
    if [ "$first" != "$EXPECTED" ]; then
        bad_count=$((bad_count + 1))
        bad_files+=("$f")
    fi
done

if [ "$bad_count" -gt 0 ]; then
    echo "❌ $bad_count of $total .rs files missing the AGPL SPDX header:" >&2
    for bf in "${bad_files[@]}"; do
        echo "  - $bf" >&2
    done
    echo >&2
    echo "Add this as the very first line of each file:" >&2
    echo "  $EXPECTED" >&2
    echo "  // Copyright 2026 Cave Runtime contributors" >&2
    exit 1
fi

echo "✅ all $total .rs files carry the AGPL SPDX header"
exit 0
