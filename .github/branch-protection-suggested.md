# Suggested branch protection rules

This file is a **recommendation only**. Applying it requires an admin
token and either the GitHub UI (`Settings → Branches → Branch protection
rules`) or the `gh api` CLI. Nothing in CI mutates branch protection.

Once main is locked down per this spec, every other ray/feature branch
inherits the workflow gates automatically (because PRs target main).

---

## Branch: `main`

### Required pull request before merging

- [x] Require a pull request before merging
- [x] Require approvals: **1** (rises to 2 once a co-maintainer joins)
- [x] Dismiss stale pull-request approvals when new commits are pushed
- [x] Require review from Code Owners (uses `.github/CODEOWNERS`)
- [ ] Require approval of the most recent reviewable push
      (turn on once the team has >1 reviewer)
- [x] Require conversation resolution before merging

### Required status checks

Mark "Require branches to be up to date before merging" and require
these checks to pass (names exactly as they appear in the workflow
`jobs.<id>.name` field):

From `ci.yml`:

- `Format`
- `Check (1.85)`
- `Check (stable)`
- `Clippy`
- `Test`
- `Charter v2 8-gate verifier`
- `Documentation`

From `license.yml`:

- `spdx-headers`
- `cargo-deny`

From `parity-tdd.yml`:

- `TDD-strict gate`
- `Tests + composite gate`
- `tdd_check.sh smoke`

From `parity-index-sync.yml`:

- `verify-in-sync`

From `security.yml`:

- `cargo-deny check advisories`
- `cargo-audit`
- `Trivy filesystem scan`

(`docs.yml` deliberately omitted — markdown-only changes shouldn't
block merge if lychee has a transient upstream failure.)

### Other

- [x] Require signed commits
- [x] Require linear history
- [x] Restrict pushes that create matching branches to `LordGnomish`
- [x] Lock force-pushes (block everyone)
- [x] Lock deletions (block everyone)
- [ ] Allow specified actors to bypass required pull requests
      (leave empty)

---

## Branch: `develop` (if/when adopted)

Same rules as `main` minus signed commits. Keep `develop` for
integration of multi-PR features that don't yet merit a release tag.

---

## How to apply

```bash
# Via gh CLI (requires admin token):
gh api -X PUT \
  repos/LordGnomish/cave-runtime/branches/main/protection \
  -f required_status_checks.strict=true \
  -F required_status_checks.contexts[]="Format" \
  -F required_status_checks.contexts[]="Check (1.85)" \
  ...
```

Or paste the list into `Settings → Branches → Branch protection
rules → main`.

---

## Why this isn't applied by CI

Branch-protection changes are admin-scoped, irreversible-without-audit,
and affect every contributor. They belong in a manually-reviewed
change, not a workflow run. This document is the source of truth; when
it's updated, an admin should reconcile the live settings.
