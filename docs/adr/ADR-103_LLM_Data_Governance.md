# ADR-103: LLM Data Governance

**Status:** Accepted

**Category:** AI

**Related ADRs:** 009, 013, 111

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## LLM inference involves sending potentially sensitive data to AI providers. Without governance, PII and classified data may leak to external systems.

## Candidates

## | Control | Mechanism | Layer |
|---|---|---|
| PII redaction | Microsoft Presidio (NER + structured field removal) | LiteLLM middleware (pre-inference) |
| Prompt injection | Pattern detection middleware | LiteLLM (pre-inference) |
| No-training guarantee | Azure OpenAI DPA / Ollama self-hosted | Provider contract / architecture |
| Token retention | Classification-based policy | Langfuse (post-inference) |
| Output redaction | Classification-based filtering | LiteLLM middleware (post-inference) |

## Decision

## PII redaction via Microsoft Presidio (structured field removal + NER for unstructured content) before LiteLLM forwards to any provider. Prompt injection defense via LiteLLM middleware. No training on tenant data (Azure OpenAI DPA / Ollama self-hosted). Token retention per classification: public/internal 90d full, confidential 30d metadata only, restricted no retention.

## Rejected

## - **No PII redaction:** Tenant PII sent to Azure OpenAI. Even with DPA, data minimization (GDPR Art.5(1)(c)) requires redaction where possible.
- **Regex-only PII detection:** Insufficient accuracy for compliance-grade redaction. Presidio NER provides entity recognition (names, addresses, IDs) beyond pattern matching. Combined with structured field removal for known PII fields.
- **Per-request human approval:** Destroys self-service UX. Operator fatigue → rubber-stamp approvals.

## Consequences

## **Positive:**
- PII redacted before any external LLM sees the data.
- Classification-based retention limits exposure window.
- No-training guarantee contractually (Azure OpenAI DPA) and architecturally (Ollama self-hosted).
- Langfuse observability enables audit of LLM interactions.

**Negative:**
- Presidio NER accuracy is not 100% (~85-95% depending on entity type). Known limitation.
- Combined with structured field removal for critical PII fields (SSN, credit card, etc.) — defense in depth.
- Token retention policies require Langfuse configuration per classification level.
- Restricted classification disables all logging — no observability for restricted AI interactions.

## Compliance Mapping

## GDPR Art.5(1)(c) (data minimisation). GDPR Art.25 (data protection by design). ISO A.5.12 (information classification applied to AI). NIS2 Art.21 (supply chain risk — LLM provider data handling).
