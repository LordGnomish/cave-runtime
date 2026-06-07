<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-005-RUNTIME — Buildah for Hermetic, Rootless, Multi-Arch Image Builds (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign CI — air-gap-capable)
**Category:** Infrastructure — CI/CD / Supply-chain
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-005 (Buildah for Container Image Building)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-005 **Buildah**'ı (rootless, daemonless) seçti — rootless +
hermetic (`--no-network`) + Dockerfile-uyumlu + dil-agnostik. Karar **universal**
ve Cave Runtime'ın sovereign CI postürüyle birebir uyumlu. Bu Runtime variant
Buildah'ı korur ve **dört Runtime-özgü yükseltme** ekler:

1. **Distroless / scratch base** — final image'lar minimal taban (no shell, no
   pkg-mgr) ile, Talos felsefesinin (ADR-003-RUNTIME) image-katmanına yansıması.
2. **Multi-arch: x86_64 + ARM64 + RISC-V** — Platform amd64+arm64'tü; Runtime
   **RISC-V** ekler (ADR-010-RUNTIME Phase 2/3 multi-arch matrisiyle hizalı).
3. **SLSA Level 4** — Platform SLSA L3 (ADR-101) hedefliyordu; Runtime **L4**'e
   yükseltir (hermetic + iki-kez-reproducible byte-diff + cave-ledger transparency).
4. **ML-DSA hybrid signing** — keyless OIDC yerine **cave-vault ML-DSA hybrid**
   PQC imza + cave-identity SPIFFE SVID; transparency log = cave-ledger.

## Context

### Neden bir Runtime variant gerekli
Platform variant imzayı **cosign keyless OIDC** (GitHub OIDC'ye bağlı) ile
yapıyordu — sovereignty ve air-gap ihlali. Ayrıca Platform SLSA **L3** ve
amd64+arm64 hedefliyordu. Cave Runtime'ın CI'ı (ADR-010-RUNTIME) air-gap-capable,
PQC-ready ve RISC-V-dahil multi-arch olmak zorunda; imza in-cluster sovereign
anahtarla yapılır.

### Korunan değer
Buildah'ın rootless + daemonless + native `--no-network` hermetic modeli — CI'da
sıfır privilege-escalation, ephemeral runner pod'a tam uyum, dil-agnostik tek
tool. Hepsi korunur.

## Candidates

| Kriter | **Buildah** | Kaniko | Docker (BuildKit/DinD) | Podman Build | Jib/ko |
|---|---|---|---|---|---|
| Rootless | ✅ native | ✅ userns | ⚠️ kısmen | ✅ (Buildah altta) | ✅ runtime'sız |
| Daemonless | ✅ CLI-only | ✅ pod | ❌ daemon | ✅ | ✅ |
| Hermetic build | ✅ `--no-network` (tool-level) | ⚠️ NetworkPolicy gerekir | ⚠️ daemon ağ erişimi | ✅ | ⚠️ JVM/Go ağ |
| Dockerfile | ✅ tam | ✅ tam | ✅ native | ✅ | ❌ dil-özgü |
| Multi-arch | ✅ `buildah manifest` | ⚠️ sınırlı | ✅ buildx | ✅ | ✅ |
| K8s CI runner | ✅ unprivileged pod | ✅ pod | ❌ DinD `--privileged` | ✅ | ✅ |
| Dil-agnostik | ✅ any Dockerfile | ✅ | ✅ | ✅ | ❌ Java/Go only |
| License | Apache-2.0 | Apache-2.0 | Apache-2.0 | Apache-2.0 | Apache-2.0 |

### Güvenlik postürü (CI)

| Vektör | Buildah | Kaniko | Docker DinD |
|---|---|---|---|
| Privileged escape | ❌ rootless | ❌ no daemon | ✅ `--privileged` escape |
| Docker socket attack | ❌ socket yok | ❌ | ✅ socket host'u açar |
| Build-time exfiltration | ❌ `--no-network` | ⚠️ NetworkPolicy'e bağlı | ⚠️ daemon ağı açık |
| Multi-tenant CI izolasyon | ✅ ephemeral unpriv pod | ✅ ayrı pod | ❌ shared daemon |

## Decision

**Buildah** (rootless, daemonless, `--no-network` hermetic) tüm Cave CI
pipeline'larında konteyner image build tool'u. cave-runtime CI'ında
([ADR-010-RUNTIME](ADR-010-RUNTIME-ci-pipeline.md)) Phase 3'ün build stage'idir.

### Runtime yükseltmeleri (Decision detayı)

- **Distroless/scratch base** — final image `FROM scratch` veya distroless;
  Rust static binary'leri için ideal. Shell/pkg-mgr yok → attack surface minimum.
- **Multi-arch x86_64 + ARM64 + RISC-V** — `buildah manifest` ile manifest list;
  RISC-V native runner (provider-agnostic, bare-metal/Hetzner native build).
- **SLSA L4** — hermetic build + **iki-kez reproducible byte-diff** doğrulaması +
  in-toto link metadata; provenance **cave-ledger** transparency log'a yazılır.
- **ML-DSA hybrid imza** — image `cosign sign` yerine **cave-sign** (cave-vault
  ML-DSA hybrid anahtarı) ile imzalanır; runner kimliği cave-identity SPIFFE SVID
  (keyful-but-sovereign + transparency = cave-ledger). GitHub OIDC bağımlılığı yok.

### Cave CI pipeline entegrasyonu

```
Phase 2 — Hadolint Dockerfile lint
Phase 3 / Stage 23 — Buildah build (rootless, --no-network, hermetic)
          → bağımlılıklar cave-registry pull-through cache (Harbor parity)
          → digest-pinned distroless/scratch base
Phase 3 / Stage 24 — WASM/WASI build (cave-knative serving target)
Phase 3 / Stage 26 — cave-sign: ML-DSA hybrid PQC imza (cave-vault key)
Phase 3 / Stage 27 — SLSA L4 provenance → cave-ledger transparency
Phase 3 / Stage 29 — ikinci build, byte-diff reproducibility doğrulaması
Phase 3 / Stage 30 — cave-registry push (sovereign OCI)
```

## Rejected

- **Kaniko** — native hermetic mod yok; ağ izolasyonu external NetworkPolicy'e
  bağlı (infra-level, build-level değil). SLSA L4 için Buildah'ın tool-level
  `--no-network`'ü daha güçlü. Registry-based cache yavaş.
- **Docker (BuildKit/DinD)** — DinD `--privileged` gerektirir → multi-tenant CI'da
  container escape riski; socket mount eşit tehlikeli. BuildKit rootless daemonless
  değil — ephemeral runner modeliyle çelişir.
- **Podman Build** — altta Buildah kullanır; gereksiz soyutlama katmanı. Cave
  doğrudan Buildah kullanır.
- **Jib / ko** — dil-özgü (Java / Go); Cave çok-dilli (Rust ağırlıklı). Tek tool
  operasyonel olarak basit; Hadolint + container scan akışını bypass eder.

> Platform variant imza için **cosign keyless OIDC** kullanıyordu; Runtime bunu
> **cave-sign ML-DSA hybrid + SPIFFE SVID** ile değiştirir (sovereignty + PQC).

## Consequences

### Olumlu
- Rootless + daemonless → CI'da sıfır privilege escalation.
- Native `--no-network` → SLSA L4 hermetic, tool-level garanti.
- Dil-agnostik, ephemeral runner'a tam uyum, sıfır developer öğrenme eğrisi.
- **RISC-V dahil multi-arch** → provider-diverse + edge-ready.
- **PQC-ready imza** bugünden (ML-DSA hybrid, harvest-now-decrypt-later'a karşı).
- **Air-gap-capable** — imza in-cluster sovereign anahtarla, GitHub OIDC'siz.

### Olumsuz / maliyet
- ~%30 daha yavaş cold build (rootless overhead); warm build comparable.
- RISC-V cross-compile + native build matrisi pipeline süresini uzatır.
- Multi-arch `buildah manifest` `docker buildx`'ten daha az streamline.
- ML-DSA imza anahtar yönetimi cave-vault'a operasyonel yük bindirir.

### Riskler & azaltım
- **Buildah regression** → versiyon + digest pin (ADR-010-RUNTIME gate_2);
  staging doğrular.
- **Reproducibility flake** → iki-kez build byte-diff; non-determinizm kaynağı
  (timestamp, build-id) pin'lenir.
- **cave-ledger transparency erişilemez** → build BLOCK; provenance zorunlu.

## Compliance Mapping

- **SOC2 CC8.1** — build integrity (hermetic, reproducible).
- **SLSA Level 4** — hermetic + two-party/reproducible + provenance (cave-ledger).
- **ISO A.8.25** — secure development, controlled build environment.
- **NIS2 Art.21** — supply-chain security (build isolation, PQC imza).

## Charter v2 8-gate linkage

Strict TDD: Buildah build/sign/provenance stage'leri ADR-010-RUNTIME Phase 1
Stage 3 (Charter v2 native gate) ile her PR'da yeniden doğrulanır. İlgili crate'ler
**cave-sign** (imza), **cave-sbom** (CycloneDX), **cave-registry** (OCI),
**cave-ledger** (transparency). Bu ADR'in stage-tally'si gerçek (gate_1 no
fabrication). `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-010-RUNTIME](ADR-010-RUNTIME-ci-pipeline.md) — CI pipeline (Phase 3 build/sign/provenance)
- [ADR-003-RUNTIME](ADR-003-RUNTIME-talos-linux.md) — Talos immutable felsefesi (distroless image katmanı)
- [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) — PQC anahtar hiyerarşisi
- [ADR-157](ADR-157_Sigstore_Cosign_Adoption.md) — cosign adoption (Runtime: cave-sign ML-DSA)
- **Platform ADR-005** — Buildah for Container Image Building (cosign keyless OIDC reference variant)

---

*Bu ADR Platform ADR-005'in **Runtime sovereign variant**'ıdır. Buildah hermetic
rootless build korunmuş; distroless/scratch, RISC-V multi-arch, SLSA L4 ve
ML-DSA hybrid sovereign imza eklenmiştir. Cave Runtime AGPL-3.0-or-later.*
