# ADR-087: ResourceQuota + LimitRange per Tenant

**Status:** Accepted

**Scope:** Universal

**Category:** Multi-Tenancy

**Related ADRs:** 084

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Multi-tenant clusters must prevent noisy neighbor effects. Without quotas, a single tenant can consume all cluster resources. Without limit ranges, individual pods can monopolize node resources.

## Candidates

## | Mechanism | ResourceQuota | LimitRange | PriorityClass |
|---|---|---|---|
| Scope | Namespace-level aggregate | Per-pod/container defaults | Scheduling priority |
| Prevents | Total tenant resource overconsumption | Individual pod overconsumption | Important pods evicted first |
| OPA enforced | ✅ Quota existence validated | ✅ LimitRange existence validated | ✅ PriorityClass required |

## Decision

## ResourceQuota + LimitRange applied per tenant per environment per tier. OPA validates at admission. Tier defaults (prod): Soft 4CPU/8Gi, Hard 16CPU/32Gi, Dedicated custom. Quota increase via Backstage self-service with Tenant Admin approval. K8s PriorityClass enforces scheduling priority aligned with workload criticality labels (ADR-126).

## Rejected

## - **No quotas:** Single tenant consumes all cluster CPU/memory. Other tenants starved. Classic noisy neighbor.
- **Cluster-level quotas only:** No per-tenant control. Fair-share impossible to enforce.
- **Manual quota management:** Doesn't scale with tenant count. Quota changes require platform team intervention.

## Consequences

## **Positive:**
- Noisy neighbor prevention at namespace level.
- Per-pod resource limits prevent individual pod runaway.
- Self-service quota increase via Backstage reduces platform team toil.
- OPA ensures every namespace has both ResourceQuota and LimitRange.

**Negative:**
- Quota limits can block legitimate tenant growth if set too conservatively.
- Quota increase approval process adds friction (mitigated: self-service via Backstage).
- LimitRange defaults may not suit all workload types (ML training pods need more resources than web services).

## Compliance Mapping

## SOC2 CC6.1 (resource access controls). ISO A.8.22 (resource segregation). NIS2 Art.21 (availability — prevent resource exhaustion).
