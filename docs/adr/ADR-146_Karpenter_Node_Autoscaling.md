# ADR-146 — Karpenter for Node Autoscaling

| Status | Accepted (scaffold only — implementation pending) |
| ------ | ------------------------------------------------- |
| Date   | 2026-05-06                                        |
| Track  | Backend 1/4, Portal 0/4, cavectl 0/4, Observ. 0/4 |

## Context

Cave Runtime needs a node-autoscaling layer that:

1. Reacts to pending Pods in seconds, not minutes — Cluster Autoscaler's
   `nodegroup` model is too coarse for the spiky workload mix we run
   (LLM inference, batch ETL, GPU bursts).
2. Schedules nodes against arbitrary pod requirements (instance-type,
   zone, GPU class, custom labels) without forcing operators to
   pre-create homogeneous ASGs.
3. Works on both providers in our dual-profile stance (Hetzner sovereign,
   Azure enterprise) — Karpenter's `NodeClass` envelope keeps the
   provider boundary clean.
4. Stays open-source under an OSI license so it can ship with the
   sovereign-OSS Hetzner profile.

Cluster Autoscaler is the alternative. We rejected it because:

- ASG / nodegroup discretisation forces the workload to fit existing
  pools rather than the other way round.
- Bin-packing decisions live in the cloud provider, not the cluster —
  hard to extend with custom scoring (GPU-aware, cost-aware, sovereign-
  region-aware).
- Drift / consolidation is an after-thought; Karpenter makes it a
  first-class disruption controller.

## Decision

Adopt **kubernetes-sigs/karpenter v1.12.0** as the upstream we track.
Reimplement under `cave-karpenter` (Rust + the cave-runtime kernel), not
as a forked Go binary, so the autoscaler shares state, telemetry, and
auth with the rest of the runtime.

Provider plug-points:

- Hetzner profile → `HetznerNodeClass` (cave-cloud-controller-manager
  Hetzner provider). Bare-metal + cloud-server pools.
- Azure profile  → `AKSNodeClass` (delegates to AKS managed-node-pool
  API via Crossplane XRs, per ADR-002 / ADR-049).

## Status — 4-track 1/4

| Track       | State | Notes                                                      |
| ----------- | ----- | ---------------------------------------------------------- |
| Backend     | 1/4   | `cave-karpenter` crate scaffolded: models, store, scheduler stub. Five unit tests pass; five `#[ignore]`'d parity tests enumerate the gap (admission, claim launch, consolidation, drift, scheduler resource fit). |
| Portal      | 0/4   | No `crates/cave-portal/src/admin/karpenter.rs` yet.        |
| cavectl     | 0/4   | No `cavectl karpenter` subcommand yet.                     |
| Observ.     | 0/4   | No alerts (`docs/observability/alerts/cave-karpenter.yaml`) and no Grafana dashboard yet. |

The honest declaration matters: future agents reading this ADR should
not assume Portal/cavectl/Observ. exist. They do not.

## HA / DR / multi-region

Karpenter runs as a leader-elected Deployment. Cave-runtime's leader-
election (cave-controller-manager) will host it; replicas=2 across two
control-plane nodes per cluster, each cluster autoscales its own pool.
Multi-region is per-cluster — there is no cross-cluster Karpenter; that
remains the multi-cluster scheduler's job (cave-cluster + cave-kamaji).

## Open questions

- Whether `NodeClass.spec` stays as `serde_json::Value` or grows typed
  per-provider variants. Decision deferred until the Hetzner provider
  module lands.
- How disruption budgets compose with cave-incidents' freeze windows.
  Tracked as a follow-up once the disruption controller is non-stub.

## References

- [parity.manifest.toml](../../crates/cave-karpenter/parity.manifest.toml)
- Upstream: <https://github.com/kubernetes-sigs/karpenter> (v1.12.0,
  released 2026-04-24)
- ADR-001 — Hetzner sovereign infrastructure
- ADR-002 — Azure enterprise infrastructure
