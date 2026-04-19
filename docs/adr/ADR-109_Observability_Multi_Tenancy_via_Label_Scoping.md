# ADR-109: Observability Multi-Tenancy via Label Scoping

**Status:** Accepted

**Scope:** Universal

**Category:** Observability

**Related ADRs:** 029

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Multi-tenant observability must prevent cross-tenant data visibility while using shared Prometheus/Loki infrastructure.

## Candidates

## | Mechanism | Enforcement Layer | Purpose |
|---|---|---|
| tenant-id label injection | Admission webhook | Every metric/log gets tenant-id at ingestion |
| Missing label rejection | Webhook | Metrics/logs without tenant-id are rejected |
| Grafana org-per-tenant | Grafana | Tenant sees only own dashboards |
| Datasource token scoping | Grafana | Org token can only query own tenant-id |
| Direct API block | CiliumNetworkPolicy | Prometheus/Loki API not accessible except through Grafana |

## Decision

## Tenant-id label injected at metric/log ingestion via admission webhook. Missing/mismatched labels rejected. Grafana org-per-tenant with pre-provisioned dashboards. Direct Prometheus/Loki API access blocked by NetworkPolicy — all queries traverse Grafana with org-scoped datasource tokens.

## Rejected

## - **No label enforcement:** Cross-tenant visibility. Tenant A can query Tenant B's metrics.
- **Index-per-tenant:** Loki doesn't support multi-index natively. Prometheus labels are more efficient than separate instances.
- **Frontend-only restriction (Grafana RBAC):** Bypassed via direct API call to Prometheus/Loki. NetworkPolicy blocks this path.

## Consequences

## **Positive:**
- Complete tenant isolation in shared observability stack.
- No separate Prometheus/Loki per tenant (cost efficient).
- Dashboard pre-provisioning via Grafana provisioning — tenants get dashboards at onboarding.
- NetworkPolicy enforcement means even API-savvy users cannot bypass.

**Negative:**
- Admission webhook adds small latency to metric/log ingestion.
- Label cardinality: tenant-id adds one label to every metric/log. At scale, this increases Prometheus TSDB size.
- Grafana org management scales linearly with tenant count (mitigated: automated provisioning via Crossplane/ArgoCD).

## Compliance Mapping

## SOC2 CC6.1 (data segregation). ISO A.8.22 (segregation in networks). GDPR Art.32 (security of processing — tenant data isolation).
