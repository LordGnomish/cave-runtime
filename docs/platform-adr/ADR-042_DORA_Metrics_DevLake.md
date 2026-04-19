# ADR-042: DORA Metrics — DevLake

**Status:** Accepted

**Scope:** Universal

**Category:** Observability

**Related ADRs:** 010

## Context

CAVE needs to measure software delivery performance across all tenants: deployment frequency, lead time for changes, change failure rate, mean time to recovery (MTTR). DORA metrics are the industry standard.

## Candidates

| Criteria | DevLake | Backstage DORA plugin | Custom Prometheus metrics | Sleuth/LinearB |
|---|---|---|---|---|
| DORA metrics | ✅ Native (DF, LT, CFR, MTTR) | ⚠️ Basic | ⚠️ Custom build | ✅ |
| Data sources | ✅ GitHub, GitLab, Jira, Jenkins, ArgoCD | ⚠️ GitHub only | ❌ Custom | ✅ SaaS integrations |
| Dashboard | ✅ Grafana plugin | ✅ Backstage | ✅ Grafana | ✅ SaaS dashboard |
| Self-hosted | ✅ K8s Helm | ✅ | ✅ | ❌ SaaS |
| Multi-tenant | ✅ Project-based scoping | ⚠️ | ⚠️ | ❌ |
| License | Apache 2.0 | Apache 2.0 | N/A | Proprietary |

## Decision

**DevLake** (Apache Incubating) for DORA metrics collection and reporting. CI stage 26 sends deployment event to DevLake webhook. Grafana dashboards visualize per-tenant DORA metrics. DevLake data scoped per tenant — platform-wide DORA visible to Platform Admin only.

## Rejected

- **Backstage DORA plugin:** Basic metrics, limited data source integration. DevLake provides richer analysis.
- **Custom Prometheus:** Would require building DORA calculations from scratch. DevLake provides this out of box.
- **Sleuth/LinearB:** SaaS. Code metadata sent externally. Contradicts sovereign profile.

## Consequences

**Positive:**
- Industry-standard DORA metrics for all tenants.
- Grafana integration — DORA visible alongside platform observability.
- Per-tenant scoping — tenants see only their delivery metrics.
- Apache 2.0 — no licensing concerns.

**Negative:**
- DevLake requires MySQL/PostgreSQL backend + ~1GB RAM.
- Data source connectors (GitHub, ArgoCD) need API token configuration per profile.
- DORA metric accuracy depends on CI event reporting discipline (stage 26).

## Compliance Mapping

SOC2 CC7.1 (operational monitoring — delivery performance). ISO A.14.2 (secure development — delivery performance measurement).
