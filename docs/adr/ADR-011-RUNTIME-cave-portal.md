<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-011-RUNTIME — cave-portal: Sovereign Developer Portal, Rust-native Backstage parity (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — single binary, no external service)
**Category:** Developer Experience / Portal
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-011 (Backstage as Developer Portal)
**Upstream:** backstage/backstage `v1.50.3` (`source_sha` pinned)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-011 developer portal için **Backstage**'i (Spotify, CNCF) seçti:
Software Catalog, Scaffolder, TechDocs ve TypeScript/React eklenti ekosistemi.
Backstage **runtime'ı bir Node.js/TypeScript servisidir** — ayrı bir process,
ayrı bir dil toolchain'i, ayrı bir deployment. Bu, Cave Charter **§5 single-binary
mandate**'ini doğrudan ihlal eder: Cave Runtime tüm yüzeyini tek Rust binary'sinde
ship eder; yanına ikinci bir TS/Node servisi koymak sovereignty, supply-chain ve
operasyonel postürü kırar.

Bu Runtime variant **Backstage runtime'ını KULLANMAZ**. Bunun yerine Backstage'in
**kavramlarını** — entity model, scaffolder, docs-as-code, plugin ecosystem,
RBAC — **Rust-native** olarak `cave-portal` crate'ine port eder. Karar **inherit
yaklaşımı** (Burak onaylı): *"upstream UI/CLI port edilmez; her crate cave-portal +
cave-cli içine entegre"* (memory `cave_runtime_unified_portal_cli_integration.md`).
Backstage'in YAML-tabanlı Declarative Integration'ı, Rust **attribute makroları**
(`#[cave_portal::page]`, `#[cave_portal::route]`) ile değiştirilir — compile-time
doğrulama, Backstage'in runtime YAML check'inden güçlüdür.

## Context

### Neden bir Runtime variant gerekli
Platform variant portal'ı Backstage **çalıştırarak** materialize ediyor. Cave
Runtime'da bu mümkün değil:

1. **Charter §5 (single binary)** — Backstage'in `backend` + `app` (React SPA)
   ayrı Node process'lerdir; webpack bundle + npm dependency ağacı tek Rust
   binary'sine sığmaz. İkinci runtime = §5 ihlali.
2. **Sovereignty / supply-chain** — npm transitive bağımlılık yüzeyi (binlerce
   paket) Cave'in source-pinned, SLSA-L4, ML-DSA-signed build hattı (ADR-005) ile
   denetlenemez. TS toolchain ayrı bir tedarik zinciri açar.
3. **Operasyonel tekillik** — Cave operatörü tek binary deploy eder, tek metrics
   port'u, tek config yüzeyi. Backstage ayrı lifecycle, ayrı upgrade, ayrı CVE
   takibi getirir.

### Korunan değer
Backstage'in **fikirleri** birinci sınıf değerdir ve korunur: katalog entity
modeli, template-driven scaffolding, docs-as-code, plugin genişletilebilirliği,
tenant-scoped RBAC. Korunmayan tek şey **implementasyon runtime'ı** (TS/Node).
cave-portal bunları Rust-native (Yew + axum) materialize eder ve halihazırda
`crates/cave-portal` olarak yaşar (Backstage `v1.50.3` parity, manifest-pinned).

## Candidates

**None — direct Backstage parity decision.** Platform ADR-011 upstream seçimini
(Backstage) zaten yaptı; bu Runtime variant *yeni bir aday değerlendirmesi
açmaz*. Tek karar **runtime-mı yoksa parity-mi** sorusudur ve cevap **parity**
(Charter §5 gereği). Karşılaştırma tablosu bu nedenle aday-eksenli değil,
**runtime-vs-parity** eksenlidir:

| Kriter | **cave-portal (Rust-native parity)** | ~~Backstage runtime (TS/Node)~~ |
|---|---|---|
| Tek binary (Charter §5) | ✅ Rust binary içinde | ❌ ayrı Node servis |
| Supply-chain denetlenebilir | ✅ source-pinned, SLSA-L4 (ADR-005) | ❌ npm transitive yüzeyi |
| Sovereign / air-gap | ✅ in-binary, external reach-out yok | ⚠️ ayrı runtime + npm registry |
| Eklenti modeli | ✅ Rust trait + attribute macro (compile-time) | YAML Declarative Integration (runtime) |
| Tip güvenliği | ✅ Rust type system (compile-time) | TS (build-time) + runtime YAML check |
| Upstream kavram parity | ✅ Catalog/Scaffolder/TechDocs/RBAC | (referans) |

> **Backstage runtime sütunu çizilmiştir** — Charter §5 single-binary mandate
> nedeniyle Runtime'da çalıştırılması **kategorik kapsam-dışıdır**. Değerlendirme,
> "hangi portal" değil "Backstage runtime'ı mı çalıştırılır yoksa kavramları
> Rust'a mı port edilir" sorusudur.

## Decision

**cave-portal**, Cave Runtime'ın **tek** developer portal yüzeyidir: Backstage
`v1.50.3` **kavramsal parity**'sinin Rust-native reimpl'i (Yew UI + axum backend,
single binary içinde). Backstage runtime'ı **çalıştırılmaz**. Aşağıdaki Backstage
konseptleri Rust-native port edilir:

### 1. Software Catalog (entity model parity)
- **YAML-driven entity tanımları**: Component, API, Resource, System, Domain,
  Group, User, Location — Backstage entity kind'leri birebir.
- **Catalog backend**: persistence **cave-pg** üzerinden (Backstage'in kendi
  PostgreSQL'i değil → `cave-data-persistence` / ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001).
- **Catalog processor**: auto-discovery — cave-runtime crate'lerinden entity
  keşfi (her crate'in `parity.manifest.toml` + portal kaydı taranır).
- **Search backend**: cave-search veya inline **Tantivy** embed (in-binary,
  external Elasticsearch yok).

### 2. Scaffolder (template-driven project creation)
- **Cargo template engine** — `cargo-generate` parity.
- **Multi-step wizard UI** — Yew tabanlı cave-portal akışı.
- **Skeleton repo'lar** — cave-runtime crate skeleton, cave-home adapter
  skeleton vb. (yeni crate "next free number" konvansiyonuyla — bkz. ADR
  numbering policy).

### 3. TechDocs (docs-as-code)
- **Markdown + diagramlar** — mdBook veya zola parity (in-binary render).
- **Plugin: cargo doc integration** — Rust crate API docs portal'a mount.
- **Search integrated** — Catalog search backend ile ortak indeks.

### 4. Plugin ecosystem (Rust-native, no TypeScript)
- cave-portal'ın **native plugin trait + registry**'si.
- **Per-crate UX page mount** — her `cave-*` crate `portal/page.rs` sağlar
  (memory `cave_runtime_unified_portal_cli_integration.md`).
- **4-track completion mandate** — her crate Portal + CLI + API + observability
  dört izini de ship eder (memory `cave_runtime_four_track_completion.md`).

### 5. RBAC + permissions (tenant-scoped)
- **cave-auth** entegrasyonu (Keycloak parity, **ADR-006-RUNTIME inherit**):
  OIDC + RBAC + ABAC.
- **SPIFFE workload identity** her erişimde (ADR-006 SPIFFE inherit) —
  tenant-scoped, [[cave-identity-honest-cont2-2026-05-31]] iş hattı.

### 6. Declarative Integration replacement
- Backstage'in YAML **Declarative Integration** (no-TS) modeli → cave-portal
  **Rust attribute makroları**: `#[cave_portal::page]`, `#[cave_portal::route]`.
- **Compile-time doğrulama** — Rust type system, Backstage'in runtime YAML
  check'inden güçlü; geçersiz page/route mount'u derlenmez.

> **Backstage runtime'ı (TS/Node servisi) çalıştırılmaz** — Charter §5
> single-binary mandate gereği kategorik kapsam-dışı. Operatör isterse kendi
> sorumluluğunda yan tarafta upstream Backstage koşabilir; bu Runtime
> catalogue'un sovereign default'u **değildir**.

### cave-portal ↔ komşu crate'ler / ADR'lar
- **cave-auth** (ADR-006-RUNTIME) — RBAC/ABAC/SPIFFE sağlayıcı.
- **cave-data-persistence** (ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001) — catalog
  backend storage (cave-pg).
- **ADR-PORTAL-AUTH-001 / -PERSONAS-001 / -DESKTOP-001** — mevcut cave-portal
  auth, persona-gated chrome ve GPUI native admin shell kararları; bu ADR onların
  Backstage-parity kapsamını charter'a bağlar.

## Rejected

- **Backstage runtime'ını çalıştırmak (TS/Node servisi olarak)** — **Charter §5
  single-binary mandate ihlali**. Ayrı process, ayrı dil toolchain, npm
  transitive supply-chain yüzeyi (SLSA-L4 + ML-DSA build hattıyla — ADR-005 —
  denetlenemez), ayrı lifecycle/CVE takibi. **Kategorik kapsam-dışı.**
- **Hibrit (Backstage frontend + cave backend)** — yine bir Node/React runtime'ı
  taşır; §5 hâlâ ihlal, supply-chain hâlâ açık. Reddedildi.
- **YAML Declarative Integration'ı runtime'da taklit etmek** — Rust attribute
  makroları compile-time doğrulama verir; runtime YAML check daha zayıf ve
  ekstra parse yüzeyi açar. Demote edildi (makro lehine).
- **Portal'sız (sadece cavectl CLI)** — developer experience parity hedefiyle
  çelişir; katalog/scaffolder/techdocs görsel yüzeyi gerekli. Reddedildi.

> Platform variant portal'ı Backstage **çalıştırarak** sağlıyordu; Runtime
> Backstage'in **kavramlarını Rust'a port eder**, runtime'ını çalıştırmaz —
> tek binary korunur.

## Consequences

### Olumlu
- **Charter §5 korunur** — portal tek Rust binary içinde, ikinci runtime yok.
- **Tam sovereignty / air-gap** — npm registry'ye veya external Backstage
  servisine reach-out yok.
- **Denetlenebilir supply-chain** — Rust crate grafiği source-pinned, SLSA-L4
  hermetic build (ADR-005); TS toolchain yüzeyi elenir.
- **Compile-time güvence** — page/route mount'ları (`#[cave_portal::page]`) tip
  sisteminde doğrulanır; geçersiz entegrasyon derlenmez.
- **4-track tutarlılığı** — her crate'in portal page'i kendi crate'inde yaşar;
  portal genişlemesi crate ekleyince otomatik (auto-discovery processor).
- Backstage entity modeli + scaffolder + techdocs developer-experience parity'si
  korunur.

### Olumsuz / maliyet
- **Reimplementation eforu** — Backstage'in olgun TS eklenti ekosistemi (3rd-party
  plugin marketplace) birebir taşınmaz; Cave kendi Rust plugin trait'ini büyütür.
- **Ekosistem tavanı** — Backstage community plugin'lerinden (örn. niş entegrasyonlar)
  doğrudan faydalanılamaz; her biri Rust-native port gerektirir (bilinçli trade-off).
- **Parity bakım yükü** — Backstage `v1.50.3`→sonraki sürümler için cave-portal
  manifest'i always-latest takip etmeli (upstream-watch / ADR-RUNTIME-UPSTREAM-WATCH-001).

### Riskler & azaltım
- **Backstage upstream drift** → `parity.manifest.toml` `source_sha` pin + günlük
  upstream tracker (ADR-RUNTIME-UPSTREAM-WATCH-001).
- **Plugin ekosistem boşluğu** → 4-track mandate her crate'in kendi portal
  page'ini sağlamasını zorunlu kılar; merkezi marketplace'e bağımlılık azalır.
- **Honest parity inflation** → fill_ratio manifest-authored, Charter 8-gate
  self-audit (gate_1 no fabrication); scope-cut'lar PARITY_REPORT'ta dokümante.

## Compliance Mapping

Platform ADR-011'den **inherit** edilen mapping'ler:

- **SOC2 CC8.1** (change management visibility) — cave-portal **Software Catalog**
  entity/owner/system görünürlüğü değişiklik yönetimini izlenebilir kılar.
- **ISO/IEC 27001 A.5.37** (documented operating procedures) — cave-portal
  **TechDocs** (docs-as-code) operasyonel prosedürlerin dokümante ve sürümlü
  tutulmasını sağlar.
- **GDPR Art.25** (data protection by design) — catalog/persistence cave-pg'de,
  veri küme dışına çıkmaz (in-binary, no external Backstage).
- **ISO A.9 / A.5.15** (access control) — tenant-scoped RBAC/ABAC + SPIFFE
  (cave-auth, ADR-006-RUNTIME inherit).

## Charter v2 8-gate linkage

Strict TDD: cave-portal `parity.manifest.toml` upstream **backstage/backstage
`v1.50.3`** (`source_sha = v1.50.3` pinned) Charter v2 self-audit gate'lerine
bağlı. 8-gate per crate: TDD strict, SPDX header, source-pinned to Backstage
version, no-stubs, no-backcompat, always-latest, 4-track ship, honest fill_ratio.
Backstage runtime usage honest gerekçeyle (Charter §5 single-binary) kapsam-dışı;
Rust-native parity scope PARITY_REPORT'ta dokümante. `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — air-gap + sovereignty + single-binary charter (§5)
- [ADR-005-RUNTIME](ADR-005-RUNTIME-buildah.md) — SLSA-L4 hermetic build (supply-chain denetlenebilirliği gerekçesi)
- [ADR-006-RUNTIME](ADR-006-RUNTIME-cave-auth.md) — cave-auth RBAC/ABAC/SPIFFE (cave-portal access control inherit)
- [ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001](ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md) — catalog backend storage (cave-pg)
- [ADR-RUNTIME-CLI-CONSOLIDATION-001](ADR-RUNTIME-CLI-CONSOLIDATION-001-cavectl-native-and-compat.md) — cavectl (CLI track of 4-track)
- [ADR-PORTAL-AUTH-001](ADR-PORTAL-AUTH-001.md) — cave-portal authentication
- [ADR-PORTAL-PERSONAS-001](ADR-PORTAL-PERSONAS-001.md) — cave-portal persona-gated chrome
- [ADR-PORTAL-DESKTOP-001](ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md) — cave-portal GPUI native admin shell
- **Platform ADR-011** — Backstage as Developer Portal (reference variant)

---

*Bu ADR Platform ADR-011'in **Runtime sovereign variant**'ıdır. Backstage'in
developer-portal kavramları (Software Catalog, Scaffolder, TechDocs, plugin
ecosystem, RBAC) **Rust-native** olarak cave-portal'a port edilmiştir; Backstage
runtime'ı (TS/Node servisi) **Charter §5 single-binary mandate** gereği
çalıştırılmaz — tek binary, in-cluster, external reach-out yok. Upstream
backstage/backstage v1.50.3 source-pinned. Cave Runtime AGPL-3.0-or-later.*
