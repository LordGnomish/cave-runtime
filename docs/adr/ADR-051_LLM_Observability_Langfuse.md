# ADR-051: LLM Observability — Langfuse

**Status:** Accepted

**Scope:** Azure, Hetzner, Universal

**Category:** AI

**Related ADRs:** 013, 103

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE routes LLM inference through LiteLLM gateway (ADR-013) to Ollama (Hetzner) or Azure OpenAI (Azure). Every inference request needs observability: prompt/response content logging, token usage tracking, latency metrics, cost attribution per tenant, model version tracking, and prompt version management. Observability must respect data classification — restricted classification must disable all content logging (ADR-103).

## Candidates

## | Criteria | Langfuse | Weights & Biases | MLflow Tracking | Helicone | Custom Prometheus |
|---|---|---|---|---|---|
| Self-hosted | ✅ K8s Helm, MIT license | ❌ SaaS only | ✅ Apache 2.0 | ❌ SaaS | ✅ |
| Prompt/response logging | ✅ Full trace with generations | ✅ | ⚠️ ML experiments, not LLM traces | ✅ | ❌ |
| Token usage tracking | ✅ Per-request, per-model | ✅ | ⚠️ | ✅ | ⚠️ Custom |
| Cost attribution | ✅ Per-tenant via metadata | ✅ | ❌ | ✅ | ❌ |
| Model/prompt versioning | ✅ Prompt management + versioning | ⚠️ | ✅ Model registry | ⚠️ | ❌ |
| LiteLLM integration | ✅ Native callback | ⚠️ Custom | ⚠️ Custom | ✅ | ❌ |
| Classification retention | ✅ Configurable per project | ❌ | ❌ | ❌ | ⚠️ |
| License | MIT | Proprietary | Apache 2.0 | Proprietary | N/A |

## Decision

## **Langfuse** (self-hosted via Helm, MIT license) for all LLM observability. Integrated with LiteLLM as native callback handler — every inference request automatically traced. PostgreSQL backend via CNPG. Per-classification retention: public/internal 90d full traces, confidential 30d metadata only (token count, latency, model — no prompt/response content), restricted completely disabled.

## Rejected

## - **Weights & Biases:** SaaS-only for full features. Prompt/response data sent to external service — contradicts restricted/confidential classification. Enterprise on-prem exists but W&B is SaaS-first, expensive per-seat.
- **MLflow Tracking:** Designed for ML experiments (hyperparameters, metrics, model artifacts), not LLM prompt/response observability. Lacks prompt versioning, generation tracing, LiteLLM callback. Used for MLOps (ADR-074), not LLM observability — different concerns.
- **Helicone:** SaaS-only. Proxy-based (sits between app and LLM) — CAVE already uses LiteLLM as proxy. Double-proxy adds latency and complexity. Data leaves cluster.
- **Custom Prometheus:** No prompt/response logging, no trace visualization, no prompt versioning. Only provides aggregate metrics (token counts, latency) — misses what was asked, what was answered.

## Consequences

## **Positive:**
- Complete LLM observability: prompt, response, tokens, latency, cost in one dashboard.
- Native LiteLLM integration — zero custom code.
- Classification-aware retention prevents restricted data from being logged.
- Per-tenant project isolation. MIT license.
- Prompt versioning tracks which system prompt produced which quality outcomes.

**Negative:**
- PostgreSQL backend adds DB to manage (mitigated: CNPG operator).
- ~512MB-1GB RAM per Langfuse instance.
- Restricted = zero observability for restricted AI interactions — debugging requires alternative approaches.
- Younger project (~2023) — smaller community than W&B/MLflow but growing rapidly.

## Compliance Mapping

## GDPR Art.5(1)(c) (data minimisation — classification-based retention). GDPR Art.25 (data protection by design — restricted disables logging). SOC2 CC7.2 (AI monitoring — inference audit trail). ISO A.8.16 (monitoring activities). NIS2 Art.21 (AI system monitoring).
