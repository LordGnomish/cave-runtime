<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-012-RUNTIME — Multi-Tier Tenancy: Hybrid cave-kamaji (Hard) + cave-vcluster (Ephemeral PR) + cave-policy (Soft) (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — single binary, no external service)
**Category:** Multi-Tenancy
**Decided:** 2026-06-07 / 2026-06-08 (Burak Tartan)
**Variant-of:** Platform ADR-012 (vcluster for Hard Tenancy + PR Environments) — **HYBRID update** (supersedes vcluster-only)
**Upstream (Hard tier):** clastix-labs/kamaji `v1.0.0` (CNCF Sandbox, Apache-2.0)
**Upstream (Ephemeral PR):** loft-sh/vcluster `v0.34.2` (Loft Labs, Apache-2.0)
**Related platform ADRs:** 070 (resource caps), 084 (tenant model)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-012 başlangıçta **vcluster**'ı hem Hard/Dedicated tier production hem
de ephemeral PR ortamları için **tek** çözüm olarak seçti; Kamaji o aşamada
değerlendirilen adaylar arasında **değildi**. Hard tier production izolasyonu
gözden geçirilince **Kamaji'nin dedicated control-plane modeli**, vcluster'ın
syncer-tabanlı virtual-cluster yaklaşımından **maddi olarak daha güçlü izolasyon**
sağladığı görüldü (Burak hibrit onayı, platform `ADR-12-HYBRID-update-2026-06-07`).

Bu Runtime variant kararı **katmana göre (tier-differentiated)** ayırır ve onu
Cave Runtime'ın sovereign, single-binary primitive'lerine re-root eder:

- **Hard tier (kalıcı production)** → **cave-kamaji** (clastix-labs/kamaji parity):
  her tenant'a gerçek **dedicated control-plane** (apiserver + controller-manager
  + scheduler ayrı Pod'lar), shared host CP üzerine syncer projeksiyonu değil.
- **Ephemeral PR ortamı** → **cave-vcluster** (loft-sh/vcluster parity, **YENİ crate
  scaffold gerekir**): ~30 sn lightweight create, native short-lived lifecycle,
  PR başına temiz cost attribution.
- **Soft tier (paylaşımlı, namespace-scoped)** → **namespace + cave-policy**
  (Capsule-tarzı namespace-only izolasyon, ayrı virtual CP yok).

Charter §5 single-binary mandate korunur: her iki tenancy yöneticisi de upstream
runtime'ları **çalıştırmaz**; Kamaji (Go) ve vcluster (Go) **kavramları** Rust-native
olarak `cave-kamaji` / `cave-vcluster` crate'lerine port edilir.

## Context

### Neden bir Runtime variant + neden hibrit

Platform variant tenancy'yi tek araçla (vcluster) materialize ediyordu. İki ayrı
gözden geçirme bunu değiştirdi:

1. **Hard tier izolasyon gücü.** Production Hard/Dedicated tenant'ı, shared host
   control-plane'e kaynak projekte eden bir syncer yerine **gerçekten dedicated**
   control-plane bileşenlerini hak eder. Kamaji bunu standart Kubernetes
   resource'ları olarak verir (apiserver/cm/scheduler tenant başına ayrı Pod) —
   syncer işletme/debug/version-lockstep yükü olmadan.
2. **Kubernetes upgrade bağımsızlığı.** Kamaji'de her tenant'ın control-plane
   sürümü kendi hızında ilerler; host/management cluster upgrade döngüsünden
   decoupled'dır. vcluster'da vcluster sürümü host K8s sürümünü izlemek zorundadır.
3. **Ephemeral hız.** PR ortamı için vcluster'ın ~30 sn create + native TTL teardown
   modeli, dedicated CP overhead'i hiç amorti edilemeyecek kısa-ömürlü kullanımda
   doğru maliyet/fayda dengesidir. Kamaji bu kullanım için over-provisioned olur.

### cave-kamaji ≠ cave-vcluster (kapsam ayrımı, çelişki değil)

`crates/cave-kamaji/parity.manifest.toml` "vcluster is NOT a target" notu
**crate kapsamı** içindir: cave-kamaji yalnızca **Kamaji**'yi port eder, vcluster'ı
değil. Bu ADR o notu çürütmez — vcluster **ayrı bir crate** (`cave-vcluster`)
olarak yaşar. Platform tenancy modeli **iki crate'i birden** kullanır; tek bir
crate iki upstream'i karıştırmaz. Charter "next free number" / crate-per-upstream
hijyeni korunur.

### Korunan değer

Tenant-scoped izolasyon, dedicated kubeconfig deneyimi, ephemeral PR lifecycle ve
FinOps cost attribution birinci sınıf değerlerdir ve korunur. Korunmayan tek şey
upstream **implementation runtime'ı** (Go binary'leri ayrı servis olarak
çalıştırılmaz); kavramlar Rust-native materialize edilir, tek binary içinde.

## Candidates

Platform ADR-012 aday tablosu (vcluster / Capsule / Dedicated Clusters / Kata)
upstream seçimini zaten yaptı. Bu Runtime variant *yeni aday açmaz*; karar
**tier'a göre hangi mevcut sovereign primitive** ve **runtime-mı-parity-mi**
sorusudur. Karar ekseni bu nedenle **tier × tool**:

| Kriter | **cave-kamaji (Hard)** | **cave-vcluster (PR)** | namespace+cave-policy (Soft) | ~~upstream runtime (Go)~~ |
|---|---|---|---|---|
| İzolasyon modeli | Dedicated CP Pod'ları (apiserver+cm+scheduler) tenant başına | Syncer ile virtual cluster (shared host CP) | Namespace + policy, virtual CP yok | (referans) |
| Tenant kubeconfig | ✅ gerçek dedicated kubeconfig | ✅ gerçek kubeconfig (virtual) | ⚠️ namespace-scoped | — |
| K8s upgrade bağımsızlığı | ✅ tenant kendi hızında | ⚠️ host sürümünü izler | host'a bağlı | — |
| Create süresi | Ağır (dedicated CP provision) | ✅ ~30 sn lightweight | ✅ anında (namespace) | — |
| Ephemeral lifecycle uyumu | over-provisioned | ✅ native TTL teardown | ✅ ucuz | — |
| Kaynak overhead | Yüksek (full CP Pod'ları) | ~300MB/vcluster | minimal | — |
| Operasyonel karmaşıklık | standart K8s resource, syncer yok | syncer işlet+version-lockstep | en düşük | — |
| Tek binary (Charter §5) | ✅ Rust binary içinde | ✅ Rust binary içinde | ✅ | ❌ ayrı Go servis |
| License | Apache-2.0 (clastix-labs) | Apache-2.0 (Loft Labs) | Apache-2.0 | — |
| **En iyi uyum** | **Kalıcı Hard tier production** | **Ephemeral PR ortamı** | **Soft / shared** | — |

> **Upstream-runtime sütunu çizilmiştir** — Charter §5 single-binary mandate
> gereği Go runtime'larının ayrı servis olarak çalıştırılması **kategorik
> kapsam-dışıdır**; her ikisi de Rust-native port edilir.

## Decision

Cave Runtime **tier-differentiated (katmana göre) tenancy modeli** benimser:

| Tier | Workload profili | Çözüm (Runtime crate) | Upstream | License |
|---|---|---|---|---|
| **PR ortamı** | Ephemeral, **4h TTL, tenant başına max 5**, cap 2CPU/4Gi (ADR-070) | **cave-vcluster** *(yeni scaffold)* | loft-sh/vcluster v0.34.2 | Apache-2.0 |
| **Hard tier** | Kalıcı production | **cave-kamaji** *(mevcut crate)* | clastix-labs/kamaji v1.0.0 | Apache-2.0 |
| **Soft tier** | Paylaşımlı, namespace-scoped | **namespace + cave-policy** | (Capsule-tarzı) | Apache-2.0 |

### 1. Hard tier → cave-kamaji (dedicated control-plane)
- Her Hard/Dedicated tenant için **gerçek dedicated CP**: apiserver +
  controller-manager + scheduler ayrı Pod'lar (cave-kamaji'nin `components.rs` +
  `reconcile.rs` orchestration plan'ı ile — [[cave-kamaji-lineport-2026-06-07]]).
- **Kubernetes upgrade bağımsızlığı** — tenant CP sürümü host'tan decoupled.
- **Syncer yok** — standart K8s resource'ları; işletme/debug yükü düşük.
- **Stable API** — vcluster v2 "stale flag" churn'ü yok.
- Tenant isolation cave-kamaji `isolation.rs` (label/prefix/UsedBy) +
  `manager.rs` TenantManager cross-tenant guard ile zorlanır.

### 2. Ephemeral PR ortamı → cave-vcluster (YENİ crate scaffold)
- **~30 sn lightweight create** — per-PR CI spin-up için yeterince hızlı.
- **Native ephemeral lifecycle** — **4h TTL teardown**, tenant başına **max 5**
  vcluster, **2CPU/4Gi** cap (ADR-070 resource policy).
- **Easy cost attribution** — per-vcluster footprint PR/tenant'a temiz map'lenir
  (FinOps; cave-metrics token/cost metering ile aynı pattern).
- **Scaffold mandate:** `crates/cave-vcluster` Charter "next free number" /
  crate-per-upstream hijyeniyle açılır; loft-sh/vcluster `v0.34.2` source-pinned,
  4-track ship (Portal + cavectl + API + observability), strict-TDD, honest
  fill_ratio (manifest-authored). Bu ADR scaffold'u **mandate eder**, bu commit'te
  **implement etmez**.

### 3. Soft tier → namespace + cave-policy
- Capsule-tarzı **namespace-only** izolasyon; ayrı virtual CP yok.
- Tenant guardrail'leri **cave-policy** (network/RBAC/quota policy) ile zorlanır;
  namespace-scoped RBAC + CiliumNetworkPolicy (ADR-004-RUNTIME).

### Networking (her iki virtual tier için)
- vcluster ↔ host cluster ve Kamaji tenant CP ↔ workload trafiği **cave-net /
  cave-cilium CiliumNetworkPolicy** ile default-deny + explicit-allow kurulur
  (ADR-004-RUNTIME). Tenant kubeconfig re-issue'da policy binding güncellenir.

### Migration: mevcut Hard tier vcluster tenant'ları → cave-kamaji
Platform hibrit update'inin 7-adım planı Runtime'a taşınır (PR-ortamı
vcluster'ları **etkilenmez**):
1. **Inventory** — kalıcı Hard/Dedicated tenant'ları say (ephemeral PR hariç).
2. **Provision** — her Hard tenant'a paralel cave-kamaji dedicated CP ayağa kaldır.
3. **Workload migration** — GitOps state'i (Argo/Flux) Kamaji tenant CP'ye re-point;
   manuel `kubectl` yerine GitOps re-point tercih.
4. **Data & networking cutover** — PV/state migrate; kubeconfig re-issue;
   CiliumNetworkPolicy binding güncelle.
5. **Validation** — parity doğrula (workload health, RBAC, network policy,
   observability) kaynağı decommission etmeden önce.
6. **Decommission** — soak/verification penceresinden sonra Hard tier vcluster'ı
   teardown; PR-ortamı vcluster'ları unaffected.
7. **Rollback** — kaynak vcluster soak boyunca **stopped (silinmemiş)** tutulur;
   Kamaji parity validation başarısızsa hızlı rollback.

## Rejected

- **vcluster-only (orijinal ADR-012)** — Hard tier production için yetersiz
  izolasyon: syncer shared host CP'ye projekte eder, gerçek dedicated CP vermez;
  vcluster sürümü host K8s'e kilitlenir. Hard tier'da **cave-kamaji lehine
  superseded** (Burak hibrit onayı). vcluster **ephemeral PR tier'da korunur**.
- **Kamaji-only (her şey için dedicated CP)** — ephemeral PR için over-provisioned;
  ~30 sn yerine ağır CP provision, hiç amorti edilmeyen overhead. Reddedildi.
- **Capsule (Hard tier için namespace-only)** — virtual control-plane yok; tenant
  gerçek kubeconfig / cluster-admin-benzeri deneyim alamaz. Hard tier izolasyonu
  için yetersiz; yalnızca **Soft tier**'da (cave-policy ile) kabul.
- **Dedicated clusters (PR için)** — PR ortamı için aşırı pahalı, 10-20 dk provision;
  vcluster ~30 sn'de cluster semantiğini namespace maliyetine verir. Reddedildi.
- **Kata Containers** — runtime/pod-sandbox seviyesi izolasyon; kubeconfig /
  control-plane izolasyon gereksinimini karşılamaz. Tenancy kararı için kapsam-dışı.
- **Upstream Go runtime'larını çalıştırmak** — Charter §5 single-binary ihlali;
  ayrı servis + ayrı toolchain + supply-chain yüzeyi. Kavramlar Rust-native port
  edilir.

> Platform variant tenancy'yi tek araçla (vcluster) sağlıyordu; Runtime **tier'a
> göre** böler — Hard'da gerçek dedicated CP (cave-kamaji), PR'da lightweight
> ephemeral (cave-vcluster), Soft'ta namespace (cave-policy) — ve hepsini tek
> Rust binary içinde sovereign tutar.

## Consequences

### Olumlu
- **Daha güçlü Hard tier izolasyonu** — dedicated CP Pod'ları, syncer-tabanlı
  virtual cluster'dan maddi olarak güçlü tenant ayrımı (SOC2 CC6.1 strengthened).
- **K8s upgrade bağımsızlığı** — Hard tenant CP'leri host'tan decoupled ilerler.
- **Ephemeral hız korunur** — PR ortamı ~30 sn create + 4h TTL teardown, doğru
  maliyet/fayda.
- **FinOps temiz** — per-vcluster ve per-dedicated-CP footprint tenant'a map'lenir.
- **Charter §5 korunur** — her iki manager Rust-native, tek binary; ayrı Go runtime
  yok.
- **Doğru araç-doğru iş** — over/under-provisioning yok; tier başına optimize.

### Olumsuz / maliyet
- **İki tenancy primitive** — cave-kamaji + cave-vcluster iki ayrı parity yüzeyi
  (+ Soft için cave-policy); tek araçtan daha fazla bakım.
- **cave-vcluster henüz yok** — yeni crate scaffold + full 4-track + parity
  build gerekir (bu ADR mandate eder, ayrı iş kalemi).
- **Migration eforu** — mevcut Hard tier vcluster tenant'ları için 7-adım
  parallel-run + soak + rollback penceresi.
- **Networking dikkati** — vcluster↔host ve Kamaji CP↔workload trafiği için
  özenli CiliumNetworkPolicy.

### Riskler & azaltım
- **Migration parity riski** → adım-5 validation + adım-7 stopped-not-deleted
  rollback penceresi.
- **vcluster upstream drift (v0.34.2)** → `parity.manifest.toml` source-pin +
  günlük upstream tracker (ADR-RUNTIME-UPSTREAM-WATCH-001).
- **Honest parity inflation** → fill_ratio manifest-authored, Charter 8-gate
  self-audit (gate_1 no fabrication); scope-cut'lar PARITY_REPORT'ta dokümante.

## Compliance Mapping

Platform ADR-012'den **inherit** + hibrit ile güçlendirilen mapping'ler:

- **SOC2 CC6.1** (logical access controls per tenant) — **strengthened**: Hard
  tier için Kamaji dedicated control-plane'leri, syncer-tabanlı virtual cluster'dan
  güçlü tenant logical separation sağlar; PR tier'da vcluster izolasyonu sürer.
- **ISO/IEC 27001 A.8.22** (segregation in networks / tenant environments) —
  **strengthened**: Hard tenant başına dedicated CP bileşenleri + her virtual tier
  için CiliumNetworkPolicy segregasyonu.

> Her iki kontrol Hard tier'da **cave-kamaji** altında orijinal vcluster-only
> karardan **daha güçlü** karşılanır; vcluster ephemeral PR ortamları için
> karşılamaya devam eder.

## Charter v2 8-gate linkage

Strict TDD: **cave-kamaji** `parity.manifest.toml` upstream clastix-labs/kamaji
`v1.0.0` source-pinned, Charter v2 self-audit gate'lerine bağlı (mevcut crate,
[[cave-kamaji-cont2-2026-06-07]]). **cave-vcluster** scaffold'u aynı 8-gate
kontratını miras alır: TDD strict, SPDX header, source-pin (loft-sh/vcluster
`v0.34.2`), no-stubs, no-backcompat, always-latest, 4-track ship, honest
fill_ratio. Upstream Go runtime kullanımı honest gerekçeyle (Charter §5) kapsam-dışı.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — sovereignty + single-binary charter (§5)
- [ADR-MULTI-TENANT-001](ADR-MULTI-TENANT-001.md) — Cave Runtime is multi-tenant by construction
- [ADR-004-RUNTIME](ADR-004-RUNTIME-cilium-istio.md) — CiliumNetworkPolicy (vcluster↔host / CP↔workload network segregation)
- [ADR-006-RUNTIME](ADR-006-RUNTIME-cave-auth.md) — tenant RBAC/ABAC/SPIFFE (kubeconfig access control)
- [ADR-RUNTIME-CLI-CONSOLIDATION-001](ADR-RUNTIME-CLI-CONSOLIDATION-001-cavectl-native-and-compat.md) — cavectl (CLI track of 4-track)
- **Platform ADR-012** — vcluster for Hard Tenancy + PR Environments (HYBRID update, reference variant)
- **Platform ADR-070 / ADR-084** — PR-env resource caps (2CPU/4Gi/4h TTL/max 5) + tenant model

---

*Bu ADR Platform ADR-012'nin **Runtime sovereign hibrit variant**'ıdır.
Tenancy katmana göre bölünür: Hard tier kalıcı production **cave-kamaji**
(clastix-labs/kamaji v1.0.0, dedicated control-plane), ephemeral PR ortamı
**cave-vcluster** (loft-sh/vcluster v0.34.2, ~30 sn lightweight — **yeni crate
scaffold gerekir**), Soft tier **namespace + cave-policy**. Upstream Go runtime'ları
**Charter §5 single-binary mandate** gereği çalıştırılmaz; kavramlar Rust-native
port edilir. Cave Runtime AGPL-3.0-or-later.*
