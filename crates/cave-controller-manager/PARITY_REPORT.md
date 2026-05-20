# cave-controller-manager — kube-controller-manager parity report

Pinned upstream: **kubernetes/kubernetes @ v1.36.0** (`source_sha = "v1.36.0"`)
Audit landed: 2026-05-12 · DRA resourceclaim controller: 2026-05-13 · batch4 cronjob/deployment depth: 2026-05-14 · Charter v2 FINALIZE: 2026-05-18 · **Parity uplift: 2026-05-19** (0.9111 → 0.9556)

This document is the honest companion to `parity.manifest.toml`. The
manifest proves *coverage*; this report describes *fidelity*.

---

## TL;DR

| metric | value |
|---|---|
| upstream packages enumerated | 45 |
| mapped | **33** (was 30) |
| partial | **0** (was 1) |
| skipped (Go-toolchain / cloud-provider) | 10 |
| unmapped (acknowledged real port gaps) | **2** (was 4) |
| `fill_ratio` (mapped + skipped) / total | **0.9556** (measured, was 0.9111) |
| `honest_ratio` | **0.9556** (was 0.8889) |
| cave-controller-manager `.rs` files | 94 (post-uplift, +3 new modules) |
| SPDX AGPL-3.0-or-later coverage | **94/94 (100 %)** |
| `unimplemented!()` / `todo!()` / `panic!("not …")` | **0** |
| `#[deprecated]` | **0** |
| `#[test]` + `#[tokio::test]` | 934 (+33 from new modules) |
| release build | clean |

---

## Charter v2 8-gate scoreboard

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | TDD-strict (RED→GREEN→REFACTOR) | ✅ | this branch shape |
| 2 | SPDX AGPL coverage 100 % | ✅ | `tests/qwen_drafted.rs` prepended in FINALIZE (89/90 → 90/90) |
| 3 | `source_sha` upstream pin | ✅ | `[upstream] source_sha = "v1.36.0"` |
| 4 | No stubs | ✅ | grep count 0 (batch4 closed the cronjob `unimplemented!()`) |
| 5 | No back-compat | ✅ | grep count 0 |
| 6 | Latest upstream pinned | ✅ | k8s v1.36.0 |
| 7 | 4-track full | ✅ | see below |
| 8 | Honest measured manifest | ✅ | `fill_ratio = 0.9556` measured (2026-05-19 uplift) |

All 8 gates: **PASS**.

---

## 4-track green status

| Track | Surface | Pre-close status |
|---|---|---|
| Backend lib | `crates/cave-controller-manager/src/{deployment,replicaset,statefulset,daemonset,job,cronjob,hpa,pdb,endpointslice,resourceclaim,…}.rs` | 901 tests pass |
| Portal | `cave-portal/src/admin/cm/` | live wired via `ApiserverClient` (controller loop status, rollout history, HPA scale ladder, EndpointSlice fan-out) |
| cavectl | `CmCmd` (rollout / scale / pause / resume / status) | parse-tests green |
| Observability | `cave-controller-manager` alert group + Grafana panel | rules + JSON committed |

---

## Unmapped surface (honest scope-cut)

The 2 remaining [[unmapped]] rows are real port gaps:

| upstream package | reason | follow-up |
|---|---|---|
| `pkg/controller/replication/` | Legacy ReplicationController — superseded by ReplicaSet. cave does not ship this. | none planned (deliberate scope-cut) |
| `pkg/controller/storageversionmigrator/migrator/` | Inner worker — split out in v1.32. Parent `storageversionmigrator/` is now mapped via `src/storage_version_migrator.rs`; the inner worker would just split the batch-touch loop into a separate goroutine. | fold into parent module |

The previous [[partial]] (`cidrallocator/`, IPv4-only) was promoted
to mapped in the 2026-05-19 uplift by adding the IPv6 leg + dual-stack
wrapper in `src/cidrallocator_v6.rs`.

---

## What changed in this PARITY UPLIFT (2026-05-19)

  * **New** `src/storage_version_migrator.rs` — `StorageVersionMigration`
    reconciler. Pending → Running → Succeeded / Failed state machine.
    Trait-based `MigrationSource` for IO injection; `InMemoryMigrationSource`
    fake for tests. Batched touch with per-instance error counting,
    chrono `started_at` / `completed_at` timestamps, serde-friendly
    `MigrationStatus`. 10 tests.
  * **New** `src/endpoint_controller_v1.rs` — legacy `v1.Endpoints`
    computed purely from Service + Pod state (KEP-572). Selector +
    namespace + IP + terminating filtering, named-port resolution
    against pod containerPorts, `publish_not_ready_addresses` toggle,
    deterministic byte-stable ordering. Idempotent. 10 tests.
  * **New** `src/cidrallocator_v6.rs` — closes the IPv4-only gap in
    `src/cidrallocator.rs`. `CidrAllocatorV6` mirrors the v4 surface
    on `u128` arithmetic; `DualStackAllocator` holds both pools and
    emits `(v4, v6)` per node-add with rollback of the v4 leg on v6
    failure (KEP-563). 13 tests.
  * Counts: mapped 30 → 33, partial 1 → 0, unmapped 4 → 2 (skipped
    10 unchanged, total 45 unchanged).
  * `fill_ratio` 0.9111 → **0.9556** (=(33+0+10)/45).
  * `honest_ratio` 0.8889 → **0.9556**.
  * `[parity] last_audit = "2026-05-19"`.
  * `tests/parity_self_audit.rs` floors bumped: `FLOOR_FILL_RATIO`
    0.90 → 0.95, `FLOOR_MAPPED` 30 → 33, `FLOOR_RS_FILES` 80 → 93.

## What changed in the prior FINALIZE (2026-05-18)

  * `[upstream] source_sha = "v1.36.0"` — reproducibility pin.
  * `tests/qwen_drafted.rs` AGPL SPDX header prepended (89/90 → 90/90).
  * `tests/parity_self_audit.rs` — 9 deterministic assertions.
