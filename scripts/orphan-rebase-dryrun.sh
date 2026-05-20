#!/usr/bin/env bash
# orphan-rebase-dryrun.sh — OSS launch orphan-rebase dry-run + real-execute scaffold
#
# Per ADR-148 (OSS Launch History Strategy) the public `main` ships as a single
# orphan commit on 2026-05-21. This script:
#
#   1. Pre-flight: counts total commits on the current branch.
#   2. Backs up the *full* current history as a git bundle in /tmp.
#   3. Creates a `oss-launch-staging` orphan branch carrying exactly one squash
#      commit ("Cave Runtime — initial OSS release 2026-05-21") of the current
#      working tree, then deletes that staging branch (dry-run mode only).
#   4. In --EXECUTE mode it leaves the staging branch in place so the operator
#      can inspect it and force-push manually. THIS SCRIPT NEVER FORCE-PUSHES.
#
# Usage:
#   scripts/orphan-rebase-dryrun.sh                # dry-run (default)
#   scripts/orphan-rebase-dryrun.sh --EXECUTE      # real run, keeps staging branch
#   scripts/orphan-rebase-dryrun.sh --bundle PATH  # override bundle path
#   scripts/orphan-rebase-dryrun.sh --no-bundle    # skip the bundle step
#   scripts/orphan-rebase-dryrun.sh --help
#
# Idempotent: re-running the dry-run is safe; the bundle path is overwritten
# (unless --no-bundle), the staging branch is recreated, and the original
# branch is restored on EXIT via trap.
#
# SAFETY: the script refuses to run if there are untracked files in the work
# tree. The orphan + checkout-back dance commits everything (incl. untracked)
# to the staging branch; on cleanup the checkout-back removes those files
# from the working tree because they were never tracked on the original
# branch. Refusing to run keeps that footgun safe.

set -euo pipefail

readonly STAGING_BRANCH="oss-launch-staging"
readonly COMMIT_MSG="Cave Runtime — initial OSS release 2026-05-21"
readonly DEFAULT_BUNDLE="/tmp/cave-runtime-history-pre-oss-2026-05-19.bundle"

EXECUTE=0
NO_BUNDLE=0
BUNDLE_PATH="${DEFAULT_BUNDLE}"

usage() {
  sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --EXECUTE)   EXECUTE=1; shift ;;
    --bundle)    BUNDLE_PATH="$2"; shift 2 ;;
    --no-bundle) NO_BUNDLE=1; shift ;;
    --help|-h)   usage ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# --- Sanity --------------------------------------------------------------------

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "fatal: not inside a git work tree" >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "fatal: working tree is dirty (tracked changes) — commit or stash before running" >&2
  git status --short >&2
  exit 1
fi

untracked="$(git ls-files --others --exclude-standard)"
if [[ -n "${untracked}" ]]; then
  echo "fatal: untracked files present — commit or remove before running" >&2
  echo "(the orphan/checkout-back dance would consume them; see SAFETY note in script header)" >&2
  echo "${untracked}" | sed 's/^/  /' >&2
  exit 1
fi

ORIGINAL_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD)"
ORIGINAL_HEAD="$(git rev-parse HEAD)"

if git rev-parse --verify --quiet "${STAGING_BRANCH}" >/dev/null; then
  echo "note: pre-existing ${STAGING_BRANCH} found; deleting to start clean"
  git branch -D "${STAGING_BRANCH}" >/dev/null
fi

mode_label=$([[ ${EXECUTE} -eq 1 ]] && echo "EXECUTE" || echo "DRY-RUN")
echo "=== orphan-rebase mode: ${mode_label} ==="
echo "    branch:    ${ORIGINAL_BRANCH} @ ${ORIGINAL_HEAD:0:12}"
echo "    staging:   ${STAGING_BRANCH}"
echo "    commit:    ${COMMIT_MSG}"
echo ""

# --- Trap: restore original branch on EXIT ------------------------------------

restore_branch() {
  local rc=$?
  if [[ "$(git symbolic-ref --short HEAD 2>/dev/null || echo)" != "${ORIGINAL_BRANCH}" ]]; then
    echo ""
    echo "--- restoring original branch: ${ORIGINAL_BRANCH} ---"
    git checkout "${ORIGINAL_BRANCH}" --quiet 2>/dev/null || git checkout "${ORIGINAL_HEAD}" --quiet
  fi
  if [[ ${EXECUTE} -eq 0 ]] && git rev-parse --verify --quiet "${STAGING_BRANCH}" >/dev/null; then
    echo "--- dry-run cleanup: deleting ${STAGING_BRANCH} ---"
    git branch -D "${STAGING_BRANCH}" >/dev/null
  fi
  exit $rc
}
trap restore_branch EXIT

# --- Step 1: pre-flight commit count -------------------------------------------

echo "--- step 1/4: pre-flight ---"
TOTAL_COMMITS="$(git log --oneline | wc -l | tr -d ' ')"
echo "total commits on ${ORIGINAL_BRANCH}: ${TOTAL_COMMITS}"
echo ""

# --- Step 2: backup bundle -----------------------------------------------------

echo "--- step 2/4: backup bundle ---"
if [[ ${NO_BUNDLE} -eq 1 ]]; then
  echo "skipped (--no-bundle)"
else
  echo "writing bundle: ${BUNDLE_PATH}"
  git bundle create "${BUNDLE_PATH}" --all
  bundle_size="$(du -h "${BUNDLE_PATH}" | awk '{print $1}')"
  echo "bundle size: ${bundle_size}"
fi
echo ""

# --- Step 3: orphan + commit ---------------------------------------------------

echo "--- step 3/4: orphan + single-squash commit ---"
git checkout --orphan "${STAGING_BRANCH}" --quiet
git rm -rf --cached . >/dev/null
git add -A
git commit --quiet -m "${COMMIT_MSG}"

staging_count="$(git log "${STAGING_BRANCH}" --oneline | wc -l | tr -d ' ')"
staging_head="$(git rev-parse "${STAGING_BRANCH}")"
echo "${STAGING_BRANCH} commit count: ${staging_count}"
echo "${STAGING_BRANCH} HEAD:         ${staging_head:0:12}"

if [[ "${staging_count}" != "1" ]]; then
  echo "fatal: expected exactly 1 commit on ${STAGING_BRANCH}, got ${staging_count}" >&2
  exit 1
fi
echo ""

# --- Step 4: verdict -----------------------------------------------------------

echo "--- step 4/4: verdict ---"
if [[ ${EXECUTE} -eq 1 ]]; then
  echo "EXECUTE mode — staging branch kept in place."
  echo ""
  echo "next steps (Burak, manual):"
  echo "  git push origin ${STAGING_BRANCH}:main --force-with-lease"
  echo "  git tag -a v0.1.0 -m \"Initial public release\""
  echo "  git push --force --tags origin v0.1.0"
  echo ""
  echo "rollback (within ~30 days):"
  echo "  git clone ${BUNDLE_PATH} /tmp/cave-runtime-restore"
  trap - EXIT
  git checkout "${ORIGINAL_BRANCH}" --quiet
  exit 0
else
  echo "DRY-RUN PASS — ${STAGING_BRANCH} will be deleted on exit."
  echo ""
  echo "to perform the real run:"
  echo "  scripts/orphan-rebase-dryrun.sh --EXECUTE"
fi
