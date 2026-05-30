<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-karpenter deep-port — continuation ray #2 progress

**Branch:** `claude/cave-karpenter-honest-100-cont2-2026-05-28`
**Base:** `dcd82fb1` (continuation ray #1, 4 strict-TDD cycles, LOC ratio 0.0854)
**Upstream:** kubernetes-sigs/karpenter **v1.12.1** (sha `ed490e8`, Apache-2.0)
**Scope:** core port, cloud-agnostic — AWS/cloud providers excluded per
ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001.

## Method

Strict TDD, two-commit cadence per module: a `test(...)` commit carrying the
RED proof (compile-fail / assertion-fail output pasted in the body), then a
`feat(...)` commit carrying the GREEN proof. No single commit mixes test +
impl. `git log --oneline` is the audit trail.

LOC honest_ratio is measured as **total `src/**.rs` line count (incl. inline
`#[cfg(test)]` + comments) / upstream-core non-test LOC (34,772)** — the same
basis ray #1 used (2,970 / 34,772 = 0.0854), so the delta is comparable.

## Wave 1 — `pkg/scheduling` package completion ✅

Ray #1 had ported `requirement`, `requirements`, `hostport`. This ray closes
the remaining two portable files, completing the cloud-agnostic subset of the
`pkg/scheduling` package.

| Cycle | Module | Upstream file | RED SHA | GREEN SHA | Tests |
|-------|--------|---------------|---------|-----------|-------|
| 1 | `scheduling::volumeusage` | `pkg/scheduling/volumeusage.go` (226 LOC Go) | `f7d05d98` | `1cefea0b` | 11 |
| 2 | `scheduling::taints`       | `pkg/scheduling/taints.go` (81 LOC Go)       | `c3703095` | `38b654a2` | 14 |

**Cycle 1 — volumeusage.** Ported the portable kernel: `Volumes`
(`add`/`union`/`insert` with per-driver set-union + de-dup) and the per-node
`VolumeUsage` limit tracker (`new`/`add_limit`/`add`/`exceeds_limits`/
`delete_pod`). `exceeds_limits` unions tracked usage with the candidate and
rejects strictly over-limit drivers (`len > limit`, so being exactly at the
limit is allowed); `delete_pod` rebuilds the aggregate from survivors so a PVC
shared by another pod is retained. The k8s-client resolvers (`GetVolumes`,
`ResolveDriver`, `driverFromSC`, `driverFromVolume`) are scope-cut — they need
a live controller-runtime client + CSI translation lib and carry no
cloud-agnostic behaviour.

- RED `f7d05d98`: `error[E0432]: unresolved import …::scheduling::volumeusage`.
- GREEN `1cefea0b`: `test result: ok. 11 passed; 0 failed`.

**Cycle 2 — taints.** Ported the `Taints` decorated slice and the
upstream-k8s toleration matcher it leans on: `Toleration::tolerates_taint`
(effect/key wildcards + `Equal` value-equality / `Exists`),
`Taint::matches_taint` (key+effect, value-insensitive), `tolerates` /
`tolerates_pod` (multierr-style aggregation of untolerated taints), `merge`
(append-if-unmatched, preserving existing entries), and the 5-entry
`KNOWN_EPHEMERAL_TAINTS` table (not-ready NoSchedule/NoExecute, unreachable
NoSchedule, external-cloud-provider `uninitialized="true"`,
`karpenter.sh/unregistered` NoExecute).

- RED `c3703095`: `error[E0432]: unresolved import …::scheduling::taints`.
- GREEN `38b654a2`: `test result: ok. 14 passed; 0 failed`.

## Metrics

| Metric | Ray #1 end | Ray #2 end | Δ |
|--------|-----------|-----------|---|
| cave `src` total LOC | 2,970 | 3,370 | +400 |
| LOC honest_ratio (/34,772) | 0.0854 | **0.0969** | +0.0115 |
| cave-karpenter active tests | 154 | **179** | +25 |
| manifest `mapped_count` | 19 | 19 | 0 (anti-inflation) |

`pkg/scheduling` is now fully ported (5/5 portable files: requirement,
requirements, hostportusage, taints, volumeusage). The two cloud/k8s-client
resolvers in volumeusage.go remain scope-cut.

### Why `mapped_count` is unchanged

`scripts/build-parity-index.py` reads the **scalar** `mapped_count` /
`fill_ratio` / `honest_ratio` from the `[parity]` block (not the `[[files]]`
count), and the self-audit gate pins `version = v1.4.0` and
`last_audit = 2026-05-19`. Re-baselining the count to v1.12.1's ~200-file
surface would either inflate the ratio or require rewriting the gate consts.
The honest measure of incremental progress is the LOC ratio above; the
manifest gains **descriptive `[[files]]` rows** for the ported modules but the
headline count stays at the v1.4.0 baseline.

## Remaining work (for continuation ray #3)

**Wave 2 — apis/v1 (has upstream test corpus):**
- NodePool budget `AllowedDisruptions` math (`pkg/apis/v1/nodepool.go`).
- NodePool defaults + validation (~2K LOC, large `_test.go` corpus).

**Wave 3 — utils:** remaining `pkg/utils` helpers (~1.9K LOC).

**Wave 4 — controllers (large):** nodepool (555), disruption (3,092),
provisioning sim (4,539), cluster state (3,017).

**Merge gate:** LOC honest_ratio ≥ 0.95 before any merge to main. At 0.0969
this branch is **held, not merged** — honest in-progress state. Reviewed
together with ray #1 (`dcd82fb1`), which is also unmerged.
