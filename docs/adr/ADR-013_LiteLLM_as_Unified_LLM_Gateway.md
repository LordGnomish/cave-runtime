# ADR-013: LiteLLM as Unified LLM Gateway

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** AI/LLM

**Related ADRs:** 009, 103, 111

## Context

CAVE needs a single API gateway for LLM inference that routes requests to different backends based on data classification, provides token metering for FinOps, and enforces PII redaction.

## Candidates

| Criteria | LiteLLM | Direct API | Kong AI Plugin | Portkey | MLflow Gateway |
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

**LiteLLM** as unified LLM gateway for all profiles. Routes requests to Ollama (Hetzner) or Azure OpenAI (Azure) based on data classification. Provides token metering for per-tenant FinOps attribution. PII redaction via Microsoft Presidio middleware before any LLM provider receives data.

## Rejected

- **Direct API calls:** No unified interface. Each application must implement provider-specific SDKs, classification routing, token counting, and PII redaction independently. Massive duplication across tenant applications.
- **Kong AI Gateway plugin:** Kong handles API-level routing and security; LLM routing is application-domain logic (classification-aware). Mixing infrastructure gateway with ML inference concerns violates separation of responsibilities. Kong plugin lacks Presidio PII integration and Langfuse observability.
- **Portkey:** SaaS-only. Data transits external service — incompatible with restricted/confidential classification requirements and sovereign hosting.
- **MLflow Gateway (AI Gateway):** Limited provider support. No classification-based routing. No PII redaction middleware. Better suited for experiment tracking (which MLflow already handles).

## Consequences

**Positive:**
- Single API for all LLM providers — developers don't need to know which backend serves their request.
- Classification-based routing enforced at platform level, not application level.
- Token metering enables per-tenant, per-request AI cost attribution for FinOps (ADR-096).
- PII redaction protects tenant data before it leaves the platform.
- Langfuse integration provides prompt/response observability per classification rules.

**Negative:**
- Additional component to manage (LiteLLM server deployment, config, upgrades).
- Single point of failure for all AI inference (mitigated: LiteLLM HA deployment, health checks, APOL monitoring).
- Presidio NER accuracy is not 100% — known limitation for compliance-grade PII detection (supplemented by structured field removal for confidential/restricted classifications).

## Implementation Reference

**cave-llm-gateway** crate (3689 LoC) is the Rust reimplementation of LiteLLM, embedded in cave-runtime. Provides the same multi-provider routing, classification-based dispatch, token metering, and PII redaction — but as a native Rust module with zero Python dependency. LiteLLM is tracked as the upstream reference for API compatibility and feature parity.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| LiteLLM relicenses or goes proprietary (BerriAI monetization) | Medium | Low | cave-llm-gateway is independent Rust implementation. LiteLLM upstream tracked for feature ideas only — no runtime dependency. |
| LiteLLM API surface changes faster than cave-llm-gateway tracks | Medium | Medium | OpenAI-compatible API is the stable contract. LiteLLM-specific extensions are optional. Quarterly parity review. |
| Envoy AI Gateway reaches GA | Medium (2027) | Positive | **Watch:** Envoy AI Gateway (CNCF) will standardize LLM routing at the infrastructure layer. When GA, evaluate as complementary to cave-llm-gateway — Envoy handles L7 routing, cave-llm-gateway handles classification + PII. Not a replacement, potentially a layer below. Annual review. |
| PII redaction false negatives (Presidio misses PII) | Medium | High | Defense in depth: structured field removal for CONFIDENTIAL/RESTRICTED data + Presidio NER for best-effort on unstructured text. Langfuse audit trail enables post-hoc PII detection review. |
| Token metering drift (inaccurate cost attribution) | Low | Medium | Cross-validate cave-llm-gateway token counts against provider invoices monthly. Langfuse records per-request token usage as ground truth. |
| New LLM provider not supported | Low | Low | OpenAI-compatible API covers 90%+ of providers. Custom provider adapter is ~50 lines of Rust. cave-llm-gateway designed for easy provider addition. |

## Compliance Mapping

SOC2 CC6.1 (AI access controls, classification-based routing). GDPR Art.25 (data protection by design — PII redaction before external providers). ISO A.5.12 (classification of information applied to AI inference). NIS2 Art.21 (supply chain risk — LLM provider data handling).
