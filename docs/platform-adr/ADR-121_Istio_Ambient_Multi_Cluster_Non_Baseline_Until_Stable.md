# ADR-121: Istio Ambient Multi-Cluster — Non-Baseline Until Stable

**Status:** Accepted

**Scope:** Universal

**Category:** Networking

**Related ADRs:** 004

## Context

Istio ambient single-cluster is CAVE's production baseline. Multi-cluster ambient could enable active-active multi-region within same provider. Current upstream status: Beta (promoted from alpha at KubeCon EU 2026) — not yet Stable.

## Candidates

| Multi-region approach | Cloudflare DNS failover (current) | Istio ambient multicluster | Cilium ClusterMesh |
|---|---|---|---|
| Active-active | ❌ Active-passive | ✅ Active-active | ⚠️ L3/L4 only |
| L7 policy | ✅ (single cluster) | ✅ Cross-cluster | ❌ |
| Maturity | ✅ Production-proven | ⚠️ Beta (not Stable) | ✅ GA |

## Decision

Ambient single-cluster is production baseline. Multicluster remains non-baseline until: (1) upstream reaches Istio **Stable** (currently Beta as of KubeCon EU April 2026), AND (2) CAVE completes parity tests, failure drills, and tenant SLA validation. Guardian sign-off alone insufficient — upstream maturity is prerequisite. Current multi-region: Cloudflare DNS failover.

## Rejected

- **Promote to baseline now (Beta):** Beta APIs may change. Breaking changes in mesh multi-cluster disrupt all cross-cluster traffic. Risk too high for multi-tenant production.
- **Cilium ClusterMesh instead:** L3/L4 only. No unified L7 policy model across clusters. Istio ambient provides L4+L7 in one model.
- **No multi-region plan:** Phase 4 future. This ADR documents the gate conditions.

## Consequences

**Positive:**
- Multi-cluster ambient path is planned and gate conditions are clear.
- Current Cloudflare DNS failover provides adequate multi-region protection.
- No premature adoption of Beta features in production.

**Negative:**
- Active-active multi-region deferred until Istio ambient multicluster reaches Stable.
- Cloudflare DNS failover provides active-passive only (not active-active).
- Gate conditions may take 12-18 months to satisfy (Istio release cadence).

## Compliance Mapping

SOC2 CC7.5 (availability — multi-region readiness). ISO A.5.30 (ICT readiness). NIS2 Art.21 (resilience planning).
