# cave-scheduler — kube-scheduler parity report

Pinned upstream: **kubernetes/kubernetes @ v1.36.0** (`source_sha = "v1.36.0"`)
Audit landed: 2026-05-12 · batch2 imagelocality port: 2026-05-13 · Charter v2 FINALIZE: 2026-05-18

This document is the honest companion to `parity.manifest.toml`. The
manifest proves *coverage*; this report describes *fidelity*.

---

## TL;DR

| metric | value |
|---|---|
| upstream packages enumerated | 29 |
| mapped | 19 |
| partial | 0 |
| skipped (Go-toolchain / cloud-provider) | 7 |
| unmapped (acknowledged real port gaps) | **3** |
| `fill_ratio` (mapped + skipped) / total | **0.8966** (measured) |
| `honest_ratio` | **0.8966** (no partials) |
| cave-scheduler `.rs` files | 25 |
| SPDX AGPL-3.0-or-later coverage | **25/25 (100 %)** |
| `unimplemented!()` / `todo!()` / `panic!("not …")` | **0** |
| `#[deprecated]` | **0** |
| `#[test]` + `#[tokio::test]` | 384 |
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
| 8 | Honest measured manifest | ✅ | `fill_ratio = 0.8966` measured |

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

The 3 [[unmapped]] rows are real port gaps, not audit-doc placeholders:

| upstream package | reason | follow-up |
|---|---|---|
| `pkg/scheduler/framework/plugins/interpodaffinity/` | Soft-affinity weighted scoring with anti-affinity topology keys. Filter path is implemented; PreScore/Score fall back to neutral 0 — hard-affinity works, soft preferences are no-ops. | port PreScore/Score weighted formulas |
| `pkg/scheduler/framework/preemption/recovery` | Preemption-victim restoration after API failure during the victim eviction window. cave `preempt.rs` aborts and re-queues but does not re-add the would-have-victims, so transient apiserver errors can leak victim state. | victim restoration loop |
| `pkg/scheduler/framework/plugins/volumezone/` | Forbid pods from binding PVs in a zone that does not include the candidate node. `volume.rs` handles topology-aware provisioning but does not enforce per-zone restriction at scheduling time. | volumezone Filter plugin |

---

## What changed in this FINALIZE

  * `[upstream] source_sha = "v1.36.0"` — reproducibility pin.
  * `[parity] last_audit = "2026-05-18"` — close-out date.
  * `tests/parity_self_audit.rs` — 9 deterministic assertions.

Behavioural depth, fill_ratio, and honest_ratio remain at their
measured 2026-05-13 batch2 baseline.
