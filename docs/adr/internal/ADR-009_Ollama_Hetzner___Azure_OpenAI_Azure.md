# ADR-009: Ollama (Hetzner) / Azure OpenAI (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** AI/LLM

**Related ADRs:** 013 (LiteLLM Gateway), 051 (Langfuse Observability), 103 (LLM Data Governance), 111 (Classification-Aware Routing), 114 (Qdrant Vector DB)

**Review Cadence:** Monthly — AI/LLM landscape changes rapidly. Re-evaluate model selection, provider capabilities, and cost/performance quarterly at minimum.

## Context

CAVE needs LLM inference across two fundamentally different trust boundaries:

1. **Sovereign profile (Hetzner):** All data stays within the cluster. No API calls to external services. Tenants with RESTRICTED or CONFIDENTIAL data classification (ADR-102) must use self-hosted inference. GDPR Art.44-49 prohibits transfer of restricted personal data to US-based API providers without adequate safeguards.

2. **Enterprise profile (Azure):** Access to frontier models (GPT-4o, o1/o3, Claude) with enterprise-grade DPA (Data Processing Agreement) and no-training guarantees. Tenants with INTERNAL or PUBLIC data classification can use cloud LLM APIs.

**LiteLLM (ADR-013)** serves as the unified gateway — applications call one API, LiteLLM routes to the appropriate backend based on data classification (ADR-111).

### Requirements

- Self-hosted inference with zero external dependencies (sovereign)
- Frontier model quality for enterprise workloads (Azure)
- OpenAI-compatible API on both paths (unified developer experience)
- Streaming support (SSE) for real-time token generation
- Multi-model support (different models for different tasks — coding, reasoning, chat)
- Inference observability via Langfuse (ADR-051)
- Cost tracking per tenant per model

### AI/LLM Landscape (2025-2026 Snapshot)

The AI inference landscape is evolving faster than any other technology area. Key trends:

- **Open-weight models closing the gap:** Llama 3.1/4, Mistral Large 2, DeepSeek V3, Qwen 2.5-72B achieve GPT-4-level quality on many benchmarks. By 2027, self-hosted models may match frontier API quality for most tasks.
- **Inference engines maturing:** vLLM, TGI (Text Generation Inference), SGLang offer production-grade batching, PagedAttention, speculative decoding. Ollama is simpler but less performant under concurrency.
- **GPU availability expanding:** Hetzner has announced GPU server plans for 2025-2026. Cloud GPU pricing dropping (H100 spot pricing ~50% cheaper than 2024).
- **Model-as-a-Service diversifying:** Claude (Anthropic) now available on Azure Marketplace. Gemini accessible via Vertex. No single provider has exclusive frontier model access anymore.
- **Agentic AI emerging:** Tool-use, multi-step reasoning, and autonomous agent patterns (ReAct, AutoGPT) are becoming standard. Inference infrastructure must support long-context, multi-turn, and streaming.

---

## Candidates

### Self-Hosted Inference (Hetzner)

| Criteria | Ollama | vLLM | TGI (HuggingFace) | LocalAI | llama.cpp (raw) |
|---|---|---|---|---|---|
| **Setup complexity** | \u2705 Single binary, `ollama pull` + `ollama serve` | High (CUDA, Python env, model sharding) | Medium (Docker, model download) | Medium | Low (compile + run) |
| **GPU required** | No (CPU + GPU both supported) | Yes (CUDA mandatory) | Yes (CUDA for production perf) | Optional | No (CPU inference native) |
| **Concurrency/batching** | \u26a0\ufe0f Limited (single request queue, no continuous batching) | \u2705 Excellent (PagedAttention, continuous batching, speculative decode) | \u2705 Good (continuous batching, Flash Attention) | \u26a0\ufe0f Basic | \u274c Single request |
| **Model ecosystem** | \u2705 Large (Ollama Model Library — 1-click pull) | \u2705 Any HuggingFace model | \u2705 Any HuggingFace model | Medium | GGUF models only |
| **Quantization** | \u2705 GGUF (Q4, Q5, Q8) auto-detected | \u2705 AWQ, GPTQ, SqueezeLLM | \u2705 GPTQ, AWQ, EETQ | \u2705 GGUF | \u2705 GGUF (native) |
| **API compatibility** | \u2705 OpenAI-compatible `/v1/chat/completions` | \u2705 OpenAI-compatible | \u2705 OpenAI-compatible (with adapter) | \u2705 OpenAI-compatible | \u274c Custom API |
| **Streaming (SSE)** | \u2705 | \u2705 | \u2705 | \u2705 | \u26a0\ufe0f Manual |
| **Multi-model** | \u2705 Hot-swap models via API | \u26a0\ufe0f One model per instance | \u26a0\ufe0f One model per instance | \u2705 Multiple models | \u274c Single model |
| **Production maturity** | Medium (dev/small teams, growing) | High (used by Anyscale, Databricks) | High (HuggingFace production stack) | Low-Medium | Low (library, not server) |
| **K8s deployment** | Helm chart available | KubeAI, Helm | Helm, HF Inference Endpoints | Helm | Manual |
| **License** | MIT | Apache 2.0 | Apache 2.0 (HF TGI) | MIT | MIT |

### Cloud LLM APIs (Azure)

| Criteria | Azure OpenAI | Anthropic (Claude) on Azure | AWS Bedrock | Google Vertex AI |
|---|---|---|---|---|
| **Models** | GPT-4o, o1, o3-mini, GPT-4-turbo, DALL-E 3 | Claude Opus/Sonnet/Haiku (via Azure Marketplace) | Claude, Llama, Mistral, Titan | Gemini, Claude, Llama |
| **Enterprise DPA** | \u2705 No-training guarantee, data residency options | \u2705 Anthropic DPA via Azure | \u2705 AWS DPA | \u2705 Google DPA |
| **EU region** | \u2705 Sweden, France, Switzerland | \u2705 (via Azure region) | \u2705 Frankfurt, Ireland | \u2705 Belgium, Netherlands |
| **Pricing (GPT-4o equiv)** | ~$2.50/1M input, ~$10/1M output | ~$3/1M input (Sonnet), ~$15/1M (Opus) | ~$3/1M (Claude Sonnet) | ~$1.25/1M (Gemini 1.5 Pro) |
| **Rate limits** | Configurable (TPM/RPM quotas) | Azure Marketplace limits | Per-model quotas | Per-model quotas |
| **Azure native** | \u2705 (first-party service) | \u2705 (marketplace) | \u274c (AWS only) | \u274c (GCP only) |

---

## Decision

**Ollama** for self-hosted inference on Hetzner (sovereign, zero data egress). **Azure OpenAI + Claude on Azure Marketplace** for enterprise profile (frontier models with DPA). **LiteLLM** (ADR-013) as unified gateway routing by data classification.

**Model recommendations (2026):**

| Use Case | Sovereign (Ollama) | Enterprise (Azure) |
|---|---|---|
| General chat / assistant | Llama 3.1 70B (Q5) | GPT-4o |
| Code generation | DeepSeek Coder V2 33B | Claude Sonnet |
| Reasoning / analysis | Qwen 2.5 72B | o1 / o3-mini |
| Embeddings | nomic-embed-text | text-embedding-3-large |
| Fast / cheap tasks | Phi-3.5 mini (3.8B) | GPT-4o-mini |

**Migration path:** When Hetzner GPU servers become available OR open-weight models reach frontier parity, evaluate migrating enterprise workloads to self-hosted vLLM for cost savings and full sovereignty.

---

## Rejected Options

### vLLM as Primary (Hetzner) — Rejected for Now

**Primary:** GPU dependency. vLLM requires CUDA — Hetzner does not currently offer GPU servers. Ollama supports CPU inference with GGUF quantized models, making it deployable today on Hetzner\'s CX/CCX instances.

**Secondary:** Operational complexity. vLLM requires CUDA drivers, Python environment management, model sharding configuration, and careful memory tuning. Ollama is a single binary with 1-command model pull.

**Watch:** vLLM is the correct choice for production GPU inference. When Hetzner GPU servers become available (expected 2025-2026) or CAVE provisions GPU nodes on Azure for self-hosted inference, migrate from Ollama to vLLM for continuous batching and 5-10x throughput improvement. This is a planned evolution, not a rejection.

### LocalAI — Rejected

**Primary:** Smaller community and slower development velocity than Ollama. Model ecosystem is a subset of what Ollama provides. No significant advantage over Ollama for CAVE\'s use case.

### AWS Bedrock — Rejected for Enterprise

**Primary:** Not Azure-native. CAVE\'s enterprise profile runs on Azure (ADR-002). Using Bedrock would require cross-cloud networking or a separate AWS account. Azure OpenAI + Claude on Azure Marketplace provides equivalent model access without leaving the Azure ecosystem.

### Google Vertex AI — Rejected for Enterprise

**Primary:** Same cross-cloud concern as Bedrock. Gemini is available but CAVE\'s enterprise tenant expectations center on GPT-4 and Claude. Vertex adds no unique value over Azure OpenAI + Claude marketplace.

---

## Consequences

### Positive

- Full data sovereignty on Hetzner: restricted/confidential data never leaves the cluster
- Frontier model access on Azure: GPT-4o, o1, Claude with enterprise DPA
- Unified API: LiteLLM gateway means applications code against one interface
- Classification-aware routing (ADR-111): automatic compliance enforcement
- Multi-model support: different models optimized for different tasks
- Cost transparency: per-tenant, per-model token tracking via Langfuse (ADR-051)
- Graceful upgrade path: Ollama → vLLM when GPU available; Azure OpenAI → self-hosted when open models reach parity

### Negative

- Ollama CPU inference is 10-50x slower than GPU inference (acceptable for sovereign profile\'s current scale)
- Open-weight model quality gap vs frontier (shrinking rapidly but still measurable on complex reasoning)
- No GPU on Hetzner limits batch inference throughput (single-digit tokens/second on 70B models with CPU)
- Two inference stacks to maintain (Ollama + Azure OpenAI) — LiteLLM abstracts this but ops complexity remains
- Model updates require testing: new Llama/Mistral releases may change output behavior

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Ollama performance insufficient at scale | Medium | Medium | Monitor inference latency via Langfuse. If P95 > 5s for chat, evaluate vLLM on dedicated CCX nodes (CPU-optimized AMD EPYC). Queue-based inference for batch workloads. |
| Azure OpenAI pricing increases | Medium | Medium | LiteLLM enables instant failover to Claude on Azure or Gemini via API. Cost alerts via FinOps (ADR-096). Self-hosted frontier alternative evaluated quarterly. |
| Open-weight model reaches frontier parity | High (2027) | Positive | **This is a good outcome.** Migrate enterprise workloads to self-hosted vLLM for cost savings + full sovereignty. LiteLLM switch is configuration change, not code change. |
| Hetzner launches GPU servers | Medium (2026) | Positive | Upgrade Ollama to vLLM on GPU nodes. 5-10x throughput improvement. Budget for GPU instances in FinOps planning. |
| Model supply chain attack (poisoned weights) | Low | High | Pin model digests. Verify checksums against HuggingFace/Ollama registry. YARA scan model files for known malware patterns. Air-gapped model loading on sovereign profile. |
| Anthropic/OpenAI changes API or terms | Low | Medium | LiteLLM abstraction + Ollama fallback. No single provider dependency. Open-weight models as ultimate fallback. |
| EU AI Act compliance requirements | Medium (2026) | Medium | **Watch:** EU AI Act mandates transparency for high-risk AI systems. Track model provenance, document training data origins for open-weight models. Langfuse audit trail covers inference decisions. Monthly review. |

---

## Compliance Mapping

**SOC2 CC6.1:** AI access controls — classification-based routing ensures restricted data stays on sovereign inference.
**GDPR Art.25:** Data protection by design — restricted personal data processed only by self-hosted Ollama (no third-party API).
**GDPR Art.44-49:** International data transfers — Ollama on Hetzner (German DC) eliminates EU→US transfer for restricted data.
**ISO A.5.12:** Information classification applied to AI inference — LiteLLM routes based on data classification labels (ADR-102).
**NIS2 Art.21:** Supply chain risk — LLM provider data handling governed by DPA (Azure) or eliminated (self-hosted).
**EU AI Act (2026+):** High-risk AI transparency — Langfuse audit trail records all inference requests, model versions, and outputs. Monthly review of compliance obligations as AI Act implementing regulations are published.
