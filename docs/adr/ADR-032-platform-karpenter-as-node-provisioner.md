# ADR-032: Platform Karpenter as Node Provisioner

**Status:** Accepted

**Date:** 2026-04-26

**Scope:** Universal (Hetzner + Azure profiles)

**Category:** Platform / Cluster Lifecycle

**Related ADRs:** 002 (Azure Enterprise Infra), 062 (Azure Day-0 OpenTofu), 040 (ARC Self-Hosted Runners), 095 (Reflex Engine — KEDA + Argo Workflows), 075 (Knative + KEDA Phase 4), 067 (Crossplane v2), 098 (Talos Immutable Infrastructure), 003 (Talos for Hetzner)

## Context

CAVE clusters run a heterogeneous mix of always-on platform components (73 services), bursty CI workloads (ARC runners, ADR-040), event-driven jobs (Reflex Engine via KEDA, ADR-095), and tenant pods with widely varying CPU/memory shapes. A single static node pool over-provisions during quiet windows and under-provisions during burst windows. The platform needs:

- **Pod-driven provisioning:** When a pending pod cannot be scheduled, a node of the right shape is brought up automatically — without human intervention or per-team node-pool tuning.
- **Scale-to-zero:** Idle node groups (e.g. CI runners overnight) collapse to zero, eliminating idle compute spend.
- **Fast burst:** Pending pod → ready node in 30–60 seconds so CI queues and event-driven workflows do not block.
- **Instance flexibility:** A workload that fits on multiple instance types (different CPU/memory ratios, different generations, spot vs on-demand) should be scheduled on whatever is cheapest right now.
- **Bin-packing & consolidation:** Continuously consolidate underutilised nodes — replace two half-full nodes with one full node, drain and remove the empty one.
- **Spot awareness:** Tolerate spot interruption with graceful eviction and on-demand fallback.

This must hold across both supported provider profiles: Azure (AKS) and Hetzner (K3s + Talos, ADR-003 / ADR-098). Karpenter has previously been referenced as a *dependency* in ADR-002 and ADR-062, but never given its own decision record. This ADR formalises that choice and unifies the multi-profile story.

## Candidates

| Criteria | Karpenter | Cluster Autoscaler | Manual Node Pools | HPA / VPA only |
|---|---|---|---|---|
| Pod-driven provisioning | ✅ Direct (pending pod → node, no node-group abstraction) | ⚠️ Indirect (scales pre-defined node groups) | ❌ Human ops loop | ❌ Pod scaling only |
| Scale-to-zero | ✅ Native | ⚠️ Per-node-group, fragile | ❌ | N/A |
| Provisioning latency | ✅ 30–60s typical | ⚠️ 2–5min (group resize → cloud API → kubelet join) | ❌ Hours/days (ticket) | N/A |
| Instance flexibility | ✅ Selects from many shapes per NodePool | ❌ One shape per node group | ❌ Pre-allocated | N/A |
| Spot integration | ✅ First-class (interruption handler, on-demand fallback) | ⚠️ Supported but coarse | ⚠️ Manual | N/A |
| Consolidation / bin-packing | ✅ Continuous, online | ❌ Limited | ❌ | N/A |
| Multi-profile (Azure + Hetzner) | ⚠️ Azure provider mature; Hetzner via Cluster API custom | ⚠️ Per-cloud autoscaler, varying maturity | ✅ Works anywhere — at a cost | ✅ |
| KEDA / event-driven fit | ✅ Reacts to pods queued by KEDA ScaledObjects | ⚠️ Slow to keep up with bursts | ❌ | ❌ |
| License | Apache 2.0 (CNCF) | Apache 2.0 (CNCF) | N/A | Apache 2.0 |
| Operational complexity | Medium (NodePool/NodeClass CRDs) | Low–Medium | Low–High depending on scale | Low |

## Decision

**Karpenter** (Apache 2.0, CNCF) for node provisioning across both provider profiles.

- **Azure profile (AKS):** [`Azure/karpenter-provider-azure`](https://github.com/Azure/karpenter-provider-azure) — Microsoft-maintained provider, AKS-native (uses VM Scale Sets / VMs API directly, integrates with AKS networking). This replaces the implicit Karpenter dependency previously named in ADR-002 and ADR-062 with a formal platform-wide decision.
- **Hetzner profile (K3s + Talos):** Karpenter's NodePool / NodeClass CRDs driving a Cluster API back-end via `cluster-api-provider-hetzner` (syself / community). Where the Hetzner provider lacks features (e.g. ad-hoc placement groups, Talos image rotation), a thin custom provisioner crate bridges Karpenter's `Provisioner` interface to Hetzner Cloud APIs and Talos machine config. The bridge is owned by the platform team, not the tenant.
- **NodePool taxonomy (per profile):** at minimum `system` (always-on platform critical), `general` (tenant default, mixed shapes, spot-preferred), `bursty` (CI / Reflex / Knative — scale-to-zero, short TTL), and `gpu` (LLM inference, ADR-009). Tenants do **not** create or edit NodePools; they request capacity via labels/taints and let Karpenter pick.
- **Consolidation:** `consolidationPolicy: WhenUnderutilized` (Azure) and equivalent (Hetzner). Disruption budgets aligned with PodDisruptionBudgets per ADR-141 (Shared-Fate & Tenant Priority).
- **Spot:** spot is the default for `general` and `bursty`; on-demand fallback after configurable interruption count. `system` is on-demand only.

## Rejected Options

### Cluster Autoscaler — Rejected
Pre-defined node groups bring an extra abstraction the platform must size and maintain. Provisioning latency (2–5 min) misses the burst window for ARC and Reflex. Spot handling is per-node-group and coarse. Instance-flexibility story is essentially "make many node groups," which is exactly the operational toxin we are trying to remove.

### Manual Node Pools — Rejected
Not a sovereignty issue (Hetzner + Talos can be operated by hand), but operationally toxic at the cluster fleet size CAVE projects. Idle-cost burn is high, burst response is human-paced, and per-tenant capacity disputes turn into ticket queues. Manual is acceptable only as a *break-glass* fallback, not as the steady state.

### HPA / VPA only — Rejected
Pod-level scaling does not create capacity. Once node CPU is exhausted the HPA simply produces pending pods. Karpenter is the missing layer between "more pods please" and "more nodes please."

### Cloud-native autoscaler-as-a-service (e.g. AKS Cluster Autoscaler add-on) — Rejected for Azure profile
AKS's bundled autoscaler is Cluster Autoscaler under the hood, with the same limitations above, plus add-on lifecycle coupling that conflicts with our self-hosted control-plane components story (ADR-063 — ArgoCD self-hosted, not AKS GitOps add-on).

## Consequences

### Positive
- **Scale-to-zero** for `bursty` NodePools — material idle-spend reduction on CI and event-driven workloads.
- **Burst latency** drops from minutes to ~30–60s; ARC queue depth and Reflex playbook tail latency both improve.
- **Spot integration** is first-class; expected compute spend on `general` workloads drops materially with controlled risk.
- **Bin-packing / consolidation** runs continuously; long-running platform components no longer strand fragments of nodes.
- **Instance flexibility** lets capacity track price and availability instead of a fixed shape choice from cluster-creation day.
- **Single mental model** for Azure and Hetzner — operators learn Karpenter once, apply it twice.

### Negative
- **Hetzner provider maturity gap.** `cluster-api-provider-hetzner` is community-maintained, not vendor-backed. The custom Cluster-API-bridge crate becomes a platform-team responsibility with its own backlog and on-call surface.
- **KEDA × Karpenter interplay.** A ScaledObject (ADR-095) creating pods that trigger Karpenter creating nodes that take 60s to join can race with KEDA's cool-down. Misconfiguration produces oscillation. We accept this risk and document tuning playbooks.
- **NodePool RBAC surface.** Tenants must not be able to create or edit NodePools (cluster-wide capacity decisions). This is enforced via OPA Gatekeeper (ADR-030) and platform RBAC (ADR-078); a regression here is a blast-radius bug.
- **Spot interruption noise.** `system` workloads are isolated to on-demand, but tenant `general` workloads will see occasional interruptions. PodDisruptionBudgets and tenant priority (ADR-141) absorb this; tenants who cannot tolerate it must label out of spot explicitly.
- **Consolidation churn.** Aggressive consolidation can move pods more often than tenants expect. We tune `consolidateAfter` conservatively at first and revisit.

### Risks
- **Provider drift between Azure and Hetzner.** Feature parity between `karpenter-provider-azure` and our Hetzner bridge is not guaranteed. We cap divergence with the Provider Parity Contract (ADR-135): both providers must implement the same NodePool feature surface; gaps are tracked as parity defects.
- **CRD upgrade churn.** Karpenter has rev'd its CRD schema between minor versions; in-place upgrades require care. Covered by ADR-132 (Version Channel & Soak Policy).

## Compliance Mapping

- **SOC2 CC7.2** — System operations / capacity planning. Karpenter provides automated capacity provisioning with audit trail.
- **ISO 27001 A.12.3** — Capacity management. Pod-driven provisioning + consolidation satisfies the documented capacity-management control.
- **NIS2 Art. 21** — Resilience. Fast burst response and spot interruption handling are part of the documented resilience posture.

## Implementation Reference

**Implementation Status:** Accepted; rollout per profile.

- **Azure profile:** Karpenter v1.x (current stable) via `karpenter-provider-azure`. Helm chart pinned by digest (ADR-108). NodePools defined in `cave-gitops-config` (ADR-026), reconciled by ArgoCD.
- **Hetzner profile:** Karpenter v1.x with Cluster-API bridge (`cluster-api-provider-hetzner` + custom provisioner crate). Talos machine images pinned per ADR-098.
- **NodePools shipped at v0.1:** `system`, `general`, `bursty`. `gpu` follows once ADR-009 GPU runtime story stabilises.
- **Observability:** Karpenter metrics scraped into Prometheus (ADR-029); dashboards for provisioning latency, consolidation churn, and spot interruption rate.
- **Disruption controls:** PodDisruptionBudgets enforced via OPA Gatekeeper (ADR-030); tenant priority per ADR-141.

## Notes

- **Runtime mirror.** The Cave Runtime side will own a separate decision (`ADR-RUNTIME-NODE-PROVISIONING-001`) that wraps Karpenter behind a `cave-node-provisioner` crate so that Reflex Engine (ADR-095) and Knative + KEDA (ADR-075) hit a stable internal interface independent of provider differences. That ADR has not yet been written; this Platform ADR is the upstream.
- **Why a standalone ADR now.** Karpenter has been a dependency in ADR-002 (Azure Enterprise Infrastructure) and ADR-062 (Azure Day-0 OpenTofu) since their respective acceptances. It has never had its own decision record, which means the multi-profile story (especially Hetzner) was carried as tribal knowledge. This ADR closes that gap and gives both profiles a single referent.
- **Not in scope.** Tenant-level autoscaling (HPA, VPA, KEDA ScaledObject definition) is out of scope here; those remain pod-level concerns owned by the tenant. Karpenter is the layer beneath them that produces capacity.
