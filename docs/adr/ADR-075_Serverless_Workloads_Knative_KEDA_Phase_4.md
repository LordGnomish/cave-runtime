# ADR-075: Serverless Workloads — Knative + KEDA (Phase 4)

**Status:** Proposed (Phase 4)

**Category:** Platform

**Related ADRs:** 067, 095

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Some tenant workloads have bursty traffic patterns suited to serverless execution: event-driven functions, webhook handlers, batch processors, and scheduled data transforms. KEDA already exists in CAVE for Reflex Engine triggers (ADR-095). Adding Knative would provide HTTP-triggered serverless with scale-to-zero.

## Candidates

## | Criteria | Knative Serving + KEDA | OpenFaaS | AWS Lambda / Azure Functions | KEDA only (no Knative) |
|---|---|---|---|---|
| HTTP scale-to-zero | ✅ Knative Serving | ✅ | ✅ | ❌ No HTTP routing/cold start |
| Event-driven scaling | ✅ KEDA (already deployed) | ⚠️ Separate scaling | ✅ | ✅ |
| K8s native | ✅ CRDs | ✅ | ❌ Proprietary runtime | ✅ |
| Cold start management | ✅ Knative concurrency + min replicas | ⚠️ | ✅ (vendor-managed) | ❌ |
| Revision management | ✅ Knative revisions + traffic split | ❌ | ⚠️ | ❌ |
| License | Apache 2.0 (both) | MIT | Proprietary | Apache 2.0 |

## Decision

## **Knative Serving** for HTTP-triggered serverless. **KEDA** for event-driven scaling (already deployed for Reflex Engine). **Phase 4** — built only when tenant explicitly requests serverless runtime. Exempt from complexity budget removal rule (documented Phase 4 exception in One Prompt).

## Rejected

## - **OpenFaaS:** Smaller community than Knative. Separate scaling model — KEDA already in CAVE for Reflex Engine, adding OpenFaaS scaling is redundant.
- **AWS Lambda / Azure Functions:** Cloud-specific proprietary runtimes. Not self-hostable on Hetzner. Vendor lock-in.
- **KEDA only (no Knative):** KEDA handles event-driven scaling but not HTTP request routing, cold start management, concurrency control, or revision-based traffic splitting. Knative provides the HTTP serverless layer that KEDA doesn't.

## Consequences

## **Positive:**
- Serverless capability when tenants need it — scale-to-zero saves cost for bursty workloads.
- KEDA already deployed — no new scaling infrastructure.
- Knative revisions enable canary-style serverless deployments.
- Apache 2.0 for both components.

**Negative:**
- Knative adds ~20 CRDs and several controllers — GOT impact.
- Cold start latency (seconds) may not suit all workloads — must be documented for tenants.
- Phase 4 deferral means serverless not available in first 12 months.
- Knative + Istio ambient integration maturity must be validated before deployment.

## Compliance Mapping

## Phase 4 — compliance mapping deferred until build decision. Expected: SOC2 CC6.1 (serverless access controls), ISO A.8.25 (secure development — serverless security model).
