# cave-kubelet parity — 2026-05-12 measured audit

**Upstream pin:** `kubernetes/kubernetes` `pkg/kubelet/*` + `pkg/probe/*` + `pkg/volume/csi/` (v1.31.x; manifest pin `v1.28.0` is preserved).

## Why

2026-05-01 audit: **tier 100, parity_ratio = 1.0** (mechanical wave3 metric: 30 files / 24 fns / 118 tests / 3 surfaces). The number ignored the dozen `pkg/kubelet/*` sub-packages that cave-kubelet does NOT implement (status manager, prober worker pool, lifecycle hooks, userns, nodeshutdown, ...). This pass replaces it with a measured ratio over enumerated upstream sub-packages.

## Counts

| Bucket | Count |
|---|---:|
| `[[mapped]]` | **22** (was 20) |
| `[[skipped]]` | 9 |
| `[[unmapped]]` | **7** (was 9) |
| **Total** | **38** |
| **fill_ratio** | **0.8158** (was 0.7632) |

`parity_ratio = 1.0` → `fill_ratio = 0.8158` (2026-05-13 update).

### 2026-05-13 k8s-core push update

The two biggest unmapped packages in the 2026-05-12 audit are now
ported:

* **`pkg/kubelet/status/`** → `src/pod_status_manager.rs`.
  Lazy hash-dedupe (status content hash excludes free-text
  `message`), bounded queue with oldest-eviction on overflow,
  exponential backoff via `cave_kernel::backoff::Backoff::Exponential`
  with a default 200ms → 30s schedule, transient vs permanent
  failure separation (only transient bumps the failure counter),
  deleted-pod drop semantics so racing `set_status` on a removed pod
  is silently suppressed. `DispatchOutcome` + `AttemptOutcome` make
  the kubelet sync-loop integration testable without an apiserver.
  15 deterministic tests cover every transition.
* **`pkg/kubelet/prober/`** → `src/prober.rs`.
  `ProberCoordinator` wraps the existing per-probe `ProberManager`
  state machine and adds the missing coordinator layer: a worker
  pool (`cave_kernel::semaphore::Semaphore`, default 16 concurrent),
  per-container ledger that suppresses duplicate `RestartContainer`
  events while a previous restart is still in flight (cleared by
  `mark_restart_completed` or auto-cleared after a configurable
  safety window), readiness flip dedup so `MarkReady`/`MarkNotReady`
  only emit on true transitions. 17 tests.

## Mapped highlights (20)

- **Container Manager**: cpumanager, memorymanager, topology_manager, devicemanager, DRA v1alpha2.
- **Eviction + Image GC** under resource pressure.
- **Streaming server**: exec / attach / port-forward (streaming.rs).
- **Probes**: HTTP / TCP / Exec / gRPC.
- **Sidecar init containers** (KEP-753, v1.31 GA).
- **CSI client** for volume nodes.
- **AppArmor** integration.
- **Node lease + plugin watcher + pod-resources**.

## Unmapped (7 as of 2026-05-13, ordered by likely demand)

1. ~~**`pkg/kubelet/status/`**~~ — **CLOSED 2026-05-13**, see the
   k8s-core push update above.
2. ~~**`pkg/kubelet/prober/`**~~ — **CLOSED 2026-05-13**, see the
   k8s-core push update above.
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
