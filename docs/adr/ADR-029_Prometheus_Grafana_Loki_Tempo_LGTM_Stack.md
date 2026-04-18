# ADR-029: Prometheus + Grafana + Loki + Tempo (LGTM Stack)

**Status:** Accepted

**Category:** Observability

**Related ADRs:** 072, 109, 117

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs unified observability (metrics, logs, traces) across all profiles with multi-tenant isolation.

## Candidates

## | Criteria | LGTM Stack (self-hosted) | Datadog | Elastic/ELK | New Relic | Grafana Cloud |
|---|---|---|---|---|---|
| Self-hosted | ✅ Full (Helm) | ❌ SaaS | ✅ (SSPL for ES) | ❌ SaaS | ❌ SaaS |
| Metrics | Prometheus (CNCF Graduated) | Agent-based | Metricbeat | Agent-based | Hosted Prometheus |
| Logs | Loki (label-indexed, efficient) | Log Management | Elasticsearch (full-text) | Log Management | Hosted Loki |
| Traces | Tempo (trace-by-ID, no indexing) | APM | Elastic APM | APM | Hosted Tempo |
| Dashboards | Grafana (gold standard) | Datadog UI | Kibana | NR UI | Hosted Grafana |
| Multi-tenant | Grafana org-per-tenant, label scoping | Tag-based | Index-per-tenant | Account-based | Built-in |
| Long-term storage | Thanos (ADR-072) → MinIO/ADLS | Included | Hot-warm-cold | Included | Included |
| Cost (73 components, 10+ tenants) | ~€0 (self-hosted infra only) | ~€3,000-10,000/mo | SSPL license risk | ~€2,000-5,000/mo | ~€1,000-3,000/mo |
| License | Apache 2.0 (all) | Proprietary | SSPL (Elasticsearch) | Proprietary | Proprietary (SaaS) |

## Decision

## **Self-hosted LGTM stack** (Prometheus + Grafana + Loki + Tempo) on all profiles. Thanos for federation (ADR-072). Multi-tenant via tenant-id label scoping (ADR-109).

## Rejected

## - **Datadog:** SaaS-only. Per-host/per-metric pricing explodes at 73 components × 7 profiles. No self-hosting. Vendor lock-in. €3K-10K/mo vs €0.
- **Elastic/ELK:** Elasticsearch SSPL license (same concern as Vault BSL). OpenSearch chosen for search workloads (ADR-049) but not for platform observability.
- **Grafana Cloud:** SaaS. Data leaves cluster. Incompatible with sovereign/restricted data classifications.

## Consequences

## (+) Zero licensing cost. Full control. Multi-tenant by design. Same dashboards across all profiles. Loki WORM-backed (ADR-106) for forensics compliance.
(-) Operational overhead (self-managed). Prometheus scaling requires careful resource planning. Loki query performance at scale needs tuning. Tempo has limited query capability (trace-by-ID only).

## Compliance Mapping

## SOC2 CC7.1-7.2 (monitoring), ISO A.8.15-16 (logging, monitoring), NIS2 Art.21 (incident detection).
