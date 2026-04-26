# ADR-111: Classification-Aware LLM Inference Routing

**Status:** Accepted

**Scope:** Azure, Runtime, Universal

**Category:** AI

**Related ADRs:** 009, 013, 102, 103

## Context

Data classification determines which LLM providers may process tenant data. Routing must be enforced at platform level, not left to application developers.

## Candidates

| Classification | Allowed Providers | Rationale |
|---|---|---|
| public | Any approved (Ollama, Azure OpenAI) | No restrictions |
| internal | Any approved | No restrictions, PII-filtered |
| confidential | Azure OpenAI (DPA) or Ollama | DPA guarantees no-training |
| restricted | Ollama ONLY | Data never leaves cluster |

## Decision

LiteLLM routes inference based on classification header. restricted → Ollama only (self-hosted). confidential → Azure OpenAI with DPA or Ollama. public/internal → any approved. System prompts Git-managed + cosign-signed. Langfuse tracks versions. OPA validates routing decision matches policy.

## Rejected

- **Single provider for all classifications:** Restricted data cannot go to Azure OpenAI. Architectural violation.
- **No routing policy:** Classification is cosmetic — no enforcement. Developer sends restricted data to external LLM.
- **Application-level routing:** Not enforceable at platform level. Developers can bypass. Must be infrastructure-enforced.

## Consequences

**Positive:**
- Restricted data never reaches external LLM providers. Full sovereignty.
- Classification enforcement at infrastructure level, not application trust.
- System prompt version control + signing prevents unauthorized prompt changes.
- Langfuse observability per classification rules.

**Negative:**
- Ollama model quality lower than Azure OpenAI (GPT-4/o1). Restricted tenants accept quality trade-off.
- Classification changes on running workloads trigger routing re-evaluation.
- System prompt signing maintenance overhead.

## Compliance Mapping

GDPR Art.25 (data protection by design). GDPR Art.44-49 (data transfers — restricted stays EU). ISO A.5.12 (classification applied to AI). NIS2 Art.21 (supply chain — LLM provider risk).
