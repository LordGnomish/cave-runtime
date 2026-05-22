# ADR-029: Prometheus + Grafana + Loki + Tempo — LGTM Stack

**Status:** Accepted

**Scope:** Universal

**Category:** Observability / Monitoring

**Related ADRs:** 072, 106, 109, 042

## Context

CAVE platform requires unified observability across all 73 components, 7 profiles, and 10+ tenants:

- **Metrics:** CPU, memory, request latency, custom business metrics (DORA signals, ADR-042)
- **Logs:** Structured logs from platform + tenant workloads, searchable, forensic retention (ADR-106: WORM-backed for compliance)
- **Traces:** Distributed traces for multi-service request flows (authentication → API gateway → service → database)
- **Multi-tenant isolation:** Tenant can view only their metrics/logs/traces (label-scoped dashboards)
- **Cost efficiency:** 73 components × 7 profiles × 12 months self-hosted must cost less than SaaS alternatives

## Candidates

| Criteria | LGTM Stack | Datadog | Elastic/ELK | New Relic | Grafana Cloud |
|---|---|---|---|---|---|
| Self-hosted | ✅ Full Helm | ❌ SaaS | ✅ SSPL risk | ❌ SaaS | ❌ SaaS |
| Metrics | Prometheus (CNCF) | Agent-based | Metricbeat | Agent-based | Hosted Prom |
| Logs | Loki (label-indexed) | Datadog Logs | Elasticsearch (full-text) | NR Logs | Hosted Loki |
| Traces | Tempo (trace-by-ID) | APM | Elastic APM | NR APM | Hosted Tempo |
| Dashboards | Grafana | Datadog UI | Kibana | NR UI | Grafana |
| Multi-tenancy | Org-per-tenant + label scoping | Tag-based | Index-per-tenant | Org-per-tenant | Built-in |
| Long-term storage | Thanos → MinIO/ADLS | Included | Hot-warm-cold tiering | Included | Included |
| Cost (73 comp, 10 tenants, 1y) | ~€0 (infra only) | €3K-10K/mo | SSPL risk | €2K-5K/mo | €1K-3K/mo |
| License | Apache 2.0 (all) | Proprietary | SSPL | Proprietary | SaaS |
| Data residency | Full control | Datadog cloud | Self-hosted possible | NR cloud | Grafana cloud |
| WORM logging (ADR-106) | ✅ Loki WORM-backed | ❌ | ⚠️ Custom | ❌ | ❌ |

## Decision

**Self-hosted LGTM stack** on all 7 profiles:
- **Prometheus:** Metrics scrape from kubelet, Cilium (Hubble), application endpoints (custom /metrics)
- **Grafana:** Dashboards per tenant (grafana org = tenant). RBAC: tenant admins manage own org. Platform admins manage cluster dashboards.
- **Loki:** Log aggregation from stderr/stdout (k8s logging driver). WORM-backed storage (Loki + S3 immutability, ADR-106) for forensic compliance
- **Tempo:** Trace backend. Instrumentation via OpenTelemetry SDKs (all 73 components emit traces)
- **Thanos:** Multi-profile federation (sovereign-prod + azure-prod + edge metrics in single query, ADR-072)
- **Storage:** MinIO (sovereign) / Azure Blob (Azure) for long-term retention

## Implementation Reference

**Implementation Status:** Production

- **cave-metrics** crate: Prometheus deployment, scrape config management, alerting rules (PrometheusRule)
- **cave-logs** crate: Loki deployment, LogQL dashboards, log aggregation pipeline
- **cave-trace** crate: Tempo deployment, span ingestion (OTLP gRPC), trace backend
- **cave-observability** crate: Grafana provisioning, tenant-scoped dashboards, data source management
- **Storage:** MinIO (Hetzner default) or Azure Blob (Azure profiles). S3-compatible API.

## Rejected Options

### Datadog — Not Acceptable

**Reasons:**
1. **SaaS-only:** No self-hosting option. Data must leave cluster → incompatible with sovereign/restricted classifications.
2. **Per-metric pricing explosion:** At 73 components × 7 profiles, per-metric costs become prohibitive. Custom metrics from applications add overhead. Estimate: €3K-10K/mo.
3. **Vendor lock-in:** Proprietary data format. Switching away requires data extraction tools + metric re-ingestion.

### Elastic/ELK — Not Recommended

**Reasons:**
1. **SSPL license risk:** Elasticsearch switched to SSPL (Server-Side Public License) after v7.10. Same concern as HashiCorp Vault's BSL. SSPL requires derivative work disclosures. CAVE's cloud products cannot use SSPL.
2. **Alternative:** OpenSearch (AWS fork, dual-licensed) available for search workloads (ADR-049) but not ideal for observability stack.
3. **Cost comparison:** ELK self-hosted requires 3-node ES cluster + Kibana + Beats. Infrastructure cost rivals SaaS pricing.

### Grafana Cloud — Not Recommended

**Reasons:**
1. **SaaS-only:** Metrics/logs/traces leave cluster → sovereign deployment violation.
2. **Pricing non-linear:** Ingestion costs scale with volume. At 73 components × 1000+ metrics each, costs accumulate.
3. **No WORM logging:** Grafana Cloud doesn't support WORM-backed log storage (compliance requirement, ADR-106).

## Consequences

### Positive

- **Zero licensing cost:** Self-hosted. No per-metric/per-host/per-GB fees.
- **Full data control:** Metrics/logs/traces stored in cluster storage. No external data transit.
- **Multi-tenant by design:** Grafana org-per-tenant. Label-scoped dashboards (tenant A cannot query tenant B's data).
- **WORM-backed logs:** Loki + S3 immutability (ADR-106) = forensic compliance (SEC, GDPR, SOC2 CC7.1).
- **Federation across profiles:** Thanos enables querying metrics from all 7 profiles in single dashboard (ADR-072).
- **Unified observability:** Metrics + logs + traces in single tool ecosystem. Fewer tool context-switches for on-call engineers.

### Negative

- **Operational complexity:** Self-managed. Must handle scaling, upgrades, backup/restore, disk space management.
- **Prometheus scaling:** Metric cardinality can explode. 10+ tenants × 73 components × 100+ metrics per component = millions of time series. Requires memory planning.
- **Loki query performance:** Label-indexed approach scales well but complex queries can be slow. Query tuning required.
- **Tempo limitations:** Trace-by-ID only. Can't search traces by span name/duration without custom plugins.
- **Alerting complexity:** PrometheusRule + AlertManager + custom webhook handlers required for PagerDuty/Slack integration.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Prometheus runs out of memory under cardinality explosion | Medium | High | Metric relabeling drops unused labels. Cardinality limits per tenant. Monitoring via Prometheus itself. |
| Loki disk fills with log ingestion | Low | High | Log retention policy per tenant (e.g., 30d). S3 long-term archive. Monitoring disk usage %. |
| Tempo span collection overwhelms ingestion | Low | Medium | Sampling policy (e.g., 10% traces in dev, 1% in prod). OTLP batch size limits. |
| WORM storage prevents log corrections | Low | Low | WORM is feature, not risk. Compliance requirement. Mitigated: logs immutable by design. |

## License

**Prometheus:** Apache 2.0 (https://github.com/prometheus/prometheus/blob/main/LICENSE)
**Grafana:** AGPL-3.0 (https://github.com/grafana/grafana/blob/main/LICENSE) + proprietary plugins
**Loki:** AGPL-3.0 (https://github.com/grafana/loki/blob/main/LICENSE)
**Tempo:** AGPL-3.0 (https://github.com/grafana/tempo/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC7.1:** Monitoring — continuous observability of all platform components.
**SOC2 CC7.2:** Incident detection and response — alerting on anomalies triggers on-call workflow.
**ISO/IEC 27001 A.8.15:** Logging — comprehensive logging of security events, user actions, configuration changes.
**ISO/IEC 27001 A.8.16:** Monitoring activities — real-time monitoring of system and network activity.
**NIS2 Directive Article 21:** Incident detection and response — observability enables fast incident triage.
