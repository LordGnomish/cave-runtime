# cave-scheduler — kube-scheduler parity report

Pinned upstream: **kubernetes/kubernetes @ v1.36.0** (`source_sha = "v1.36.0"`)
Audit landed: 2026-05-12 · batch2 imagelocality port: 2026-05-13 · Charter v2 FINALIZE: 2026-05-18 · Parity uplift: 2026-05-19

This document is the honest companion to `parity.manifest.toml`. The
manifest proves *coverage*; this report describes *fidelity*.

---

## TL;DR

| metric | value |
|---|---|
| upstream packages enumerated | 29 |
| mapped | 21 |
| partial | 0 |
| skipped (Go-toolchain / cloud-provider) | 7 |
| unmapped (acknowledged real port gaps) | **1** |
| `fill_ratio` (mapped + skipped) / total | **0.9655** (measured) |
| `honest_ratio` | **0.9655** (no partials) |
| cave-scheduler `.rs` files | 27 |
| SPDX AGPL-3.0-or-later coverage | **27/27 (100 %)** |
| `unimplemented!()` / `todo!()` / `panic!("not …")` | **0** |
| `#[deprecated]` | **0** |
| `#[test]` + `#[tokio::test]` | 398 |
| release build | clean |

---

## Charter v2 8-gate scoreboard

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | TDD-strict (RED→GREEN→REFACTOR) | ✅ | this branch shape |
| 2 | SPDX AGPL coverage 100 % | ✅ | `tests/parity_self_audit::every_rs_file_carries_agpl_spdx` |
| 3 | `source_sha` upstream pin | ✅ | `[upstream] source_sha = "v1.36.0"` |
| 4 | No stubs | ✅ | grep count 0 |
| 5 | No back-compat | ✅ | grep count 0 |
| 6 | Latest upstream pinned | ✅ | k8s v1.36.0 |
| 7 | 4-track full | ✅ | see below |
| 8 | Honest measured manifest | ✅ | `fill_ratio = 0.9655` measured |

All 8 gates: **PASS**.

---

## 4-track green status

| Track | Surface | Pre-close status |
|---|---|---|
| Backend lib | `crates/cave-scheduler/src/{scheduler,framework,plugins,preempt,…}.rs` | 384 tests pass |
| Portal | `cave-portal/src/admin/scheduler/` | live wired via `ApiserverClient` (plugin chain, queue depth, preempt history) |
| cavectl | `SchedulerCmd` (status/explain/profile/…) | parse-tests green |
| Observability | `cave-scheduler` alert group + Grafana panel | rules + JSON committed |

---

## Unmapped surface (honest scope-cut)

The remaining 1 [[unmapped]] row is a real port gap, not an audit-doc placeholder:

| upstream package | reason | follow-up |
|---|---|---|
| `pkg/scheduler/framework/preemption/recovery` | Preemption-victim restoration after API failure during the victim eviction window. cave `preempt.rs` aborts and re-queues but does not re-add the would-have-victims, so transient apiserver errors can leak victim state. | victim restoration loop |

---

## What changed in this 2026-05-19 UPLIFT

  * Two [[unmapped]] → [[mapped]] (fill_ratio 0.8966 → 0.9655):
    * `pkg/scheduler/framework/plugins/interpodaffinity/` →
      `src/interpodaffinity_scoring.rs`. Three-step soft-affinity pipeline
      mirroring upstream's `scoring.go`:
      1. **PreScore** — walk the snapshot once, build a table keyed by
         `(topology_key, topology_value)` of summed `±weight` contributions
         from each existing pod that matches a preferred-affinity or
         preferred-anti-affinity term.
      2. **Score** — O(1) per candidate node: sum the precomputed table at
         the node's value for each relevant topology key. Negative raw
         scores are allowed at this step.
      3. **Normalize** — linear map `[min, max] → [0, MAX_NODE_SCORE]`.
         Flat input pins every node to `MAX_NODE_SCORE` (upstream
         behaviour).
    * `pkg/scheduler/framework/plugins/volumezone/` →
      `src/volumezone_plugin.rs`. Dedicated module mirroring upstream's
      package boundary. Filter walks the pod's PVCs, resolves each bound
      PV through `VolumeStore`, and rejects nodes whose
      `topology.kubernetes.io/{zone,region}` (or legacy
      `failure-domain.beta.*`) label is absent or absent from the PV's
      `node_affinity` allow-list. Unbound PVCs and PVs without zone
      affinity are unconstrained.
  * 14 new `#[test]` (6 + 8) in the new modules — all PASS.
  * `tests/parity_self_audit.rs` floors bumped (mapped ≥ 21,
    fill_ratio ≥ 0.95, rs_files ≥ 27).
  * `[parity] last_audit = "2026-05-19"`.

## What landed in the 2026-05-18 FINALIZE

  * `[upstream] source_sha = "v1.36.0"` — reproducibility pin.
  * `tests/parity_self_audit.rs` — 9 deterministic assertions.
