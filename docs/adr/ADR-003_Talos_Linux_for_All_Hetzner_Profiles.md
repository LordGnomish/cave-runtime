# ADR-003: Talos Linux for All Hetzner Profiles

**Status:** Accepted

**Scope:** Hetzner

**Category:** Infrastructure

**Related ADRs:** 001 (Hetzner), 004 (Cilium+Istio), 085 (Rolling Upgrades), 088 (Resurrection), 098 (Immutable Infra)

001 (Hetzner), 004 (Cilium+Istio), 085 (Rolling Upgrades), 088 (Resurrection), 098 (Immutable Infra)

Back to Index:

## Context

CAVE needs a Kubernetes distribution for all Hetzner profiles (dev, staging, prod). Requirements:

- Immutable infrastructure (no configuration drift, no SSH, no manual changes)
- API-driven management (compatible with OpenTofu Day 0 and GitOps Day 1+)
- Minimal attack surface (no shell, no package manager, no unnecessary services)
- Fast, reproducible node provisioning (destroy-and-recreate, never patch)
- Consistent across dev/staging/prod (eliminate environment parity gaps)
- CNI-agnostic (must support Cilium + Istio ambient, ADR-004)
- etcd encryption support (KMS integration for ADR-105)

---


## Candidates

### 3.1 K8s Distribution Comparison

| Criteria | Talos Linux | k3s + Ubuntu/Debian | kubeadm + Ubuntu | Flatcar Container Linux | Bottlerocket (AWS) | RKE2 |
|---|---|---|---|---|---|---|
| **Immutability** | ✅ Fully immutable. No SSH, no shell, no package manager. Root filesystem read-only. | ❌ Mutable. SSH access, apt/yum, manual changes possible. | ❌ Mutable. Full Linux. | ✅ Immutable OS, container-optimized. | ✅ Immutable, purpose-built for containers. | ❌ Mutable (standard Linux base). |
| **Management API** | `talosctl` API only. Declarative machine config (YAML). | SSH + systemd + kubeconfig. | SSH + kubeadm commands. | SSH + Ignition config + systemd. | SSM (AWS-only), API-driven. | SSH + rke2 CLI. |
| **Attack surface** | Minimal. No SSH daemon, no shell, no package manager, no unnecessary services. ~30MB base image. | Large. Full Linux userspace, SSH, systemd, packages. | Large. Full Linux. | Small. Minimal userspace but SSH available. | Small. No SSH by default, but SSM access. | Medium. Standard Linux + Rancher. |
| **Node upgrade model** | Destroy old node, provision new from versioned image. Atomic. No in-place patching. | In-place apt upgrade + k3s binary replace. Risk of drift. | In-place kubeadm upgrade. | In-place atomic OS update via update_engine. | In-place update via API. | In-place upgrade via rke2 binary. |
| **CNI support** | ✅ Any CNI. No default CNI bundled — bring your own. | ⚠️ Flannel default. Cilium requires disabling Flannel. | ✅ Any CNI (manual setup). | ✅ Any CNI. | ⚠️ AWS VPC CNI default. Cilium requires override. | ⚠️ Canal default. Cilium requires override. |
| **etcd** | ✅ Built-in etcd with KMS encryption support (OpenBao Transit, ADR-105). | ⚠️ Uses embedded SQLite (default) or external etcd. | ✅ etcd (manual setup or kubeadm manages). | ✅ Any (not K8s-specific). | ✅ Managed by EKS (not self-hosted). | ✅ Built-in etcd. |
| **Multi-profile consistency** | ✅ Identical OS on dev/staging/prod. Same talosctl, same machine config format. | ⚠️ Different Ubuntu versions possible. Package drift between environments. | ⚠️ Same risk as k3s. | ✅ Same OS across profiles (if disciplined). | ❌ AWS-only. | ⚠️ Rancher-managed, can drift. |
| **Hetzner support** | ✅ First-class Hetzner support. Official machine config examples. Community Terraform module. | ✅ Runs on any Linux VM. | ✅ Runs on any Linux VM. | ✅ Runs on any VM (Ignition provisioning). | ❌ AWS-only. | ✅ Runs on any Linux VM. |
| **Debugging** | `talosctl` commands: health, logs, dmesg, netstat, pcap, debug containers (ephemeral, 30min TTL). | Full SSH access, standard Linux tools. | Full SSH access. | SSH + standard tools (limited userspace). | SSM session, limited tools. | SSH + standard tools. |
| **License** | Mozilla Public License 2.0 (OSS) | Apache 2.0 (OSS) | Apache 2.0 (OSS) | Apache 2.0 (Flatcar) / Proprietary (some tools) | Apache 2.0 + AWS terms | Apache 2.0 (OSS) |
| **Community** | Active. Sidero Labs backed. ~5K GitHub stars. Growing CNCF presence. | Very active. Rancher/SUSE backed. ~28K stars. | Part of K8s core. | Active. Microsoft-backed (acquired Kinvolk 2021). | AWS-maintained. | Active. Rancher/SUSE backed. |
| **Production readiness** | ✅ Production-proven at scale. Used by enterprises for edge, bare-metal, cloud K8s. | ✅ Widely deployed. | ✅ Reference implementation. | ✅ Production-proven (originally CoreOS). | ✅ AWS production standard. | ✅ Production-proven. |

### 3.2 Security Posture Comparison

| Attack Vector | Talos | Standard Linux (k3s/kubeadm) | Flatcar |
|---|---|---|---|
| SSH brute force | ❌ No SSH daemon | ✅ Exposed (must harden) | ⚠️ SSH available but can disable |
| Package supply chain | ❌ No package manager | ✅ apt/yum supply chain | ❌ No package manager |
| Privilege escalation via shell | ❌ No shell access | ✅ Full shell, sudo, root | ⚠️ Limited but possible |
| Configuration drift | ❌ Immutable. Reconciled from API. | ✅ Manual edits persist across reboots | ❌ Immutable OS partition |
| Kernel exploit surface | Minimal userspace reduces exploitable services | Full userspace (systemd, cron, SSH, logging daemons) | Minimal userspace |
| Compliance posture | CIS Kubernetes Benchmark aligned by design | Requires manual CIS hardening (100+ controls) | Partially aligned |

### 3.3 Operational Model Comparison

| Operation | Talos | k3s + Ubuntu | Flatcar |
|---|---|---|---|
| **Node provisioning** | 1. OpenTofu creates VM. 2. Apply machine config via talosctl. 3. Node joins cluster. (~3 min) | 1. Create VM. 2. SSH. 3. Install k3s. 4. Configure. (~10 min + manual steps) | 1. Create VM with Ignition config. 2. Install K8s distribution. (~5 min) |
| **Node upgrade** | Destroy VM. Create new VM with new Talos version. Apply config. Old node drained+destroyed. Zero drift. | SSH in. apt update. Replace k3s binary. Restart. Hope nothing broke. | Atomic OS update via locksmith/FLUO. K8s upgrade separate. |
| **Node troubleshooting** | talosctl health → logs → dmesg → pcap → debug container (ephemeral, 30min TTL, auto-destroy) | SSH in. Run standard Linux diagnostic tools. | SSH in. Limited tools in minimal userspace. |
| **Configuration management** | Machine config YAML in Git. ArgoCD/OpenTofu manages. No config drift possible. | Ansible/Chef/Puppet to manage VM config. Drift possible between runs. | Ignition config at provision time. Post-boot changes limited but possible. |
| **Disaster recovery** | Machine config in WORM (ADR-088). Provision new nodes from config. Deterministic rebuild. | Backup entire VM state. Restore. Configuration consistency not guaranteed. | Ignition config + K8s state backup. |

---


## Decision

**Talos Linux** for all Hetzner deployment profiles (dev, staging, prod). Same OS, same management model, zero dev/prod parity gap.

---


## Rejected Options

### 4.1 k3s + Ubuntu/Debian — Rejected

**Primary:** Mutable infrastructure. SSH access enables manual changes that create drift between environments. A production node that was "fixed" via SSH becomes a snowflake — unreproducible from Git. This fundamentally violates CAVE's GitOps-everything principle (ADR-026). With 70+ components, configuration drift on the OS layer cascades into unpredictable platform behavior.

**Secondary:** Dual maintenance path. Must manage both K8s (k3s upgrades) and OS (Ubuntu security patches, kernel updates, package management). Talos eliminates OS maintenance entirely — one upgrade path, one image, one config format. SSH attack surface requires firewall rules, fail2ban, key rotation, audit logging — all eliminated by Talos's no-SSH design.

### 4.2 kubeadm + Ubuntu — Rejected

Same issues as k3s + Ubuntu, plus kubeadm adds manual etcd management complexity. kubeadm is a reference implementation, not a production-optimized distribution.

### 4.3 Flatcar Container Linux — Rejected

**Primary:** While Flatcar is immutable at the OS level, it is not purpose-built for Kubernetes. Flatcar provides a container-optimized Linux — you still need to install and manage a K8s distribution on top (kubeadm, k3s, etc.). Talos is purpose-built for K8s: etcd, kubelet, containerd, and the Talos API are the only services. No extra layers.

**Secondary:** Flatcar still provides SSH access by default. While SSH can be disabled, the security posture is weaker than Talos's architectural elimination of SSH. Microsoft acquired Kinvolk (Flatcar maintainers) in 2021 — long-term open-source commitment uncertain (compare with Talos's independent Sidero Labs backing).

### 4.4 Bottlerocket — Rejected

**Primary:** AWS-only. Designed for EKS. Not available on Hetzner. Eliminates itself from consideration for the sovereign profile.

### 4.5 RKE2 — Rejected

**Primary:** Mutable Linux base (standard RHEL/Ubuntu). Same drift risk as k3s + Ubuntu. RKE2's value proposition is FIPS compliance and Rancher integration — neither is relevant to CAVE (no US gov compliance, no Rancher).

---


## Consequences

### Positive

- Zero configuration drift across all environments (dev = staging = prod OS)
- No SSH attack surface — eliminates entire class of security vulnerabilities
- Single upgrade path: new image → new node → destroy old node
- Machine configs stored in WORM (ADR-088) enable deterministic resurrection
- CIS Kubernetes Benchmark compliance by design (not by manual hardening)
- ~30MB base image — fastest node provisioning of all alternatives
- talosctl debug containers provide deep diagnostic capability without persistent shell access

### Negative

- Debugging learning curve. Engineers accustomed to SSH must learn talosctl workflow.
- No standard Linux tools on nodes. Cannot install tcpdump, strace, etc. directly. Must use talosctl debug containers (ephemeral, 30min TTL).
- Smaller community than k3s/kubeadm. Fewer StackOverflow answers, blog posts.
- Talos upgrades require full node replacement (by design, but slower than in-place patch)
- Vendor dependency on Sidero Labs (mitigated: MPL 2.0 license, source available)

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Sidero Labs discontinues Talos | Very Low | Critical | MPL 2.0 allows community fork. Machine config format is stable. Flatcar is fallback with k3s overlay. |
| Talos upgrade breaks CAVE stack | Low | High | Staging validates full upgrade before prod (ADR-085). cave-ctl upgrade check generates dependency-aware plan. |
| Engineers resist no-SSH model | Medium | Low | Training on talosctl debugging workflow. Debug containers provide deep access when needed. Document top-20 troubleshooting scenarios in Runbook Section 4. |
| etcd KMS integration instability | Low | Medium | OpenBao Transit tested in staging. Break-glass Kit includes static decryption key (ADR-079). |

Compliance Mapping

SOC2 CC6.1 (access controls — no SSH, API-only management, zero drift). SOC2 CC6.6 (system hardening — immutable OS, no shell, no package manager). ISO A.8.8 (technical vulnerability management — immutable image-based patching). ISO A.8.9 (configuration management — machine config is declarative, GitOps-managed). NIS2 Art.21 (secure infrastructure — hardened OS baseline). CIS Kubernetes Benchmark (Talos scores high due to minimal attack surface).

