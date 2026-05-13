#!/usr/bin/env bash
# tdd_check.sh — Charter v2 TDD-strict pre-commit / pre-push hook.
#
# Looks at the *currently staged* changes (when invoked from `pre-commit`)
# or the working tree against the merge-base with main (when invoked from
# `pre-push` or manually). If the change touches impl files but not test
# files, refuses the commit unless `CAVE_TDD_SKIP=1` is set.
#
# Install as a hook:
#
#   ln -sf ../../scripts/tdd_check.sh .git/hooks/pre-commit
#   chmod +x scripts/tdd_check.sh
#
# Override per-commit (e.g. for docs-only changes that the heuristic flags):
#
#   CAVE_TDD_SKIP=1 git commit ...
#
# The script intentionally implements a *fast* version of the classifier
# rules baked into `cave_upstream_watchd::charter_gate::classifier`. It does
# not invoke the cave-tdd-check binary — that is meant for CI on the full
# branch, not per-commit. The pre-commit hook is just a guard rail.

set -euo pipefail

# --------------------------------------------------------------------
# Override switch.
# --------------------------------------------------------------------
if [[ "${CAVE_TDD_SKIP:-0}" == "1" ]]; then
    echo "tdd_check: CAVE_TDD_SKIP=1 — skipping TDD-strict gate." >&2
    exit 0
fi

# --------------------------------------------------------------------
# Mode detection — `pre-commit` looks at the index, others at HEAD diff.
# --------------------------------------------------------------------
mode="${1:-pre-commit}"
case "$mode" in
    pre-commit)
        # Files staged for commit. Filter status: A/M/R (deletes ignored).
        files=$(git diff --cached --name-only --diff-filter=AMR || true)
        ;;
    pre-push|manual)
        # Diff against merge-base with main.
        base=$(git merge-base HEAD main 2>/dev/null || git merge-base HEAD origin/main 2>/dev/null || echo "")
        if [[ -z "$base" ]]; then
            echo "tdd_check: cannot find merge-base with main; skipping." >&2
            exit 0
        fi
        files=$(git diff --name-only --diff-filter=AMR "$base"..HEAD || true)
        ;;
    *)
        echo "tdd_check: usage: $0 [pre-commit|pre-push|manual]" >&2
        exit 2
        ;;
esac

if [[ -z "$files" ]]; then
    exit 0
fi

# --------------------------------------------------------------------
# Classify each file (mirrors charter_gate::classifier::classify_file).
# --------------------------------------------------------------------
impl_files=()
test_files=()

is_test() {
    local f="$1"
    # `/tests/` segment or top-level `tests/`
    case "$f" in
        tests/*|*/tests/*) return 0 ;;
    esac
    # filename suffixes
    case "$f" in
        *_test.rs|*_tests.rs|tests.rs|*/tests.rs|*_test.go) return 0 ;;
    esac
    return 1
}

is_code() {
    case "$1" in
        *.rs|*.go|*.ts|*.tsx|*.js|*.jsx|*.py) return 0 ;;
    esac
    return 1
}

while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    if ! is_code "$f"; then
        continue # non-code is ignored
    fi
    if is_test "$f"; then
        test_files+=("$f")
    else
        impl_files+=("$f")
    fi
done <<< "$files"

# --------------------------------------------------------------------
# Rule 1: if any impl files are staged, at least one test file must be
# staged too. Pure-test commits are fine. Pure-noncode commits are fine.
# --------------------------------------------------------------------
if [[ ${#impl_files[@]} -gt 0 && ${#test_files[@]} -eq 0 ]]; then
    cat >&2 <<EOF

✗ Charter v2 TDD gate (tdd_check): impl staged without tests.

  Staged impl files:
$(printf '    %s\n' "${impl_files[@]}")

  Charter §1 (line-by-line TDD) requires a red→green cycle:
    1. commit a failing test
    2. then commit the impl that turns it green

  Add a test for the impl change above, or override with:
    CAVE_TDD_SKIP=1 git commit ...

EOF
    exit 1
fi

# --------------------------------------------------------------------
# Rule 2: warn (but allow) if both impl and test files are staged in the
# *same* commit. Per Charter §1 this is discouraged — the red→green
# cycle is not observable. Hook flow:
#
#   * pre-commit: warn only (otherwise the workflow becomes hostile to
#     people who stash work mid-cycle).
#   * pre-push: same warning, since the branch will be inspected as a
#     whole by `cave-tdd-check` in CI anyway.
# --------------------------------------------------------------------
if [[ ${#impl_files[@]} -gt 0 && ${#test_files[@]} -gt 0 ]]; then
    cat >&2 <<EOF
⚠ Charter v2 TDD gate (tdd_check): impl + tests in same commit.

  This is allowed by the hook but counts against your branch in CI
  (test_first will be false because no test-only commit precedes
  the impl). Consider splitting into two commits.

  Staged impl files (${#impl_files[@]}):
$(printf '    %s\n' "${impl_files[@]}")
  Staged test files (${#test_files[@]}):
$(printf '    %s\n' "${test_files[@]}")

EOF
fi

# --------------------------------------------------------------------
# Rule 3: refuse `#[ignore]` attributes in any staged test file.
# --------------------------------------------------------------------
ignore_hits=()
for f in "${test_files[@]:-}"; do
    [[ -z "$f" || ! -f "$f" ]] && continue
    # match lines that start with whitespace then #[ignore (...) | = ... | ]
    # but not lines starting with `//` (comments).
    while IFS=: read -r lineno content; do
        ignore_hits+=("$f:$lineno: $content")
    done < <(grep -nE '^[[:space:]]*#\[ignore(\]|[[:space:]]|=|\()' "$f" 2>/dev/null \
              | grep -vE '^[[:space:]]*//' || true)
done

if [[ ${#ignore_hits[@]} -gt 0 ]]; then
    cat >&2 <<EOF

✗ Charter v2 TDD gate (tdd_check): #[ignore] attribute(s) found.

$(printf '    %s\n' "${ignore_hits[@]}")

  Ignored tests defeat the gate by silently passing. Remove the
  attribute and either make the test green or delete it.

  Override (NOT recommended) with CAVE_TDD_SKIP=1.

EOF
    exit 1
fi

exit 0
