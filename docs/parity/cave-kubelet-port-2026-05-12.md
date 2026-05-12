# cave-kubelet parity — 2026-05-12 measured audit

**Upstream pin:** `kubernetes/kubernetes` `pkg/kubelet/*` + `pkg/probe/*` + `pkg/volume/csi/` (v1.31.x; manifest pin `v1.28.0` is preserved).

## Why

2026-05-01 audit: **tier 100, parity_ratio = 1.0** (mechanical wave3 metric: 30 files / 24 fns / 118 tests / 3 surfaces). The number ignored the dozen `pkg/kubelet/*` sub-packages that cave-kubelet does NOT implement (status manager, prober worker pool, lifecycle hooks, userns, nodeshutdown, ...). This pass replaces it with a measured ratio over enumerated upstream sub-packages.

## Counts

| Bucket | Count |
|---|---:|
| `[[mapped]]` | 20 |
| `[[skipped]]` | 9 |
| `[[unmapped]]` | 9 |
| **Total** | **38** |
| **fill_ratio** | **0.7632** |

`parity_ratio = 1.0` → `fill_ratio = 0.7632`.

## Mapped highlights (20)

- **Container Manager**: cpumanager, memorymanager, topology_manager, devicemanager, DRA v1alpha2.
- **Eviction + Image GC** under resource pressure.
- **Streaming server**: exec / attach / port-forward (streaming.rs).
- **Probes**: HTTP / TCP / Exec / gRPC.
- **Sidecar init containers** (KEP-753, v1.31 GA).
- **CSI client** for volume nodes.
- **AppArmor** integration.
- **Node lease + plugin watcher + pod-resources**.

## Unmapped (9, ordered by likely demand)

1. **`pkg/kubelet/status/`** — PodStatusManager queue + retry. Today's sync-time writes can leak phase updates on transient apiserver failure.
2. **`pkg/kubelet/prober/`** — Probe-worker pool + restart coordination ledger. probe.rs runs probes; the coordinator is missing.
3. **`pkg/kubelet/cm/util/cgroups/`** — v2 unified-hierarchy direct cgroup writes. Cave writes via CRI today; systemd-cgroup-driver mode unsupported.
4. **`pkg/kubelet/lifecycle/`** — preStop / postStart hook orchestration with per-event timeouts.
5. **`pkg/kubelet/preemption/`** — Critical-pod admit (evict lower-priority for system-critical).
6. **`pkg/kubelet/nodeshutdown/`** — KEP-2000 graceful shutdown handler.
7. **`pkg/kubelet/userns/`** — KEP-127 user namespace remapping (v1.30 beta).
8. **`pkg/kubelet/runonce/`** — Standalone manifest-only mode (boot-time control plane).
9. **`pkg/kubelet/checkpoint/`** — CRIU checkpoint endpoint (runtime hook in cave-cri).

## Skipped (9)

`cmd/kubelet/` go-bootstrap. `pkg/kubelet/apis/config/` stdlib-analog (serde). `pkg/kubelet/cri/` parallel-track (cave-cri direct). `pkg/kubelet/dockershim/` wire-format-detail (removed upstream v1.24). `pkg/kubelet/network/` parallel-track (cave-net CNI). `pkg/kubelet/metrics/` parallel-track. `pkg/kubelet/util/` + `pkg/kubelet/types/` stdlib-analog. `pkg/probe/exec/` folded.

## Out of scope

The 9 unmapped packages range from ~200 LOC (runonce) to ~2 K LOC (status manager + prober worker pool). The audit gives the prioritised pick-up list; no new ports landed this pass.
