# ADR-151 — Phantom Crate Audit-Doc Cleanup (ADR-147 Pattern Continued)

**Status:** Accepted — 2026-05-19
**Scope:** Cave Runtime / Compliance & Parity surface
**Category:** Workspace hygiene
**Date:** 2026-05-19
**Related ADRs:**
- ADR-147 — `cave-pg → cave-rdbms-operator` + `cave-iceberg+cave-datafusion → cave-lakehouse` (the rename precedent)
- ADR-012 — vcluster (originating ADR for cave-vcluster)
- ADR-004 — Cilium / Hubble surface routing (Hubble visibility folds into cave-forensics)

## Context

Five crate names remained in `docs/parity/full-audit-2026-05-01.md` (the Tier C audit table) after their on-disk directories were removed:

| Audit-doc name | On-disk dir | Removed by | Successor / absorber |
|---|---|---|---|
| `cave-pg` | absent | ADR-147 rename (commit `d7d6cb61`) | `cave-rdbms-operator` (operator surface) + `cave-rdbms` (engine surface) |
| `cave-spire` | absent | commit `5d6a067b` (2026-05-03 orphan-dir purge) | Deferred to a future `cave-pki` extension. Not on the OSS-launch path. |
| `cave-external-secrets` | absent | commit `5d6a067b` | `cave-vault` (External Secrets surface) per upstream-tracker projects.rs |
| `cave-hubble` | absent | commit `5d6a067b` | `cave-forensics` (Cilium Hubble L7 visibility) per upstream-tracker projects.rs |
| `cave-vcluster` | absent | commit `5d6a067b` | `cave-kamaji` (Hosted Control Plane canonical, per ADR-012 footer) |

These entries were stale: the audit document is the upstream source-of-truth feeding `scripts/build-parity-index.py`, so every regen pulled the phantom rows back into `docs/parity/parity-index.json` and the compliance dashboard. The disk-overlay step downstream tagged them `phantom: true`, but they kept inflating the Tier C denominator (58 crates → actually 53) and the `parity_ratio = 0.0` slot for crates that were not "0.0" so much as "no longer exist."

The pattern was already partially documented in two places:

1. `scripts/build-parity-index.py::overlay_disk_state()` — already flips `phantom: true` when `crates/<name>/` is missing.
2. Commit `5d6a067b` (`chore(workspace): remove 6 orphan crate dirs ...`) — already deleted the dirs with explicit rationale for each: the upstream surface belongs in a different crate (the "absorber" column above).

What was missing: removing the names from the *audit document itself* and recording an ADR so future maintainers do not "fill the manifest" for a crate that does not exist.

## Decision

1. **Remove the 5 phantom rows from `docs/parity/full-audit-2026-05-01.md`** Tier C table (lines previously around 136, 153, 155, 158, 159). Tier C header count `(58 crates)` → `(53 crates)` with an inline note pointing at this ADR.

2. **Codify the cleanup pattern**: when a `crates/<name>/` directory is deleted from disk, the same commit must remove the matching row from `docs/parity/full-audit-2026-05-01.md`. The disk-overlay `phantom: true` flag is a *secondary* safety net, not a substitute for cleaning the audit doc.

3. **Successor mapping**: when an audit-doc row is removed because the upstream surface was absorbed into another crate, the absorber's `parity.manifest.toml` should grow a second-class `[[upstreams]]` block (or an inline `notes` field on the existing block) noting the absorbed identity. This work is deferred per-crate as those crates land their next manifest revision; this ADR does not block on it.

## Consequences

**Positive**:

- `parity-index.json` regen no longer emits 5 phantom entries → 107 real crate slots instead of 112. Dashboard `≥0.95` / `<0.95` / `0.0` counts become honest.
- Future maintainers reading the audit document will not "fill the manifest" for a crate that does not have a directory (a problem hit twice during Wave 3 prep).
- Sets a clear bar for ADR-147 follow-on cleanups: the audit doc is the source-of-truth, edits to it require an ADR pointer.

**Negative**:

- Historical SLO comparisons against `parity-index.json` will see a 5-row delta on 2026-05-19. Documented here so the diff is not surprising.
- The audit doc is now "post-ADR-151 corrected" but otherwise still dated `2026-05-01` in its header — readers must rely on commit history for the corrections trail. Acceptable: the alternative (renaming or republishing the audit doc) would invalidate every existing `parity_ratio_source = "audit"` pointer in the index for the rows that *are* still legitimate audit-frozen entries.

## Out of scope

- `cave-iceberg` / `cave-datafusion`: per ADR-147 these were deprecated-alias crates, but on 2026-05-19 the worktree at HEAD shows them *re-instantiated* with their own manifests and src/. They are real workspace members today (the alias-stub phase is over). They stay in the audit doc.
- `cave-pgbouncer`: the audit doc never carried this name; PgBouncer is the upstream of `cave-rdbms-operator` per ADR-147 §3.1, not its own crate.

## Verification

After this change:

```bash
python3 scripts/build-parity-index.py
python3 -c "import json; d=json.load(open('docs/parity/parity-index.json')); \
  print('phantom:', sum(1 for c in d['crates'].values() if c.get('phantom'))); \
  print('total:', len(d['crates']))"
```

Expected: `phantom: 0`, `total: 107` (was: `phantom: 5`, `total: 112`).

## Status note

This ADR continues the line started by ADR-147 (`cave-pg`, `cave-iceberg`, `cave-datafusion`) and commit `5d6a067b` (the 4 orphan-dir deletions). Together they close out the 2026-Q1 phantom-crate debt. Future deletions in the same vein should reference this ADR rather than re-deriving the rationale.
