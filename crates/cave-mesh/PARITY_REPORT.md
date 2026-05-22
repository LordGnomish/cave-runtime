# cave-mesh — Charter v2 8-gate Close-out Report (Ambient-only)

**Audit date**: 2026-05-19 (Wave-2 close-out)
**Upstream pin**: `istio/istio @ 1.30.0` (commit `badd809ed7d57954d4c16e12e75e15a7722a7b96`)
**Crate root**: `crates/cave-mesh/`

Companion to `parity.manifest.toml`. The manifest proves *coverage*; this
report describes *scope* — what the Ambient-only re-baseline ports, what
is partial, and what is intentionally out-of-scope under the Cave Runtime
no-backcompat mandate.

---

## TL;DR

| metric | value |
|---|---|
| upstream Istio top-level dirs covered | 37 entities |
| mapped                                  | **16** (+2 vs pre-wave-2) |
| partial                                 | **1** |
| skipped (UI / spec / orchestrator / **ambient-only cuts** / vendor-spec) | **20** (+2 vs pre-wave-2) |
| unmapped (acknowledged gaps)            | **0** (-4 vs pre-wave-2) |
| `fill_ratio`                            | **1.0** (37/37, +0.1081) |
| `honest_ratio`                          | **0.9730** (mapped + skipped) / total = 36/37 |
| `parity_ratio_source`                   | `"manifest"` |
| `source_sha`                            | `"badd809ed7d57954d4c16e12e75e15a7722a7b96"` |
| `last_audit`                            | `2026-05-19` |
| SPDX `AGPL-3.0-or-later` coverage       | 100% |

---

## Wave-2 close-out delta (2026-05-19)

| Δ | upstream surface | provenance |
|---|---|---|
| → | `pilot/pkg/networking/serviceentry/external` (VM-mesh) | unmapped → mapped · `src/vm_mesh.rs` (WorkloadEntry enrolment + SPIFFE id + per-VM health probes + healthy-endpoint filter) |
| → | `ztunnel:src/state/policy/L7` | unmapped → mapped · `src/ambient/l7_policy.rs` (canonical filter ordering jwt→authz→rate-limit→fault→route→telemetry + chain evaluator) |
| → | `telemetry/api/v1/AnalyticsClient` | unmapped → skipped (vendor backend telemetry — cave-metrics owns workspace observability) |
| → | `istioctl/pkg/{analyze,debug}/` | unmapped → skipped (CLI tooling absorbed by cavectl) |

Net: 14 → **16** mapped, 4 → **0** unmapped, fill_ratio **0.8919 → 1.0**.

---

## In-scope (Ambient-only)

* **Ambient data-plane** (`src/ambient/`)
  * `ztunnel.rs`         — node-local L4 zero-trust tunnel
  * `waypoint.rs`        — L7 routing tier
  * `hbone.rs`           — HBONE (HTTP/2 CONNECT + mTLS tunnel)
  * `svid.rs`            — SVID issuance for ambient identities
  * `authz.rs`           — Ambient AuthZ (DENY-first, principal/jwt rules)
  * `telemetry.rs`       — Ambient-mode telemetry hooks
  * `virtualservice.rs`  — VS routing compiled for waypoint
  * `destinationrule.rs` — DR subsets compiled for waypoint
  * **`l7_policy.rs`** — Wave-2 close-out: canonical L7 filter ordering + chain evaluator
* **Control-plane (Pilot subset)** — VS / DR / Gateway / ServiceEntry; TrafficPolicy + outlier + connectionPool; MultiCluster federation
* **Security** — SPIFFE workload identity, mTLS root CA, AuthorizationPolicy
* **Telemetry** — Prometheus metric facade
* **VM mesh** — Wave-2 close-out: `src/vm_mesh.rs` enrols VM/bare-metal workloads under ServiceEntry parents

## Out-of-scope (skipped — 20)

### Ambient-only mandate cuts (5)

Per Cave Runtime's no-backcompat policy, the sidecar data plane is **removed**,
not deferred. Files deleted in commit `d1b4e0c6` (2710 LOC).

| upstream package | deleted file | LOC | reason |
|---|---|---|---|
| `pilot/pkg/networking/sidecar`       | `src/sidecar.rs`      |  297 | Sidecar/EnvoyFilter/WorkloadGroup |
| `pilot/pkg/networking/proxy`         | `src/proxy.rs`        |  257 | Envoy sidecar abstraction |
| `pilot/pkg/xds`                      | `src/xds.rs`          | 1095 | xDS v3 LDS/RDS/CDS/EDS/SDS |
| `pilot/pkg/networking/extension/`    | `src/wasm_plugin.rs`  |  460 | EnvoyFilter direct-patch |
| `pkg/wasm/`                          | `src/wasm_runtime.rs` |  601 | wasmtime-backed proxy-wasm |

### Pre-existing skips (13)

* `istioctl/` CLI tooling (cavectl absorbs)
* `operator/` Helm chart packaging (cave-deploy)
* `cni/` install-side networking (cave-net + node DaemonSet)
* `tools/`, `tests/integration`, `tests/e2e`
* `samples/`, `manifests/charts`, `releasenotes/`, `manifests/profiles`
* `bin/`, `prow/` build/CI
* `architecture/`, `release/` docs
* `pkg/log/` + `pkg/util/` (stdlib analog — `tracing` + `std`)

### Wave-2 reclassifications (2 — unmapped → skipped 2026-05-19)

* `telemetry/api/v1/AnalyticsClient` — vendor backend usage telemetry; out of cave-mesh scope.
* `istioctl/pkg/{analyze,debug}/` — CLI static-analysis tooling; cavectl absorbs.

## Unmapped (0)

All four pre-existing unmapped subsystems closed in Wave-2 close-out 2026-05-19 (2 promoted to mapped, 2 reclassified as skipped with explicit reasons).

## Partial (1)

* `pilot/pkg/networking/federation/` — MeshNetwork enrolment + EastWestGateway lifecycle covered; cross-mesh data-plane forwarding tracked separately.

---

## Phase 2 backlog (sonraki ray)

* Ambient telemetry deep — per-workload metric labels + tracing exemplars
* Multi-cluster ambient federation — cross-mesh ztunnel peering
* IPv6 dual-stack waypoint

---

## Charter v2 8-gate status — **8/8 PASS**

| # | Gate                                  | Status | Evidence                                  |
|---|---------------------------------------|--------|-------------------------------------------|
| 1 | SPDX `AGPL-3.0-or-later` on every `.rs` | PASS | `gate_1_spdx_full_coverage` |
| 2 | `source_sha` pinned to Istio 1.30.0 commit | PASS | `[upstream].source_sha = "badd809e…"` |
| 3 | `last_audit = "2026-05-19"`           | PASS   | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "manifest"`    | PASS   | `[parity].parity_ratio_source`            |
| 5 | `fill_ratio >= 0.85`                  | PASS   | measured **1.0** (37/37)                  |
| 6 | counts sum to total (16+1+20+0 == 37) | PASS   | `gate_6_count_invariants`                 |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | PASS | `gate_7_no_stub_macros_in_src`        |
| 8 | `PARITY_REPORT.md` exists with summary | PASS  | this file (`gate_8_parity_report_exists`) |

All nine `tests/parity_self_audit.rs` assertions pass.

---

## Notes

* Ztunnel is a separate Rust repo upstream (`https://github.com/istio/ztunnel`); the L7-policy port cites the upstream `release-1.29` branch.
* Cave Charter (4) tracks: this report covers the **Backend** track. Portal `/admin/mesh/*` + cavectl `mesh` group + obs alert rules are present.
* The 2710 LOC sidecar removal is **permanent**, not deferred.
