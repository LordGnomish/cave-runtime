# cave-mesh parity — 2026-05-12 audit

**Upstreams:** `istio/istio v1.29.2` (Apache-2.0) +
`istio/ztunnel` for ambient-mode L4 mTLS.

## Methodology

Standard cave-etcd pattern. Istio is a ~100K-LOC Go control plane
with an extensive packageage tree. The inventory focuses on the
domain packages (pilot/, security/, telemetry/, ambient/) and
skips operator tooling, CLI, CNI plugin, helm, tests.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 14 |
| Skipped  | 13 |
| Unmapped | 8 |
| **Total** | **35** |
| **fill_ratio** | **0.7714** |

## What lands in the inventory

* **Mapped (14)** covers the xDS discovery service, the
  VirtualService → routing pipeline, SPIFFE + mTLS, sidecar +
  proxy state, circuit breaker (with sweep-011 backoff adopted),
  rate limiter, service registry, RED telemetry, the ambient-mode
  CRDs (AuthZ, VS, DR, waypoint), HBONE tunnel, SVID issuance for
  ambient identities, and multi-cluster.
* **Skipped (13)** covers Helm operator, CLI (istioctl, samples),
  CNI plugin (cave-cni's job), test suites, build tooling, and
  Go-stdlib helpers.
* **Unmapped (8)** covers honest gaps: WASM filter extensions,
  multi-network federation, VM-mesh expansion, the full JWT
  validation pipeline, ServiceEntry CRD (manual catalog),
  istioctl analyze/debug, and ambient L7 policy chain.

## What this PR does NOT claim

* `fill_ratio = 0.7714` does NOT mean cave-mesh is 77% of a
  production Istio. It claims 77% of Istio's domain packages are
  either covered (40%) or honestly out of scope (37%, CLI / helm /
  CNI / tests).
* The 8 unmapped entries — particularly WASM filters and VM
  expansion — are real production-feature gaps.
