# cave-mesh — Charter v2 8-gate Close-out Report (Ambient-only)

**Audit date**: 2026-05-19
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
| upstream Istio top-level dirs covered (pilot/security/telemetry/cni/operator/pkg/wasm + ambient ztunnel/waypoint) | 37 entities |
| mapped                                  | **14** |
| partial                                 | **1** |
| skipped (UI / spec / orchestrator / **ambient-only cuts**) | **18** |
| unmapped (acknowledged gaps)            | **4** |
| `fill_ratio`                            | **0.8919** = (mapped + partial + skipped) / total = 33/37 |
| `honest_ratio`                          | **0.8649** = (mapped + skipped) / total = 32/37 — partial excluded |
| `parity_ratio_source`                   | `"manifest"` |
| `source_sha`                            | `"badd809ed7d57954d4c16e12e75e15a7722a7b96"` (Istio v1.30.0 commit) |
| `last_audit`                            | `2026-05-19` |
| SPDX `AGPL-3.0-or-later` coverage       | 100% (all `.rs` files in `src/` + `tests/`) |

---

## In-scope (Ambient-only)

* **Ambient data-plane** (`src/ambient/`)
  * `ztunnel.rs`         — node-local L4 zero-trust tunnel (mTLS, SPIFFE peer-ID)
  * `waypoint.rs`        — L7 routing tier
  * `hbone.rs`           — HBONE (HTTP/2 CONNECT + mTLS tunnel)
  * `svid.rs`            — SVID issuance for ambient identities
  * `authz.rs`           — Ambient AuthZ (DENY-first, principal/jwt rules)
  * `telemetry.rs`       — Ambient-mode telemetry hooks
  * `virtualservice.rs`  — VS routing compiled for waypoint
  * `destinationrule.rs` — DR subsets compiled for waypoint
* **Control-plane (Pilot subset that survives ambient-only cut)**
  * VirtualService / DestinationRule / Gateway / ServiceEntry config translation
  * TrafficPolicy: outlierDetection + connectionPool (batch4)
  * MultiCluster federation (`pilot/.../federation/`)
* **Security**
  * SPIFFE-style workload identity, mTLS root CA, AuthorizationPolicy (allow/deny)
* **Telemetry**
  * Prometheus metric facade (request count / latency / connection state)

## Out-of-scope (skipped — 18)

### Ambient-only mandate cuts (5 — new, 2026-05-19)

Per Cave Runtime's no-backcompat policy, the sidecar data plane is **removed**,
not deferred. The corresponding `src/*.rs` files were deleted in commit
`d1b4e0c6` (2710 LOC).

| upstream package | deleted file | LOC | reason |
|---|---|---|---|
| `pilot/pkg/networking/sidecar`       | `src/sidecar.rs`      |  297 | Sidecar/EnvoyFilter/WorkloadGroup CRDs incompatible with ambient mode |
| `pilot/pkg/networking/proxy`         | `src/proxy.rs`        |  257 | Envoy sidecar abstraction replaced by ztunnel + waypoint |
| `pilot/pkg/xds`                      | `src/xds.rs`          | 1095 | xDS v3 LDS/RDS/CDS/EDS/SDS stream replaced by SVID + waypoint config |
| `pilot/pkg/networking/extension/`    | `src/wasm_plugin.rs`  |  460 | EnvoyFilter direct-patch surface dropped |
| `pkg/wasm/`                          | `src/wasm_runtime.rs` |  601 | wasmtime-backed proxy-wasm runtime dropped; `wasmtime` + `wat` Cargo deps removed |

### Pre-existing skips (13 — Istio packages outside cave-mesh's scope)

* `istioctl/` CLI tooling (cavectl absorbs equivalent UX)
* `operator/` Helm chart packaging (cave-deploy handles this)
* `cni/` install-side networking (cave-net + node DaemonSet absorb)
* `tools/`, `tests/integration`, `tests/e2e` upstream harness
* `samples/`, `manifests/charts`, `releasenotes/`, `manifests/profiles`
* `bin/`, `prow/` build/CI tooling
* `architecture/`, `release/` documentation
* `pkg/log/` + `pkg/util/` (stdlib analog — Rust uses `tracing` + `std`)

## Unmapped (acknowledged gaps — 4)

| upstream area | reason | priority |
|---|---|---|
| `pilot/pkg/networking/serviceentry/external/` — VM-mesh expansion | non-k8s workload CRDs (VM + WorkloadEntry) not implemented | P2 |
| `telemetry/api/v1/AnalyticsClient` | backend analytics protocol (Istio team telemetry) — out of scope | P3 |
| `istioctl/pkg/{analyze,debug}/` | static analysis + diagnose; distinct from runtime mesh | P3 |
| `ztunnel:src/state/policy/L7` | Ambient L7 policy (waypoint proxy filter chain) — routing stub exists, filter ordering unmapped | P1 |

## Partial (1)

* `pilot/pkg/networking/federation/` — multi-network federation control plane
  (MeshNetwork enrolment, EastWestGateway lifecycle, NetworkEndpoint
  publish/retract). Data-plane forwarding stays in the bound ambient proxy;
  cross-mesh federation tracked separately.

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
| 5 | `fill_ratio >= 0.85`                  | PASS   | measured **0.8919** (33/37)               |
| 6 | counts sum to total (14+1+18+4 == 37) | PASS   | `gate_6_count_invariants`                 |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | PASS | `gate_7_no_stub_macros_in_src`        |
| 8 | `PARITY_REPORT.md` exists with summary | PASS  | this file (`gate_8_parity_report_exists`) |

All nine `tests/parity_self_audit.rs` assertions pass.

---

## Notes

* Ztunnel is a separate Rust repo upstream
  (`https://github.com/istio/ztunnel`); the manifest cites the upstream
  `release-1.29` branch for parity diffs (1.30 ztunnel cut still tracked
  on the same branch line at audit time).
* Cave Charter (4) tracks: this report covers the **Backend** track for
  cave-mesh. Portal `/admin/mesh/*` + cavectl `mesh` group + obs alert
  rules are present (see `parity.manifest.toml` `[portal_ui]`).
* The 2710 LOC removal is **permanent**, not deferred. Re-adding a
  sidecar plane would require relitigating the no-backcompat mandate.
