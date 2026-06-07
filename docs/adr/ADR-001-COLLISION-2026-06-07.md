<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-001 Collision Reconciliation Report

**Date:** 2026-06-07
**Author:** автopilot bulk-port (handoff — Burak Tartan sabah karar verecek)
**Status:** Open — **karar bekliyor** (3 seçenek, öneri: **Option A**)
**Scope:** Cave Runtime ADR catalogue numbering integrity
**License:** AGPL-3.0-or-later

---

## 1. Problem

Platform xlsx kataloğundan (`CAVE_ADR_Catalog_Consolidated.xlsx`) altı ADR
Runtime variant olarak port edilirken **ADR-001 numarasında bir çakışma** tespit
edildi. İki farklı ADR aynı numarayı paylaşıyor ve **içerikleri farklı**:

| | Mevcut **Runtime** ADR-001 | **Platform** ADR-001 (xlsx) |
|---|---|---|
| **Başlık** | Sovereign Bare-Metal Hosting Reference Profile | Hetzner Cloud as Sovereign Infrastructure Provider |
| **Dosya** | `docs/adr/ADR-001-sovereign-bare-metal-hosting.md` | (xlsx sheet `ADR-001`) |
| **Postür** | **Provider-agnostic** — "Linux 7.1+ bare-metal, hiçbir cloud provider'a hard-dependency yok" | **Provider-specific** — "tüm sovereign profiller için Hetzner Cloud" |
| **Charter rule** | Rule 3 (sovereignty: kritik yolda external SaaS yok) | Hetzner'i tek sovereign provider olarak pin'ler |
| **OSS uygunluğu** | ✅ OSS catalogue'a uygun (vendor-neutral) | ⚠️ Hetzner-branded — OSS posture ile gerilimde |

**Kök neden:** Runtime ADR catalogue'u (`docs/adr/README.md`) bilinçli olarak
platform/hosting/day-0 kararlarını **dışlar** ("Platform / hosting / day-0
infrastructure choices … are documented in a separate platform repository and
are **not** carried by this OSS Cave Runtime catalogue"). Platform ADR-001 tam da
bu dışlanan kategoriye girer; Runtime ADR-001 ise aynı numarayı **vendor-neutral**
bir charter kararı için zaten kullanıyor.

Ek olarak `docs/adr/README.md` **numbering policy** açıkça şunu der:

> Numbers are **stable** — no renumbering after merge. … their numbers are not
> reused.

Bu, mevcut Runtime ADR-001'i yeniden numaralamayı (Option B) policy ihlaline
sokar — rapor bunu aşağıda tartar.

---

## 2. Seçenekler

### Option A — Runtime ADR-001 kalır; Platform Hetzner ADR-100+ inherit

Mevcut Runtime ADR-001 (vendor-neutral) **olduğu gibi kalır**. Platform'un
Hetzner-specific ADR-001'i, OSS catalogue'a **provider example** olarak girmesi
gerekirse, yeni bir serbest numaraya (ADR-100+ band, örn. **ADR-100**) inherit
edilir ve başlığı netleştirilir: *"Hetzner as a Reference Sovereign Provider
(example profile)"*.

**Pros**
- ✅ **Numbering policy korunur** — hiçbir merge'lenmiş numara yeniden
  numaralanmaz; "numbers are stable" garantisi bozulmaz.
- ✅ **OSS posture korunur** — ADR-001 vendor-neutral kalır; Hetzner bir
  *örnek* provider'a iner, AWS/GCP/Azure ile eşit seviyeye.
- ✅ **En düşük blast-radius** — ADR-001'e link veren mevcut ADR'ler
  (ADR-010-RUNTIME §Context, ADR-RUNTIME-STACK-001, vb.) **hiç değişmez**.
- ✅ Platform ↔ Runtime mirror prensibi (ADR-RUNTIME-UPSTREAM-MIRROR-001) ile
  uyumlu: platform kararı kopyalanır ama renumber edilerek Runtime number-space'e
  yerleşir.

**Cons**
- ⚠️ Platform ↔ Runtime arasında **numara hizası kaybolur** — Platform ADR-001
  ≠ Runtime ADR-001. Cross-repo okuyucu için bir mapping tablosu gerekir.
- ⚠️ Hetzner içeriği OSS'e girerse "neden ADR-100, ADR-001 değil" sorusu
  dökümantasyon notu ister.

---

### Option B — Runtime ADR-001 → ADR-002 rename; Platform ADR-001 doğrudan inherit

Mevcut Runtime ADR-001 (Sovereign Bare-Metal) **ADR-002**'ye taşınır (Runtime
number-space'inde ADR-002 şu an boş). Platform Hetzner ADR-001 **doğrudan
ADR-001 numarasıyla** inherit edilir → Platform ile **tam numara hizası**.

**Pros**
- ✅ Platform ↔ Runtime **birebir numara hizası** (ADR-001 = ADR-001).
- ✅ Cross-repo mapping tablosuna gerek kalmaz.

**Cons**
- ❌ **Numbering policy İHLALİ** — README "no renumbering after merge" der;
  Runtime ADR-001 zaten merge'li ve canlı. Bu kuralı bu seçenek bozar.
- ❌ **Yüksek blast-radius** — ADR-001'e link veren her dosya güncellenir
  (ADR-010-RUNTIME, ADR-RUNTIME-STACK-001, README index, vb.). Kırık-link riski.
- ❌ **OSS posture gerilimi** — ADR-001 vendor-neutral bir charter kararından
  Hetzner-branded bir provider kararına döner; OSS catalogue'un "no hosting
  decisions" prensibiyle çelişir.
- ❌ Git history / dış linkler (varsa) `ADR-001-sovereign-bare-metal-hosting.md`
  slug'ına işaret ediyorsa kopar.

---

### Option C — İkisini tek ADR-001'de consolidate et

Tek bir ADR-001: *"Sovereign Hosting Profile (incl. Hetzner reference)"*.
Vendor-neutral bare-metal charter kararı **ana gövde** kalır; Hetzner bir
**appendix / reference profile** olarak içine gömülür (AWS/GCP/Azure ile eşit
seviyede listelenen örneklerden biri).

**Pros**
- ✅ Tek numara, tek dosya — okuyucu için en az "nereye baksam" sürtünmesi.
- ✅ Hetzner içeriği kaybolmaz; ama vendor-neutral çerçeve içinde *örnek* olarak
  konumlanır (no-Hetzner-branding posture ile uyumlu).
- ✅ Numbering policy korunur (yeni numara yok, rename yok).

**Cons**
- ⚠️ **Kapsam genişlemesi** — şu an 1.4KB'lik temiz bir charter ADR'i, Hetzner
  cost/sovereignty/rejected-providers matrisleriyle ~10KB'a şişer; ADR'in tek-karar
  netliği bulanır.
- ⚠️ "Bir ADR = bir karar" ilkesi zayıflar — bir charter prensibi + bir provider
  reference profili tek dosyada karışır.
- ⚠️ Platform ADR-001'in *Hetzner-as-sole-provider* kararı, Runtime'ın
  *provider-agnostic* kararıyla **doğrudan çelişir**; consolidate ederken bu
  çelişki çözülmeli (Hetzner "tek" değil "örnek" olarak yeniden yazılmalı) —
  yani Platform kararının özü zaten değişir.

---

## 3. Öneri — **Option A**

**Gerekçe:**

1. **Numbering policy en üstün kısıt.** README "no renumbering after merge"
   diyor; Runtime ADR-001 canlı ve merge'li. Option B bunu doğrudan ihlal eder,
   Option A ve C etmez. A, sıfır renumber ile en temiz.
2. **OSS no-Hetzner-branding posture.** Runtime catalogue bilinçle hosting
   kararlarını dışlıyor. Hetzner'i ADR-001 koltuğuna oturtmak (Option B) bu
   postürü bozar. Option A Hetzner'i bir *örnek provider*'a indirir — AWS/GCP/Azure
   ile eşit, ki bu da port edilen **ADR-003-RUNTIME** (Talos, provider-agnostic)
   ve **ADR-006-RUNTIME** (cave-auth, Hetzner sadece example) ile birebir tutarlı.
3. **Blast-radius minimum.** Option A mevcut hiçbir link veya slug'ı kırmaz;
   bu bulk-port'taki 6 yeni ADR ADR-001'e mevcut haliyle (vendor-neutral) güvenle
   link verir.
4. **Platform↔Runtime hiza kaybı yönetilebilir.** Mirror prensibi
   (ADR-RUNTIME-UPSTREAM-MIRROR-001) zaten platform→runtime numara kaymalarını
   öngörür; küçük bir mapping notu (bu rapor + README) yeterli.

**Önerilen somut adım (Burak onaylarsa):**
- Runtime ADR-001 → **değişmez**.
- Platform Hetzner ADR-001 → OSS'e girmesi istenirse **ADR-100** olarak,
  *"Hetzner as a Reference Sovereign Provider"* başlığıyla, içeriği AWS/GCP/Azure'u
  eş-seviye candidate olarak konumlandıracak şekilde yeniden çerçevelenir.
- README index'e bir satır + bu raporda bir Platform→Runtime numara mapping notu.

> **Not:** Bu bulk-port'taki **ADR-003-RUNTIME** Talos'u bilinçle
> provider-agnostic yazar (Hetzner = cave-cloud-controller-manager'ın bir
> provider'ı, AWS/GCP/Azure ile eşit). Option A bu kararla doğal uyumludur;
> Option B Talos ADR'ini de Hetzner'e geri-bağlamaya zorlardı.

---

## 4. Platform → Runtime numara mapping (bu bulk-port)

| Platform ADR | Konu | Runtime karşılık | Numara stratejisi |
|---|---|---|---|
| ADR-001 | Hetzner sovereign provider | (reframe → ADR-100, **karar bekliyor**) | Option A |
| ADR-003 | Talos Linux | `ADR-003-RUNTIME-talos-linux.md` | numara korundu |
| ADR-004 | Cilium + Istio Ambient | `ADR-004-RUNTIME-cilium-istio.md` | numara korundu |
| ADR-005 | Buildah | `ADR-005-RUNTIME-buildah.md` | numara korundu |
| ADR-006 | Keycloak identity | `ADR-006-RUNTIME-cave-auth.md` | numara korundu |
| ADR-008 | Valkey cache | `ADR-008-RUNTIME-cave-cache.md` | numara korundu |
| ADR-009 | Ollama LLM | `ADR-009-RUNTIME-cave-hermes.md` | numara korundu |

ADR-003/004/005/006/008/009 için Platform↔Runtime **numara hizası korunur**
(çakışma yok, bu numaralar Runtime catalogue'da boştu). Çakışma **yalnızca
ADR-001'de** ve yukarıdaki 3 seçenekle çözülür.

---

*Bu rapor karar değil, karar-hazırlığıdır. Burak sabah Option A / B / C arasında
seçim yapacak. Cave Runtime AGPL-3.0-or-later.*
