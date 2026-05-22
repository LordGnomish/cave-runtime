# ADR-RUNTIME-STACK-001 — Cave Runtime Stack Architecture

**Status:** Accepted
**Scope:** Cave Runtime (universal, no deployment-target dependency)
**Category:** Charter / Architecture
**Decided:** 2026-04-25 (Burak Tartan)

## Context

Cave Runtime'ın altta neye oturduğu sorusu birden fazla ADR ve memory dosyasında çelişkili izler bırakmıştı. Bu ADR Cave Runtime'ın **stack layering**'ini bağlayıcı olarak tanımlar ve diğer ADR'lerin uyumlu olmadığı yerlerde önceliklidir.

## Decision

Cave Runtime tam bir **Cloud OS**'tür — Layer 1'den Layer 4'e kadar tüm katmanları kendi yazımı/reimpl'i ile sağlar. Tek bir Rust artifact olarak deploy edilir.

### Stack layering

```
Layer 5: Tenant uygulamaları         (Cave kullanıcısı)
Layer 4: Ekosistem (Keycloak, Harbor, Vault, PG, MongoDB, Redis,
         Kafka, Iceberg, DataFusion, ...)        — unified Rust reimpl
Layer 3: K8s control plane (apiserver, etcd, scheduler, kubelet,
         cri, net, mesh, gateway)                 — unified Rust reimpl
Layer 2: Userspace (init, libc-equivalent, systemd-equivalent,
         coreutils-equivalent)                    — unified Rust reimpl
Layer 1: Linux kernel 7.1, NO backward compat    — Cave's own kernel
Layer 0: Hardware (VM / bare metal)               — DIŞ, Cave değil
```

### Net kararlar

1. **Cave Runtime = Layer 1 + 2 + 3 + 4** — tek artifact.
2. **Layer 1 kernel 7.1**: backward compat yok. Eski syscall'lar, eski POSIX zorunluluğu, glibc uyumluluğu mecburiyeti yok. PQC-ready (ML-KEM, ML-DSA, SLH-DSA) baştan tasarlanır.
3. **Layer 2 userspace**: Rust unified reimpl. systemd, glibc, bash, coreutils — yok. Yerlerinde Cave Rust impl.
4. **Layer 3 K8s**: cave-apiserver + cave-etcd + cave-kubelet + cave-scheduler + cave-cri + cave-net + cave-mesh + cave-gateway — TDD line-by-line upstream parity (golden rule).
5. **Layer 4 ekosistem**: cave-auth (Keycloak), cave-pg (PostgreSQL), cave-docdb (MongoDB), cave-cache (Valkey), cave-streams (Kafka), cave-vault (OpenBao), cave-registry (Harbor), cave-iceberg, cave-datafusion, ... — TDD parity.

### Talos / dış host OS

Cave Runtime **Talos kullanmaz**. Hiç. Cave Runtime kendisi OS olduğu için altına başka bir OS koymaya gerek yok.

Talos (veya benzeri) sadece **Platform repo deployment ADR'lerinde** opsiyondur — Cave Runtime kullanmayan workload'lar veya geçiş döneminde Burak'ın iş yerindeki uygulamaların altyapısı olarak.

### Bugünkü gerçeklik vs vizyon

- **Bugün:** Layer 3 + Layer 4 büyük kısmı yazılmış (cave-apiserver, cave-etcd, cave-keycloak, cave-pg, vs.). Layer 1 + Layer 2 henüz yok — geliştirme sırasında host OS gerektiriyor (macOS dev, Linux deploy).
- **Vizyon (uzun vade):** Layer 1 (Cave kernel 7.1) + Layer 2 (Cave userspace) yazılarak Cave Runtime tek artifact ile bare-metal'e veya VM'e doğrudan boot eder.
- **Geçiş yolu:** Layer 3+4 olgunlaştıkça Layer 2 (init, userspace) reimpl başlar, sonra Layer 1 (kernel fork/reimpl).

## Consequences

### Positive
- Net mimari: Cave Runtime'ın sınırı belirsiz değil, Layer 1-4 tek owner
- Sovereignty maximum — dış OS dependency yok
- Backward compat zorunluluğu olmadığı için modern syscall set, modern ABI, modern crypto baştan
- "Vazgeçilmez Cloud OS" vizyonu (memory'deki uzun vade) bu mimariyle uyumlu
- ADR-003 (Talos) ve diğer host-OS karışıklıkları çözüldü

### Negative
- Layer 1 (kernel 7.1) + Layer 2 (userspace) henüz mevcut değil — büyük çalışma
- Linux ekosistemiyle backward compat yok → POSIX-bağımlı uygulamalar Cave'de doğrudan çalışmayabilir, Cave-uyumlu hale gelmeli
- glibc/musl yok → upstream yazılımları Cave'de derlemek için Rust port veya Cave-libc bridge gerekebilir
- Geliştirme döneminde host OS hâlâ gerekli (macOS dev ortamı, CI Linux runner) — bu Cave Runtime mimarisi değil, sadece dev tooling

### Risks
| Risk | Mitigation |
|---|---|
| Layer 1+2 reimpl çok büyük iş, vakit yetmez | Aşamalı: önce Layer 3+4 olgunlaştır, sonra Layer 2, en son Layer 1. Kısa vadede dev/deploy için host OS kullanmaya devam et (sadece dependency olarak değil, sahte underlay olarak değil — geçici). |
| Backward compat olmaması adoption'ı zorlar | OSS launch'ta net iletişim: "Cave Runtime modern Cloud OS, POSIX-only legacy değil" |
| Kernel 7.1 fork bakım yükü | Linux mainline'a yakın kalan minimal patch set; sovereign özellikler ayrı modül olarak |

## Compliance / Cross-cutting
- **Altın kural** (TDD upstream parity): Layer 3+4 için zaten uygulanıyor, Layer 1+2 reimpl başlayınca da geçerli.
- **Multi-tenant invariant** (ADR-MULTI-TENANT-001): Layer 1+ tüm katmanlar tenant_id taşıması zorunlu.
- **No-backcompat + PQC-ready** (cave_runtime_no_backcompat_pqc.md): Bu ADR'nin özel hali — Layer 1 kernel 7.1 ve crypto baştan PQC.

## Supersedes / Conflicts
- Önceki ADR-003 (Talos for Hetzner) Cave Runtime için irrelevant — Cave Runtime Talos kullanmaz. Talos artık sadece Platform repo'da sovereign-cloud deployments ADR'si olarak kalır.
- Daha önce "Cave Runtime Layer 3+4" diye yazılmış memory/ADR ifadeleri bu ADR ile **Layer 1-4** olarak güncellenir.

## Related
- `cave_runtime_charter.md` (memory) — bu ADR onun formal hali
- `cave_runtime_no_backcompat_pqc.md` (memory) — bu ADR'nin Layer 1 kısmı
- ADR-MULTI-TENANT-001 — tenant invariant tüm katmanlarda
- ADR-GOLDEN-001..004 — TDD parity altın kuralı

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-25 ADR review session.*
