# ADR-131: OPAL for Real-Time Policy Data Distribution

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 030, 089

## Context

OPA needs external data (tenant metadata, Keycloak roles, classification state) to make admission decisions. ArgoCD sync cycle (minutes) is too slow for policy-critical data changes.

## Candidates

| Approach | OPAL real-time push | ArgoCD-only bundle sync | OPA built-in data loading | Custom controller |
|---|---|---|---|---|
| Update latency | ✅ Seconds | ❌ Minutes (sync cycle) | ⚠️ Pull-based (interval) | ⚠️ Custom |
| External data sources | ✅ Keycloak, K8s API, PostgreSQL, webhooks | ❌ Git/OCI only | ⚠️ HTTP pull | ⚠️ |
| Topology-aware | ✅ Pushes to all OPA instances | ❌ Per-cluster sync | ❌ | ⚠️ |

## Decision

OPAL distributes external data (Keycloak roles, tenant metadata, classification state) to OPA in real-time. Git remains ONLY policy source of truth — OPAL accelerates data distribution, cannot introduce non-Git policy state. Direct mutation via OPAL prohibited. Failure mode: OPA continues with last-synced data. Staleness tolerance: 15min tenant metadata, 5min classification. Exceeding tolerance → P2 alert + APOL Class C freeze.

## Rejected

- **ArgoCD-only sync:** Tenant onboarding takes 3-5 minutes to propagate to OPA (ArgoCD sync cycle). Classification change takes 3-5 minutes. Too slow for security-critical state changes.
- **OPA built-in HTTP data loading:** Pull-based, not push. Polling interval creates similar latency to ArgoCD. No topology awareness (doesn't know which OPA instances need which data).
- **Custom controller:** Build cost. OPAL is a mature, purpose-built solution for this exact problem.

## Consequences

**Positive:**
- Policy data updates in seconds, not minutes.
- Tenant onboarding immediately reflected in OPA decisions.
- Classification changes propagate instantly to enforce routing and access policies.
- Git-as-truth contract preserved — OPAL is distribution layer only.

**Negative:**
- Additional component to manage (OPAL server, health monitoring).
- OPAL failure creates stale data risk (mitigated: staleness tolerance + P2 alert + APOL freeze).
- OPAL data source configuration per external system (Keycloak, K8s API).
- Classification mutability model must be well-defined — frequent classification changes create OPAL sync pressure.

## Compliance Mapping

SOC2 CC6.1 (real-time access control updates). ISO A.5.15 (access control — responsive to changes). NIS2 Art.21 (security policy responsiveness).
