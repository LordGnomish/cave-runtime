# ADR-096: Unit Economics & FinOps Attribution

**Status:** Accepted

**Category:** FinOps

**Related ADRs:** 110, 126

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE is a multi-tenant SaaS backbone. Cost transparency requires per-tenant, per-request attribution. Without attribution, infrastructure costs are opaque shared overhead.

## Candidates

## | Cost Source | Metering Method | Attribution Key |
|---|---|---|
| API requests | Kong per-request metrics | tenant-id × API × HTTP method |
| LLM tokens | LiteLLM per-token metering | tenant-id × model × classification |
| Kafka messages | Consumer lag + throughput metrics | tenant-id × topic |
| Network egress | Cilium eBPF byte counters | tenant-id × destination |
| Compute | Prometheus container metrics | tenant-id × namespace |
| Storage | MinIO/ADLS usage metrics | tenant-id × bucket/prefix |

## Decision

## Per-request (Kong), per-token (LiteLLM), per-message (Kafka), per-egress (Cilium eBPF), per-compute (Prometheus), per-storage (MinIO/ADLS) cost attribution per tenant. Real-time P&L dashboards in Grafana (per-tenant org). Kill switch by workload criticality (ADR-126). Platform provides authoritative metering and export APIs; commercial invoicing remains tenant responsibility.

## Rejected

## - **No cost attribution:** Opaque shared infrastructure billing. Tenants cannot understand their cost drivers. FinOps impossible.
- **Manual cost allocation:** Inaccurate (percentage-based, not usage-based). Unfair to low-usage tenants.
- **Cloud-provider cost only:** Misses Hetzner (flat pricing, no per-request breakdown). Doesn't attribute at tenant level.

## Consequences

## **Positive:**
- Every cost attributed to the tenant that generated it. No opaque shared costs.
- Real-time P&L enables data-driven capacity decisions.
- Kill switch prevents unbounded spend with graceful degradation (ADR-126).
- Platform metering API enables tenant-built billing systems.

**Negative:**
- Metering infrastructure adds compute overhead (eBPF counters, Prometheus scraping, dashboard rendering).
- Attribution accuracy depends on tenant-id label correctness (enforced by OPA — ADR-109).
- Kill switch ethics: business-critical services never auto-suspended (requires accurate criticality labels).

## Compliance Mapping

## SOC2 CC6.1 (cost accountability per tenant). ISO A.5.23 (cloud service agreements — transparent pricing). NIS2 Art.21 (risk management — cost overrun prevention).
