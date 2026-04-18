# ADR-122: Cilium Gateway API Reserved for Future Internal Routing

**Status:** Accepted

**Category:** Networking

**Related ADRs:** 004, 027

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Cilium supports full K8s Gateway API. Kong handles all tenant API traffic. The question: should Cilium Gateway API replace or supplement Kong?

## Candidates

## | Role | Kong (current) | Cilium Gateway API (reserved) |
|---|---|---|
| Tenant-facing APIs | ✅ Full plugin ecosystem | ❌ No rate limiting, JWT, OpenAPI validation plugins |
| Internal platform routing | ⚠️ Overkill (plugins unnecessary) | ✅ Lightweight, eBPF-native |
| L7 features | ✅ 100+ plugins | ⚠️ Basic HTTP routing only |

## Decision

## Cilium Gateway API reserved for future internal platform routing optimization where Kong's L7 features are unnecessary (e.g., internal admin traffic, inter-service platform communication). Kong remains primary for all tenant API traffic. Cilium Gateway API not deployed in current phase.

## Rejected

## - **Replace Kong with Cilium Gateway:** Insufficient plugin ecosystem for tenant APIs (no rate limiting, JWT auth, OpenAPI validation, request transformation, Sunset headers). Would require building all these as custom Envoy filters.
- **Run both for same traffic:** Complexity without benefit. Two routing layers for same request adds latency and debugging difficulty.

## Consequences

## **Positive:**
- Clear separation: Kong = tenant APIs, Cilium Gateway = future internal routing.
- No premature deployment of unused capability.
- Path exists for future optimization when internal routing needs lighter-weight gateway.

**Negative:**
- Internal platform traffic currently routes through Kong (minor overhead).
- Cilium Gateway API deployment deferred — must be evaluated when internal routing optimization is justified.

## Compliance Mapping

## N/A (reserved, not deployed).
