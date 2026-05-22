# ADR-141: Shared-Fate & Tenant Priority

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** Platform Governance — Multi-Tenancy

**Related ADRs:** 012 (vcluster), 084 (Default-Deny), 087 (Quotas), 096 (Unit Economics), 109 (Observability Multi-Tenancy), 110 (Egress), 126 (Workload Criticality)

## Context

CAVE's multi-tenant architecture shares control-plane and platform services across tenant tiers (Soft, Hard, Dedicated). This creates shared-fate scenarios: a noisy Soft tenant could degrade shared Prometheus performance affecting Hard tenant observability. FinOps kill-switch could suspend workloads in ways that create cascading tenant impact.

"Best effort" SLA for Soft tier is undefined — tenants don't know what it means operationally.

---

## Candidates

| Approach | Shared-fate priority model (chosen) | Equal treatment (no priority) | Strict isolation per tenant | Priority by size |
|---|---|---|---|---|
| Resource contention handling | ✅ Dedicated > Hard > Soft degradation order | ❌ All degraded equally | ❌ N/A (no sharing) | ⚠️ Size ≠ criticality |
| Cost efficiency | ✅ Shared infra with priority | ✅ | ❌ Dedicated clusters expensive | ✅ |
| SLA honoring | ✅ Dedicated SLA protected first | ❌ SLA violations | ✅ Always | ⚠️ |
| Multi-tenant economics | ✅ Works at scale | ✅ | ❌ Not viable | ✅ |
| Predictability for paying tenants | ✅ Tier determines priority | ❌ Luck of the draw | ✅ | ⚠️ Size-dependent |

## Decision

## ### Noisy Neighbor Prevention

| Mechanism | Scope | Enforcement |
|---|---|---|
| ResourceQuota + LimitRange | Per tenant per namespace per env | OPA validates at admission (ADR-087) |
| Cilium bandwidth management | Per-pod egress/ingress bandwidth caps | eBPF EDT (Earliest Departure Time) |
| Kafka consumer quotas | Per tenant-id consumer group | Strimzi/Confluent quota config |
| Prometheus cardinality limits | Per-tenant metric label cardinality cap | Prometheus recording rules + Grafana alerts |
| Search query rate limits | Per-tenant query rate | OpenSearch/Azure AI Search plugin |

### Priority Model

| Priority | Applies To | K8s PriorityClass | FinOps Behavior | Incident Response |
|---|---|---|---|---|
| **P0 — Platform** | Control-plane pods (ArgoCD, Crossplane, OPA, Cilium, etc.) | system-cluster-critical | Never suspended | Platform team immediate |
| **P1 — Dedicated business-critical** | Dedicated tier `business-critical` workloads | dedicated-critical (1000) | Never auto-suspended | 1h SLA |
| **P2 — Hard business-critical** | Hard tier `business-critical` workloads | hard-critical (900) | Never auto-suspended | 4h SLA |
| **P3 — Standard workloads** | All tiers `standard` workloads | tenant-standard (500) | Suspended at 150% budget | Tier-dependent |
| **P4 — Batch** | All tiers `batch` workloads | tenant-batch (100) | Suspended at 120% budget | Best effort |
| **P5 — Soft best-effort** | Soft tier all workloads (when P4 is exhausted) | soft-besteffort (50) | First throttled under pressure | Business hours only |

### "Best Effort" Operational Definition (Soft Tier)

| Aspect | What It Means |
|---|---|
| Availability | No SLA commitment. Platform makes reasonable effort. |
| Resource pressure | Soft workloads are first to be throttled/evicted when cluster resources are constrained. |
| Incident response | Business hours only. No pager escalation for Soft-only incidents. |
| Feature access | All platform features available (same UX) but no guaranteed performance or capacity. |
| Egress | Default quota. Quarantine autonomous at any confidence level. |
| Support queue | Soft tickets processed after Hard/Dedicated in same severity. |

### Emergency Freeze Impact

During platform incident or APOL freeze:
- All tiers: running workloads continue (survivability invariant)
- Soft: new deployments paused until freeze lifts
- Hard: new deployments require guardian approval during freeze
- Dedicated: new deployments allowed (isolated resources)

---

## Rejected

- **No priority model (equal treatment):** During resource contention, all tenants degraded equally. Dedicated tier paying premium gets same treatment as Soft tier. Unfair and violates SLA commitments.
- **Strict isolation only (no shared fate):** Complete isolation for every tenant = dedicated clusters = enormous cost. Shared infrastructure with priority model is the cost-effective middle ground.
- **Priority by tenant size (largest first):** Size doesn't equal criticality. Priority should be based on tier (SLA commitment) not usage volume.

## Consequences

## ### Positive
- "Best effort" is now operationally defined and communicable to tenants
- Priority inversion prevented: `business-critical` always wins over `batch` regardless of tier
- Noisy neighbor mechanisms prevent shared-resource degradation
- Emergency freeze behavior explicit per tier

### Negative
- Soft tier tenants may perceive unfair treatment (mitigated: transparent SLA documentation, upgrade path to Hard)
- Priority model adds K8s PriorityClass complexity (6 levels)
- Kafka/search quotas require per-tenant configuration (mitigated: Backstage template automates at onboarding)

## Compliance Mapping

SOC2 CC6.1 (multi-tenant resource governance). SOC2 CC7.5 (availability — tenant priority model). ISO A.5.23 (cloud service — tenant isolation and priority). ISO A.8.22 (resource segregation). NIS2 Art.21 (availability — noisy neighbor prevention). GDPR Art.32 (availability of processing — tenant priority during resource contention).
