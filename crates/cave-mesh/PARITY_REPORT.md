# cave-mesh — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Upstream pin**: `istio/istio @ 1.29.2` (Istio v1.29.2 release tag)
**Crate root**: `crates/cave-mesh/`

Companion to `parity.manifest.toml`. The manifest proves *coverage*; this
report describes *scope* — which control-plane / data-plane surfaces are
ported, what is partial, and what is intentionally deferred.

---

## TL;DR

| metric | value |
|---|---|
| upstream Istio top-level dirs covered (pilot/security/telemetry/cni/operator/pkg + ambient ztunnel/waypoint) | 36 entities |
| mapped                                  | **17** |
| partial                                 | **2** |
| skipped (UI / spec / orchestrator)      | **13** |
| unmapped (acknowledged gaps)            | **4** |
| `fill_ratio`                            | **0.8889** = (mapped + partial + skipped) / total |
| `honest_ratio`                          | **0.8333** = (mapped + skipped) / total — partial excluded |
| `parity_ratio_source`                   | `"manifest"` |
| `source_sha`                            | `"1.29.2"` |
| `last_audit`                            | `2026-05-19` |
| SPDX `AGPL-3.0-or-later` coverage       | 100% (37/37 `.rs` files in `src/` + `tests/`) |

---

## In-scope (Ambient mode + sidecar control-plane MVP)

* **Ambient data-plane**
  * `src/ambient/hbone.rs`     — HBONE (HTTP/2 CONNECT + mTLS tunnel)
  * `src/ambient/ztunnel.rs`   — node-local L4 zero-trust tunnel (mTLS, SPIFFE peer-ID)
  * `src/ambient/waypoint.rs`  — L7 routing tier (VirtualService / DestinationRule)
* **Control-plane (Pilot)**
  * VirtualService / DestinationRule / Gateway / ServiceEntry config translation → Envoy xDS
  * TrafficPolicy: outlierDetection + connectionPool (batch4)
  * MultiCluster federation (`pilot/.../federation/`)
* **Security**
  * SPIFFE-style workload identity, mTLS root CA, AuthorizationPolicy (allow/deny)
* **Telemetry**
  * Prometheus metric facade (request count / latency / connection state)

## Out-of-scope (skipped — 13)

* `istioctl/` CLI tooling (cavectl absorbs equivalent UX)
* `operator/` Helm chart packaging (cave-deploy handles this)
* `cni/` install-side networking (cave-net + node DaemonSet absorb)
* `tools/`, `tests/integration`, `tests/e2e` upstream harness
* `samples/`, `manifests/charts`, `releasenotes/`, `manifests/profiles`
* `bin/`, `prow/` build/CI tooling
* `architecture/`, `release/` documentation

## Unmapped (acknowledged gaps — 4)

| upstream area | reason | priority |
|---|---|---|
| `pilot/pkg/networking/grpcgen/` — gRPC-targeted xDS variant   | scope: Envoy xDS path is enough for cave MVP | P3 |
| `security/pkg/server/ca/` — full CA hierarchy / cert rotation | partial today, full rotation deferred       | P2 |
| `pilot/pkg/credentialcontroller/` — secret-driven cert reload | needs cave-certs hot-reload hook            | P2 |
| `pkg/wasm/` — WASM filter sandbox loader                      | wasmtime adoption pending (post-OSS)         | P3 |

## Partial (2)

* `security/pkg/pki/` — SPIFFE-style identity issued (X.509 SAN URI), but rotation
  cadence + intermediate CA chains are simplified to a single root for the MVP.
* `pilot/pkg/networking/core/v1alpha3/listener.go` — Envoy listener fabrication
  covers HTTP/TCP+mTLS; QUIC + non-Envoy bridges deferred.

---

## Charter v2 8-gate status — **8/8 PASS**

| # | Gate                                  | Status | Evidence                                  |
|---|---------------------------------------|--------|-------------------------------------------|
| 1 | SPDX `AGPL-3.0-or-later` on every `.rs` | PASS | 37/37 (verified by `gate_1_spdx_full_coverage`) |
| 2 | `source_sha = "1.29.2"`               | PASS   | `[upstream].source_sha`                   |
| 3 | `last_audit = "2026-05-19"`           | PASS   | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "manifest"`    | PASS   | `[parity].parity_ratio_source`            |
| 5 | `fill_ratio >= 0.85`                  | PASS   | measured **0.8889**                       |
| 6 | counts sum to total (17+2+13+4 == 36) | PASS   | `gate_6_count_invariants`                 |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | PASS | `gate_7_no_stub_macros_in_src`        |
| 8 | `PARITY_REPORT.md` exists with summary | PASS  | this file (`gate_8_parity_report_exists`) |

All nine `tests/parity_self_audit.rs` assertions pass.

---

## Notes

* Ztunnel is a separate Rust repo upstream
  (`https://github.com/istio/ztunnel`); citation in manifest points at the
  upstream `release-1.29` branch.
* Cave Charter (4) tracks: this report covers the **Backend** track for
  cave-mesh. Portal `/admin/mesh/*` + cavectl `mesh` group + obs alert
  rules are present (see `parity.manifest.toml` `[portal_ui]`).
