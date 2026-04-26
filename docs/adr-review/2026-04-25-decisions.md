# ADR Review — 2026-04-25 (Cave Runtime + Platform)

**Lead:** Burak | **Recorder:** Sonnet (Opus orchestrator)

**Goal:** Cave Runtime'ı OSS launch (21 May 2026) için temizlemek. Her ADR için karar:
- **KEEP** — Cave Runtime OSS branch'inde kalır
- **PLATFORM-only** — Üst platform repo'sunda kalır, OSS Runtime branch'inden çıkar
- **HETZNER-only** — Hetzner profili spesifik, Platform repo'sunda kalır
- **AZURE-only** — Azure profili spesifik, Platform repo'sunda kalır
- **RETIRE** — Stale, kaldırılır
- **MERGE-WITH-X** — başka ADR ile birleştirilir
- **SHRINK** — kalır ama N satıra indirilir

**OSS countdown:** 26 gün

---

## Decisions

### ADR-001 — Hetzner Cloud as Sovereign Infrastructure Provider
- **File:** `docs/adr/ADR-001_Hetzner_Cloud_as_Sovereign_Infrastructure_Provider.md`
- **Original status:** Accepted (Hetzner)
- **Decision (2026-04-25):** **PLATFORM, HETZNER-only**
- **Rationale:** Deployment target seçimi; Cave Runtime cloud-agnostic, OSS branch'inde olmamalı. Burak'ın iş yerindeki sovereign profili için üst Platform repo'sunda kalır.
- **Action:** OSS Cave Runtime branch'inden çıkar; `platform/docs/adr/` altında kalır.

### ADR-002 — Azure as Enterprise Infrastructure Provider
- **File:** `docs/adr/ADR-002_Azure_as_Enterprise_Infrastructure_Provider.md`
- **Original status:** Accepted (Azure)
- **Decision (2026-04-25):** **PLATFORM, AZURE-only**
- **Rationale:** Burak'ın iş yerindeki enterprise deployment profili. Cave Runtime cloud-agnostic, bu karar OSS kullanıcısı için değil.
- **Action:** OSS Cave Runtime branch'inden çıkar; `platform/docs/adr/` altında kalır.

### ADR-003 — Talos Linux for All Hetzner Profiles
- **File:** `docs/adr/ADR-003_Talos_Linux_for_All_Hetzner_Profiles.md`
- **Original status:** Accepted (Hetzner)
- **Decision (2026-04-25):** **RETIRE FROM RUNTIME** (Cave Runtime için irrelevant); **PLATFORM, HETZNER+AZURE option**
- **Rationale (Burak):** Cave Runtime kendisi bir Cloud OS — Layer 1 (Linux kernel 7.1, no backward compat) + Layer 2-4 (unified Rust reimpl: userspace + K8s + ekosist). Talos Cave Runtime'a TEMAS ETMEZ. Talos sadece Platform deployment'larında (Cave Runtime kullanmayan workload veya geçiş döneminde) opsiyondur.
- **Action:** Cave Runtime ADR'lerinden çıkar. Platform repo'da kalır, scope: Hetzner deployment için zorunlu (Hetzner managed K8s yok), Azure deployment için AKS-alternative seçenek.
- **Yeni ADR ihtiyacı:** Runtime tarafına `ADR-RUNTIME-STACK-001 — Cave Runtime Stack Architecture` yazılacak (Layer 1-4 tanımı + no Talos).

### ADR-RUNTIME-UPSTREAM-MIRROR-001 — Platform–Runtime Upstream Mirror (NEW)
- **File:** `docs/adr/ADR-RUNTIME-UPSTREAM-MIRROR-001-platform-runtime-mirror.md`
- **Status:** Accepted (2026-04-25)
- **Decision (2026-04-25):** **NEW META-ADR** — her sovereign Platform OSS seçimi otomatik olarak bir Runtime upstream-reimpl ADR'si üretir. İstisnalar: Layer 1 (kernel), Layer 0 (hardware) — Cave Runtime kendi OS olduğu için.
- **Rationale (Burak):** "Platform sovereign için seçtiğimiz OSS uygulamaları Runtime'da upstream'e alıp reimplemente ediyoruz." Aynı OSS seçimi iki perspektif üretir; Platform onu kullanır, Runtime onu reimpl eder.
- **Action:** ADR review sırasında her ürün seçimi ADR'si SPLIT olarak işlenir (Platform side + Runtime mirror).
- **ADR Quality Bar:** Her ürün seçimi ADR'si zorunlu olarak (1) karşılaştırma matrisi (2) 2-yıllık roadmap analizi (3) decision+rejected (4) forward-looking risks (5) mirror reference içerir.

### ADR-004 — Cilium CNI + Istio Ambient Mesh
- **File:** `docs/adr/ADR-004_Cilium_CNI_Istio_Ambient_Mesh.md`
- **Original status:** Accepted (Universal)
- **Decision (2026-04-25):** **SPLIT per ADR-RUNTIME-UPSTREAM-MIRROR-001**
  - **Platform side (KEEP)**: ADR-004 Platform repo'da kalır — sovereign deployment networking (Cilium + Istio Ambient + Kong). Roadmap analysis eklenecek (ADR Quality Bar).
  - **Runtime side (NEW)**: `ADR-RUNTIME-UPSTREAM-NETWORKING-001` yazılacak — cave-net + cave-mesh + cave-gateway upstream'leri (Cilium-inspired eBPF / Istio Ambient-inspired sidecar-less / Kong/Envoy-inspired N-S). TDD line-by-line parity scope.
- **Rationale (Burak):** Cave Runtime kendi networking'ini reimpl ediyor; Cilium+Istio = Runtime'ın upstream listesi. Aynı zamanda Platform deployment'larında bu OSS'ler kullanılır.
- **Action:** Platform ADR-004 SHRINK + roadmap; yeni Runtime ADR oluşturulacak (ileride task'la).

### Inheritance Model — Runtime ⟵ Platform (Burak 2026-04-25)
- **Default:** Runtime ADR'leri Platform'dan **inherit** eder. Platform bir tool seçtiyse, Runtime onu upstream'e alır ve reimplemente eder.
- **Override:** Cloud OS vizyonu ile çelişen Platform kararı → Runtime'a corresponding **override ADR** yazılır (örn. Talos: Platform kullanır, Runtime kullanmaz çünkü Runtime kendi OS'i — ADR-RUNTIME-STACK-001).
- **Independent:** Runtime'a özgü, Platform'da karşılığı olmayan ADR'ler de olabilir (örn. multi-tenant invariant, layering, kernel 7.1 PQC tasarımı).

### Project Scopes (Burak 2026-04-25)
- **Platform** — sovereign deployment için seçilen OSS uygulamalar + ortak deployment kararları.
- **Runtime (Cave Runtime)** — bu OSS'leri unified Rust ile reimpl eden Cloud OS. Platform'dan inherit + Cloud OS override + Runtime-independent ADR'ler.
- **Pipeline** — Platform için geliştiriliyor (CI/CD scaffolding). **Mümkünse Runtime'da da kullanılır** (dogfooding — Cave Runtime'ın kendi CI/CD'si).
- **MuleForge** — kesinlikle Platform için (Mule → Spring Boot/Camel migration tool).

### Inheritance Model — Refined (Burak 2026-04-25)
**Platform ADR'leri tek kaynak (single source of truth) sub-projeler kendileriyle alakalı olanları inherit eder.**

```
Platform (üst, ADR'ler burada)
  ├─ Runtime: Platform'dan inherit + Cloud OS override + independent
  ├─ Pipeline: Platform'dan CI/CD-related inherit
  └─ MuleForge: Platform'dan migration-related inherit
```

Yani bir ADR Pipeline-spesifik olsa bile (örn. Buildah CI tool seçimi), **Platform ADR'sidir**, Pipeline onu inherit eder. Sub-proje ADR'lerini sıfırdan yazmayız; ekleme veya override yaparız.

### ADR-005 — Buildah for Container Image Building
- **File:** `docs/adr/ADR-005_Buildah_for_Container_Image_Building.md`
- **Original status:** Accepted (Universal, CI/CD)
- **Decision (2026-04-25):** **PLATFORM ADR (Pipeline inherits)** + SHRINK + add 2-yıllık roadmap
- **Rationale (Burak):** Pipeline'a ait olsa bile Platform ADR'sidir; Pipeline sub-projesi Platform'dan inherit eder.
- **Action:**
  - Platform repo'ya taşı (kendi yerinde kalsın); Cave Runtime ADR klasöründen çıkar.
  - 2-yıllık roadmap analizi ekle (Buildah vs Kaniko vs apko/Wolfi disruption beklentisi vs Docker BuildKit; her birinin announced features + community direction + vendor strategy).
  - Pipeline projesi config-level override'ı (örn. ARC runner spesifik tuning) Pipeline repo'da ayrı bir notes/config olarak referans verir; ADR Platform'da kalır.
  - **Runtime mirror YOK** (Cave Runtime Buildah'i reimpl etmiyor; cave-cri = container RUNTIME, image building değil).

### CORRECTION — Inheritance default = automatic mirror, no separate Runtime ADR
Önceki ADR-004 split kararı revize: ayrı `ADR-RUNTIME-UPSTREAM-NETWORKING-001` yazılmaz. Mirror **otomatik** (ADR-RUNTIME-UPSTREAM-MIRROR-001 zaten kapsıyor). Ayrı Runtime ADR sadece şu 3 durumda yazılır:
1. Cloud OS override (örn. Talos — Cave Runtime kullanmaz)
2. Reimpl scope upstream'den anlamlı şekilde farklı
3. Runtime-specific architectural override (örn. PQC-hybrid)

### ADR-004 — REVİZE
- **Decision (revize 2026-04-25):** **PLATFORM KEEP**, otomatik Runtime mirror (cave-net + cave-mesh + cave-gateway = Cilium + Istio Ambient + Kong/Envoy reimpl). Ayrı ADR YOK.
- **Action:** ADR Platform repo'da kalır; runtime mirror ADR-RUNTIME-UPSTREAM-MIRROR-001'de implicit.

### ADR-006 — Keycloak for Hetzner Identity Provider
- **File:** `docs/adr/ADR-006_Keycloak_for_Hetzner_Identity_Provider.md`
- **Original status:** Accepted (Hetzner)
- **Decision (2026-04-25):** **PLATFORM KEEP**. Otomatik Runtime mirror (cave-auth = Keycloak reimpl).
- **Rationale (Burak):** Mirror otomatik, ayrı Runtime ADR yazmaya gerek yok. ADR-006 quality bar'ı zaten karşılıyor (4 alternatif karşılaştırma + Zitadel 2027 watch + risk analizi).
- **Action:** ADR Platform repo'da kalır. cave-auth zaten Keycloak reimpl yapıyor (91 test passing, main'de).

### ADR-007 — Okta + Entra ID for Azure Identity
- **File:** `docs/adr/ADR-007_Okta_+_Entra_ID_for_Azure_Identity.md`
- **Original status:** Accepted (Azure)
- **Decision (2026-04-25):** **PLATFORM KEEP, AZURE-only**
- **Rationale (Burak):** Azure deployment için SaaS identity (Okta workforce + Entra Azure RBAC). Cave Runtime SaaS reimpl etmiyor; cave-auth (Keycloak reimpl) zaten BYOID brokering ile enterprise tenants'ın Okta/Entra ile federate olmasını destekliyor.
- **Action:** Platform repo'da kalır (Azure-only). Cave Runtime Azure'da deploy edilirse cave-auth + Okta/Entra brokering çalışır.

### ADR-008 — Cache: Valkey (Hetzner) / Azure Redis (Azure)
- **File:** `docs/adr/ADR-008_Cache_-_Valkey_Hetzner___Azure_Redis_Azure.md`
- **Original status:** Accepted (Universal+Hetzner+Azure)
- **Decision (2026-04-25):** **PLATFORM KEEP**
- **Rationale:** Hetzner Valkey self-hosted + Azure Cache for Redis managed, Crossplane XR ile unified. Quality bar OK (4 alternatif + Dragonfly/Spotahome/Glide forward-looking watch'lar).
- **Action:** Platform repo'da kalır. Otomatik Runtime mirror: cave-cache = Valkey upstream reimpl (ADR-RUNTIME-UPSTREAM-MIRROR-001 kapsamında).

### ADR-009 — Ollama (Hetzner) / Azure OpenAI (Azure)
- **File:** `docs/adr/ADR-009_Ollama_Hetzner___Azure_OpenAI_Azure.md`
- **Original status:** Accepted (Universal+Hetzner+Azure)
- **Decision (2026-04-25):** **PLATFORM KEEP** + **monthly review cadence (zorunlu)**
- **Rationale (Burak):** AI/LLM alanında çok hızlı gelişme var; ADR-009 ayda bir review edilip güncellenmeli (model recommendations, provider capabilities, cost/perf, EU AI Act compliance). Quality bar zaten "Review Cadence: Monthly" diyor — kalın altını çiz.
- **Action:** Platform repo'da kalır. Monthly review item olarak workflow doc'ta tracked. Otomatik Runtime mirror: cave-local-llm = Ollama wrapper (şu an), full reimpl uzun vade.

### ADR Quality Bar — Review Cadence (NEW field)
**Burak 2026-04-25:** Ürün seçimi ADR'lerinde alanın değişim hızına göre review cadence belirtilmeli:
- **Monthly** — fast-moving (AI/LLM, frontier models, vendor strategies)
- **Quarterly** — moderate (cloud provider features, observability tools)
- **Annual** — stable (DBs like Postgres, well-established CNCF projects)
- **On-event** — license-changes / acquisitions / major incidents

Workflow doc'a "ADR Review Calendar" section eklenecek.

### ADR-010 — CI Pipeline Architecture (27 Stages) — REFRAMED
- **File:** `docs/adr/ADR-010_CI_Pipeline_Architecture_27_Stages.md`
- **Original status:** Accepted (Universal)
- **Decision (2026-04-25):** **PLATFORM ADR (Pipeline inherits)** — meta-pipeline pattern, BUILD side only.
- **Refined understanding (Burak 2026-04-25):**
  - 27-stage = **iskelet pattern**, language adapter ile parametrize edilir
  - **Per-project adapters:** Platform=opentofu, Runtime=rust, Backstage=node, Java API=java, Python MCP=python, MuleForge=hibrit (rust+node)
  - **CI ≠ CD**: ADR-010 BUILD side (image + manifests + SBOM + provenance). Deploy composition AYRI ADR'lerde.
  - **Deploy composition** (Crossplane XR + multi-resource):
    - K8s manifest + cave-pg DB XR + cave-cache XR + cave-gateway Route XR + cave-vault secret XR + cave-streams topic XR
    - Service registration: Backstage Catalog entity, LibreChat MCP registration (eğer MCP), OPA policy, Prometheus scrape + alert + dashboard JSON
    - Tenant scoping (multi-tenant invariant): tenant_id label tüm resources
- **Action:**
  - ADR-010 Platform repo'da (BUILD pattern)
  - Pipeline projesi inherit eder + adapter scaffolding (cookiecutter per language)
  - **Yeni ADR ihtiyacı:** `ADR-DEPLOY-COMPOSITION-001` — Crossplane XR + Backstage entity + LibreChat MCP registration + multi-tenant scoping (deploy side)
- **Yeni ADR Quality Bar field:** Deploy composition, sadece "kubectl apply" değil; multi-resource Crossplane XR + service registration zorunlu pattern.

### NEW: ADR-RUNTIME-STREAMING-CONSOLIDATION-001 (Kafka + Pulsar → cave-streams)
- **File:** `docs/adr/ADR-RUNTIME-STREAMING-CONSOLIDATION-001-kafka-pulsar-into-cave-streams.md`
- **Status:** Accepted (2026-04-25)
- **Decision (Burak):** Cave Runtime streaming için Kafka VE Pulsar **birden konsolide reimpl** — `cave-streams` tek crate, iki wire protokolü.
- **Override of:** ADR-RUNTIME-UPSTREAM-MIRROR-001 default (multi-upstream consolidation istisnası).

### NEW: ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001 (multi-upstream data layer)
- **File:** `docs/adr/ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md`
- **Status:** **Proposed** (Burak finalize edecek — açık sorular var)
- **Decision draft:** Cave Runtime persistence çoklu upstream konsolide: cave-pg (PG), cave-docdb (Mongo), cave-cache (Valkey), cave-iceberg+cave-datafusion (analytics) — eklenmesi muhtemel: cave-distsql (CockroachDB/TiDB/YugabyteDB), cave-tsdb (Influx/TS/VictoriaMetrics), cave-search (ES/OpenSearch/Quickwit), cave-blobs (S3-compat), cave-graph (Neo4j/Dgraph/JanusGraph).
- **Açık sorular:** distsql/tsdb/search/graph/blobs için spesifik upstream + cave-pg vs cave-distsql sınırı. Burak yanıtlayacak.

### ADR-011 — Backstage as Developer Portal
- **File:** `docs/adr/ADR-011_Backstage_as_Developer_Portal.md`
- **Decision (2026-04-25):** **PLATFORM KEEP** (Burak onayladı 2026-04-25). Auto Runtime mirror via cave-portal reimpl. Quality bar OK, Kratix 2027 forward-watch.

## ADR-012 — Tenant Isolation: Tenant Kamaji + Long-term Env Nested Kamaji + Capsule Ephemeral + Suspend/Resume Governance

Status: Accepted (v7) — finalized with Burak 2026-04-26
Scope: Universal
Category: Multi-Tenancy + Cost Optimization

### Architecture (3 building blocks)
1. **Top tenant** — Kamaji TCP per account, hard isolation always
2. **Long-term env** — Nested Kamaji TCP per env (prod/staging/dev), real CP per env
3. **Capsule namespace** — Ephemeral, optional parent (within long-term env OR direct under tenant)

### vcluster: dropped — Kamaji recursive + Capsule covers all use cases

### Suspend/Resume (every Kamaji TCP suspendable)
- Capsule ephemeral: TTL-based, no governance
- Long-term env dev: single confirmation, 1h cancel window
- Long-term env staging: double confirmation, 4h cancel window
- Long-term env production: **two-person rule** (M-of-N=2/2) OR super-admin triple-confirm, 24h hold + 24h cancel
- Top tenant: two-person rule + super-admin override, 48h hold + 48h cancel

### Implementation
- `cave-kamaji` — recursive TCP + suspend controller + ApprovalRequest CRD
- `cave-capsule` — direct-at-tenant + in-env namespace + TTL controller
- `cave-audit` — immutable signed (PQC ML-DSA) audit log per tenant, 7y retention
- `cavectl approval list/approve/cancel/show` + `cavectl env|tenant suspend/resume`

### Acceleration
1. Pre-warm pool top-tenant + long-term env (each level)
2. Snapshot clone (APFS CoW host, overlayfs Linux equivalent)
3. Minimal CP profile child TCP'lerde
4. Tiered etcd (long-term dedicated, ephemeral shared)
5. Rust impl 2-3x faster than Go upstream

### Rejected
- vcluster (everywhere) — Kamaji recursive covers ephemeral
- Capsule-only soft tier — namespace isolation insufficient for compliance
- Single-person production suspend — too risky, two-person rule
- No suspend hold window — irreversible mistakes

