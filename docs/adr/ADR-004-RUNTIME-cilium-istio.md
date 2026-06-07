<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-004-RUNTIME — Cilium CNI + Istio Ambient Mesh, PQC-ready, Single-Binary (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — universal, provider-agnostic)
**Category:** Infrastructure — Networking
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-004 (Cilium CNI + Istio Ambient Mesh, absorbs ADR-068)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-004 ağ yapısını **Cilium (CNI) + Istio Ambient (mesh)** olarak
seçti — sidecar'sız mTLS + eBPF gözlemlenebilirlik. Karar doğru ve **universal**.
Cave Runtime bu kararı **kendi reimpl'leriyle** materialize eder:

- **cave-net** — Cilium **datapath** parity (eBPF LB/DSR/conntrack/source-range
  sim, Hubble flow).
- **cave-cilium** — Cilium **control-plane** parity (CNP CRD, identity, IPAM,
  policy reconciler).
- **cave-mesh** — Istio **Ambient** parity (ztunnel L4 mTLS + opt-in Waypoint L7).

İki Runtime-özgü yükseltme eklenir:
1. **PQC-ready WireGuard variant** — node-to-node şifreleme **ML-KEM/ML-DSA
   hybrid** ([[cave-cilium-fresh-port-2026-06-07]] `encryption.rs`; seal anahtarı
   cave-vault PQC, [[cave-vault-pqc-seal-2026-06-07]]).
2. **Single-binary integration (Charter §5)** — CNI + mesh ayrı daemon'lar
   değil; cave-runtime tek binary'sinde mount'lu modüller. Sidecar yok, ayrı
   control-plane process'i yok.

## Context

### Neden bir Runtime variant gerekli

Platform variant Cilium + Istio'yu **upstream Helm chart'ları** olarak deploy
ediyordu. Cave Runtime sovereign Cloud OS'tür: ağ yapısı **in-binary** olmalı,
external control-plane'e reach-out etmemeli, air-gap'te tam çalışmalı. Ayrıca
Cave'in Charter v2 (ADR-RUNTIME-STACK-001) §5 **single-binary mandate**'i, CNI ve
mesh'in ayrı uzun-ömürlü daemon'lar olarak değil, cave-runtime'ın mount'lu router'ları
olarak yaşamasını gerektirir.

### Korunan değer
Sidecar-less ambient mimarisinin RAM tasarrufu (~7.5GB / 100-pod), Hubble L3/L4/L7
flow görünürlüğü, default-deny network policy, FQDN-based egress — hepsi korunur.

## Candidates

### CNI

| Kriter | **Cilium** (→ cave-net + cave-cilium) | Calico | Flannel | AWS/Azure CNI |
|---|---|---|---|---|
| Teknoloji | eBPF (kernel-level) | iptables / eBPF (yeni) | VXLAN overlay | provider-native |
| Network policy | ✅ CNP L3/L4/L7 + FQDN/DNS-aware | ✅ Calico + K8s NP | ❌ | ⚠️ K8s NP only |
| eBPF | ✅ Native (core mimari) | ⚠️ opt-in | ❌ | ❌ |
| Hubble flow viz | ✅ Built-in L3/L4/L7 | ❌ (Enterprise) | ❌ | ❌ |
| Egress gateway | ✅ Per-tenant + eBPF byte counter | ⚠️ Enterprise | ❌ | ❌ provider NAT |
| Encryption | ✅ WireGuard / IPsec (+ **PQC hybrid**, Runtime) | ✅ WireGuard | ❌ | ❌ |
| Talos uyumu | ✅ default-CNI-less Talos'a temiz plug | ✅ | ✅ | ❌ provider-locked |
| License | Apache-2.0 (CNCF Graduated) | Apache-2.0 / propr. | Apache-2.0 | provider terms |

### Service Mesh

| Kriter | **Istio Ambient** (→ cave-mesh) | Istio Sidecar | Linkerd | Cilium Mesh | No Mesh |
|---|---|---|---|---|---|
| Mimari | ztunnel (node L4) + Waypoint (opt-in L7) | Envoy/pod | Rust proxy/pod | eBPF L4 + Envoy L7 | yok |
| Overhead | Düşük (~50MB/node, 0/pod) | Yüksek (~80MB/pod) | Orta (~30MB/pod) | Düşük | Sıfır |
| mTLS | ✅ ztunnel L4 otomatik | ✅ sidecar | ✅ proxy | ✅ eBPF | ❌ |
| L7 policy | ✅ Waypoint opt-in | ✅ her zaman | ✅ sınırlı | ⚠️ az olgun | ❌ |
| Canary (Rollouts) | ✅ native Istio traffic mgmt | ✅ | ⚠️ SMI adapter | ⚠️ Gateway API | ❌ |
| CNCF | ✅ Graduated | ✅ Graduated | ✅ Graduated | part of Cilium | — |

## Decision

**Cilium** CNI olarak (L3/L4 fabric + network policy + Hubble eBPF gözlemlenebilirlik
+ egress governance), Cave'de **cave-net** (datapath) + **cave-cilium**
(control-plane) reimpl'leriyle.

**Istio Ambient** (sidecar-less) service mesh olarak (ztunnel L4 mTLS + opt-in
Waypoint L7), Cave'de **cave-mesh** reimpl'iyle.

### Sorumluluk sınırı (no overlap)

| Trafik | Handler (Cave crate) | Kapsam |
|---|---|---|
| **North-South** (tenant API) | **cave-gateway** (Kong+Gravitee consolidation, ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001) | rate-limit, JWT/OAuth2, OpenAPI, versioning |
| **East-West** (service-to-service) | **cave-mesh** (Istio Ambient) | mTLS (ztunnel), canary traffic shift, L7 (Waypoint) |
| **Network policy** (L3/L4) | **cave-cilium** / **cave-net** | default-deny, cross-tenant/env block, egress quota |
| **Network observability** | **cave-net** (Hubble) | flow viz, Prometheus metrics, forensics |
| **Egress governance** | **cave-cilium** egress gateway | per-tenant quota, eBPF byte counter |

> Platform ADR-004 N-S handler olarak **Kong**'u listeliyordu; Runtime'da bu
> **cave-gateway**'e konsolide edilir (Kong + Gravitee tek sovereign surface).

### Runtime yükseltmeleri

1. **PQC-ready WireGuard.** Node-to-node WireGuard tüneli klasik X25519 yerine
   **ML-KEM-768 hybrid** anahtar-anlaşmasıyla ve **ML-DSA hybrid** kimlik
   doğrulamasıyla kurulabilir (cave-cilium `encryption.rs`; lattice primitive'i
   vetted crate'e *delegated*, fake değil). Seal anahtarı cave-vault PQC
   ([[cave-vault-pqc-seal-2026-06-07]]). 2026-ötesi harvest-now-decrypt-later
   tehdidine karşı bugünden hazır.
2. **Single-binary (Charter §5).** cave-net/cave-cilium/cave-mesh ayrı daemon
   değil; cave-runtime binary'sine mount'lu router/modüller. Sidecar sıfır,
   ayrı control-plane process'i yok → air-gap-capable, tek-surface ops.

## Rejected

- **Calico (CNI)** — native eBPF flow gözlemlenebilirliği yok (Hubble eşdeğeri
  Enterprise/paid); FQDN egress kuralları OSS'te eksik. Cave forensics (Hubble
  Prometheus) buna bağımlı.
- **Istio Sidecar** — Envoy/pod overhead'i (~80MB) küçük profillerde RAM'in
  ~%50'sini yer; ambient ztunnel node-level mTLS ile bunu sıfırlar.
- **Linkerd** — Argo Rollouts native entegrasyonu yok (SMI adapter, az olgun);
  Waypoint-eşdeğeri opt-in L7 yok.
- **Cilium Service Mesh (Istio'suz)** — L7 traffic mgmt az olgun; tüm ağ
  yığınında tek-vendor riski. CNI (Cilium) ↔ mesh (Istio) ayrımı defense-in-depth.
- **No Mesh** — otomatik mTLS yok; zero-trust E-W 70+ bileşende uygulanamaz.
  SOC2 CC6.7 / ISO A.8.24 / GDPR Art.32 boşluğu.

## Consequences

### Olumlu
- eBPF-native ağ, near-kernel performans, iptables bypass.
- Hubble L3/L4/L7 flow görünürlüğü ek tooling'siz.
- Ambient ile sidecar overhead'i yok (~7.5GB/100-pod tasarruf).
- **PQC-ready bugünden** — WireGuard ML-KEM/ML-DSA hybrid.
- **Single-binary** — air-gap-capable, sidecar/ayrı-daemon yok (Charter §5).
- Temiz L7 sınırı: cave-gateway (N-S), cave-mesh (E-W), cave-cilium/net (L3/L4).

### Olumsuz / maliyet
- İki L7 bileşen (cave-mesh + cave-gateway) bilişsel yük; sınır dokümante edilmeli.
- L4'te Cilium ↔ Istio örtüşmesi — Cilium L3/L4'te authoritative, ztunnel sadece
  mTLS; çelişki çözümü runbook'ta.
- Ambient multi-cluster alpha — multi-region için ayrı değerlendirme (cave-mesh
  ClusterMesh backlog'da, bkz. [[cave-net-cont3-2026-05-31]]).
- PQC hybrid handshake klasik X25519'dan biraz daha pahalı (lattice KEM maliyeti).

### Riskler & azaltım
- **cave-cilium agent crash** → node cordon, pod reschedule, Talos node rebuild
  (ADR-003-RUNTIME).
- **Istio ambient breaking change** → staging doğrular; upgrade order Cilium→Istio.
- **PQC primitive drift** → ML-KEM/ML-DSA vetted crate, cave-runtime-tracker
  upstream izler.

## Compliance Mapping

- **SOC2 CC6.1** — network segmentation (cave-cilium default-deny + cave-mesh mTLS).
- **SOC2 CC6.6 / ISO A.8.24** — encryption in transit (ambient mTLS + **PQC
  WireGuard hybrid**).
- **ISO A.8.22** — segregation in networks (eBPF kernel-level enforcement).
- **NIS2 Art.21** — zero-trust ağ mimarisi.
- **GDPR Art.32** — şifreli service-to-service iletişim.

## Charter v2 8-gate linkage

Strict TDD: cave-net (`parity.manifest.toml`, honest 0.9851), cave-cilium
(honest 0.7667, 4 unmapped honest), cave-mesh manifestleri Charter v2 self-audit
gate'lerine bağlı. PQC encryption modülü lattice'i *delegate* eder — gate_3
(no stub macros) ve gate_1 (no fabrication) buna uyar. `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-003-RUNTIME](ADR-003-RUNTIME-talos-linux.md) — Talos default-CNI-less node OS
- [ADR-RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) — Charter v2 single-binary
- [ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001](ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001-kong-gravitee-into-cave-gateway.md) — cave-gateway (N-S)
- [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) — PQC cert hierarchy
- [ADR-010-RUNTIME](ADR-010-RUNTIME-ci-pipeline.md) — CI pipeline (PQC sign, multi-arch)
- **Platform ADR-004** — Cilium CNI + Istio Ambient Mesh (Helm reference variant, absorbs ADR-068)

---

*Bu ADR Platform ADR-004'ün **Runtime sovereign variant**'ıdır. Cilium → cave-net
+ cave-cilium, Istio Ambient → cave-mesh reimpl'leriyle materialize edilmiş;
PQC-ready WireGuard ve single-binary integration eklenmiştir. Cave Runtime
AGPL-3.0-or-later.*
