# ADR-RUNTIME-UPSTREAM-MIRROR-001 — Platform–Runtime Upstream Mirror

**Status:** Accepted
**Scope:** Cross-project organizing principle (Platform + Runtime ADR'leri)
**Category:** Charter / Process
**Decided:** 2026-04-25 (Burak Tartan)

## Context

CAVE projesinde iki ana proje var: **Platform** (Burak'ın iş yerinde sovereign deployment için OSS uygulamaları seçen ve operate eden katman) ve **Runtime** (Cave Runtime — bu OSS uygulamalarını line-by-line TDD ile Rust'a reimplemente eden Cloud OS).

ADR review sırasında çelişki çıktı: bir ADR (örn. "Cilium CNI seçimi") hangi tarafa yazılacak? Platform'a mı (deployment kararı) Runtime'a mı (cave-net upstream kaynağı)?

Cevap **ikisine birden**: Platform onu KULLANIR, Runtime onu REIMPLEMENTE eder. Aynı OSS seçimi iki perspektifte iki ayrı karar üretir.

## Decision

**Her sovereign Platform OSS seçimi otomatik olarak bir Runtime upstream-reimpl kararı doğurur.** Bu doğal eşleşme her ürün seçimi ADR'si için zorunlu split olarak uygulanır.

### Eşleşme örnekleri

| OSS app | Platform (deployment) ADR'si | Runtime (upstream-reimpl) ADR'si |
|---|---|---|
| Hetzner Cloud | "Sovereign infra Hetzner" | yok (Cave Runtime cloud-agnostic) |
| Talos Linux | "Sovereign K8s host OS Talos" | yok (Cave Runtime kendi OS, ADR-RUNTIME-STACK-001) |
| Cilium | "Sovereign CNI Cilium" | "cave-net upstream = Cilium-inspired eBPF" |
| Istio Ambient | "Sovereign mesh Istio Ambient" | "cave-mesh upstream = Istio Ambient-inspired" |
| Kong / Envoy | "Sovereign N-S gateway Kong" | "cave-gateway upstream = Kong/Envoy-inspired" |
| Keycloak | "Sovereign IdP Keycloak" | "cave-auth upstream = Keycloak reimpl" |
| Valkey | "Sovereign cache Valkey" | "cave-cache upstream = Valkey reimpl" |
| PostgreSQL | "Sovereign DB Postgres" | "cave-pg upstream = Postgres reimpl" |
| MongoDB | "Sovereign docdb MongoDB" | "cave-docdb upstream = MongoDB reimpl" |
| Apache Kafka | "Sovereign streaming Kafka" | "cave-streams upstream = Kafka reimpl" |
| OpenBao / Vault | "Sovereign secrets OpenBao" | "cave-vault upstream = OpenBao reimpl" |
| Harbor | "Sovereign registry Harbor" | "cave-registry upstream = Harbor reimpl" |
| Apache Iceberg | "Sovereign data layer Iceberg" | "cave-iceberg upstream = Iceberg reimpl" |
| DataFusion | "Sovereign query engine DataFusion" | "cave-datafusion upstream = DataFusion reimpl" |
| ... | ... | ... |

### Genel kural

```
Platform-ADR(X)  ⟹   Runtime-ADR(cave-X)
```

Bir OSS app `X` Platform için seçildiğinde:
1. Platform repo'ya `ADR(deployment of X)` yazılır — kullanım, profile config, alternatives rejected
2. Runtime repo'ya `ADR(cave-X upstream = X)` yazılır — reimpl scope, parity hedefi, TDD test source

İki ADR farklı perspektif ama aynı OSS seçimi referansı.

### Cave Runtime kendisi olduğu yerlerde (istisna)

Cave Runtime kendisi Cloud OS olduğu için bazı katmanlarda eşleşme **kırılır**:

- **Layer 1 (Linux kernel 7.1)**: Cave Runtime kendi kernel'i. Platform'da Talos kullanılır (deployment), Runtime'da Talos KULLANILMAZ — Runtime kendi kernel'i olur. (ADR-RUNTIME-STACK-001)
- **Layer 0 (Hardware)**: Platform deployment-specific (Hetzner VM / Azure VM / bare metal); Runtime hardware-agnostic.

Bu istisnalar açıkça belgelenir; aksi her durumda mirror geçerlidir.

### Mevcut ADR'lere uygulama

ADR review sırasında **her ürün seçimi ADR'si**:
1. **Platform tarafı kalır** — orijinal ADR Platform repo'ya taşınır (deployment kararı olarak), gerekirse SHRINK + roadmap eklenir
2. **Runtime tarafı yeni ADR** — `ADR-RUNTIME-UPSTREAM-<X>` formatında, reimpl scope ile

## Consequences

### Positive
- Net iki perspektif: deployment (Platform) vs reimpl (Runtime). Karışmaz.
- Cave Runtime'ın upstream listesi otomatik elde edilir (Platform'da ne kullanıldıysa)
- ADR review sırasında "bu ADR nereye gider" sorusu otomatik cevaplanır — split.
- OSS launch'ta Cave Runtime repo'sundaki ADR'ler self-explanatory (her biri "şu OSS upstream'i şöyle reimpl ediyoruz")
- Kullanıcılar Cave Runtime'ın hangi OSS'lerin Rust reimpl'i olduğunu doğrudan görür

### Negative
- ADR sayısı 2× artar (her ürün için 2 ADR)
- Mitigation: Platform ADR'sinde "Runtime mirror: ADR-RUNTIME-UPSTREAM-<X>" şeklinde cross-reference + Runtime ADR'sinde "Platform mirror: ADR-<original>" şeklinde aynı.
- ADR review iki repo'da paralel ilerlemeli

### Risks
| Risk | Mitigation |
|---|---|
| Platform OSS değişir, Runtime upstream güncellenmez | Runtime ADR'sinde "Platform mirror: ADR-X" referansı tutulur. Platform ADR güncellendiğinde Runtime ADR otomatik review tetiklenir. |
| Kapsam karışması (Platform ADR Runtime detaylarına girer) | ADR template ile zorunlu kısım: "Bu ADR sadece deployment yöneten / sadece reimpl yöneten" başlığı. Karışım = REWRITE. |
| Mevcut 134 ADR'yi split etmek büyük iş | OSS launch öncesi sadece KEEP olarak işaretlenenler split edilir. PLATFORM-only veya STALE olanlar split etmeden kalır. |

## Process — ADR Review uygulaması

ADR review sırasında her bir ürün seçimi ADR'si için karar tablosuna şu kolonu ekle:

| ADR | Platform decision | Runtime mirror needed? | Runtime ADR ID (if needed) |
|---|---|---|---|

Örnek:
- ADR-001 Hetzner → Platform KEEP, Runtime mirror **YOK** (Cave Runtime cloud-agnostic)
- ADR-003 Talos → Platform KEEP, Runtime mirror **YOK** (Layer 1 istisna, Cave Runtime kendi OS)
- ADR-004 Cilium+Istio → Platform KEEP, Runtime mirror **VAR** → `ADR-RUNTIME-UPSTREAM-NETWORKING-001`
- ADR-006 Keycloak → Platform KEEP, Runtime mirror **VAR** → `ADR-RUNTIME-UPSTREAM-AUTH-001`
- ADR-008 Valkey → Platform KEEP, Runtime mirror **VAR** → `ADR-RUNTIME-UPSTREAM-CACHE-001`

## ADR Quality Bar (her ürün seçimi ADR'si için zorunlu)

Burak 2026-04-25'te belirledi: ürün seçimi ADR'leri **forward-looking** olmalı.

Her ürün seçimi ADR'si zorunlu olarak:
1. **Karşılaştırma tablosu** — current capabilities matrix
2. **2-yıllık roadmap analizi** — her alternatif için: announced features, deprecation timeline, community direction, vendor strategy
3. **Decision + rejected** — alternatives rejected with WHY
4. **Forward-looking risks** — alternatif 2 yıl içinde aşar mı? Vendor lock-in?
5. **Mirror reference** — Runtime mirror ADR ID (varsa)

Bu standart şu anki çoğu ADR'de eksik. ADR review sırasında "SHRINK + add roadmap" işaretlenebilir.

## Related
- ADR-RUNTIME-STACK-001 — Cave Runtime stack (Layer 1-4) tanımı; Layer 1 Talos istisnasının kaynağı
- ADR-MULTI-TENANT-001 — Cross-cutting invariant
- 2026-04-25 ADR review session decisions tracker — bu prensibin uygulanma kayıtları

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-25 ADR review session.*
