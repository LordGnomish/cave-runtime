<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-009-RUNTIME — cave-hermes: Sovereign Local LLM Gateway over Ollama (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — data never leaves cluster)
**Category:** AI/LLM
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-009 (Ollama (Hetzner) / Azure OpenAI (Azure))
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-009 LLM inference için **Ollama** (Hetzner, sovereign) + **Azure
OpenAI** (Azure, GPT-4/o1) ikili stratejisini, LiteLLM gateway ile classification-aware
routing arkasında seçti. Cave Runtime **sovereign** bir Cloud OS'tür: **veri
asla küme dışına çıkmaz**. **Azure OpenAI Runtime için ilgisizdir** — external
managed servis, harvest edilen prompt/completion'lar dışarı akar, air-gap'te
çalışmaz. Bu Runtime variant **yalnızca Ollama**'yı (local inference) tutar ve
gateway'i **cave-hermes** ile materialize eder.

**cave-hermes** = Hermes Agent (NousResearch) parity'sinin Rust reimpl'i
([[cave-autopilot-cont2-2026-06-07]] ile aynı tiered-router ailesi): persistent
memory, tool registry, workflow checkpoint/resume, ve **tiered model router**.
Router **Ollama HTTP API** üzerinden local model'lara yönlendirir; tier ladder:
**L1 Mellum2 (router/triage)** → **L2 Qwen3-Coder-Next (codegen)** → local Ollama
resident model fallback. LiteLLM yerine cave-hermes'in kendi sovereign router'ı.

## Context

### Neden bir Runtime variant gerekli
Platform variant'ın "Azure OpenAI (Azure)" tarafı Cave'in sovereignty postürüyle
çelişir: prompt + completion verisi external Azure servisine gider (GPT-4/o1),
no-training DPA'ya rağmen veri küme dışına çıkar → ADR-001 charter rule 3 ve GDPR
Art.44-49 data-residency ihlali (sovereign profilde). LiteLLM gateway iki-provider
classification routing'i Runtime için gereksizdir — Cave tek sovereign inference
yolu çalıştırır. Gateway de **in-binary** (cave-hermes), external'a reach-out
etmez.

### Korunan değer
Ollama'nın self-hosted, single-binary, GPU-opsiyonel (CPU inference), geniş
açık-model ekosistemi (Llama, Mistral, Phi, Qwen, Mellum), OpenAI-uyumlu API'si —
hepsi korunur. Classification-aware routing korunur ama tüm tier'lar **local**'dir.

## Candidates

| Kriter | **Ollama** (← cave-hermes gateway) | vLLM (→ cave-local-llm) | LocalAI | ~~Azure OpenAI~~ | ~~AWS Bedrock~~ |
|---|---|---|---|---|---|
| Self-hosted | ✅ | ✅ | ✅ | ❌ Azure-only | ❌ AWS-only |
| Setup | Düşük (single binary) | Yüksek (CUDA, sharding) | Orta | managed | managed |
| Model ekosistemi | Geniş (Llama/Mistral/Phi/Qwen/Mellum) | Geniş | Orta | GPT-4/o1 | Claude/Llama |
| GPU gerekli | ❌ (CPU inference) | ✅ CUDA zorunlu | opsiyonel | — | — |
| API uyumu | OpenAI-compatible | OpenAI-compatible | OpenAI-compatible | native | farklı |
| Sovereignty | ✅ veri kümede kalır | ✅ | ✅ | ❌ veri dışarı | ❌ veri dışarı |

> **Azure OpenAI** ve **AWS Bedrock** sütunları Runtime için **çizilmiştir** —
> external managed servisler, veri küme dışına çıkar; sovereignty + air-gap
> ihlali; değerlendirme-dışı.

## Decision

**Ollama** Cave Runtime'ın **tek** local LLM inference runtime'ıdır (sovereign,
veri küme dışına çıkmaz). Gateway ve routing **cave-hermes** (Hermes Agent parity)
ile yapılır: tiered model router Ollama HTTP API üzerinden local model'lara
yönlendirir.

### Tier ladder (hepsi local, Ollama-backed)

| Tier | Model | Rol |
|---|---|---|
| **L1** | **Mellum2** | Router / triage / classification (hızlı, ucuz) |
| **L2** | **Qwen3-Coder-Next** | Kod üretimi / yapılandırılmış görev |
| **L3** | Ollama resident fallback | L1/L2 named-MoE çözülemezse resident model'a düşüş ([[cave-autopilot-cont2-2026-06-07]]) |

> **Azure OpenAI ve tüm external managed LLM servisleri kapsam-dışıdır** —
> sovereignty gerekçesiyle. Prompt/completion verisi **asla** küme dışına çıkmaz.
> Enterprise "GPT-4 kalitesi" gerektiren operatör, kendi provider'ında kendi
> sorumluluğunda ayrı bir non-sovereign profil koşabilir; bu Runtime catalogue'un
> sovereign default'u değildir.

### cave-hermes ↔ komşu crate'ler
- **cave-local-llm** — vLLM port'u burada yaşar ([[cave-vllm-cont3-2026-06-01]]);
  GPU-resident yüksek-throughput inference için alternatif backend.
- **cave-llm-gateway** (ADR-153) — daha geniş gateway MVP; cave-hermes onun
  agent-router katmanı.
- **cave-llm-tracker** (ADR-152) — günlük "always-latest" model takibi; tier
  model pin'lerini taze tutar.

## Rejected

- **Azure OpenAI** — external managed; prompt/completion verisi dışarı akar;
  **sovereignty + air-gap + GDPR data-residency ihlali**. Runtime için kategorik
  kapsam-dışı.
- **AWS Bedrock / GCP Vertex** — Hetzner'de yok + external managed; aynı
  sovereignty ihlali. Provider-agnostic charter'a da aykırı (tek-cloud lock).
- **vLLM (primary olarak)** — yüksek-performans GPU inference ama CUDA zorunlu;
  GPU'suz profillerde çalışmaz. Cave'de **reddedilmez, demote edilir**: cave-local-llm'de
  GPU-resident alternatif backend olarak yaşar, default değil.
- **LocalAI** — daha az olgun, daha küçük model ekosistemi, Ollama'dan az community
  katkısı.

> Platform variant inference'i **classification'a göre Hetzner-Ollama ↔ Azure-OpenAI**
> arasında bölüyordu; Runtime tüm tier'ları **local Ollama**'da tutar — bölünme yok.

## Consequences

### Olumlu
- **Tam veri sovereignty** — prompt/completion küme dışına çıkmaz.
- **Air-gap-capable** — inference + gateway in-cluster, external'a reach-out yok.
- Single-binary Ollama + single-binary cave-hermes (Charter §5).
- Classification-aware tiered routing (L1 Mellum2 → L2 Qwen3-Coder-Next → fallback)
  korunur ama **tamamen local**.
- GPU-opsiyonel (CPU inference); GPU varsa cave-local-llm/vLLM backend'e geçilebilir.
- MIT-licensed Hermes Agent upstream; açık-model ekosistemi.

### Olumsuz / maliyet
- **Model kalite tavanı** — açık model'ler (Qwen/Mellum/Llama) bazı görevlerde
  GPT-4/o1'in altında; sovereign trade-off bilinçli kabul.
- CPU inference GPU'dan yavaş; GPU'suz profilde latency yüksek.
- Named-MoE tier'lar (Mellum2, Qwen3-Coder-Next) resident değilse fallback
  ([[cave-autopilot-foundation-2026-06-07]] GOTCHA: tek resident model trap).
- cave-hermes Backend-only ship (Portal/cavectl/observability scope-cut, follow-up).

### Riskler & azaltım
- **Tier model drift** → cave-llm-tracker günlük always-latest takip (ADR-152).
- **Ollama OOM / resident model eksik** → ladder fallback + cave-metrics OOM gate.
- **Local kalite yetersizliği** → operatör opsiyonel non-sovereign profil koşabilir
  (default değil, sovereign posture korunur).

## Compliance Mapping

- **SOC2 CC6.1** — AI access controls (classification-based local routing).
- **GDPR Art.25** — data protection by design (restricted data self-hosted kalır).
- **GDPR Art.44-49** — data transfers (Ollama restricted data'nın EU/küme dışına
  çıkmamasını garanti eder).
- **ISO A.5.12** — information classification AI inference'a uygulanır.
- **NIS2 Art.21** — supply-chain (LLM provider veri işleme; in-cluster = sıfır
  external veri akışı).

## Charter v2 8-gate linkage

Strict TDD: cave-hermes `parity.manifest.toml` (NousResearch/hermes-agent
v2026.5.16) Charter v2 self-audit gate'lerine bağlı. Backend-only ship honest
dokümante (PARITY_REPORT §7 scope-cut, gate_1 no fabrication). Azure OpenAI honest
gerekçeyle (sovereignty) kapsam-dışı. `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — air-gap + sovereignty charter
- [ADR-150](ADR-150_Hermes_Agent_Adoption_AC_Path.md) — cave-hermes agent adoption (A+C path)
- [ADR-153](ADR-153_LLM_Gateway_MVP.md) — cave-llm-gateway MVP
- [ADR-152](ADR-152_LLM_Tracker_Daily_Always_Latest.md) — cave-llm-tracker (tier model pins fresh)
- [ADR-010-RUNTIME](ADR-010-RUNTIME-ci-pipeline.md) — cave-agent AI PR review (Phase 1/7)
- **Platform ADR-009** — Ollama (Hetzner) / Azure OpenAI (Azure) dual-provider reference variant

---

*Bu ADR Platform ADR-009'un **Runtime sovereign variant**'ıdır. Ollama local
inference + cave-hermes (Hermes Agent parity) tiered router ile materialize edilmiş;
Azure OpenAI (external managed) sovereignty gerekçesiyle kapsam-dışı bırakılmıştır
— veri asla küme dışına çıkmaz. Cave Runtime AGPL-3.0-or-later.*
