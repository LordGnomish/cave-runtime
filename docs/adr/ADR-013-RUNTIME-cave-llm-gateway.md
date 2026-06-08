<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-013-RUNTIME — cave-llm-gateway: Sovereign Unified LLM Gateway, Rust-native LiteLLM parity (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — single binary, no external service)
**Category:** AI/LLM
**Decided:** 2026-06-07 / 2026-06-08 (Burak Tartan)
**Variant-of:** Platform ADR-013 (LiteLLM as Unified LLM Gateway)
**Upstream:** BerriAI/litellm `v1.85.1` (`source_sha` pinned; upstream latest `v1.88.0` tracked always-latest)
**Crate:** `crates/cave-llm-gateway` (cave-ai umbrella)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-013 birleşik LLM inference gateway'i için **LiteLLM**'i (BerriAI, MIT)
seçti: 100+ provider'a OpenAI-compatible tek API, classification-based routing,
per-tenant token metering, Presidio PII redaction ve Langfuse observability.
LiteLLM **runtime'ı bir Python servisidir** (K8s Helm deployment) — ayrı process,
ayrı dil toolchain'i, ayrı supply-chain. Bu, Cave Charter **§5 single-binary
mandate**'ini ihlal eder.

Bu Runtime variant LiteLLM runtime'ını **çalıştırmaz**. LiteLLM'in **kavramlarını**
— OpenAI-compatible proxy yüzeyi, multi-provider routing, classification-aware
dispatch, token/cost metering, PII redaction middleware — **Rust-native** olarak
`crates/cave-llm-gateway` crate'ine port eder. Crate **halihazırda yaşıyor**
(LiteLLM `v1.85.1` parity, manifest-pinned, OpenAI-compatible `/v1/chat/completions`,
provider trait + concrete backend'ler, capability router, cost ledger, cache,
Prometheus exposition, cave-hermes bridge). Bu ADR o crate'in kararını charter'a
bağlar.

Sovereign re-rooting: routing **classification-based** kalır ama varsayılan
**local-first**'tür — `restricted` sınıf **cave-hermes (Ollama parity, local)**'a
gider, external provider'a **hiç** ulaşmaz. Platform'un "Azure OpenAI"
hedefi **provider-equal** bir örneğe demote edilir (cave-llm-gateway provider
trait'i Anthropic/OpenAI/Mistral/generic OpenAI-compat'i eşit dispatch eder);
hiçbir bulut sağlayıcı sovereign default değildir. License **MIT → AGPL-3.0-or-later**
(Cave Runtime geneli; Burak onaylı).

## Context

### Neden bir Runtime variant gerekli

Platform variant gateway'i LiteLLM **çalıştırarak** materialize ediyor. Cave
Runtime'da mümkün değil:

1. **Charter §5 (single binary)** — LiteLLM proxy ayrı bir Python process'tir
   (uvicorn/gunicorn + Helm). Python runtime + pip dependency ağacı tek Rust
   binary'sine sığmaz. İkinci runtime = §5 ihlali.
2. **Sovereignty / supply-chain** — pip transitive bağımlılık yüzeyi Cave'in
   source-pinned, SLSA-L4, ML-DSA-signed build hattı (ADR-005) ile denetlenemez.
3. **Operasyonel tekillik** — Cave operatörü tek binary, tek metrics port'u, tek
   config yüzeyi deploy eder; ayrı LiteLLM lifecycle/upgrade/CVE takibi getirmez.
4. **Sınıflandırma egemenliği** — `restricted` / `confidential` veri, bir Python
   ara-servise bile değil, doğrudan in-binary router → local inference'a gitmeli.

### Korunan değer

LiteLLM'in **fikirleri** birinci sınıf değerdir ve korunur: OpenAI-compatible tek
yüzey, multi-provider routing, classification-aware dispatch, per-tenant token
metering (FinOps), PII redaction middleware, observability callback'leri.
Korunmayan tek şey **implementation runtime'ı** (Python servisi). cave-llm-gateway
bunları Rust-native (axum) materialize eder, tek binary içinde.

### Platform → Runtime sapması (mirror principle)

Cloud-managed kıyas sütunları demote/drop edilir (ADR-RUNTIME-UPSTREAM-MIRROR-001):
- **"restricted → Ollama, confidential → Azure OpenAI"** → Runtime'da
  **restricted → cave-hermes (local Ollama parity)**; `confidential`/diğer sınıflar
  için **provider-equal** external backend seçimi (Anthropic/OpenAI/Mistral/
  generic OpenAI-compat eşit), Azure OpenAI **sole-provider değil**.
- **Microsoft Presidio** PII redaction → Runtime'da **Presidio entegrasyonu**
  middleware hook olarak korunur; structured-field removal `confidential`/
  `restricted` için NER eksikliğini tamamlar (Presidio NER %100 değil — bilinen
  sınır, platform ile aynı).
- **Langfuse** observability → Runtime'da **cave-metrics** Prometheus exposition +
  observability track'i ile sağlanır (in-binary, external Langfuse SaaS'a reach-out
  zorunlu değil).

## Candidates

Platform ADR-013 aday tablosu (LiteLLM / Direct API / Kong AI Plugin / Portkey /
MLflow Gateway) upstream seçimini zaten **LiteLLM** yaptı; bu Runtime variant
*yeni aday açmaz*. Tek karar **runtime-mı yoksa parity-mi** sorusudur; cevap
**parity** (Charter §5). Eksen aday değil, **runtime-vs-parity**:

| Kriter | **cave-llm-gateway (Rust-native parity)** | ~~LiteLLM runtime (Python servisi)~~ |
|---|---|---|
| Tek binary (Charter §5) | ✅ Rust binary içinde | ❌ ayrı Python servis (Helm) |
| Multi-provider routing | ✅ Provider trait + concrete backend'ler, OpenAI-compatible | ✅ 100+ provider |
| Classification-based routing | ✅ in-binary router (restricted→cave-hermes local) | ✅ custom router |
| Per-tenant token metering | ✅ cost ledger, cave-metrics scrape | ✅ per-request/tenant/model |
| PII redaction | ✅ Presidio middleware hook + structured-field removal | ✅ Presidio pre/post |
| Observability | ✅ Prometheus (cave-metrics), in-binary | Langfuse native callback |
| Supply-chain denetlenebilir | ✅ source-pinned, SLSA-L4 (ADR-005) | ❌ pip transitive yüzeyi |
| Sovereign / air-gap | ✅ in-binary, local-first, external reach-out yok | ⚠️ ayrı runtime + external default'lar |
| License | ✅ AGPL-3.0-or-later (Cave geneli) | MIT |

> **LiteLLM runtime sütunu çizilmiştir** — Charter §5 single-binary mandate
> nedeniyle Runtime'da çalıştırılması **kategorik kapsam-dışıdır**. Değerlendirme,
> "hangi gateway" değil, "LiteLLM runtime'ı mı çalıştırılır yoksa kavramları
> Rust'a mı port edilir" sorusudur.

## Decision

**cave-llm-gateway**, Cave Runtime'ın **tek** birleşik LLM gateway yüzeyidir:
LiteLLM `v1.85.1` **kavramsal parity**'sinin Rust-native reimpl'i (axum, single
binary içinde, cave-ai umbrella). LiteLLM runtime'ı **çalıştırılmaz**. Port edilen
LiteLLM konseptleri:

### 1. OpenAI-compatible unified API
- **`/v1/chat/completions`** OpenAI-compatible yüzey — geliştirici hangi backend'in
  isteği karşıladığını bilmek zorunda değil.
- **Provider trait + concrete backend'ler**: Ollama / llama.cpp / MLX / Anthropic /
  OpenAI / Mistral / generic OpenAI-compat (mevcut crate).

### 2. Classification-based routing (local-first, sovereign)
- **in-binary router** veri sınıfına göre dispatch eder:
  **`restricted` → cave-hermes (Ollama parity, local)** — external provider'a
  **hiç** ulaşmaz.
- Diğer sınıflar için **provider-equal** seçim (capability router: context / tools /
  vision / **locality** / cost); hiçbir bulut sağlayıcı sovereign default değildir.
- Routing **platform seviyesinde** zorlanır, application seviyesinde değil.

### 3. Per-tenant token metering (FinOps)
- **cost ledger** — per-request, per-tenant, per-model token/cost attribution.
- **cave-metrics** Prometheus exposition ile scrape; per-tenant AI maliyet
  attribution (FinOps, platform ADR-096 mantığı).

### 4. PII redaction middleware (Presidio)
- **Presidio entegrasyonu** — herhangi bir LLM provider veriyi almadan **önce**
  pre/post redaction middleware hook.
- **structured-field removal** — `confidential`/`restricted` sınıflar için
  Presidio NER'in %100 olmayan doğruluğunu tamamlar (bilinen sınır).

### 5. Observability + reliability
- **Prometheus-format exposition** (cave-metrics scrape uyumlu), aggregate health
  probe, exponential-backoff retry, response cache (mevcut crate).
- **cave-hermes MultiGateway bridge** + cave-llm-tracker bench wire + cavectl
  `llm-gateway` subcommand (4-track ship).

> **LiteLLM runtime'ı (Python servisi) çalıştırılmaz** — Charter §5 single-binary
> mandate gereği kategorik kapsam-dışı. Operatör isterse kendi sorumluluğunda yan
> tarafta upstream LiteLLM koşabilir; bu Runtime catalogue'un sovereign default'u
> **değildir**.

### cave-llm-gateway ↔ komşu crate'ler / ADR'lar
- **cave-hermes** (ADR-009-RUNTIME) — local Ollama parity gateway; `restricted`
  routing hedefi + MultiGateway bridge.
- **cave-metrics** — token/cost metering Prometheus exposition + observability track.
- **cave-llm-tracker** (ADR-152) — daily always-latest upstream bench wire.
- **ADR-153** (cave-llm-gateway MVP) — bu ADR'ın MVP öncülü; charter-binding genişletme.

## Rejected

- **LiteLLM runtime'ını çalıştırmak (Python servisi olarak)** — **Charter §5
  single-binary mandate ihlali**. Ayrı process, ayrı dil toolchain, pip transitive
  supply-chain (SLSA-L4 + ML-DSA — ADR-005 — denetlenemez), ayrı lifecycle/CVE.
  **Kategorik kapsam-dışı.**
- **Direct API calls (gateway'siz)** — birleşik arayüz yok; her uygulama
  provider-specific SDK, classification routing, token counting, PII redaction'ı
  bağımsız implement eder → tenant uygulamaları arasında devasa duplikasyon.
  Reddedildi.
- **Kong AI Gateway plugin** — Kong API-seviyesi routing/security yapar; LLM routing
  application-domain (classification-aware) mantığıdır. Infra gateway ile ML
  inference concern'lerini karıştırmak sorumluluk ayrımını ihlal eder; Presidio PII
  + observability eksik. Reddedildi (cave-gateway ile cave-llm-gateway ayrı tutulur).
- **Portkey** — SaaS-only; veri external servise transit eder — `restricted`/
  `confidential` sınıflandırma ve sovereign hosting ile uyumsuz. Reddedildi.
- **MLflow Gateway** — sınırlı provider desteği, classification-based routing yok,
  PII redaction yok. Reddedildi.
- **Azure OpenAI'yi sole external provider yapmak** — Runtime sovereign sapması;
  provider-equal'e demote edildi (local-first, hiçbir bulut sağlayıcı default değil).

> Platform variant gateway'i LiteLLM **çalıştırarak** sağlıyordu; Runtime LiteLLM'in
> **kavramlarını Rust'a port eder**, runtime'ını çalıştırmaz — tek binary korunur,
> routing local-first sovereign olur.

## Consequences

### Olumlu
- **Charter §5 korunur** — gateway tek Rust binary içinde, ikinci (Python) runtime yok.
- **Tam sovereignty / air-gap** — `restricted` veri local cave-hermes'e gider;
  external provider veya LiteLLM SaaS'a zorunlu reach-out yok.
- **Denetlenebilir supply-chain** — Rust crate grafiği source-pinned, SLSA-L4
  hermetic build (ADR-005); pip toolchain yüzeyi elenir.
- **Tek API, çok provider** — geliştirici backend'i bilmeden OpenAI-compatible
  yüzeyi kullanır; classification routing platform seviyesinde zorlanır.
- **FinOps** — per-tenant/request/model token metering cave-metrics ile attribution.
- **PII koruması** — Presidio middleware + structured-field removal veri platformu
  terk etmeden önce çalışır.

### Olumsuz / maliyet
- **Reimplementation eforu** — LiteLLM'in 100+ provider olgunluğu birebir taşınmaz;
  Cave kendi provider trait'ini büyütür (mevcut: 7 backend ailesi).
- **SPOF riski** — tüm AI inference için tek gateway (azaltım: HA deployment,
  health probe, retry, cave-metrics monitoring).
- **Presidio NER tavanı** — %100 değil; compliance-grade PII için structured-field
  removal ile tamamlanır (bilinen sınır, platform ile ortak).
- **Parity bakım yükü** — LiteLLM `v1.85.1` → `v1.88.0`+ always-latest takip
  (upstream-watch / ADR-RUNTIME-UPSTREAM-WATCH-001).

### Riskler & azaltım
- **LiteLLM upstream drift** → `parity.manifest.toml` `source_sha` pin + günlük
  upstream tracker; manifest pin `v1.85.1`, upstream latest `v1.88.0` izlenir.
- **Routing yanlış-sınıf riski** → classification default-local (fail-safe:
  belirsizse `restricted`/local).
- **Honest parity inflation** → fill_ratio manifest-authored (mevcut honest_ratio
  0.5, mapped 23 / total 46), Charter 8-gate self-audit; scope-cut'lar
  PARITY_REPORT'ta dokümante.

## Compliance Mapping

Platform ADR-013'ten **inherit** edilen + sovereign güçlendirme:

- **SOC2 CC6.1** (AI access controls, classification-based routing) — in-binary
  router veri sınıfına göre dispatch; `restricted` local-only.
- **GDPR Art.25** (data protection by design) — Presidio PII redaction external
  provider veriyi almadan önce; local-first routing veriyi küme içinde tutar.
- **ISO/IEC 27001 A.5.12** (classification of information) — AI inference'a
  uygulanan sınıflandırma; routing kararı classification-driven.
- **NIS2 Art.21** (supply chain risk) — LLM provider veri handling local-first +
  source-pinned Rust supply-chain (ADR-005), pip yüzeyi elenir.

## Charter v2 8-gate linkage

Strict TDD: cave-llm-gateway `parity.manifest.toml` upstream **BerriAI/litellm
`v1.85.1`** (`source_sha` pinned) Charter v2 self-audit gate'lerine bağlı. 8-gate
per crate: TDD strict, SPDX header, source-pinned to LiteLLM version, no-stubs,
no-backcompat, always-latest (upstream `v1.88.0` tracked), 4-track ship (Portal +
cavectl `llm-gateway` + API + cave-metrics observability), honest fill_ratio
(manifest-authored, honest_ratio 0.5). LiteLLM runtime usage honest gerekçeyle
(Charter §5 single-binary) kapsam-dışı; Rust-native parity scope PARITY_REPORT'ta
dokümante.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — sovereignty + single-binary charter (§5)
- [ADR-005-RUNTIME](ADR-005-RUNTIME-buildah.md) — SLSA-L4 hermetic build (supply-chain denetlenebilirliği)
- [ADR-009-RUNTIME](ADR-009-RUNTIME-cave-hermes.md) — cave-hermes local Ollama gateway (restricted routing hedefi)
- [ADR-153](ADR-153_LLM_Gateway_MVP.md) — cave-llm-gateway MVP (öncül)
- [ADR-152](ADR-152_LLM_Tracker_Daily_Always_Latest.md) — cave-llm-tracker daily always-latest (bench wire)
- [ADR-RUNTIME-UPSTREAM-MIRROR-001](ADR-RUNTIME-UPSTREAM-MIRROR-001-platform-runtime-mirror.md) — platform → runtime mirror (cloud-default demote)
- [ADR-RUNTIME-CLI-CONSOLIDATION-001](ADR-RUNTIME-CLI-CONSOLIDATION-001-cavectl-native-and-compat.md) — cavectl (CLI track of 4-track)
- **Platform ADR-013** — LiteLLM as Unified LLM Gateway (reference variant)

---

*Bu ADR Platform ADR-013'ün **Runtime sovereign variant**'ıdır. LiteLLM'in birleşik
LLM-gateway kavramları (OpenAI-compatible API, multi-provider + classification-based
routing, per-tenant token metering, Presidio PII redaction, observability)
**Rust-native** olarak cave-llm-gateway'e port edilmiştir; LiteLLM runtime'ı (Python
servisi) **Charter §5 single-binary mandate** gereği çalıştırılmaz — tek binary,
local-first, external reach-out yok. Routing sovereign: `restricted` → cave-hermes
(local), bulut sağlayıcılar provider-equal. Upstream BerriAI/litellm v1.85.1
source-pinned (latest v1.88.0 tracked). Cave Runtime AGPL-3.0-or-later.*
