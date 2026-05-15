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
| `[[mapped]]` | 27 | Core apiserver framework + handlers + admission + auth + audit + CRD + aggregator + per-resource registries + **CEL evaluator** |
| `[[skipped]]` | 17 | Bootstrap, client-go, code generators, kube-proxy, kubelet probes, cloud-providers, metrics, legacy CCM |
| `[[unmapped]]` | 6 | Real gaps: DRA resources, audit-backend plugin registry, live OpenAPI synthesis, admission initializer, Lease semantics, exec/attach proxy |
| **Total** | **50** | |
| **fill_ratio** | **0.88** | (mapped + skipped) / total |

**Trajectory (2026-05-12):**
1. Initial measured-audit landing: `1.0` (wave3 self-report) → `0.86` (26 mapped / 17 skipped / 7 unmapped).
2. CEL evaluator MVP landed same day: `0.86` → `0.88` (27 / 17 / 6).

The `staging/src/k8s.io/apiserver/pkg/cel/` package moved from `[[unmapped]]` to `[[mapped]]` — see [`crates/cave-apiserver/src/cel_eval.rs`](../../crates/cave-apiserver/src/cel_eval.rs). Implementation is `CelInterpreterEvaluator` backed by the `cel-interpreter` crate (pure-Rust CEL spec subset). The evaluator wires into the existing `vap_advanced::Dispatcher` via the `CelEvaluator` trait; the `PanicEvaluator` stub remains for test gating but is no longer the default.

## What this audit does NOT do (still)

- **DRA resources** — `pkg/apis/resource/` still unmapped (ResourceClaim / ResourceClass / scheduler hooks). cave-scheduler has the scheduling-side dra.rs but the storage + controller-side surface is not in cave-apiserver.
- **Audit backend plugin registry**, **live OpenAPI v3 synthesis**, **admission initializer**, **Lease holder-identity semantics**, **connect-verb proxy** — all still unmapped, sized between 200 LOC and ~1 K LOC of follow-up work each.

## What the 6 remaining unmapped entries mean

1. **DynamicResourceAllocation** — `pkg/apis/resource/`. ResourceClaim / ResourceClass / PodSchedulingContext + scheduler hooks. Beta in v1.32; alpha+gated through v1.31. Cave currently rejects these as unknown CRDs.
2. **Audit backend plugin registry** — `staging/src/k8s.io/apiserver/plugin/pkg/audit/`. Cave's audit.rs has the inline log + WORM sinks; there is no upstream-style pluggable backend registry, so a new sink today requires editing the central handler.
3. **Live OpenAPI v3** — `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/`. discovery_v2.rs emits a near-static OpenAPI document; per-resource schemas synthesised from CRD types are missing.
4. **Admission plugin initializer** — `staging/src/k8s.io/apiserver/pkg/admission/initializer/`. Cave plugins are constructed manually in `cave-runtime/main.rs` rather than via the upstream initializer-chain wiring.
5. **Lease semantics** — `pkg/registry/coordination/lease/storage/`. routes.rs surfaces Lease CRUD but does not enforce the holder-identity / renewTime semantics needed for proper leader-election clients. cave-ha uses its own Raft-derived leases instead.
6. **Connect verbs** — `staging/src/k8s.io/apiserver/pkg/registry/rest/connect.go`. Pod exec / attach / port-forward proxying. cave-cri serves exec directly; the apiserver-side proxy path is not wired.

## CEL evaluator MVP — what landed

`src/cel_eval.rs`:
- `CelInterpreterEvaluator` implements the `crate::vap_advanced::CelEvaluator` trait.
- Pure-Rust CEL via the `cel-interpreter` crate (Google CEL spec subset). Supports:
  - Scalar comparison ops (`==`, `!=`, `<`, `>`, `<=`, `>=`)
  - Logical ops (`&&`, `||`, `!`)
  - Integer arithmetic
  - String literals (`'foo'`, `"foo"`)
  - Field traversal (`object.spec.replicas`, `object.metadata.labels.team`)
  - `has(path)` macro for presence tests
  - `.startsWith(prefix)` method on strings
  - List indexing (`params[0].maxReplicas`)
- Activation slots: `object`, `oldObject`, `request`, `params`, `namespaceObject`, named user `variables`.
- Program cache: `Mutex<HashMap<String, Arc<Program>>>` so the same expression compiles once.
- Error mapping: ParseError → `CelError::Compile`, runtime/type errors → `CelError::Runtime`, non-scalar result → `CelError::Type` (dispatcher treats this as fail-policy outcome).

**Tests (23 in `cel_eval::tests`):**
- 18 grammar unit tests (boolean literals, arithmetic, comparisons, equality, logical ops, has() present/missing, startsWith, oldObject, paramRef list indexing, request metadata, user variables, invalid syntax, undeclared reference, missing field traversal, program cache, dyn-trait compat, list-result type error).
- 5 Dispatcher integration tests: admit on pass, deny on fail, matchCondition short-circuit (empty outcome), FailurePolicy::Fail surfaces Error, FailurePolicy::Ignore surfaces SilencedError.

All 23 pass; full apiserver suite 951/951 pass; `cargo check --workspace` clean.

## Out of scope (honestly)

- **MessageExpression evaluation** — the `Validation::message_expression` field is recognised but the dispatcher uses the literal `message` only. Wiring is a 30-line addition once the audit data shows demand.
- **Authorization functions** (`authorizer.path(...).check('read').allowed()`) — the `authorizer` activation slot is not bound, so policies that consult RBAC at evaluation time fail with `CelError::Runtime` (undeclared reference). Real adoption needs a bridge from the apiserver authorizer chain into cel-interpreter's `Function` registry.
- **Timestamp + Duration types** — cel-interpreter supports them under the `chrono` feature; not enabled here to keep the dep tree small. Easy follow-up.
- **CEL CSE / optimization** — cel-interpreter compiles to an AST and walks it on each evaluate. Upstream cel-go does sub-expression caching. Out of scope for MVP.
