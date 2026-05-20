<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-kamaji — Charter v2 Parity Report

**Upstream:** [clastix-labs/kamaji](https://github.com/clastix-labs/kamaji) pinned **v1.0.0**.
**Upstream license:** Apache-2.0 (Copyright 2024 Clastix Labs).
**cave-kamaji license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.

---

## 1 · Fill-ratio (honest, measured)

```
mapped     = 13
partial    =  1
unmapped   =  0
skipped    =  3
total      = 17

fill_ratio   = mapped / (mapped + partial + unmapped) = 13 / 14 = 0.9286
honest_ratio = mapped / total                          = 13 / 17 = 0.7647
parity_ratio_source = "manifest"
```

Supplementary LOC measurement: ~750 implementation lines (excluding
`#[cfg(test)]`) against ~1300 upstream in-scope lines — ~0.58 ratio on
the LOC basis.

## 2 · Mapped subsystems (13)

| # | Subsystem                  | Local file              | Upstream                                                       |
|---|----------------------------|-------------------------|----------------------------------------------------------------|
| 1 | tcp-spec                   | `src/models.rs`         | `api/v1alpha1/tenantcontrolplane_types.go`                     |
| 2 | tcp-status-phases          | `src/models.rs`         | `api/v1alpha1/tenantcontrolplane_status.go`                    |
| 3 | lifecycle-phase-machine    | `src/lifecycle.rs`      | `internal/controllers/tenantcontrolplane_controller.go`        |
| 4 | kubeconfig-generator       | `src/lifecycle.rs`      | `internal/utilities/kubeconfig.go`                             |
| 5 | rest-api                   | `src/routes.rs`         | (Cave-specific HTTP surface)                                   |
| 6 | in-memory-store            | `src/lib.rs`            | (local helper)                                                 |
| 7 | datastore-crd              | `src/datastore.rs`      | `api/v1alpha1/datastore_types.go`                              |
| 8 | datastore-validation       | `src/datastore.rs`      | `internal/datastore/{etcd,postgresql,mysql}`                   |
| 9 | konnectivity-controller    | `src/konnectivity.rs`   | `internal/resources/konnectivity`                              |
|10 | admission-webhook          | `src/webhook.rs`        | `internal/webhook`                                             |
|11 | apiserver-pod-plan         | `src/pod_mgmt.rs`       | `internal/resources/kubeapiserver`                             |
|12 | kubeadm-init-renderer      | `src/kubeadm.rs`        | `internal/utilities/kubeadm`                                   |
|13 | status-conditions          | `src/status.rs`         | `internal/controllers/conditions.go`                           |

## 3 · Partial subsystems (1)

| Subsystem                | Reason                                                                                                                                       |
|--------------------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| cluster-api-integration  | TCP CRD + kubeadm init cover the bootstrap path; full Cluster API CRDs land alongside cave-cluster-api Phase 3.                              |

## 4 · Skipped subsystems (3 — intentional out-of-scope)

| Surface                  | Reason                                                                                                          |
|--------------------------|-----------------------------------------------------------------------------------------------------------------|
| cert-controller          | cave-certs owns the certificate surface; kamaji emits cert CSRs through that API.                              |
| metrics-export           | cave-metrics owns the workspace exporter; kamaji surfaces stats via the REST API.                              |
| leader-election-helper   | Duplicated by cave-controller-manager; defer.                                                                  |

## 5 · 4-track status

| Track          | Status     | Evidence                                                                                                  |
|----------------|------------|-----------------------------------------------------------------------------------------------------------|
| Backend        | **GREEN**  | This crate — 13 mapped + 1 partial. **9 self_audit + 19 phase2_deep_port + lib tests PASS**.              |
| Portal         | Phase 3    | /admin/kamaji follows multi-tenant data-plane wave.                                                       |
| cavectl        | Phase 3    | `cavectl kamaji` follows multi-tenant data-plane wave.                                                    |
| Observability  | Phase 3    | alerts + dashboard follow multi-tenant data-plane wave.                                                   |

## 6 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                  | Status |
|---|-----------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS           | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file                            | ✅      |
| 3 | `[upstream] source_sha` pinned to `v1.0.0`                            | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in src/  | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims              | ✅      |
| 6 | Always-latest — Kamaji v1.0.0 (latest stable as of 2026-05-19)        | ✅      |
| 7 | 4-track — Backend GREEN; Portal/cavectl/Obs honestly deferred Phase 3 | ✅      |
| 8 | Honest measured `fill_ratio = 0.9286` (>= 0.50 MVP floor)             | ✅      |

## 7 · Reproducibility

```bash
cargo test -p cave-kamaji
python3 scripts/build-parity-index.py
```
