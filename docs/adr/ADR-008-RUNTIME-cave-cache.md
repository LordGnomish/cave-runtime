<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-008-RUNTIME — cave-cache: Sovereign In-Memory Store (Valkey parity) (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — provider-agnostic)
**Category:** Data
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-008 (Cache — Valkey (Hetzner) / Azure Redis (Azure))
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-008 cache için **Valkey** (Hetzner) + **Azure Cache for Redis**
(Azure) ikili stratejisini, Crossplane XR ile birleşik bir Cache API arkasında
seçti. Cave Runtime **sovereign** bir Cloud OS'tür: **Azure Redis Runtime için
ilgisizdir** — managed bir cloud servisidir, air-gap'te çalışmaz, billing'i
dışarı akıtır. Bu Runtime variant **yalnızca Valkey**'i tutar ve onu **cave-cache**
ile materialize eder: Valkey 8 = Redis 7.2-compatible in-memory store'un Rust
reimpl'i.

cave-cache provider-agnostic'tir (Hetzner/AWS/GCP/Azure/bare-metal eşit); managed
provider cache servisleri **explicitly out-of-scope**'tur — sovereignty bunu
gerektirir.

## Context

### Neden bir Runtime variant gerekli
Platform variant'ın "Azure Redis (Azure)" tarafı Cave'in sovereignty postürüyle
çelişir: managed Azure servisi external control-plane'e bağlıdır, kritik yolda
SaaS yaratır, ve air-gap senaryosunda çalışmaz (ADR-001 charter rule 3). Crossplane
XR ile iki-provider soyutlaması Runtime için gereksizdir — Cave tek sovereign cache
implementasyonu çalıştırır. Ek olarak cache **in-binary / in-cluster** olmalı,
external provider'a reach-out etmemeli.

### Korunan değer
Valkey'in BSD-3-Clause (vendor-lock-in yok), tam Redis protokol uyumu, RDB+AOF
persistence, Redis Cluster modu, ve ACL-per-tenant multi-tenant izolasyonu — hepsi
cave-cache hedefidir.

## Candidates

| Kriter | **Valkey** (→ cave-cache) | Redis OSS (post-2024) | ~~Azure Redis~~ | Dragonfly | KeyDB | Memcached |
|---|---|---|---|---|---|---|
| License | **BSD-3-Clause** (Linux Foundation) | RSALv2 + SSPLv1 | (managed, irrelevant) | BSL 1.1 | BSD-3-Clause | BSD |
| Redis protokol | ✅ Full (Redis 7.2 fork) | ✅ native | — | ✅ drop-in | ✅ drop-in | ❌ KV only |
| Cluster modu | ✅ Redis Cluster | ✅ | — | ✅ emulation | ✅ | ❌ |
| Persistence | ✅ RDB + AOF | ✅ | — | ✅ | ✅ | ❌ |
| Self-host viable | ✅ tam | ⚠️ license kısıtı | ❌ **Azure-only** | ✅ | ✅ | ✅ |
| Multi-tenant | ✅ ACL/tenant (Redis 6+) | ✅ | — | ✅ | ✅ | ❌ |
| Community | Hızla büyüyen (LF, ex-Redis) | parçalı (license sonrası) | — | küçük | küçük (Snap azalttı) | olgun |

> **Azure Cache for Redis** sütunu Runtime için **çizilmiştir** — managed cloud
> servisi sovereignty + air-gap ihlali; değerlendirme-dışı.

## Decision

**cave-cache** (Valkey 8 parity, Redis 7.2-compatible, Rust reimpl) Cave Runtime'ın
**tek** sovereign in-memory store'udur. Session yönetimi, rate limiting, pub/sub,
application caching için. RDB + AOF persistence; Redis Cluster modu; **ACL-per-tenant**
multi-tenant izolasyon; tam Redis protokol uyumu (mevcut tüm Redis client'ları
değişmeden çalışır).

**Valkey, Redis OSS yerine seçilir** — Redis Labs'ın RSALv2/SSPL dual-license
değişikliği Cave'in zero-vendor-lock-in prensibini (ADR-001) ihlal eder. Aynı
mantık OpenBao'nun Vault yerine (cave-vault, [[cave-vault-pqc-seal-2026-06-07]])
seçilmesindeki gibidir.

**Azure Redis ve tüm managed provider cache servisleri kapsam-dışıdır** —
sovereignty gerekçesiyle. cave-cache her provider'da (Hetzner/AWS/GCP/Azure/bare-metal)
**self-hosted in-cluster** çalışır; provider'ın managed cache'ine reach-out etmez.

### Runtime yükseltmeleri
- **TLS + PQC-ready transport** — client↔cache ve replica↔replica TLS,
  cave-vault PQC hierarchy ile sertifika (ADR-RUNTIME-CERT-LIFECYCLE-001).
- **Single-binary** — cave-cache cave-runtime'a mount'lu; ayrı daemon değil.
- **Persistence-consolidation** — cave-cache, cave-rdbms/cave-docdb/cave-etcd ile
  birlikte tek veri katmanı ADR'inde (ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001)
  konumlanır.

## Rejected

- **Redis OSS (post-2024)** — RSALv2 + SSPLv1 dual-license; zero-vendor-lock-in
  (ADR-001) ihlali. Valkey orijinal Redis katkıcılarının BSD-3-Clause fork'u.
- **Azure Cache for Redis** — managed Azure servisi; **sovereignty + air-gap
  ihlali**, billing dışarı akar. Runtime için kategorik kapsam-dışı.
- **Dragonfly** — BSL 1.1 (Vault/Redis ile aynı endişe); etkileyici performans
  iddialarına rağmen BSL disqualifying.
- **KeyDB** — BSD-3-Clause (kabul edilebilir) ama Valkey'den küçük community;
  Snap Inc. yatırımı azalttı. Valkey'in LF backing'i ve momentumu üstün.
- **Memcached** — persistence yok, pub/sub yok, sorted-set yok; session/rate-limit
  için fazla sınırlı.

## Consequences

### Olumlu
- **BSD-3-Clause** — vendor-lock-in riski yok.
- Tam Redis protokol uyumu — tüm Redis client'ları değişmeden çalışır.
- LF backing → community governance istikrarı.
- **ACL-per-tenant** izolasyon (shared cave-cache instance, soft tier).
- Sovereign + air-gap-capable — managed cloud bağımlılığı yok.
- Single-binary, in-cluster.

### Olumsuz / maliyet
- Valkey genç (2024 fork) — Valkey ↔ Redis diverge ettikçe edge-case uyum
  sorunları çıkabilir; cave-cache parity testleri (ADR-135 eşdeğeri) bunu kapsamalı.
- Managed cache'in operasyonel kolaylığından feragat — HA/persistence/upgrade
  Cave'in sorumluluğunda.
- cave-cache reimpl Redis 7.2 yüzey-genişliğini honest takip etmeli (manifest-authored).

### Riskler & azaltım
- **Valkey ↔ Redis protokol drift** → cave-runtime-tracker Valkey upstream'ini
  izler; parity testleri davranış-uyumunu doğrular.
- **Persistence veri kaybı** → RDB + AOF + cave-backup snapshot.
- **Multi-tenant ACL bypass** → per-tenant ACL + cave-mesh mTLS (ADR-004-RUNTIME).

## Compliance Mapping

- **SOC2 CC6.1** — access controls (ACL-per-tenant).
- **ISO A.8.24** — encryption (TLS in transit, encryption-at-rest for persisted data).
- **GDPR Art.32** — security of processing (shared-cache içinde tenant izolasyonu).

## Charter v2 8-gate linkage

Strict TDD: cave-cache `parity.manifest.toml` (Valkey/Redis 7.2 parity) Charter v2
self-audit gate'lerine bağlı. Bu ADR Azure Redis'i honest gerekçeyle (sovereignty)
kapsam-dışı bırakır — gate_1 (no fabrication) bu kararı şişirmez. `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — zero-vendor-lock-in + air-gap charter
- [ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001](ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md) — cave-rdbms/docdb/cache/etcd veri katmanı
- [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) — TLS/PQC sertifikalar
- [ADR-147](ADR-147_Data_Persistence_Crate_Naming_and_Lakehouse_Consolidation.md) — veri-persistence crate naming
- **Platform ADR-008** — Valkey (Hetzner) / Azure Redis (Azure) dual-provider reference variant

---

*Bu ADR Platform ADR-008'in **Runtime sovereign variant**'ıdır. Valkey → cave-cache
Rust reimpl ile materialize edilmiş; Azure Redis (managed cloud) sovereignty
gerekçesiyle kapsam-dışı bırakılmıştır — yalnızca Valkey. Cave Runtime AGPL-3.0-or-later.*
