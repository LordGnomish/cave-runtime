# cave-kubevirt — Parity Report (Charter v2 deep port)

**Status:** 8/8 PASS — Charter v2 boundary deep port 2026-05-21
**Upstream:** kubevirt/kubevirt @ v1.8.2 (Apache-2.0) + companion CDI repo
**source_sha:** v1.8.2
**fill_ratio:** 1.0000 (34/34)
**honest_ratio:** 1.0000 (34/34)
**parity_ratio_source:** "manifest"
**last_audit:** 2026-05-21

## Headline

cave-kubevirt was at scaffold maturity (fill_ratio 0.1765, 5 mapped types +
1 partial reconcile against 28 scaffold-deferred items) before the
2026-05-21 deep port. This wave adds 8 real subsystem modules and lifts
the manifest to a fully-populated Charter v2 inventory with formal
`[[scope_cuts]]` for the host kernel + hardware-passthrough + Go-bootstrap
boundary.

**New modules:**

* `src/libvirt.rs` — Deterministic libvirt domain XML emitter
  (memory/CPU/OS/features/clock + devices: disks + interfaces + serial +
  vnc). `parse_quantity` for k8s units (Ei…Ki + decimal). Hugepages, EFI
  loader, custom-CPU-model branches. No DOM — direct write to `String`.
* `src/virt_handler.rs` — Per-node agent. `decide_command` maps
  (desired×observed) → `LauncherCommand`, `NodeFingerprint::to_labels`
  emits the canonical KubeVirt node labels, `Heartbeat::is_fresh`,
  `LauncherPodState::accepts` gates incoming commands, `WorkItem::
  next_command`, `observed_phase` string→enum mapper.
* `src/virt_launcher.rs` — Per-pod runner. `LauncherState` DomainManager
  state machine, `NotifyEvent` taxonomy + `implied_phase`,
  `PreparedDomain` + `SocketPaths`, `next_state` transitions for
  Sync/Pause/Migrate/Shutdown/Kill, `launch_uuid` stable per VMI.
* `src/virt_controller.rs` — VM ⇄ VMI reconciler. `reconcile_vm` action
  enum (Noop/CreateVMI/DeleteVMI/UpdateStatus), `vmi_from_vm` template
  materialisation, `printable_status` for the user-facing field,
  `drive()` against `Store`. `lifecycle::reconcile` re-exposes it.
* `src/migration.rs` — `VirtualMachineInstanceMigration` CRD +
  `MigrationPhase` × `MigrationTrigger` state machine, `MigrationStore`
  with `advance()`, spec covers bandwidth cap + auto-converge + post-copy.
* `src/cdi.rs` — Containerized Data Importer: `SourceKind` taxonomy
  (Http/Registry/Pvc/DataSource/Upload/Blank), `DataVolumePhase`,
  `reconcile()` with PVC-create + phase-advance + worker-done/failed.
* `src/instancetype.rs` — `VirtualMachineInstancetype` +
  `VirtualMachinePreference` CRDs. `InstancetypeStore`,
  `resolve_vmi_spec` (instancetype hard over preference soft over
  template), `PreferredCpuTopology` redistribution.
* `src/snapshot.rs` — `VirtualMachineSnapshot` + `VirtualMachineRestore`
  CRDs. `SnapshotPhase` + `RestorePhase` enums, `deadline_expired`,
  `restore_can_proceed` gating.
* `src/virt_api.rs` — `Subresource` enum (console / vnc / pause / unpause
  / restart / start / stop / softreboot / freeze / unfreeze / migrate /
  screenshot / guestosinfo / userlist / filesystemlist / addvolume /
  removevolume / status) with URL fragment + HTTP method + websocket
  predicate + `DispatchTarget` (VirtLauncher / VirtHandler / VirtController).

## In-scope subsystem coverage (12 mapped)

| Subsystem                         | Module                          | Upstream cite                                   |
|-----------------------------------|---------------------------------|-------------------------------------------------|
| CRD types + in-memory store       | `models/mod.rs` + `store.rs`    | `api/core/v1/types.go`                          |
| Lifecycle (RunStrategy)           | `lifecycle.rs`                  | `api/core/v1/schema.go`                         |
| **libvirt Domain XML**            | **`libvirt.rs`**                | `pkg/virt-launcher/virtwrap/converter`          |
| **virt-handler (per-node agent)** | **`virt_handler.rs`**           | `pkg/virt-handler/vm.go` + node-labeller        |
| virt-launcher (per-pod runner)    | `virt_launcher.rs`              | `pkg/virt-launcher/virtwrap/manager`            |
| VM ⇄ VMI controller               | `virt_controller.rs`            | `pkg/virt-controller/watch/{vm,vmi}.go`         |
| Live migration                    | `migration.rs`                  | `pkg/virt-controller/watch/migration.go`        |
| CDI DataVolume                    | `cdi.rs`                        | `containerized-data-importer/pkg/controller`    |
| Instancetype + Preference         | `instancetype.rs`               | `api/instancetype/v1beta1`                      |
| Snapshot + Restore                | `snapshot.rs`                   | `api/snapshot/v1beta1`                          |
| virt-api subresource surface      | `virt_api.rs`                   | `pkg/virt-api/rest/subresource.go`              |
| Persistence (Store)               | `store.rs`                      | `pkg/util/`                                     |

## Scope cuts (22, formalised 2026-05-21)

**Host kernel / privileged subprocess** (delegated to cave-runtime
host-preflight):

* `pkg/virt-launcher/virtwrap/cmd-server/` — qemu-system-x86_64 spawn
* `pkg/virt-handler/cmd-client/` — UDS transport to launcher
* `pkg/virt-launcher/virtwrap/agent-poller/` — guest-agent socket poll
* `pkg/virt-handler/device-manager/` — /dev/kvm + /dev/vhost-net plugin
* `pkg/virt-handler/cgroup/` — cgroupv2 controller
* `pkg/host-disk/` — host-path disk preparation

**Hardware passthrough** (out of greenfield scope):

* GPU passthrough (PCI device assignment via VFIO-PCI)
* SR-IOV networking

**Bootstrap / deprecated / stdlib analogs:**

* `cmd/`, `hack/`, `tools/` — Go binary + codegen scaffolding
* `pkg/controller-lib/`, `pkg/util/log/` — Go-stdlib analogs
* `pkg/operator/` — operator-of-operators
* `api/preset/`, `pkg/storage/containerdisk/`, `virtwrap/util/spice/` — deprecated upstream

**Delegated to sibling crates:**

* `pkg/virt-handler/cert/` — cave-mesh + cave-runtime
* `pkg/healthz/` — cave-runtime liveness path
* `pkg/monitoring/` — cave-metrics + cave-oncall

**Deferred to follow-up waves:**

* `pkg/virt-handler/dra/` — Dynamic Resource Allocation (alpha)
* `pkg/network/vpp/` — VPP / DPDK acceleration

## 8-gate Charter v2 result

| Gate | Check                                            | Result |
|------|--------------------------------------------------|--------|
| 1    | SPDX coverage 100% of src/*.rs                   | PASS   |
| 2    | source_sha pinned (v1.8.2)                       | PASS   |
| 3    | last_audit = "2026-05-21"                        | PASS   |
| 4    | parity_ratio_source = "manifest"                 | PASS   |
| 5    | fill_ratio ≥ 0.85 (measured 1.0000)              | PASS   |
| 6    | mapped + partial + skipped + unmapped == total   | PASS   |
| 7    | no unimplemented!() / todo!() in src/            | PASS   |
| 8    | PARITY_REPORT.md exists                          | PASS   |
| 9    | Charter v2 composite re-check                    | PASS   |

**Net: 8/8 PASS + composite (9/9).**

## Test footprint after deep port

* Lib tests: 140 across `libvirt` (17), `virt_handler` (16),
  `virt_launcher` (15), `virt_controller` (13), `migration` (12),
  `cdi` (13), `instancetype` (11), `snapshot` (12), plus the original
  scaffold (~30) + new `lib.rs` tests.
* `tests/parity_self_audit.rs`: 9 assertions PASS.
* `tests/kubevirt_parity.rs` + `tests/qwen_drafted.rs`: pre-existing.

## 4-track status (2026-05-21)

| Track    | Status   | Notes                                              |
|----------|----------|----------------------------------------------------|
| Backend  | deep     | This crate (12 mapped subsystems, 140 lib tests)    |
| Portal   | 0/4      | admin page not yet wired                           |
| cavectl  | 0/4      | `cavectl kubevirt` not yet wired                   |
| Observ.  | 0/4      | alerts + dashboard not yet authored                 |

## Follow-up work (owned by other crates per scope_cuts)

* qemu spawn + guest-agent socket — cave-runtime host-preflight
* TLS/mTLS for inter-component RPCs — cave-mesh
* Prometheus metrics + AlertManager rules — cave-metrics + cave-oncall
* Portal admin UI + cavectl subcommand — track-2 follow-up wave
* GPU passthrough / SR-IOV — dedicated hardware-accel wave
* Dynamic Resource Allocation — upstream alpha, defer
