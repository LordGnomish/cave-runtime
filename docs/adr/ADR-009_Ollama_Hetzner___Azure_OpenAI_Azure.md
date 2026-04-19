# ADR-009: Ollama (Hetzner) / Azure OpenAI (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** AI/LLM

**Related ADRs:** 013, 103, 111

Status:

Category:

AI/LLM

Related ADRs:

013, 103, 111

Back to Index:

## Context

CAVE needs LLM inference on both providers. Sovereign profile requires data to never leave the cluster. Enterprise profile needs GPT-4/o1 quality.


## Candidates

| Criteria | Ollama | vLLM | LocalAI | Azure OpenAI | AWS Bedrock |
|---|---|---|---|---|---|
| Self-hosted | Yes | Yes | Yes | No (Azure only) | No (AWS only) |
| Setup complexity | Low (single binary) | High (CUDA, model sharding) | Medium | Managed | Managed |
| Model ecosystem | Large (Llama, Mistral, Phi, etc.) | Large | Medium | GPT-4, o1 (exclusive) | Claude, Llama |
| GPU required | No (CPU inference supported) | Yes (CUDA mandatory) | Optional | N/A | N/A |
| Enterprise DPA | N/A (self-hosted) | N/A | N/A | Yes (no-training guarantee) | Yes |
| API compatibility | OpenAI-compatible | OpenAI-compatible | OpenAI-compatible | Native | Different API |


## Decision

**Ollama** for self-hosted inference on Hetzner (sovereign, no data leaves cluster). **Azure OpenAI** for Azure (GPT-4/o1, enterprise DPA, no-training guarantee). LiteLLM gateway (ADR-013) routes based on data classification (ADR-111).


## Rejected Options

- **vLLM:** Higher performance for GPU inference but requires CUDA — Hetzner has no GPU instances. Overkill for platform team size.
- **LocalAI:** Less mature, smaller model ecosystem, fewer community contributions than Ollama.
- **AWS Bedrock/GCP Vertex:** Not available on Hetzner. Azure OpenAI exclusive GPT-4 access + Knauf ecosystem alignment (ADR-002).


## Consequences

(+) Full data sovereignty on Hetzner. Enterprise GPT-4 on Azure. Classification-aware routing via LiteLLM.
(-) Ollama CPU inference slower than GPU. Model quality gap between Ollama (open models) and Azure OpenAI (GPT-4/o1). No GPU on Hetzner limits inference performance.

Compliance Mapping

SOC2 CC6.1 (AI access controls — classification-based routing). GDPR Art.25 (data protection by design — restricted data stays self-hosted). GDPR Art.44-49 (data transfers — Ollama ensures restricted data never leaves EU). ISO A.5.12 (information classification applied to AI inference). NIS2 Art.21 (supply chain — LLM provider data handling).

