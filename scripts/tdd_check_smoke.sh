#!/usr/bin/env bash
# tdd_check_smoke.sh — exercises scripts/tdd_check.sh against scenarios in a
# disposable temp repo. Run manually:
#
#   bash scripts/tdd_check_smoke.sh
#
# Exit 0 = all scenarios behaved as expected. Exit 1 = drift.
set -euo pipefail

HOOK_SRC="$(cd "$(dirname "$0")" && pwd)/tdd_check.sh"
[[ -x "$HOOK_SRC" ]] || { echo "$HOOK_SRC not executable"; exit 1; }

PASS=0
FAIL=0

run_case() {
    local name="$1"
    local expected="$2"   # "pass" or "fail"
    shift 2
    local td
    td=$(mktemp -d)
    pushd "$td" >/dev/null

    git init -q -b main >/dev/null
    git config user.email t@t.t
    git config user.name t
    git config commit.gpgsign false
    echo init > README.md
    git add README.md
    git commit -q -m init >/dev/null

    # Apply scenario setup — function passed as remaining args
    "$@"

    # stage all
    git add -A
    set +e
    "$HOOK_SRC" pre-commit >/tmp/tdd_smoke_stdout 2>/tmp/tdd_smoke_stderr
    rc=$?
    set -e

    local actual
    if [[ $rc -eq 0 ]]; then actual="pass"; else actual="fail"; fi

    if [[ "$actual" == "$expected" ]]; then
        echo "✓ $name (expected=$expected, got=$actual)"
        PASS=$((PASS + 1))
    else
        echo "✗ $name (expected=$expected, got=$actual)"
        echo "  stderr:"; sed 's/^/    /' /tmp/tdd_smoke_stderr | head -20
        FAIL=$((FAIL + 1))
    fi

    popd >/dev/null
    rm -rf "$td"
}

# --- scenarios ----------------------------------------------------------

scenario_pure_test() {
    mkdir -p crates/x/tests
    echo "fn t() {}" > crates/x/tests/a.rs
}

scenario_pure_impl_no_tests() {
    mkdir -p crates/x/src
    echo "pub fn ok() {}" > crates/x/src/lib.rs
}

scenario_impl_with_tests() {
    mkdir -p crates/x/src crates/x/tests
    echo "pub fn ok() {}" > crates/x/src/lib.rs
    echo "fn t() {}" > crates/x/tests/a.rs
}

scenario_ignore_attr() {
    mkdir -p crates/x/tests
    cat > crates/x/tests/a.rs <<'EOF'
#[test]
#[ignore = "later"]
fn t() {}
EOF
}

scenario_docs_only() {
    mkdir -p docs
    echo "# notes" > docs/notes.md
}

run_case "pure-test commit" pass scenario_pure_test
run_case "pure-impl commit (no tests)" fail scenario_pure_impl_no_tests
run_case "impl + tests same commit (allowed, warned)" pass scenario_impl_with_tests
run_case "ignore attr in staged test" fail scenario_ignore_attr
run_case "docs-only commit" pass scenario_docs_only

echo ""
echo "summary: $PASS passed / $FAIL failed"
[[ $FAIL -eq 0 ]] || exit 1
