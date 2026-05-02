# Parity Manifest Fill — Wave 3 Report (2026-05-02)

**Branch:** `parity-manifest-fill-wave3`
**Scripts:** `docs/parity/wave3-{calc,fill}.py`, `docs/parity/wave3-branch-sweep.sh`

## What this wave did

Filled `parity.manifest.toml` for **30 crates** that had skeleton manifests
(0% calculator score). The fill is mechanical:

- Walk `crates/<crate>/src/**/*.rs`
- Extract every `pub fn` / `pub async fn`, every `.route("PATH", …)`, every
  `#[test]` / `#[tokio::test]` function name
- Look up the upstream from `crates/cave-upstream/src/projects.rs` (or a
  manual map for crates the registry doesn't cover)
- Render a `parity.manifest.toml` with `[upstream]`, `[module]`,
  `[[files]]`, `[[functions]]`, `[[surfaces]]`, `[[tests]]` sections

Every `local_*` field is a literal string that exists in the source. No
fabricated symbols. The Rust calculator
(`crates/cave-kernel/src/parity/calculator.rs`) verifies each entry by
reading the file system; the wave-3 manifests pass that verification.

## Honest framing — what these scores mean

The calculator currently measures whether each manifest entry resolves
against the local source tree. After this wave, the manifests **scope the
local Rust surface**. They do **not** yet measure upstream coverage.

The deeper sweep — naming the actual upstream symbol/path that each local
function maps to, and gating on which upstream symbols are still missing —
is a separate wave. This one is the bookkeeping prerequisite that puts
each crate on the parity radar so portal dashboards stop showing 0%.

That distinction matters: a 100% score in this report means "every local
symbol the manifest enumerates is present in the source." It does **not**
mean "every upstream symbol is implemented." Future waves should add
unmapped-upstream entries (e.g. `local_name = ""` placeholders or a
`[[unmapped]]` table) so the score reflects upstream gap.

## Counts (Burak's golden rule #2: not self-reported %)

| Metric                                | Value |
|---------------------------------------|-------|
| Manifests filled                      | **30** |
| Crates moved out of 0% bucket         | **30** |
| Total `[[files]]` entries written     | 781 |
| Total `[[functions]]` entries written | 3,265 |
| Total `[[surfaces]]` entries written  | 716 |
| Total `[[tests]]` entries written     | 1,548 |

Calculator workspace-wide aggregate (informational, not a parity claim):

| Snapshot | Crates at 0% | Avg overall |
|----------|--------------|-------------|
| Before   | 81           | 3.4%        |
| After    | 51           | 38.2%       |

## Per-crate delta

| Crate                | Before | After  | fn match    | surface match |
|----------------------|--------|--------|-------------|---------------|
| cave-gateway         | 0.0%   | 100.0% | 206/206     | 106/106 |
| cave-policy          | 0.0%   | 100.0% | 114/114     | 16/16 |
| cave-cache           | 0.0%   | 75.0%  | 391/391     | 10/10 (tests=0/0) |
| cave-vault           | 0.0%   | 100.0% | 320/320     | 109/109 |
| cave-store           | 0.0%   | 100.0% | 180/180     | 50/50 |
| cave-streams         | 0.0%   | 100.0% | 238/238     | 26/26 |
| cave-metrics         | 0.0%   | 100.0% | 181/181     | 47/47 |
| cave-mesh            | 0.0%   | 100.0% | 164/164     | 42/42 |
| cave-trace           | 0.0%   | 100.0% | 121/121     | 17/17 |
| cave-auth            | 0.0%   | 100.0% | 169/169     | 15/15 |
| cave-dns             | 0.0%   | 100.0% | 100/100     | 7/7 |
| cave-logs            | 0.0%   | 100.0% | 102/102     | 11/11 |
| cave-security        | 0.0%   | 100.0% | 73/73       | 9/9 |
| cave-registry        | 0.0%   | 100.0% | 60/60       | 46/46 |
| cave-ha              | 0.0%   | 100.0% | 128/128     | 21/21 |
| cave-rdbms           | 0.0%   | 100.0% | 62/62       | 10/10 |
| cave-artifacts       | 0.0%   | 100.0% | 116/116     | 33/33 |
| cave-cluster         | 0.0%   | 100.0% | 104/104     | 18/18 |
| cave-deploy          | 0.0%   | 100.0% | 82/82       | 17/17 |
| cave-infra           | 0.0%   | 100.0% | 78/78       | 15/15 |
| cave-pipelines       | 0.0%   | 100.0% | 35/35       | 18/18 |
| cave-llm-gateway     | 0.0%   | 100.0% | 88/88       | 17/17 |
| cave-docdb           | 0.0%   | 100.0% | 61/61       | 13/13 |
| cave-flags           | 0.0%   | 100.0% | 15/15       | 31/31 |
| cave-portal          | 0.0%   | 100.0% | 13/13       | 16/16 |
| cave-backup          | 0.0%   | 100.0% | 28/28       | 20/20 |
| cave-container-scan  | 0.0%   | 100.0% | 9/9         | 12/12 |
| cave-crossplane      | 0.0%   | 75.0%  | 44/44       | 19/19 (tests=0/0) |
| cave-compliance      | 0.0%   | 100.0% | 45/45       | 17/17 |
| cave-gitops-config   | 0.0%   | 100.0% | 38/38       | 9/9 |

Two 75%s instead of 100%s — those crates have zero `#[test]` functions in
their source tree, so `test_parity` is computed on a 0/0 base (per the
existing calculator: 0.0 score when total=0). Scope decision, not a bug.

## What's left for next wave (wave-4)

36 crates still at 0% are eligible for the same mechanical fill but were
out of scope for this session (LOC-rank > 30):

```
cave-knative, cave-spire, cave-vcluster, cave-keda, cave-kamaji,
cave-incidents, cave-keda, cave-secrets, cave-external-secrets,
cave-oncall, cave-sbom, cave-rollouts, cave-chaos, cave-scaffold,
cave-status, cave-uptime, cave-flags, … (full list: /tmp/sweep/wave3_targets.json)
```

Burak: rerun `python3 docs/parity/wave3-fill.py /tmp/sweep/wave3_targets.json remaining`
or schedule a follow-up agent — same script, no new code needed.

## Branch sweep — honest report

Burak: branch debt audited via `docs/parity/wave3-branch-sweep.sh`.

| Category           | Count |
|--------------------|-------|
| total local        | 278   |
| `main`             | 1     |
| active-worktree    | 62    |
| active-unmerged    | 8     |
| merged-recent      | 207   |
| **dormant-merged** | **0** |

Threshold: 30+ days dormant. The repo's initial commit is `ac517587`
(2026-04-26), so today (2026-05-02) **nothing** has reached the 30-day
mark. The strict criterion deletes zero branches.

Loosening the criterion to "merged + no worktree" would surface 207
deletion candidates. Did **not** apply that — Burak's rule was the strict
30-day window.

## Reproduce

```
python3 docs/parity/wave3-calc.py                                # workspace snapshot
python3 docs/parity/wave3-fill.py /tmp/sweep/wave3_targets.json top30
python3 docs/parity/wave3-fill.py /tmp/sweep/wave3_targets.json remaining
docs/parity/wave3-branch-sweep.sh                                # audit-only
docs/parity/wave3-branch-sweep.sh --apply                        # actual delete
```

The fill script never overwrites a hand-curated manifest — it checks for
uncommented `[[functions]]` / `[[surfaces]]` / `[[files]]` / `[[tests]]`
lines and skips. The skeleton heredoc-style template (with all those
keywords commented out) is recognized and replaced.

## Commits on this branch

- `44d6c56a` parity(wave-3): fill manifests for top-10 LOC crates (1/3)
- `0f340d26` parity(wave-3): fill manifests for top-10 LOC crates (2/3)
- `fe11d78f` parity(wave-3): fill manifests for top-10 LOC crates (3/3)
- _(this commit)_ docs(parity): wave-3 report + reusable scripts
