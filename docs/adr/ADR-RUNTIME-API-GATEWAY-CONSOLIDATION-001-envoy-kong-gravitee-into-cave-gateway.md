# ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 — Consolidate Envoy + Kong + Gravitee into `cave-gateway`

**Status:** Accepted
**Date:** 2026-04-27
**Track:** runtime-consolidation
**Supersedes (in part):** ADR-027 (Kong API Gateway), ADR-013 (LiteLLM Gateway — for the gateway slot only; LLM features keep `cave-llm-gateway`)
**Relates to:** ADR-122 (Cilium Gateway API — reserved for future internal east-west; out of scope here), ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001, ADR-RUNTIME-STREAMING-CONSOLIDATION-001

## Context

`cave-runtime` is a single-binary Internal Developer Platform that re-implements 70+ upstream OSS tools as in-process Rust modules. Three of those upstreams cover overlapping ground at the north–south edge:

- **Envoy** — L4/L7 proxy, xDS control plane, HTTP/1.1/2/3, mTLS, observability primitives.
- **Kong** — Admin API CRUD over Services / Routes / Upstreams / Consumers / Plugins, plus a rich plugin pipeline (rate-limiting, auth, transformations, observability).
- **Gravitee** — API designer, developer portal, monetization, governance lints, federation, debug.

Running these as three separate processes gives us three of everything (config plane, hot-reload, observability, TLS, routing) — each with its own data model, its own deployment, its own attack surface. For a sovereign, single-binary IDP this is a non-starter.

Today the `cave-gateway` crate already carries a Kong-shaped Admin API and a partial xDS surface; it needs to grow into the full edge gateway with parity across all three upstreams.

## Decision

Consolidate Envoy + Kong + Gravitee into a single Rust crate `cave-gateway` with a layered model:

1. **L4/L7 data plane** (Envoy parity): listener, filter chain, routing, upstream, TLS, mTLS, observability.
2. **Management plane** (Kong parity): Admin API (`/admin/v1/*`) over Services/Routes/Upstreams/Targets/Consumers/Plugins/Certs/SNIs, plus the plugin pipeline.
3. **Designer plane** (Gravitee parity): API designer, dev portal, governance, federation, debug, marketplace.
4. **Control plane** (Envoy parity): xDS REST + JSON over HTTP — LDS / RDS / CDS / EDS / SDS.

All four planes share one in-process `GatewayStore` and one router assembly in `lib.rs`.

### What ships in cave-gateway (this ADR's scope)

| Capability                              | Upstream parity        |
|-----------------------------------------|------------------------|
| Listener + filter chain (H1/H2/H3)      | Envoy `Listener`       |
| Routing engine (path/host/header/regex) | Envoy `RouteConfig`    |
| Weighted route splitting (canary)       | Envoy `WeightedClusters` |
| Retry policy / timeout / circuit breaker| Envoy `RetryPolicy`    |
| L7 filters (RL, CORS, JWT, ext-authz, validation, lua) | Envoy + Kong combined |
| Service discovery (xDS/EDS)             | Envoy v3 xDS           |
| Active/passive health check + outlier detection | Envoy outlier   |
| TLS termination, SNI, mTLS              | Envoy + Kong           |
| ACME rotation, SPIFFE SVID, PQC hybrid  | New (sovereign req.)   |
| Access log, W3C trace, Prom + gRPC stats| Envoy access_log + tap |
| WAF (OWASP CRS rule engine)             | Envoy `network.ext_authz` extensions, Coraza-style |
| Multi-tenant routing (per-tenant vhost) | Sovereign profile      |
| Admin API CRUD (Kong shape)             | Kong 3.x Admin         |
| API designer + dev portal + governance  | Gravitee APIM          |

### What is explicitly out of scope here

- **East-west service mesh** — covered by `cave-mesh` and ADR-004; intentionally separate. `cave-gateway` is north–south only.
- **LLM-specific gateway features** (provider routing, token-budget guardrails, semantic cache) — stay in `cave-llm-gateway` (ADR-013), invoked by `cave-gateway` only as a downstream service.
- **Cilium Gateway API CRDs** — ADR-122; reserved for future internal traffic and re-evaluated separately.

### Module layout (target shape)

```
crates/cave-gateway/src/
├── lib.rs                  # router assembly, GatewayState
├── models.rs               # Kong-compatible entities
├── store.rs                # in-memory GatewayStore
├── matcher.rs              # path/host/header/method matching
├── router_engine.rs        # M2: weighted splits, retry, timeout
├── circuit_breaker.rs      # per-target CB
├── listener.rs             # M1: listener + filter chain config
├── lb.rs                   # round-robin, least-conn, ketama
├── eds.rs                  # M4: endpoint discovery
├── outlier.rs              # M4: outlier detection
├── health.rs               # active + passive
├── tls.rs                  # SNI, ACME, mTLS, PQC config
├── spiffe.rs               # M5: SVID validation
├── access_log.rs           # M6: access log
├── trace.rs                # M6: W3C trace propagation
├── stats_sink.rs           # M6: gRPC stats sink
├── tenant.rs               # M8: per-tenant virtual hosts
├── proxy.rs                # ProxyEngine
├── handler.rs              # axum handler glue
├── admin.rs                # Kong Admin API
├── xds.rs                  # Envoy xDS (REST/JSON)
├── plugins/                # L7 filter chain
│   ├── mod.rs              # GatewayPlugin trait
│   ├── ext_authz.rs        # M3
│   ├── request_validation.rs  # M3
│   ├── lua.rs              # M3 (sandboxed pre/post hooks)
│   ├── waf.rs              # M7: OWASP CRS engine
│   └── …                   # rate_limiting, cors, jwt, …
└── gravitee/               # designer + portal + governance
    └── …
```

### Milestones

The implementation lands in 8 plumbing-merged milestones on `main`. Each is independently testable and atomic:

- **M1** Listener + filter chain (HTTP/1.1 + HTTP/2 + HTTP/3 protocol model, TLS termination, conn pooling) — +60 tests.
- **M2** Routing engine — regex, weighted split, retry, timeout, integrate circuit breaker — +60 tests.
- **M3** L7 filter chain — ext-authz, request-validation, lua, ratelimit-service slot — +70 tests.
- **M4** Service discovery + health check — EDS, outlier detection — +40 tests.
- **M5** mTLS — PQC ML-KEM-768 hybrid config, SPIFFE SVID validation, ACME rotation — +50 tests.
- **M6** Observability — access log, W3C trace propagation, Prometheus emit, gRPC stats sink — +30 tests.
- **M7** WAF plugin — OWASP CRS rule engine — +40 tests.
- **M8** Multi-tenant routing — tenant_id label virtual host, default-deny cross-tenant, per-tenant RL — +40 tests.

Total target: +390 unit tests (baseline 60 → goal 450).

## Consequences

**Positive**

- Single binary, single config plane, single observability surface for the edge gateway slot. No three-process operational tax.
- All edge state lives in `GatewayStore`, so consistent cross-feature invariants are trivial (e.g. "deleting a Service cascades to its Routes and Plugins").
- Sovereign-by-default: PQC, SPIFFE, multi-tenant default-deny, OWASP CRS — all in one crate, no upstream coordination.
- Plumbing-merged milestones keep `main` green and reviewable.

**Negative**

- `cave-gateway` becomes a large crate. Mitigated by strict module boundaries and a per-milestone test cluster.
- Some Envoy v3 features (HTTP/3 transport, eBPF datapath, Wasm filters) are out of scope for the consolidation. `cave-gateway` ships the *config layer* and parsing/validation surface for these; transport layer is feature-gated and left for a follow-up ADR.
- We do not run upstream Envoy/Kong/Gravitee binaries, so we cannot import their plugin ecosystems verbatim. We pay the parity cost in our own code.

**Neutral**

- ADR-027 (Kong) is superseded *only for the gateway slot*; its plugin semantics still drive Kong-parity behaviour.
- ADR-013 (LiteLLM) keeps its scope: LLM gateway concerns stay in `cave-llm-gateway`.

## Verification

- `cargo test -p cave-gateway` passes after each milestone with the cumulative test count meeting the per-milestone target.
- `parity.manifest.toml` is updated as each milestone lands a new upstream-parity surface.
- Plumbing merge to local `main` after each milestone: `git push . HEAD:main` (fast-forward only).
