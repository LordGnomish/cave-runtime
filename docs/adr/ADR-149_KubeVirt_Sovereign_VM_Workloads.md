# ADR-149 — KubeVirt for Sovereign VM Workloads

| Status | Accepted (scaffold only — implementation pending) |
| ------ | ------------------------------------------------- |
| Date   | 2026-05-06                                        |
| Track  | Backend 1/4, Portal 0/4, cavectl 0/4, Observ. 0/4 |

## Context

Cave Runtime is a Kubernetes-native platform. But not every legacy
workload that sovereign customers run can be containerised:

- Vendor appliances shipped as VM images (firewalls, identity gateways,
  industry-specific ERPs).
- Workloads with kernel-module dependencies (telco DPDK, CAN-bus
  bridges) that cannot be lifted into containers without OS-level
  changes.
- Compliance regimes (defence, regulated finance) that require an
  ephemeral, auditable hypervisor boundary even for software that
  *could* be containerised.

We need these workloads to live alongside Pods on the same cluster, with
the same auth, the same observability stack, and the same sovereign-OSS
guarantee — not on a separate VMware estate.

The alternatives we considered:

- **Run a separate hypervisor** (ESXi, oVirt, Proxmox). Reject — splits
  the operational surface, breaks identity unification, and oVirt is
  end-of-life.
- **Use Firecracker / Cloud Hypervisor directly via a custom CRI**.
  Reject for now — interesting for serverless but doesn't speak the VM
  CRD shape sovereign customers' tooling expects.

## Decision

Adopt **kubevirt/kubevirt v1.8.2** as the upstream we track for
VM-as-K8s-Pod. Reimplement under `cave-kubevirt` (Rust + cave-runtime
kernel) so VMs share the same store, auth, and telemetry as the rest of
the runtime. Concrete model surface mirrors v1: `VirtualMachine`,
`VirtualMachineInstance`, `DataVolume` (CDI), with libvirt-shaped
`Domain` kept opaque until we wire a `virt-launcher`-equivalent.

Provider notes:

- **sovereign-cloud profile** (sovereign): VMs land on bare-metal nodes via the
  sovereign-cloud provider in cave-cloud-controller-manager. KVM/QEMU on
  Linux 6.x; SR-IOV available where the hardware supports it.
- **Azure profile**: VMs are scheduled onto AKS nodes that have nested
  virtualisation enabled (Dv5/Ev5 v-series); KubeVirt itself runs in
  the cluster and the Azure VM serves as the libvirt host. Not all SKU
  families support nested virt — operators must opt in per node pool.

## Status — 4-track 1/4

| Track       | State | Notes                                                      |
| ----------- | ----- | ---------------------------------------------------------- |
| Backend     | 1/4   | `cave-kubevirt` crate scaffolded: models, store, lifecycle stub. Eight unit tests pass; five `#[ignore]`'d parity tests enumerate the gap (admission, virt-launcher Pod, live migration, DataVolume CDI, instancetype). |
| Portal      | 0/4   | No `crates/cave-portal/src/admin/kubevirt.rs` yet.         |
| cavectl     | 0/4   | No `cavectl kubevirt` subcommand yet.                      |
| Observ.     | 0/4   | No alerts (`docs/observability/alerts/cave-kubevirt.yaml`) and no Grafana dashboard yet. |

## HA / DR / multi-region

- **HA**: KubeVirt's controllers (virt-controller, virt-handler,
  virt-api) run as leader-elected Deployments / DaemonSets. We will
  reimplement them as cave-controller-manager submodules so leader
  election shares the same lease object.
- **DR**: Live-migration is the in-cluster DR primitive. Cross-cluster
  VM mobility is **not in scope** for this ADR — that lives behind
  cave-cluster's federation work and is years out.
- **Multi-region**: Per-cluster only. A region failure does not
  automatically migrate VMs.

## Open questions

- Whether `Domain.devices` / `Domain.features` ever become typed Rust
  shapes or stay as `serde_json::Value`. Decision deferred until the
  virt-launcher equivalent is closer to landing.
- Whether the CDI sub-project is reimplemented in `cave-kubevirt` or
  hived off into its own `cave-cdi` crate. Tracked as a follow-up; the
  scaffold keeps `DataVolume` co-located for now.

## References

- [parity.manifest.toml](../../crates/cave-kubevirt/parity.manifest.toml)
- Upstream: <https://github.com/kubevirt/kubevirt> (v1.8.2, released
  2026-04-20)
- ADR-001 — sovereign infrastructure
- ADR-002 — Azure enterprise infrastructure
- ADR-145 — Karpenter node autoscaling (companion ADR; both expand the
  workload surface beyond stateless containers)
