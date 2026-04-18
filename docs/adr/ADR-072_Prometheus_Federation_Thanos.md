# ADR-072: Prometheus Federation — Thanos

**Status:** Accepted

**Category:** Observability

**Related ADRs:** 029, 109

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE runs Prometheus per profile (7 profiles). Platform team needs cross-profile metric queries for: global SLO dashboards, cross-provider comparison, APOL anomaly detection baselines. Long-term metric retention beyond Prometheus default (15d).

## Candidates

## | Criteria | Thanos | Cortex/Mimir | VictoriaMetrics | Prometheus Federation (native) |
|---|---|---|---|---|
| Long-term storage | ✅ MinIO/ADLS (object storage) | ✅ | ✅ | ❌ |
| Cross-cluster query | ✅ Thanos Query (fan-out) | ✅ | ✅ | ⚠️ /federate endpoint |
| PromQL compatible | ✅ Full | ✅ Full | ✅ Extended | ✅ |
| K8s native | ✅ Sidecar + Store + Query + Compact | ✅ | ✅ | N/A |
| Downsampling | ✅ Automatic (5m, 1h) | ✅ | ✅ | ❌ |
| License | Apache 2.0 | AGPL-3.0 (Mimir) | Apache 2.0 | Apache 2.0 |

## Decision

## **Thanos** for cross-profile Prometheus federation and long-term storage. Thanos Sidecar per Prometheus instance. Thanos Store Gateway reads from MinIO (Hz) / ADLS (Az). Thanos Query provides unified PromQL endpoint across all profiles. Retention: 90d raw, 1y downsampled. Thanos queries are ephemeral — data not copied cross-region (metadata residency compliance).

## Rejected

## - **Cortex/Mimir:** AGPL-3.0 (Mimir). Cortex is feature-frozen. Thanos sidecar model is simpler than Mimir's write-path model for CAVE's use case.
- **VictoriaMetrics:** Capable but smaller community for K8s-native deployment. Thanos + Prometheus is the CNCF standard.
- **Native Prometheus federation:** /federate endpoint is pull-based, resource-intensive, doesn't provide long-term storage.

## Consequences

## **Positive:**
- Unified PromQL across all 7 profiles — global SLO dashboards, cross-provider comparison.
- Long-term storage (1y downsampled) in MinIO/ADLS at object storage cost.
- Automatic downsampling (5m, 1h) reduces query time for historical data.
- Thanos Query fan-out is ephemeral — no data copied cross-region (metadata residency compliance).

**Negative:**
- Thanos Sidecar per Prometheus instance adds ~200MB RAM per profile.
- Thanos Store Gateway requires object storage access — additional network path.
- Thanos Compactor must run single-instance per bucket (no HA for compaction).
- Cross-profile queries slower than single-Prometheus queries (fan-out latency).

## Compliance Mapping

## SOC2 CC7.1 (long-term monitoring data retention). ISO A.8.15 (logging — metric retention). NIS2 Art.21 (monitoring).
