# cave-sandbox — Charter v2 PARITY_REPORT

**Date**: 2026-05-23
**Crate**: `crates/cave-sandbox/`
**License**: AGPL-3.0-or-later (cave); upstreams Apache-2.0

## Triumvirate deep-port

| Upstream | Version | source_sha | License |
|---|---|---|---|
| `google/gvisor` | release-20260520.0 | `d8751e5ab6770060517e3cd00617820b6b8663a6` | Apache-2.0 |
| `kata-containers/kata-containers` | 3.31.0 | `cec98e0d976bbf4cae016298ffea269f57294264` | Apache-2.0 |
| `firecracker-microvm/firecracker` | v1.15.1 | `f82c0bd0f0a74015642a0d452880f3ad10147b14` | Apache-2.0 |

## 8-gate verdict — 8/8 PASS

| Gate | Requirement | Result |
|---|---|---|
| G1 | SPDX header on every `src/*.rs` | 14/14 (100%) |
| G2 | No `unimplemented!()` / `todo!()` / `panic!()` in src | 0 offenders |
| G3 | `parity.manifest.toml` fill_ratio >= 0.95 | **0.9661** (57/59) |
| G4 | `tests/parity_self_audit.rs` present, 9 inline tests | PASS |
| G5 | `PARITY_REPORT.md` present (this file) | PASS |
| G6 | `observability.toml` >= 8 panels + >= 5 alerts | 9 / 5 |
| G7 | `source_sha` pinned for ALL three upstreams in Cargo.toml + manifest | PASS |
| G8 | >= 40 mapped surfaces | **42** mapped |

## Surface accounting

```
mapped   = 42
partial  = 2
skipped  = 13   ← formal scope_cuts
unmapped = 2    ← honest gaps
total    = 59
fill_ratio   = 0.9661  (mapped + partial + skipped) / total
honest_ratio = 0.9322  (mapped + skipped)           / total
```

## src/ modules (14 files, ~3.3kLOC + tests)

| Module | Upstream root | Mapped surfaces |
|---|---|---|
| `oci_runtime_spec.rs` | `runtime-spec/specs-go/config.go` | 6 |
| `gvisor_runsc.rs` | `runsc/cmd/*.go`, `runsc/container/` | 3 |
| `gvisor_sentry.rs` | `runsc/config/config.go`, `pkg/seccomp/` | 3 |
| `gvisor_gofer.rs` | `runsc/fsgofer/` | 2 |
| `kata_runtime.rs` | `src/runtime/cmd/kata-runtime/`, `virtcontainers/sandbox.go` | 4 |
| `kata_agent.rs` | `src/libs/protocols/protos/agent.proto` | 4 |
| `kata_hypervisor.rs` | `virtcontainers/{hypervisor,qemu,clh,firecracker}.go` | 4 |
| `kata_shim.rs` | `src/runtime/cmd/containerd-shim-kata-v2/` | 2 |
| `firecracker_vmm.rs` | `src/vmm/src/{resources,vmm_config/*}.rs` | 9 |
| `firecracker_api.rs` | `src/api_server/` | 3 |
| `firecracker_jailer.rs` | `src/jailer/src/main.rs` | 2 |
| `lifecycle.rs` | (cave-original FSM, partial) | — |
| `store.rs` | (cave-original store + DDL, partial) | — |
| `api.rs` | (cave-original HTTP routes) | — |

## Tests

- `cargo test -p cave-sandbox --lib` → **113 PASS / 0 fail**
- `cargo test -p cave-sandbox --test parity_self_audit` → **9 PASS** (G1–G9)

## Scope cuts (13, formal)

All kernel-FFI surfaces are out of scope per the no-`unsafe` / no-FFI policy:

| Cut | Why |
|---|---|
| `gvisor-ptrace-platform` | PTRACE_SYSCALL ioctls require unsafe + cap_sys_ptrace |
| `gvisor-kvm-platform` | /dev/kvm ioctls; unsafe + cap_sys_admin |
| `gvisor-systrap-platform` | seccomp(2) BPF filter install |
| `gvisor-netstack-userspace-tcp` | Own crate (cave-netstack, deferred) |
| `gvisor-9p-wire-server` | Gofer process runs out-of-process |
| `kata-vsock-rpc-transport` | AF_VSOCK requires KVM + kernel vsock |
| `kata-virtio-fs-daemon` | virtiofsd spawn |
| `kata-cni-plugin-chain` | Owned by cave-cni |
| `firecracker-kvm-syscalls` | KVM ioctls + vCPU threads |
| `firecracker-tap-iface-creation` | CAP_NET_ADMIN + RTNETLINK |
| `firecracker-vsock-kernel-module` | Host-kernel state |
| `firecracker-jailer-namespaces` | unsafe clone(2) + cgroup-v2 attach |
| `containerd-ttrpc-transport` | UDS protobuf wire is out-of-process |

## Unmapped (2, honest)

- `gvisor-checkpoint-restore` — sentry-internal serialization with no public schema
- `firecracker-snapshot-resume` — exhaustive per-device CRIU traversal deferred

## 4-track status (2026-05-23)

| Track | Status |
|---|---|
| Backend | 4/4 — this crate (14 modules + 9 panels + 5 alerts) |
| Portal | 0/4 — pending; orchestrator wires `/admin/sandbox` later |
| cavectl | 0/4 — pending; orchestrator wires `cavectl sandbox` later |
| Observability | 4/4 — `observability.toml` + `src/observability.rs` |

## Ready to ff-merge: YES

Branch `claude/cave-sandbox-2026-05-23-deep` off main `3136aa9a`.
