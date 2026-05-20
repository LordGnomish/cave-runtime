<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-karpenter — Charter v2 Parity Report

**Upstream:** [kubernetes-sigs/karpenter](https://github.com/kubernetes-sigs/karpenter) pinned **v1.4.0**.
**Upstream license:** Apache-2.0 (Copyright 2024 The Kubernetes Authors).
**cave-karpenter license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.

---

## 1 · Fill-ratio (honest, measured)

```
mapped     = 18
partial    =  1
unmapped   =  0
skipped    =  3
total      = 22

fill_ratio   = mapped / (mapped + partial + unmapped) = 18 / 19 = 0.9474
honest_ratio = mapped / total                          = 18 / 22 = 0.8182
parity_ratio_source = "manifest"
```

`docs/parity/parity-index.json` reads these fields directly from `parity.manifest.toml`.

Supplementary LOC measurement (for transparency): the cave-karpenter src/
tree carries ~860 implementation lines (excluding `#[cfg(test)]`) against
~1660 upstream in-scope lines — a ~0.49 LOC ratio. The subsystem-count
formula is the headline because it tracks completeness against the
named Karpenter controllers rather than line-for-line copy.

## 2 · Mapped subsystems (18)

| # | Subsystem                       | Local file                       | Upstream                                                                  |
|---|---------------------------------|----------------------------------|---------------------------------------------------------------------------|
| 1 | nodepool-crd                    | `src/models/mod.rs`              | `pkg/apis/v1/nodepool_types.go`                                           |
| 2 | nodeclaim-crd                   | `src/models/mod.rs`              | `pkg/apis/v1/nodeclaim_types.go`                                          |
| 3 | nodeclass-envelope              | `src/models/mod.rs`              | `pkg/apis/v1/nodeclass_types.go`                                          |
| 4 | requirements-operators (6 ops)  | `src/models/mod.rs`              | `pkg/apis/v1/requirements.go`                                             |
| 5 | taints                          | `src/models/mod.rs`              | `pkg/apis/v1/taints.go`                                                   |
| 6 | disruption-spec                 | `src/models/mod.rs`              | `pkg/apis/v1/disruption_types.go`                                         |
| 7 | limits-spec                     | `src/models/mod.rs`              | `pkg/apis/v1/limits.go`                                                   |
| 8 | scheduler-first-match           | `src/scheduler.rs`               | `pkg/controllers/provisioning/scheduling/scheduler.go`                    |
| 9 | in-memory-store                 | `src/store.rs`                   | (local helper)                                                            |
| 10| provisioning-batcher            | `src/batcher.rs`                 | `pkg/controllers/provisioning/batcher/batcher.go`                         |
| 11| binpacker-with-topology         | `src/binpack.rs`                 | `pkg/controllers/provisioning/scheduling/{scheduler,topology,taints}.go`  |
| 12| consolidation-controller        | `src/disruption.rs`              | `pkg/controllers/disruption/consolidation.go`                             |
| 13| drift-controller                | `src/disruption.rs`              | `pkg/controllers/disruption/drift.go`                                     |
| 14| expiration-controller           | `src/disruption.rs`              | `pkg/controllers/disruption/expiration.go`                                |
| 15| disruption-budget-arbiter       | `src/disruption.rs`              | `pkg/controllers/disruption/orchestration/queue.go`                       |
| 16| nodeclaim-launch                | `src/nodeclaim_lifecycle.rs`     | `pkg/controllers/nodeclaim/lifecycle/launch.go`                           |
| 17| termination-controller          | `src/nodeclaim_lifecycle.rs`     | `pkg/controllers/nodeclaim/lifecycle/termination.go`                      |
| 18| cloud-provider-trait + envelopes| `src/provider/mod.rs`            | `pkg/cloudprovider/cloudprovider.go`                                      |

## 3 · Partial subsystems (1)

| Subsystem        | Reason                                                                                                                                                |
|------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------|
| drain-controller | `nodeclaim_lifecycle::drain()` flips `claim.drained`; the actual pod-eviction loop with PDB respect is delegated to cave-kubelet's evict path.        |

## 4 · Skipped subsystems (3 — intentional out-of-scope)

| Surface              | Reason                                                                                       |
|----------------------|----------------------------------------------------------------------------------------------|
| webhook-validation   | Admission webhook — cave-admission owns; defer.                                              |
| ec2-provider         | AWS EC2 fleet API — autoscale-cloud Phase 3 alongside cave-ccm AWS track.                    |
| gcp-provider         | Community fork — out of MVP; revisit if/when cave-ccm gains GCP.                             |

## 5 · Scope cuts (deferred to Phase 3 ray)

| Cut                                  | Target ray                  |
|--------------------------------------|-----------------------------|
| Hetzner cloud-side API client        | autoscale-cloud-phase-3     |
| Azure RM cloud-side API client       | autoscale-cloud-phase-3     |

NodeClass *envelope* shapes (`HetznerNodeClassSpec`, `AzureNodeClassSpec`)
are mapped at `src/provider/mod.rs`. Only the cloud-side dispatch is cut.

## 6 · 4-track status

| Track          | Status     | Evidence                                                                                                      |
|----------------|------------|---------------------------------------------------------------------------------------------------------------|
| Backend        | **GREEN**  | This crate — 18 mapped + 1 partial. 23 lib + 20 phase2_deep_port + 9 parity_self_audit = **52 tests PASS**.   |
| Portal         | Phase 3    | admin/karpenter follows cave-ccm Hetzner+Azure ports.                                                          |
| cavectl        | Phase 3    | `cavectl karpenter` follows provider tracks.                                                                  |
| Observability  | Phase 3    | alerts + dashboard follow provider tracks.                                                                    |

## 7 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                  | Status |
|---|-----------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS           | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file                            | ✅      |
| 3 | `[upstream] source_sha` pinned to `v1.4.0`                            | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in src/  | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims              | ✅      |
| 6 | Always-latest — Karpenter v1.4.0 (latest stable as of 2026-05-19)     | ✅      |
| 7 | 4-track — Backend GREEN; Portal/cavectl/Obs honestly deferred Phase 3 | ✅      |
| 8 | Honest measured `fill_ratio = 0.9474` (>= 0.40 MVP floor)             | ✅      |

## 8 · Reproducibility

```bash
cargo test -p cave-karpenter
python3 scripts/build-parity-index.py
```
