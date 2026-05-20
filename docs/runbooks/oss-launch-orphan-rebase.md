# OSS Launch — Orphan Rebase Runbook

**Audience:** Burak (release operator).
**Source of authority:** ADR-148 (OSS Launch History Strategy).
**Target launch date:** 2026-05-21.
**Script:** `scripts/orphan-rebase-dryrun.sh`.

This runbook is the launch-day checklist for collapsing Cave Runtime's
prototype history into a single orphan commit on the public `main`. The
strategy itself is documented and accepted in ADR-148 — this file only
covers the **how**.

---

## TL;DR

```sh
# Dry-run (safe, idempotent, leaves repo state untouched):
scripts/orphan-rebase-dryrun.sh

# Real run (keeps a local oss-launch-staging branch, still no push):
scripts/orphan-rebase-dryrun.sh --EXECUTE

# Force-push to public main (Burak, manual, after EXECUTE):
git push origin oss-launch-staging:main --force-with-lease
git tag -a v0.1.0 -m "Initial public release"
git push --force --tags origin v0.1.0
```

The script never pushes. The force-push step is reserved for Burak's
explicit launch-day command.

---

## 1. Backup bundle

Before any rewrite happens, the dry-run/EXECUTE both write the **full
current history** (all refs, all branches, all tags) to a single git
bundle:

```
/tmp/cave-runtime-history-pre-oss-2026-05-19.bundle
```

Override the path with `--bundle PATH`, or skip the step with
`--no-bundle` (not recommended for the real run).

The bundle is a self-contained restorable archive:

```sh
# Inspect the bundle:
git bundle list-heads /tmp/cave-runtime-history-pre-oss-2026-05-19.bundle

# Restore into a fresh working copy:
git clone /tmp/cave-runtime-history-pre-oss-2026-05-19.bundle \
    /tmp/cave-runtime-restore
```

**Post-launch:** copy the bundle off `/tmp` (it is volatile on macOS) to
your long-term storage. ADR-148 commits us to keeping it locally for at
least 30 days; Burak's discretion thereafter.

The bundle is intentionally **not** committed to the repo:

- It is large (full pack of ~hundreds of MB).
- Committing it would defeat the orphan strategy by re-embedding history.
- `/tmp` is already outside the work tree (no `.gitignore` entry needed).

---

## 2. Real execute — `--EXECUTE`

When you are ready (launch day, working tree clean, on a branch you
control):

```sh
scripts/orphan-rebase-dryrun.sh --EXECUTE
```

What this does:

1. Confirms working tree is clean **and there are no untracked files**.
2. Records pre-flight commit count.
3. Writes the backup bundle (skippable with `--no-bundle`).
4. `git checkout --orphan oss-launch-staging`
5. `git rm -rf --cached . && git add -A`
6. `git commit -m "Cave Runtime — initial OSS release 2026-05-21"`
7. Verifies the staging branch has exactly 1 commit.
8. Leaves the `oss-launch-staging` branch in place.
9. Switches back to your original branch.

The staging branch is **local only** at this point. Nothing has been
pushed. Verify:

```sh
git log oss-launch-staging --oneline
# → exactly one line: "Cave Runtime — initial OSS release 2026-05-21"

git diff oss-launch-staging -- . | head
# → empty (working tree at HEAD == staging tree)
```

### Why untracked files block the script

The orphan checkout keeps the working tree intact, but `git add -A`
commits any untracked files to the staging branch. When the script
restores the original branch via `git checkout`, those files are tracked
on staging but not on the target — git removes them from the work tree.
After `git branch -D oss-launch-staging` they are gone for good. The
script refuses to start in this state to prevent silent data loss.

---

## 3. Force-push (Burak, manual)

ADR-148 specifies a single 5-minute force-push window on launch day,
*before* the public announcement. The exact commands:

```sh
# Push the orphan commit to public main:
git push origin oss-launch-staging:main --force-with-lease

# Tag the launch:
git tag -a v0.1.0 -m "Initial public release"
git push --force --tags origin v0.1.0
```

`--force-with-lease` is preferred over `--force`: it refuses to push if
someone else has advanced `origin/main` since you last fetched, which is
the only safety net we have against a race.

After the push:

```sh
# Re-fetch and verify:
git fetch origin --prune
git log origin/main --oneline
# → exactly one line
```

---

## 4. Rollback

If the force-push goes out and you need to revert *before*
announcement:

```sh
# Restore from the bundle:
git clone /tmp/cave-runtime-history-pre-oss-2026-05-19.bundle \
    /tmp/cave-runtime-restore
cd /tmp/cave-runtime-restore

# Force-push the old history back:
git remote set-url origin <github-url>
git push origin main --force-with-lease
git push origin --force --tags
```

This window closes the moment external clones happen. ADR-148 already
calls this out: "Any OSS launch follower who clones before vs after
force-push will diverge. Mitigation: force-push happens during a single
5-minute window on launch day, before public announcement."

---

## 5. Post-launch — historical bundle storage

The bundle in `/tmp` survives reboots on most macOS configs but is not
guaranteed beyond ~3 days. After the launch is announced and stable:

1. Copy the bundle to your archival storage of choice (external drive,
   private cloud, ...).
2. Optionally re-bundle with `git bundle verify` proof to keep alongside.
3. Update ADR-148 with the final archival location once chosen.

---

## 6. Idempotency / safety notes

- The script aborts if the working tree is dirty.
- The script aborts if untracked files exist (see §2 footnote).
- The dry-run uses `trap restore_branch EXIT` to always return to the
  original branch and delete the staging branch on exit, even on error.
- A pre-existing `oss-launch-staging` branch is deleted at start so
  re-runs are reproducible.
- The script never invokes `git push` of any kind.
- The script never modifies `origin/*` refs.
- The script never modifies `main` (it operates on the staging branch).
- `sudo` is not used.

---

## 7. Cross-references

- ADR-148 — OSS Launch History Strategy (rationale + alternatives).
- `docs/OSS_RELEASE_PLAN.md` — full launch plan.
- `docs/oss-launch-final-audit-2026-05-19.md` — pre-launch numerical
  audit (tests, warnings, fill ratios).
- `scripts/oss-hijyen-prep.sh` — path/email leak scrubber (run first).
