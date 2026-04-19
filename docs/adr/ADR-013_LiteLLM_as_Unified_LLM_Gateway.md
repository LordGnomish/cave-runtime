# ADR-013: LiteLLM as Unified LLM Gateway

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** AI/LLM

**Related ADRs:** 009, 103, 111

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs a single API gateway for LLM inference that routes requests to different backends based on data classification, provides token metering for FinOps, and enforces PII redaction.

## Candidates

## | Criteria | LiteLLM | Direct API | Kong AI Plugin | Portkey | MLflow Gateway |
|---|---|---|---|---|---|
| Multi-provider routing | ✅ 100+ providers, unified OpenAI-compatible API | ❌ Per-provider SDK | ⚠️ Plugin-level | ✅ | ⚠️ Limited |
| Classification-based routing | ✅ Custom router (restricted→Ollama, confidential→Azure OpenAI) | ❌ Manual | ❌ | ⚠️ | ❌ |
| Token metering | ✅ Per-request, per-tenant, per-model | ❌ Manual counting | ⚠️ Request-level only | ✅ | ⚠️ |
| PII redaction middleware | ✅ Presidio integration pre/post | ❌ Custom | ❌ | ❌ | ❌ |
| Prompt injection defense | ✅ Middleware hooks | ❌ Custom | ❌ | ⚠️ | ❌ |
| Langfuse integration | ✅ Native callback | ❌ Custom | ❌ | ✅ | ❌ |
| Self-hosted | ✅ K8s Helm | N/A | ✅ (Kong) | ❌ SaaS | ✅ |
| License | MIT | N/A | Apache 2.0 | Proprietary | Apache 2.0 |

## Decision

## **LiteLLM** as unified LLM gateway for all profiles. Routes requests to Ollama (Hetzner) or Azure OpenAI (Azure) based on data classification. Provides token metering for per-tenant FinOps attribution. PII redaction via Microsoft Presidio middleware before any LLM provider receives data.

## Rejected

## - **Direct API calls:** No unified interface. Each application must implement provider-specific SDKs, classification routing, token counting, and PII redaction independently. Massive duplication across tenant applications.
- **Kong AI Gateway plugin:** Kong handles API-level routing and security; LLM routing is application-domain logic (classification-aware). Mixing infrastructure gateway with ML inference concerns violates separation of responsibilities. Kong plugin lacks Presidio PII integration and Langfuse observability.
- **Portkey:** SaaS-only. Data transits external service — incompatible with restricted/confidential classification requirements and sovereign hosting.
- **MLflow Gateway (AI Gateway):** Limited provider support. No classification-based routing. No PII redaction middleware. Better suited for experiment tracking (which MLflow already handles).

## Consequences

## **Positive:**
- Single API for all LLM providers — developers don't need to know which backend serves their request.
- Classification-based routing enforced at platform level, not application level.
- Token metering enables per-tenant, per-request AI cost attribution for FinOps (ADR-096).
- PII redaction protects tenant data before it leaves the platform.
- Langfuse integration provides prompt/response observability per classification rules.

**Negative:**
- Additional component to manage (LiteLLM server deployment, config, upgrades).
- Single point of failure for all AI inference (mitigated: LiteLLM HA deployment, health checks, APOL monitoring).
- Presidio NER accuracy is not 100% — known limitation for compliance-grade PII detection (supplemented by structured field removal for confidential/restricted classifications).

## Compliance Mapping

## SOC2 CC6.1 (AI access controls, classification-based routing). GDPR Art.25 (data protection by design — PII redaction before external providers). ISO A.5.12 (classification of information applied to AI inference). NIS2 Art.21 (supply chain risk — LLM provider data handling).
