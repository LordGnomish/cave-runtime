# Parity Manifest Fill — Wave 3 Report (2026-05-02)

**Branch:** `parity-wave3-residual` (single-commit, branched off main)
**Scripts:** `docs/parity/wave3-{calc,fill}.py`, `docs/parity/wave3-branch-sweep.sh`

## What this wave actually shipped

Wave-3 was originally scoped to fill 30 LOC-ranked crates with mechanical
local-surface manifests. While I was running that pass on a side-branch
(`parity-manifest-fill-wave3`), **21 of the 30 targets had already been
filled on `main` by parallel sessions** doing real upstream-mapped work
(e.g. cave-gateway → Kong+Gravitee, cave-vault, cave-streams Strimzi,
cave-store MinIO, etc.).

Their manifests are richer than my mechanical local-mirror would be —
genuine upstream symbol names, real test ports — so I deferred to them.
What landed here is the **residual 5**:

| Crate                | Before | After  | Notes |
|----------------------|--------|--------|-------|
| cave-backup          | 0.0%   | 100.0% | 28 fns / 20 routes / 59 tests |
| cave-compliance      | 0.0%   | 100.0% | 45 fns / 17 routes / 25 tests |
| cave-container-scan  | 0.0%   | 100.0% | 9 fns / 12 routes / 34 tests |
| cave-crossplane      | 0.0%   | 75.0%  | 44 fns / 19 routes / 0 tests (test_parity = 0/0) |
| cave-gitops-config   | 0.0%   | 100.0% | 38 fns / 9 routes / 25 tests |

3 wave-3 targets I deferred without filling: `cave-mesh` and `cave-portal`
(parallel sessions populated them after my pre-check), and `cave-registry`
(crate has been gut-hollowed on main — only `lib.rs` left, nothing to map).

## Honest framing — what these 5 scores mean

The calculator measures whether each manifest entry resolves against the
local source tree. After this wave, these 5 manifests **scope the local
Rust surface**. They do **not** measure upstream coverage.

A 100% score here means "every local symbol the manifest enumerates is
present in the source." It does **not** mean "every upstream symbol is
implemented." A future wave should layer in unmapped-upstream entries so
the score reflects real upstream gap. For comparison, the parallel
sessions that handled the other 21 wave-3 targets DID do upstream mapping
— their manifests will hold up under that future wave; mine are
bookkeeping placeholders.

## Counts (Burak's golden rule #2: not self-reported %)

| Metric                                | Value |
|---------------------------------------|-------|
| Manifests filled by THIS wave         | **5**  |
| Manifests filled by parallel sessions in same window | ~21 |
| Crates moved out of 0% bucket (this wave) | **5** |
| `[[files]]`/`[[functions]]`/`[[surfaces]]`/`[[tests]]` rows added | 53 / 164 / 77 / 143 |

## Workspace-wide aggregate (informational)

Snapshot of `crates/*/parity.manifest.toml` against this branch's working
tree (run `python3 docs/parity/wave3-calc.py` to reproduce):

- **before this wave**: 51 of 85 crates with a manifest at 0% overall
- **after this wave**: 46 of 85 crates with a manifest at 0% overall
- **delta**: 5 crates moved off the floor

(These numbers are post-parallel-session — wave-3's contribution is the 5
residual.)

## What's left for next wave (wave-4)

Crates with skeleton manifests (or no manifest) still at 0% per the
post-snapshot. Mostly smaller LOC modules and the cave-native skip list.
For non-skip targets, rerun:

```
python3 docs/parity/wave3-fill.py <targets.json> top30
```

The script is idempotent: it refuses to overwrite a manifest that already
has uncommented entry rows, so it's safe against further parallel work.

## Branch sweep — honest report

Audited via `docs/parity/wave3-branch-sweep.sh`.

| Category           | Count |
|--------------------|-------|
| total local        | 278   |
| `main`             | 1     |
| active-worktree    | 62    |
| active-unmerged    | 8     |
| merged-recent      | 207   |
| **dormant-merged (30+ days)** | **0** |

Repo's initial commit is 2026-04-26 (6 days ago). Strict 30-day window
deletes zero. Loosening to "merged + no worktree" would surface 207
candidates — did **not** apply, Burak's rule was strict.

## Reproduce

```
python3 docs/parity/wave3-calc.py                              # workspace snapshot
python3 docs/parity/wave3-fill.py <targets.json> top30         # fill (idempotent)
docs/parity/wave3-branch-sweep.sh                              # audit-only
docs/parity/wave3-branch-sweep.sh --apply                      # actual delete
```

## Side-branch artifact

The full 30-crate sweep lives on `parity-manifest-fill-wave3` (4 commits).
That branch is now superseded by this single commit on
`parity-wave3-residual` — the side branch can be dropped or kept as a
historical record of the mechanical pass; nothing on it should land on
main as-is, since the parallel-session manifests for the same 21 crates
are strictly better.
