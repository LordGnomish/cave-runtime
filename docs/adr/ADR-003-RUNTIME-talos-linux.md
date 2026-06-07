<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-003-RUNTIME — Talos Linux as the Sovereign Immutable Node OS (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — provider-agnostic)
**Category:** Infrastructure — Node OS
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-003 (Talos Linux for All Hetzner Profiles)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-003 **Talos Linux**'u "tüm Hetzner profilleri" için seçti — karar
doğru, ama çerçevesi **Hetzner'e kök salmış**. Cave Runtime
[ADR-001](ADR-001-sovereign-bare-metal-hosting.md) gereği **provider-agnostic**'tir:
hiçbir cloud provider kritik yolda hard-dependency olamaz. Bu Runtime variant,
Talos'u **sovereign immutable node OS** olarak korur, fakat onu **herhangi bir
provider** üzerinde çalışacak şekilde konumlandırır — provider seçimi
**cave-cloud-controller-manager**'ın pluggable bir provider'ıdır, OS kararının
parçası değil.

**Hetzner, AWS, GCP, Azure ve bare-metal eşit seviyededir.** Talos hepsinde aynı
makine-config formatıyla, aynı `talosctl` API'siyle, aynı immutable upgrade
modeliyle çalışır. Provider, Talos'un *altındaki* IaaS katmanıdır; Talos onu
soyutlar.

## Context

### Neden bir Runtime variant gerekli

Platform ADR-003 Talos'u Hetzner profilleriyle (dev/staging/prod) bire bir
bağlamıştı. Cave Runtime'ın port hedefi ise **Talos'un kendisi** (Sidero Labs,
MPL-2.0) değil — Talos OSS olarak zaten yeniden-implement edilmez; bunun yerine
Cave, Talos'un *üzerinde durduğu* Kubernetes node-agent yüzeyini
([cave-kubelet](../parity/parity-index.json)), CRI'yı (cave-cri) ve **provider
soyutlamasını** (cave-cloud-controller-manager) port eder.

Karar bu yüzden iki kısma ayrılır:
1. **OS seçimi (bu ADR):** immutable, SSH'sız, paket-yöneticisiz sovereign node
   OS = **Talos Linux** (base OS choice, provider-bağımsız).
2. **Provider seçimi (ayrı, pluggable):** Talos'un koştuğu IaaS —
   cave-cloud-controller-manager bir provider abstraction sunar; Hetzner *bir*
   provider, AWS/GCP/Azure de eşit-seviye provider'lar.

### Korunan değer

Platform variant'ın tüm immutability garantileri korunur: zero config drift,
no SSH attack surface, atomic image-based upgrade, CIS Kubernetes Benchmark
by-design. Tek değişen: bu garantiler artık **tek bir provider'a değil, provider
soyutlamasına** bağlanır.

## Candidates

Node OS karşılaştırması (provider-bağımsız — herhangi bir IaaS/bare-metal'de
geçerli):

| Kriter | **Talos Linux** | k3s + Ubuntu/Debian | kubeadm + Ubuntu | Flatcar | Bottlerocket | RKE2 |
|---|---|---|---|---|---|---|
| Immutability | ✅ Tam immutable, RO root, no shell, no pkg-mgr | ❌ Mutable, SSH, apt/yum | ❌ Mutable | ✅ Immutable OS | ✅ Immutable | ❌ Mutable base |
| Management | `talosctl` API + declarative machine-config | SSH + systemd | SSH + kubeadm | SSH + Ignition | SSM (**AWS-only**) | SSH + rke2 CLI |
| Attack surface | Minimal (~30MB, no SSH/shell/pkg) | Büyük | Büyük | Küçük (SSH var) | Küçük (SSM) | Orta |
| Upgrade modeli | Destroy→recreate (atomic, drift yok) | in-place apt | in-place kubeadm | atomic update_engine | in-place API | in-place |
| CNI | ✅ Any (default CNI yok → Cilium temiz) | ⚠️ Flannel default | ✅ Any (manuel) | ✅ Any | ⚠️ AWS VPC CNI | ⚠️ Canal default |
| Provider taşınabilirliği | ✅ **Any provider / bare-metal** | ✅ Any | ✅ Any | ✅ Any | ❌ **AWS-only** | ✅ Any |
| etcd | ✅ Built-in + KMS encryption | ⚠️ SQLite/external | ✅ Manuel | ✅ Any | ✅ EKS-managed | ✅ Built-in |
| License | MPL-2.0 (OSS) | Apache-2.0 | Apache-2.0 | Apache-2.0 / propr. | Apache-2.0 + AWS | Apache-2.0 |

### Provider abstraction matrisi (cave-cloud-controller-manager)

Talos seçimi provider'dan **bağımsızdır**; provider eşit-seviye pluggable'dır:

| Provider | CCM provider | Node bootstrap | Sovereignty notu |
|---|---|---|---|
| **Bare-metal** | generic / none | PXE + Talos image | Maksimum sovereign, sıfır cloud |
| **Hetzner** | hcloud CCM | Talos image + machine-config | EU-domiciled *örnek* profil |
| **AWS** | aws CCM | Talos AMI | provider-eşit; CLOUD Act operatör kararı |
| **GCP** | gce CCM | Talos image | provider-eşit |
| **Azure** | azure CCM | Talos image | provider-eşit |

> cave-cloud-controller-manager (parity: `crates/cave-cloud-controller-manager/`)
> upstream `kubernetes/cloud-provider` interface'ini port eder; her provider bir
> implementasyondur. **Hiçbir provider hard-coded değildir** — bu ADR-001'in
> provider-agnostic charter'ının doğrudan sonucu.

## Decision

**Talos Linux**, Cave Runtime'ın tüm profilleri için **sovereign immutable node
OS**'tur. Provider'dan bağımsızdır: aynı OS, aynı `talosctl` API, aynı machine-config,
aynı destroy-and-recreate upgrade modeli **her provider'da ve bare-metal'de**.

**Provider seçimi ortogonaldir** ve **cave-cloud-controller-manager**'ın provider
abstraction'ı ile yapılır. Hetzner bu soyutlamanın bir *örneğidir*, varsayılan
veya zorunlu değildir; AWS/GCP/Azure/bare-metal eşit-seviye seçeneklerdir
(bkz. [ADR-001](ADR-001-sovereign-bare-metal-hosting.md)).

## Rejected

- **k3s/kubeadm + mutable Linux** — SSH + paket-yöneticisi config drift üretir;
  GitOps-everything (ADR-RUNTIME-STACK-001) ihlali. Çift bakım yolu (OS + K8s).
- **Flatcar** — immutable ama K8s-purpose-built değil; üstüne ayrı bir distro
  kurmak gerekir, SSH default açık. Talos'un mimari SSH-elimination'ı daha güçlü.
- **Bottlerocket** — **AWS-only**; provider-agnostic charter'ı doğrudan ihlal
  eder, kendini eler.
- **RKE2** — mutable Linux base, aynı drift riski; değer önerisi (FIPS, Rancher)
  Cave için ilgisiz.

> Platform variant'ta provider olarak **Hetzner pin'lenmişti**; Runtime'da bu
> *reddedilmez* ama **demote edilir** — provider artık bir OS kararı değil,
> cave-cloud-controller-manager'ın çalışma-zamanı seçimidir.

## Consequences

### Olumlu
- **Provider lock-in yok** — Talos + cave-cloud-controller-manager ile operatör
  Hetzner/AWS/GCP/Azure/bare-metal arasında geçebilir; machine-config taşınır.
- **Zero config drift** her profilde ve her provider'da (dev = staging = prod).
- **No SSH attack surface** — koca bir güvenlik açığı sınıfı mimari olarak yok.
- **Atomic immutable upgrade** — yeni image → yeni node → eski node drain+destroy.
- **CIS Kubernetes Benchmark by-design** — manuel hardening değil.

### Olumsuz / maliyet
- **Talos upstream OSS-port edilmez** (MPL-2.0, Sidero Labs) — Cave Talos'u
  *yeniden yazmaz*, üstündeki K8s yüzeyini (cave-kubelet/cave-cri/CCM) port eder.
  Bu honest bir kapsam sınırıdır.
- **Debug öğrenme eğrisi** — SSH yok; `talosctl` debug container (ephemeral,
  30dk TTL) workflow'u öğrenilir.
- **Provider-paritesi CCM kalitesine bağlı** — her provider'ın CCM
  implementasyonu cave-cloud-controller-manager'da eşit olgunlukta tutulmalı.

### Riskler & azaltım
- **Provider CCM drift** → cave-runtime-tracker upstream `cloud-provider`
  sürümünü günlük izler ([[cave-runtime-tracker-bootstrap-2026-06-07]]).
- **Talos breaking upgrade** → staging full-upgrade doğrular; cave-cluster
  upgrade-plan dependency-aware.

## Compliance Mapping

- **SOC2 CC6.1 / CC6.6** — no SSH, API-only management, immutable OS, no shell,
  no package manager, zero drift.
- **ISO A.8.8 / A.8.9** — image-based vulnerability patching + declarative,
  GitOps-managed configuration.
- **NIS2 Art.21** — hardened OS baseline, provider-diverse supply chain (lock-in
  azaltımı).
- **CIS Kubernetes Benchmark** — Talos minimal attack surface ile yüksek skor.

## Charter v2 8-gate linkage

Strict TDD bu ADR'ye de uygulanır. İlgili crate parity'leri Charter v2 self-audit
gate'lerine bağlıdır: provider soyutlaması `cave-cloud-controller-manager`
(`parity.manifest.toml`), node-agent yüzeyi `cave-kubelet`, runtime `cave-cri`.
ADR'in kendisi `last_audit == 2026-06-07` ile bu crate'lerin manifestlerine
referansla doğrulanır.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — Sovereign bare-metal (provider-agnostic)
- [ADR-001-COLLISION-2026-06-07](ADR-001-COLLISION-2026-06-07.md) — Hetzner/bare-metal numara çakışması reconciliation
- [ADR-RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) — Cave Runtime stack (Charter v2)
- [ADR-004-RUNTIME](ADR-004-RUNTIME-cilium-istio.md) — Cilium + Istio Ambient (Talos default-CNI-less ile uyumlu)
- [ADR-149](ADR-149_KubeVirt_Sovereign_VM_Workloads.md) — cave-kubevirt sovereign VM
- **Platform ADR-003** — Talos Linux for All Hetzner Profiles (provider-pinned reference variant)

---

*Bu ADR Platform ADR-003'ün **Runtime sovereign variant**'ıdır. Talos sovereign
immutable node OS olarak korunmuş, provider seçimi cave-cloud-controller-manager
soyutlamasına demote edilmiştir (Hetzner = AWS/GCP/Azure eşit-seviye örnek).
Cave Runtime AGPL-3.0-or-later.*
