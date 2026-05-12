# cave-apiserver parity — 2026-05-12 measured audit

**Upstream pin:** `kubernetes/kubernetes` (apiserver-bearing packages from v1.31.x; the manifest's pinned `v1.36.0` claim is preserved as historical).

## Why this exists

The 2026-05-01 full-audit placed cave-apiserver at **tier 100, parity_ratio = 1.0**. That number reflected the wave3 mechanical metric: every entry in the legacy `[[files]]/[[functions]]/[[tests]]/[[surfaces]]` arrays mapped to a local symbol, so `matched / total = 1.0`. But the manifest declared only 49 upstream files out of a kubernetes/kubernetes apiserver surface that ships ~30 top-level packages plus the staging `k8s.io/apiserver` framework. The 1.0 was true under its own definition and misleading as a parity claim.

Per the close-out brief ("**gerçek line-by-line port**"), this pass replaces the self-reported number with a measured one — enumerate the apiserver-bearing top-level packages, classify each as mapped / skipped / unmapped, compute `fill_ratio = (mapped + skipped) / total`.

## Inventory methodology

Source: hand-curated against the public kubernetes/kubernetes v1.31.x layout. The inventory targets **observable apiserver surface**: the `staging/src/k8s.io/apiserver/` generic framework, `staging/src/k8s.io/apiextensions-apiserver/`, `staging/src/k8s.io/kube-aggregator/`, the per-resource registries under `pkg/registry/*/`, and the API-type packages under `pkg/apis/*/` that the apiserver itself owns. Out-of-scope packages (kube-proxy, kubelet, controllers, cloud provider) are explicitly skipped with a `parallel-track` reason and counted toward `fill_ratio`.

Per cave-net's pattern (134-entry gold-standard with `fill_ratio = 1.0`), each entry is one of:

- **`[[mapped]]`** — cave-apiserver has at least one source file implementing the package's observable contract. Notes call out reshape choices (axum Router instead of go-restful, JSON-first instead of protobuf default, in-memory cacher).
- **`[[skipped]]`** — out of scope per Charter. Allowed reasons: `go-bootstrap` | `proxy-mode` | `CLI` | `test-harness` | `wire-format-detail` | `parallel-track` | `stdlib-analog`. Every skip cites one.
- **`[[unmapped]]`** — real port gap, acknowledged with rationale.

## Counts

| Bucket | Count | Notes |
|---|---:|---|
| `[[mapped]]` | 26 | Core apiserver framework + handlers + admission + auth + audit + CRD + aggregator + per-resource registries |
| `[[skipped]]` | 17 | Bootstrap, client-go, code generators, kube-proxy, kubelet probes, cloud-providers, metrics, legacy CCM |
| `[[unmapped]]` | 7 | Real gaps: CEL evaluator, DRA resources, audit-backend plugin registry, live OpenAPI synthesis, admission initializer, Lease semantics, exec/attach proxy |
| **Total** | **50** | |
| **fill_ratio** | **0.86** | (mapped + skipped) / total |

The previous self-reported `parity_ratio = 1.0` is replaced by `fill_ratio = 0.86` in the manifest's `[parity]` block.

## What this audit does NOT do

- **It does not change runtime behaviour.** Every cave-apiserver test continues to pass; the only edits to the crate are the manifest block + this doc.
- **It does not land the CEL evaluator** (the single largest gap). A real CEL port is 3–5K LOC of work (parser + interpreter + value model + admission integration); deferred to a follow-up.
- **It does not synthesise live OpenAPI v3 from registered CRDs.** Same scope reasoning.

## What the 7 unmapped entries actually mean

1. **CEL evaluator** — `staging/src/k8s.io/apiserver/pkg/cel/`. The validating-admission-policy code path parses CEL expressions for syntax but never evaluates them against `AdmissionRequest` payloads. Effectively makes VAP a no-op on cave today.
2. **DynamicResourceAllocation** — `pkg/apis/resource/`. ResourceClaim / ResourceClass / PodSchedulingContext + scheduler hooks. Beta in v1.32; alpha+gated through v1.31. Cave currently rejects these as unknown CRDs.
3. **Audit backend plugin registry** — `staging/src/k8s.io/apiserver/plugin/pkg/audit/`. Cave's audit.rs has the inline log + WORM sinks; there is no upstream-style pluggable backend registry, so a new sink today requires editing the central handler.
4. **Live OpenAPI v3** — `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/`. discovery_v2.rs emits a near-static OpenAPI document; per-resource schemas synthesised from CRD types are missing.
5. **Admission plugin initializer** — `staging/src/k8s.io/apiserver/pkg/admission/initializer/`. Cave plugins are constructed manually in `cave-runtime/main.rs` rather than via the upstream initializer-chain wiring.
6. **Lease semantics** — `pkg/registry/coordination/lease/storage/`. routes.rs surfaces Lease CRUD but does not enforce the holder-identity / renewTime semantics needed for proper leader-election clients. cave-ha uses its own Raft-derived leases instead.
7. **Connect verbs** — `staging/src/k8s.io/apiserver/pkg/registry/rest/connect.go`. Pod exec / attach / port-forward proxying. cave-cri serves exec directly; the apiserver-side proxy path is not wired.

## Next steps (out of scope for this audit)

- CEL evaluator port — biggest behaviour gap. Likely its own crate or `cave-cel` module.
- DRA resources — requires both apiserver storage + scheduler hooks.
- Audit backend registry — small refactor (~300 LOC), unblock pluggable sinks.
- The other four can wait for follow-ups guided by user demand.
