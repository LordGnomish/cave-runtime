# ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 — Cave Runtime API Gateway Consolidation: Kong + Gravitee → cave-gateway

**Status:** Accepted (2026-04-26)
**Scope:** Cave Runtime (independent override; multi-upstream consolidation). **Overrides ADR-027 Platform** (Kong-only) within Runtime context.
**Category:** Networking / API Management (Layer 3 north-south + Layer 4 API lifecycle)
**Decided:** 2026-04-26 (Burak Tartan)

## Override Notice

ADR-027 (Platform) — "Kong API Gateway" — **Platform repo'da değişmiyor**. Burak'ın iş yerinde sovereign Hetzner deployment'ı için Kong OSS deployment kararı geçerliliğini koruyor; Platform Kong'u OSS olarak deploy etmeye devam ediyor.

Bu ADR **sadece Cave Runtime context'inde** ADR-027'yi override ediyor: Runtime'da `cave-gateway` crate'i Kong + Gravitee'nin **konsolide reimpl'i** olarak doğuyor — Platform'daki Kong-only kararından sapan multi-upstream konsolidasyon vakası.

`ADR-RUNTIME-UPSTREAM-MIRROR-001` default eşleşmesi (1 Platform OSS → 1 Runtime crate) bu ADR ile genişletiliyor: 2 Platform-relevant OSS (Kong + Gravitee) → 1 Runtime crate (cave-gateway). Streaming (Kafka+Pulsar→cave-streams) ve persistence (multi-upstream→cave-pg/docdb/cache/...) ADR'leriyle aynı consolidation pattern.

## Context

Cave Runtime sovereign + multi-tenant Cloud OS olarak Layer 3'te kuzey-güney trafik yönetimi gerektirir. İhtiyaç iki farklı domain'i kapsıyor:

1. **Proxy data path** — high-throughput L7 routing, rate-limit, JWT/OAuth2/key-auth, request/response transform, circuit-breaker, observability. Kong'un güçlü olduğu alan; nginx/lua plugin DSL'i ile zengin ekosistem.
2. **API lifecycle management** — Developer Portal (API katalog, dokümantasyon, self-service onboarding), tenant-facing API key management, OAuth2 client registration, IAM federation, analytics dashboards. Gravitee'nin güçlü olduğu alan; tek başına proxy yetenekleri Kong'un gerisinde ama portal+IAM+analytics tarafı zengin.

Burak'ın **N-to-M LLM/IDM pattern paralelliği**: cave-llm-gateway nasıl ki çoklu LLM upstream'i (OpenAI/Anthropic/Mistral/Ollama) tek wire'a çekiyor (ADR-013 LiteLLM mirror), cave-auth nasıl ki çoklu IdM upstream'i (Keycloak/Okta/Entra) tek crate'te konsolide ediyor — aynı şekilde **API gateway için iki güçlü upstream'i (Kong + Gravitee) tek cave-gateway crate'inde konsolide etmek doğal**.

Multi-tenant invariant (ADR-MULTI-TENANT-001) gateway'in tenant_id üzerinden default-deny çalışmasını zorunlu kılıyor; ne Kong tek başına ne Gravitee tek başına Cave'in tenant modeline doğal değil — her ikisinin birleşik fonksiyonu cave-native olarak yeniden yazılmalı.

## Decision

Cave Runtime API gateway katmanı **`cave-gateway`** crate'i altında tek Rust implementasyon olarak yazılır. **Kong** (proxy data path + plugin DSL) ve **Gravitee** (Developer Portal + IAM + analytics) her ikisi upstream referansıdır; ikisinin birleşik feature seti reimplemente edilir.

### Crate yapısı

```
cave-gateway/                       — core proxy + admin API
cave-gateway-portal-ui/             — Developer Portal SPA (cave-native UI)
cave-gateway-plugins/
  ├─ rate-limit/
  ├─ key-auth/
  ├─ oauth2/
  ├─ jwt/
  ├─ transformations/               — request/response rewrite, header inject
  ├─ cors/
  ├─ circuit-breaker/
  ├─ cave-llm-bridge/               — cave-llm-gateway integration (ADR-013)
  ├─ cave-secrets/                  — secret-scan inline (ADR-RUNTIME-SECRET-SCAN-001)
  └─ cave-audit-pqc/                — PQC-signed audit log emit
```

### Mimari

```
┌──────────────────────────────────────────────────────────────────┐
│  cave-gateway (single Rust crate)                                │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ Unified Admin API (single REST/gRPC surface)                │ │
│  │   • Routes, services, consumers, plugins                    │ │
│  │   • Developer Portal config, API catalog, OAuth clients     │ │
│  │   • Per-tenant namespace scoping (multi-tenant invariant)   │ │
│  └────────────────────┬────────────────────────────────────────┘ │
│                       │                                          │
│       ┌───────────────┼────────────────┐                         │
│       ▼               ▼                ▼                         │
│  ┌─────────┐    ┌──────────┐    ┌───────────────┐                │
│  │ Proxy   │    │ Plugin   │    │ Portal API    │                │
│  │ data    │    │ chain    │    │ (catalog, IAM │                │
│  │ path    │◄──►│ runtime  │    │  fed, keys,   │                │
│  │ (L7)    │    │ (trait)  │    │  analytics)   │                │
│  └────┬────┘    └────┬─────┘    └───────┬───────┘                │
│       │              │                  │                        │
│       └──────────────┼──────────────────┘                        │
│                      ▼                                           │
│      cave-kernel (config store, WAL, Raft, PQC primitives)       │
└──────────────────────────────────────────────────────────────────┘
                       │
                       ▼
        ┌────────────────────────────┐
        │ cave-gateway-portal-ui     │  ← single Developer Portal UI
        │ (consumes Unified Admin    │     (SPA, cave-native)
        │  API; no separate UI for   │
        │  proxy admin)              │
        └────────────────────────────┘
```

### Cave-native UNIFIED API/UI invariant

**Burak corrections (zorunlu):**

1. **Tek Admin API.** Kong'un Admin API'si + Gravitee Management API'si **birleştirilir** — cave-gateway yalnızca tek REST/gRPC admin yüzeyi expose eder. İki ayrı admin endpoint **YASAK**.
2. **Tek Developer Portal UI.** Gravitee Developer Portal + Kong Konnect benzeri ayrı admin UI'sı **YASAK**. Sadece `cave-gateway-portal-ui` SPA'sı vardır; tenant developer'ları, tenant admin'leri ve platform operatörleri **aynı UI'nın role-scoped görünümlerini** kullanır.
3. **Multi-UI YASAK.** "Kong UI burada, Gravitee UI orada" gibi parçalı arayüz **olmayacak**. Birleşik UX cave-native invariant.

Sebep: Cave Runtime'ın "tenant zero-friction" hedefi parçalı admin yüzeyi ile bağdaşmıyor; Backstage entegrasyonu için de tek API yüzeyi şart.

### Plugin trait (Kong + Gravitee union)

Tüm plugin'ler ortak trait implement eder; runtime chain'de sıralı çalışır. Union set:

| Plugin | Kong | Gravitee | cave-gateway |
|---|---|---|---|
| rate-limit (per-tenant, per-consumer, per-route) | ✅ | ✅ | ✅ |
| key-auth | ✅ | ✅ | ✅ |
| oauth2 (authorization code, client credentials) | ✅ | ✅ | ✅ |
| jwt (validate, introspect) | ✅ | ✅ | ✅ |
| transformations (req/resp rewrite, header) | ✅ | ⚠️ partial | ✅ |
| cors | ✅ | ✅ | ✅ |
| circuit-breaker | ⚠️ via Envoy | ✅ | ✅ |
| OpenAPI validation | ✅ | ✅ | ✅ |
| cave-llm-bridge | — | — | ✅ (cave-native) |
| cave-secrets inline scan | — | — | ✅ (cave-native) |
| cave-audit-pqc | — | — | ✅ (cave-native) |

### Cave extensions (Runtime-only, upstream'lerde olmayan)

- **`cave-llm-bridge`** — cave-gateway → cave-llm-gateway (ADR-013 mirror) entegrasyonu. Tenant LLM trafiği gateway'den geçerken otomatik LiteLLM-equivalent routing, token accounting, model-allowlist enforcement.
- **`cave-secrets`** — tenant request/response payload'larında inline secret-scan (ADR-RUNTIME-SECRET-SCAN-001 ile aynı detector engine). Tespit edilen secret block + audit emit.
- **`cave-audit-pqc`** — her admin API call ve tenant authn event'i ML-DSA / SLH-DSA ile imzalı audit log üretir; cave-kernel WAL'a yazılır. ADR-014 (Zero-Trust) zorunluluğu.

### Multi-tenant model

- **Per-tenant gateway namespace.** Her tenant kendi `tenant/<id>/` namespace'i altında route, consumer, plugin config tutar. Cross-tenant config görünürlüğü **YOK**.
- **ResourceQuota.** Tenant başına route count, consumer count, plugin chain length, RPS quota. cave-kernel quota engine'i üzerinden enforce.
- **Cross-tenant default-deny.** Tenant A'nın route'una Tenant B'nin OAuth client'ı erişemez (consumer scope = tenant scope). İstisna ancak explicit cross-tenant grant ile.
- **Developer Portal tenant scoping.** UI'da görünen API katalog, key, analytics tamamen tenant-scoped; platform-admin role multi-tenant cross-view görür.

## Reddedilenler

| Alternatif | Neden reddedildi |
|---|---|
| **Kong-only (no Developer Portal)** | Kong OSS'te Developer Portal yok (Konnect SaaS-only). Cave sovereign — SaaS portal kullanılamaz. Tenant self-service onboarding için portal şart. |
| **Gravitee-only (no plugin DSL)** | Gravitee'nin proxy plugin ekosistemi Kong'un gerisinde; transformations, circuit-breaker, advanced rate-limit eksik. Tek başına Cave performans + extensibility hedefini karşılamıyor. |
| **İki ayrı UI (Kong admin UI + Gravitee Portal)** | **Burak yasakladı** — cave-native UNIFIED UX zorunlu. Multi-UI tenant friction yaratır, Backstage entegrasyonu parçalanır. |
| **Vendor-locked SaaS (Kong Konnect, Gravitee Cloud, Apigee, AWS API Gateway)** | Sovereignty invariant'ı ihlal. Tenant verisi vendor cloud'unda tutulamaz; PQC + GDPR + NIS2 sovereignty gerekleri. |
| **Envoy Gateway tek başına + custom portal** | Envoy düşük seviye; Kong + Gravitee'nin sağladığı API lifecycle abstraction'larını sıfırdan yazma maliyeti çok yüksek; iki upstream'in zenginliğinden faydalanmamak israf. |
| **Apache APISIX tek upstream** | Plugin ekosistemi Kong'un altında, Developer Portal yok. Tek upstream'in Kong + Gravitee birleşimini karşılamıyor. |

## Consequences

### Positive
- Tek Rust binary, iki ekosistem feature parity (Kong proxy + Gravitee portal/IAM/analytics).
- Tenant zero-friction self-service onboarding (Developer Portal cave-native).
- Multi-tenant invariant baştan içeride — Kong/Gravitee'de eklenti ile yapılan iş cave-gateway'de built-in.
- cave-llm-gateway, cave-secrets, cave-audit PQC entegrasyonu — Cave ekosistem ile derin entegre.
- Tek Admin API + tek Developer Portal UI = Backstage'den tek panele bağlanma.
- Shared cave-kernel WAL/Raft/PQC primitives.

### Negative
- İki upstream'in feature parity'si büyük scope (plugin set + portal + IAM federation + analytics).
- Test surface çok geniş (plugin chain edge-case'leri + portal flow'ları + IAM federation matrisleri).
- Performance hedefi yüksek: native Kong (Lua/nginx) throughput'u Rust async ile yakalama gerek.

### Risks
| Risk | Mitigation |
|---|---|
| Kong plugin DSL semantics (Lua) Rust trait'e tam map'lenemez | Plugin trait Kong'un üst seviye semantiğini takip eder; Lua-spesifik patterns dokümante edilir, tenant Lua plugin import etmez. |
| Gravitee Developer Portal feature drift (upstream hızlı evriliyor) | Portal feature freeze v0.2; sonraki sürümler quarterly upstream sweep. |
| Performance gap (Lua/nginx Kong vs Rust cave-gateway) | Rust async + zero-copy + kernel io_uring. Benchmark target: ≥85% Kong OSS RPS @ p99. |
| Plugin compatibility — Kong/Gravitee upstream plugin'leri direkt çalışmaz | Cave plugin trait yeniden yazım gerektirir; tenant'a "Cave plugin set" net dokümante edilir, Kong plugin import beklentisi YOK. |
| Multi-tenant cross-leak (config visibility) | Per-tenant namespace cave-kernel level isolation + integration test suite cross-tenant deny zorunlu pass. |

## Implementation phases

- **v0.1** — Core proxy data path + temel plugin set (rate-limit, key-auth, jwt, cors, transformations). Unified Admin API skeleton. Tek tenant.
- **v0.2** — `cave-gateway` implementation **bu sürümde başlar** (bu ADR sonrası ilk implementation milestone). Multi-tenant namespace, ResourceQuota, OAuth2 plugin, circuit-breaker, OpenAPI validation. cave-gateway-portal-ui v1 (API katalog + key management).
- **v0.3** — Developer Portal full feature parity (Gravitee'nin docs/onboarding/analytics dashboard set'i). cave-llm-bridge, cave-secrets, cave-audit-pqc plugin'leri. IAM federation (cave-auth ile Keycloak/Okta/Entra köprüsü). Backstage entegrasyonu.

## Mirror inheritance

- Platform `ADR-027 Kong API Gateway` — Platform'da Kong OSS deployment kararı **değişmedi**, deployment ADR olarak yerinde duruyor.
- Runtime bu ADR ile **iki upstream'i (Kong + Gravitee) konsolide ediyor** ve ADR-027'yi Runtime context'inde override ediyor — Mirror'ın multi-upstream consolidation özel hali (streaming + persistence ADR'leriyle aynı pattern).

## Compliance

- **SOC2 CC6.1** — API access controls; per-tenant authn/authz, key rotation, audit trail.
- **NIS2 Art.21** — API security (authn, rate-limit, OpenAPI validation, secret scan inline).
- **GDPR Art.32** — Tenant data isolation (per-tenant namespace + cross-tenant default-deny).
- **PQC-ready** — admin API + audit log ML-DSA/SLH-DSA imzalı (ADR-014 Zero-Trust hattı).

## Related

- **ADR-027 (Platform)** — Kong API Gateway; bu ADR onu Runtime context'inde override eder.
- **ADR-RUNTIME-UPSTREAM-MIRROR-001** — bu ADR onun multi-upstream consolidation istisnası.
- **ADR-RUNTIME-CONSOLIDATION-PRINCIPLE-001** — N-to-M konsolidasyon prensibi (LLM/IDM paralel pattern).
- **ADR-013** — LiteLLM as Unified LLM Gateway; cave-llm-bridge plugin entegrasyonu.
- **ADR-RUNTIME-SECRET-SCAN-001** — cave-secrets plugin için detector engine kaynağı.
- **ADR-014** — Zero-Trust Network Architecture; cave-audit PQC signing zorunluluğu.
- **ADR-RUNTIME-STREAMING-CONSOLIDATION-001** — kardeş ADR (streaming için aynı multi-upstream pattern).
- **ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001** — kardeş ADR (persistence için aynı multi-upstream pattern).
- **ADR-RUNTIME-STACK-001** — Layer 3 (cave-gateway) ve Layer 4 (Developer Portal) konumlandırması.
- **ADR-MULTI-TENANT-001** — namespace = tenant boundary; cross-tenant default-deny.

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-26 ADR session.*
