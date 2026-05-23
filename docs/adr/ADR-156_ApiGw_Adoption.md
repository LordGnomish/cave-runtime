# ADR-156: cave-apigw adoption (Kong + Envoy reimplementation)

| Field        | Value                                       |
| ------------ | ------------------------------------------- |
| Status       | Accepted                                    |
| Date         | 2026-05-23                                  |
| Owner        | Cave Runtime                                |
| Replaces     | n/a (new crate)                             |
| Related      | ADR-155 (cave-cert-manager) — ACME hook     |

## Context

The runtime needs a first-party API gateway that:
1. Terminates north-south HTTP / HTTPS / HTTP/2 / HTTP/3 / gRPC / WebSocket traffic.
2. Provides a configurable plugin chain for AuthN, AuthZ, rate-limiting, caching,
   request shaping and observability — matching the Kong feature surface that
   downstream applications already build against.
3. Honors Kubernetes Gateway API CRDs so cluster operators can declare
   listeners and routes in the same language as the upstream `gateway-api`
   project.
4. Supports the Cave PQC + Charter v2 hygiene matrix: AGPL-3.0-or-later code,
   pinned upstream `source_sha`, no Hetzner branding, hybrid X25519+ML-KEM
   placeholder ready for `cave-net::tls`.

Two upstream projects are the de-facto reference implementations:

- **Kong v3.9.1** (Apache-2.0, commit `b724fc7154de3a9971e33490097d5ea2c1bae93b`)
  — primary reference for entity model (route / service / upstream / target /
  consumer / plugin) and Admin REST API.
- **Envoy v1.38.0** (Apache-2.0, commit `f1dd21b16c244bda00edfb5ffce577e12d0d2ec2`)
  — secondary reference for HTTP/2 + HTTP/3 + gRPC transcoding semantics and
  upstream connection pool semantics.

A previous internal crate `cave-gateway` ports the **Gravitee** developer
portal (catalogue + plan + subscription); it remains in place and is
not replaced. `cave-apigw` is the runtime data plane, complementary to
`cave-gateway`.

## Decision

Adopt `cave-apigw` as the first-party gateway data plane. Scope:

- Routes / services / upstreams / targets / consumers / plugins (Kong-modelled).
- Load balancing: round-robin, least-connections, consistent-hashing (SHA-256),
  EWMA, random.
- Active + passive health checks.
- Protocol stack: HTTP/1.1 + HTTP/2 + HTTP/3 (QUIC settings; data path via
  `cave-net::quic`) + gRPC routing with REST→gRPC transcoding + WebSocket.
- TLS termination with SNI cert resolution; hybrid X25519+ML-KEM policy ready.
- ACME integration via a small `AcmeProvider` trait — concrete implementation
  lands in `cave-cert-manager`.
- 14 plugins covering Kong's most-used feature set (key-auth, JWT, OAuth2,
  mTLS, LDAP, rate-limiting, proxy-cache, request/response transformer,
  CORS, bot detection, IP restriction, circuit breaker, retry, security
  headers + request termination).
- Admin REST API + decK-style declarative YAML / JSON.
- K8s Gateway API CRDs: Gateway, GatewayClass, HTTPRoute, GRPCRoute,
  TLSRoute, TCPRoute, UDPRoute.
- Observability: 10 Prometheus panels + 6 alert rules; W3C traceparent
  propagation; JSON access log.

## 4-track delivery

| Track          | Output                                                                                |
| -------------- | ------------------------------------------------------------------------------------- |
| Backend        | `crates/cave-apigw` (24 src files, ~3000 LOC, 242 lib + 9 self-audit + 17 smoke PASS) |
| cavectl        | `cave gw {route, service, plugin, consumer, upstream, status}` wired in cave-cli      |
| Portal         | Admin REST API exposed by `admin::AdminApi`; UI integration tracked in apigw-next     |
| Observability  | `metrics::PROMETHEUS_PANELS` + `metrics::ALERT_RULES` + `tracing_otel::SpanCtx`       |

## Charter v2 gates (8/8 PASS)

1. manifest TOML well-formed
2. `PARITY_REPORT.md` declares 8/8
3. fill_ratio = 0.9744
4. honest_ratio = 0.6410
5. source_sha pinned for Kong + Envoy
6. parity_ratio_source = "manifest"
7. last_audit = 2026-05-23
8. all `scope_cuts` target a `cave-*` crate or `apigw-next` Phase 2 group

## Scope cuts (Phase 2)

| Group                                                          | Target       |
| -------------------------------------------------------------- | ------------ |
| envoy-xds-server                                                | cave-mesh    |
| envoy-rate-limit-service + proxy-cache Redis backend            | cave-cache   |
| envoy-quic-stack (Quinn data path)                              | cave-net     |
| ldap-bind TLS channel                                           | cave-auth    |
| postgres-store (DB-backed config)                               | cave-rdbms   |
| vitals-analytics                                                | cave-metrics |
| kong-manager UI bundle                                          | cave-portal  |
| bot-detection ML model + request-validator OpenAPI 3            | apigw-next   |

## Consequences

- Cave runtime gains a first-party L4/L7 gateway data plane with feature
  parity (within scope) against Kong and Envoy.
- `cave-cli` exposes a stable Kong-shaped command surface (`cave gw …`).
- `cave-portal` UI can integrate against the Admin REST API once
  `cave-portal::apigw` lands.
- The existing `cave-gateway` (Gravitee port) stays untouched and remains
  the developer-portal-side companion.

## References

- Kong/kong repository — Apache-2.0 — https://github.com/Kong/kong
- envoyproxy/envoy repository — Apache-2.0 — https://github.com/envoyproxy/envoy
- Kubernetes Gateway API — https://gateway-api.sigs.k8s.io
- NOTICE attribution required at workspace root.
