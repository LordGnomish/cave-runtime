# ADR-027: Kong API Gateway

**Status:** Accepted

**Scope:** Universal

**Category:** Networking

**Related ADRs:** 004, 122

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs an API gateway for north-south traffic: tenant-facing API routing, rate limiting, authentication, request transformation, and API lifecycle management.

## Candidates

## | Criteria | Kong | NGINX Ingress | Traefik | Envoy Gateway | Emissary |
|---|---|---|---|---|---|
| Rate limiting | ✅ Native plugin (per-tenant, per-user, per-API) | ⚠️ Basic (annotation-based) | ⚠️ Basic middleware | ⚠️ Via Envoy filters | ⚠️ |
| JWT/OAuth2 auth | ✅ Native plugins | ❌ Requires external auth proxy | ⚠️ ForwardAuth middleware | ⚠️ Via ext_authz | ⚠️ |
| OpenAPI validation | ✅ Native plugin | ❌ | ❌ | ❌ | ❌ |
| Request transformation | ✅ Rich plugin | ❌ | ⚠️ Limited | ⚠️ | ⚠️ |
| API versioning (Sunset headers) | ✅ Plugin-based | ❌ | ❌ | ❌ | ❌ |
| Prometheus metrics | ✅ Native | ✅ | ✅ | ✅ | ✅ |
| Admin API | ✅ Full REST API (declarative config via decK) | ❌ ConfigMap-based | ❌ | ❌ | ❌ |
| Plugin ecosystem | 100+ plugins | Limited | ~30 middlewares | Envoy filters | Limited |
| K8s Gateway API | ✅ Supported | ⚠️ Partial | ✅ | ✅ (native) | ⚠️ |
| License | Apache 2.0 (Kong OSS) | Apache 2.0 | MIT | Apache 2.0 | Apache 2.0 |

## Decision

## **Kong** (OSS) for north-south API gateway.

## Rejected

## - **NGINX Ingress:** No native rate limiting, JWT auth, or OpenAPI validation plugins. Would require external tools (OPA sidecar, custom auth proxy) for features Kong provides natively. Too much glue code.
- **Traefik:** Good for simple routing but lacks Kong's rich plugin ecosystem for enterprise API management (rate limiting tiers, API deprecation headers, request transformation).
- **Envoy Gateway / Emissary:** Envoy is the proxy Kong uses internally. Going direct adds operational complexity without the plugin abstraction Kong provides.

**L7 boundary:** Kong handles north-south only. Istio ambient handles east-west (service-to-service mTLS). Cilium handles L3/L4 network policy. No overlap (ADR-004, ADR-122).

## Consequences

## (+) Rich plugin ecosystem covers all API gateway needs. Native rate limiting per tenant tier. OpenAPI validation. Sunset/Deprecation headers for API lifecycle. Prometheus metrics for FinOps (per-request per-tenant cost attribution).
(-) Kong configuration complexity (mitigated by decK declarative config in Git, ArgoCD-managed). Resource overhead (~500MB RAM). Kong upgrades require careful plugin compatibility testing.

## Compliance Mapping

## SOC2 CC6.1 (API access controls), NIS2 Art.21 (API security).
