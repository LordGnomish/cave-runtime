# ADR-032: Platform Karpenter as Node Provisioner

**Status:** Accepted

**Date:** 2026-04-26

**Scope:** Universal (sovereign cloud + hyperscaler profiles)

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

The shape of the workload mix matters here. CAVE clusters host:

- **Always-on platform components** (control plane, ArgoCD, observability, gateway, secrets) — steady CPU, low burst, must never be evicted.
- **CI / build workloads** (ADR-040 ARC runners) — bursty by 10–100×, indifferent to spot interruption, idle most nights and weekends.
- **Event-driven jobs** (ADR-095 Reflex Engine, ADR-075 Knative + KEDA) — driven by external triggers; queue depth can grow faster than a node group can resize.
- **Tenant pods** — heterogeneous shapes, mixed criticality, occasional GPU asks (ADR-009).
- **One-shot platform jobs** — backups (ADR-046), schema migrations (ADR-043), security scans (ADR-018, ADR-023). Short-lived, capacity-spiky.

A static node pool sized for the worst case wastes 60–80% of compute during the long tails. A node pool sized for the median throws pending pods on the floor during bursts. The platform needs the in-between: capacity that follows demand within the minute, drops to zero on the long tail, and stays out of the tenant's way.

## Candidates

| Criteria | Karpenter | Cluster Autoscaler | Manual Node Pools | HPA / VPA only |
|---|---|---|---|---|
| Pod-driven provisioning | ✅ Direct (pending pod → node, no node-group abstraction) | ⚠️ Indirect (scales pre-defined node groups) | ❌ Human ops loop | ❌ Pod scaling only |
| Scale-to-zero | ✅ Native | ⚠️ Per-node-group, fragile | ❌ | N/A |
| Provisioning latency | ✅ 30–60s typical | ⚠️ 2–5min (group resize → cloud API → kubelet join) | ❌ Hours/days (ticket) | N/A |
| Instance flexibility | ✅ Selects from many shapes per NodePool | ❌ One shape per node group | ❌ Pre-allocated | N/A |
| Spot integration | ✅ First-class (interruption handler, on-demand fallback) | ⚠️ Supported but coarse | ⚠️ Manual | N/A |
| Consolidation / bin-packing | ✅ Continuous, online | ❌ Limited | ❌ | N/A |
| Multi-profile (hyperscaler + sovereign cloud) | ⚠️ Azure provider mature; Hetzner via Cluster API custom | ⚠️ Per-cloud autoscaler, varying maturity | ✅ Works anywhere — at a cost | ✅ |
| KEDA / event-driven fit | ✅ Reacts to pods queued by KEDA ScaledObjects | ⚠️ Slow to keep up with bursts | ❌ | ❌ |
| License | Apache 2.0 (CNCF) | Apache 2.0 (CNCF) | N/A | Apache 2.0 |
| Operational complexity | Medium (NodePool/NodeClass CRDs) | Low–Medium | Low–High depending on scale | Low |

## Decision

**Karpenter** (Apache 2.0, CNCF) for node provisioning across both provider profiles.

- **Azure profile (AKS):** [`Azure/karpenter-provider-azure`](https://github.com/Azure/karpenter-provider-azure) — Microsoft-maintained provider, AKS-native (uses VM Scale Sets / VMs API directly, integrates with AKS networking). This replaces the implicit Karpenter dependency previously named in ADR-002 and ADR-062 with a formal platform-wide decision.
- **sovereign-cloud profile (K3s + Talos):** Karpenter's NodePool / NodeClass CRDs driving a Cluster API back-end via `cluster-api-provider-hetzner` (syself / community). Where the sovereign-cloud provider lacks features (e.g. ad-hoc placement groups, Talos image rotation), a thin custom provisioner crate bridges Karpenter's `Provisioner` interface to sovereign cloud APIs and Talos machine config. The bridge is owned by the platform team, not the tenant.
- **NodePool taxonomy (per profile):** at minimum `system` (always-on platform critical), `general` (tenant default, mixed shapes, spot-preferred), `bursty` (CI / Reflex / Knative — scale-to-zero, short TTL), and `gpu` (LLM inference, ADR-009). Tenants do **not** create or edit NodePools; they request capacity via labels/taints and let Karpenter pick.
- **Consolidation:** `consolidationPolicy: WhenUnderutilized` (Azure) and equivalent (sovereign). Disruption budgets aligned with PodDisruptionBudgets per ADR-141 (Shared-Fate & Tenant Priority).
- **Spot:** spot is the default for `general` and `bursty`; on-demand fallback after configurable interruption count. `system` is on-demand only.

### NodePool reference (excerpt)

The shapes below are illustrative; concrete instance lists live in `cave-gitops-config` and are pinned by digest (ADR-108). The point is the *shape* of the contract: NodePools never name a single instance type, and tenant-facing labels are stable across providers.

```yaml
apiVersion: karpenter.sh/v1
kind: NodePool
metadata:
  name: general
spec:
  template:
    metadata:
      labels:
        cave.io/pool: general
        cave.io/lifecycle: spot-preferred
    spec:
      requirements:
        - key: kubernetes.io/arch
          operator: In
          values: [amd64, arm64]
        - key: karpenter.sh/capacity-type
          operator: In
          values: [spot, on-demand]
        - key: cave.io/instance-class
          operator: In
          values: [standard, compute, memory]
      taints: []
      expireAfter: 720h
  disruption:
    consolidationPolicy: WhenUnderutilized
    consolidateAfter: 5m
  limits:
    cpu: "2000"
    memory: 8000Gi
```

`bursty` differs in three places: `consolidateAfter: 30s`, `expireAfter: 6h`, and a stricter `limits` block. `system` drops `spot` from the capacity-type list. `gpu` adds an `nvidia.com/gpu` requirement and an `gpu-only=true:NoSchedule` taint.

## Migration Plan

Because Karpenter has been an *implicit* dependency in ADR-002 and ADR-062 since their respective acceptances, several profiles already run a Karpenter installation that pre-dates this ADR. Migration is therefore mostly normalisation, not green-field rollout.

1. **Inventory.** Record the current Karpenter version, provider version, and NodePool CRDs on every profile. Reconcile against the v1.x baseline this ADR pins.
2. **NodePool consolidation.** Existing ad-hoc NodePools (per-team, per-experiment) are merged into the four canonical pools (`system`, `general`, `bursty`, `gpu`). Tenant labels migrate via Crossplane MRAP (ADR-124) so that tenant pods continue to schedule without manual edits.
3. **RBAC tightening.** Remove any tenant-namespace ClusterRoleBindings that grant access to NodePool / NodeClass CRDs. Cluster-wide capacity decisions are platform-only; this is enforced by Gatekeeper (ADR-030).
4. **sovereign-cloud bridge cut-over.** The custom Cluster-API-bridge crate is shipped as `cave-node-provisioner-hetzner`. Existing sovereign-cloud clusters that previously ran a hand-rolled scaler shim are migrated profile-by-profile during a maintenance window, with rollback to the shim available for one soak window per ADR-132.
5. **Soak.** Each profile soaks per ADR-132 (Version Channel & Soak Policy) before being declared production-eligible. Soak success criteria: (a) zero unscheduled pending pods exceeding 5 minutes for 7 consecutive days; (b) consolidation churn within budget; (c) no spot-interruption-driven SLA breach.
6. **Documentation.** Tenant-facing docs are updated to describe the four NodePools, the labels tenants set to opt in/out, and the spot semantics. Self-service troubleshooting added to the runbook.

## Multi-Tenancy & Isolation

Karpenter sits *under* tenants in the stack: tenants describe pods, the platform decides nodes. The tenancy model that surrounds it has three rules:

1. **Tenants do not author NodePools.** NodePool / NodeClass CRDs live in the platform's GitOps repo (`cave-gitops-config`, ADR-026) and are reconciled by ArgoCD. Tenant access to those CRDs is denied by Gatekeeper (ADR-030) and verified by Kubescape (ADR-058) on every cluster.
2. **Tenants steer via labels and tolerations.** A pod expresses "I want spot" or "I need GPU" through node selectors and tolerations defined in the tenant-facing catalog. The catalog is small (≈10 well-known labels) so tenants do not learn cloud-specific instance jargon.
3. **Capacity envelopes are enforced at NodePool limits**, not per-tenant. Per-tenant capacity is enforced by ResourceQuota + LimitRange (ADR-087); Karpenter sees those quotas as the upper bound on schedulable demand and never tries to provision beyond them.

The combination guarantees that a runaway tenant cannot exhaust cluster capacity by spamming pending pods (ResourceQuota stops them) and cannot widen the cluster's compute envelope on its own (NodePool RBAC stops them).

## Operational Playbook

Day-2 operations follow a small set of named procedures so that on-call engineers do not improvise during incidents.

- **PB-KARP-01 — Pending pods despite spare cluster headroom.** Check Karpenter logs for provider quota errors first; then verify NodePool requirement intersection (a pod with conflicting requirements will never schedule, regardless of capacity).
- **PB-KARP-02 — Consolidation oscillation.** Inspect `karpenter_consolidation_actions_total`. If churn exceeds budget, raise `consolidateAfter` and PDB minimums for the affected workload class.
- **PB-KARP-03 — Spot interruption storm.** When `karpenter_interruption_actions_total` spikes for a region, automatically widen the on-demand fallback fraction for `general` and `bursty` for the next provisioning window.
- **PB-KARP-04 — sovereign-cloud provider lag.** If the sovereign-cloud bridge falls behind Karpenter's intent for more than 10 minutes, page the platform team and fall back to the manual node-pool break-glass — without disabling Karpenter (which would compound the lag).

These are referenced from the Reflex Engine playbook catalog (ADR-095) and surfaced in the Backstage scorecard.

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
- **Single mental model** for hyperscaler and sovereign cloud — operators learn Karpenter once, apply it twice.

### Negative
- **sovereign-cloud provider maturity gap.** `cluster-api-provider-hetzner` is community-maintained, not vendor-backed. The custom Cluster-API-bridge crate becomes a platform-team responsibility with its own backlog and on-call surface.
- **KEDA × Karpenter interplay.** A ScaledObject (ADR-095) creating pods that trigger Karpenter creating nodes that take 60s to join can race with KEDA's cool-down. Misconfiguration produces oscillation. We accept this risk and document tuning playbooks.
- **NodePool RBAC surface.** Tenants must not be able to create or edit NodePools (cluster-wide capacity decisions). This is enforced via OPA Gatekeeper (ADR-030) and platform RBAC (ADR-078); a regression here is a blast-radius bug.
- **Spot interruption noise.** `system` workloads are isolated to on-demand, but tenant `general` workloads will see occasional interruptions. PodDisruptionBudgets and tenant priority (ADR-141) absorb this; tenants who cannot tolerate it must label out of spot explicitly.
- **Consolidation churn.** Aggressive consolidation can move pods more often than tenants expect. We tune `consolidateAfter` conservatively at first and revisit.

### Risks
- **Provider drift between hyperscaler and sovereign cloud.** Feature parity between `karpenter-provider-azure` and our sovereign-cloud bridge is not guaranteed. We cap divergence with the Provider Parity Contract (ADR-135): both providers must implement the same NodePool feature surface; gaps are tracked as parity defects.
- **CRD upgrade churn.** Karpenter has rev'd its CRD schema between minor versions; in-place upgrades require care. Covered by ADR-132 (Version Channel & Soak Policy).

## Compliance Mapping

- **SOC2 CC7.2** — System operations / capacity planning. Karpenter provides automated capacity provisioning with audit trail.
- **ISO 27001 A.12.3** — Capacity management. Pod-driven provisioning + consolidation satisfies the documented capacity-management control.
- **NIS2 Art. 21** — Resilience. Fast burst response and spot interruption handling are part of the documented resilience posture.

## Implementation Reference

**Implementation Status:** Accepted; rollout per profile.

- **Azure profile:** Karpenter v1.x (current stable) via `karpenter-provider-azure`. Helm chart pinned by digest (ADR-108). NodePools defined in `cave-gitops-config` (ADR-026), reconciled by ArgoCD.
- **sovereign-cloud profile:** Karpenter v1.x with Cluster-API bridge (`cluster-api-provider-hetzner` + custom provisioner crate). Talos machine images pinned per ADR-098.
- **NodePools shipped at v0.1:** `system`, `general`, `bursty`. `gpu` follows once ADR-009 GPU runtime story stabilises.
- **Observability:** Karpenter metrics scraped into Prometheus (ADR-029); dashboards for provisioning latency, consolidation churn, and spot interruption rate.
- **Disruption controls:** PodDisruptionBudgets enforced via OPA Gatekeeper (ADR-030); tenant priority per ADR-141.

## Cost Economics

Karpenter's value is proportional to workload variance. The economics model behind this decision rests on three numbers, each of which is independently observable in production:

- **Idle ratio.** With static node pools, off-peak utilisation on `bursty` workloads (ARC, Reflex, scheduled jobs) is typically 5–15% of provisioned capacity. Scale-to-zero takes that floor to 0, so all `bursty` cost becomes proportional to actual work.
- **Spot premium.** On both hyperscaler and sovereign cloud the spot/on-demand price gap is large enough that even a 20–30% interruption rate yields material savings on `general` workloads. Workloads that cannot tolerate interruption opt out via a single label.
- **Bin-packing tax.** Static node pools trap fragments — half-full nodes that no single workload fills. Continuous consolidation recovers that tax. The exact recovery is workload-dependent, but is reliably non-zero on shared multi-tenant clusters.

These numbers are tracked under FinOps attribution (ADR-096) and reported back to tenants on the cost dashboard.

## Notes

- **Runtime mirror.** The Cave Runtime side will own a separate decision (`ADR-RUNTIME-NODE-PROVISIONING-001`) that wraps Karpenter behind a `cave-node-provisioner` crate so that Reflex Engine (ADR-095) and Knative + KEDA (ADR-075) hit a stable internal interface independent of provider differences. That ADR has not yet been written; this Platform ADR is the upstream.
- **Why a standalone ADR now.** Karpenter has been a dependency in ADR-002 (Azure Enterprise Infrastructure) and ADR-062 (Azure Day-0 OpenTofu) since their respective acceptances. It has never had its own decision record, which means the multi-profile story (especially Hetzner) was carried as tribal knowledge. This ADR closes that gap and gives both profiles a single referent.
- **Not in scope.** Tenant-level autoscaling (HPA, VPA, KEDA ScaledObject definition) is out of scope here; those remain pod-level concerns owned by the tenant. Karpenter is the layer beneath them that produces capacity.
