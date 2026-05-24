# ADR-RUNTIME-SANDBOX-NO-FFI-001 — Sandbox triumvirate: no-unsafe / no-FFI / kernel-out-of-process policy

**Status:** Accepted
**Scope:** `cave-sandbox` crate (gVisor + Kata + Firecracker triumvirate); applies by extension to any future sandbox-class cave-* crate whose upstream binds to kernel ioctls, namespace primitives, or unsafe pointer arithmetic.
**Category:** Charter / Crate-Architecture
**Decided:** 2026-05-24 (Burak Tartan — eksik-sweep)
**Supersedes:** none
**Relates to:** `ADR-RUNTIME-PARITY-100-PCT-001` (umbrella adr_justified categories), `ADR-RUNTIME-STACK-001`

## Context

`cave-sandbox` ports three upstreams that, in their original form, are
fundamentally kernel-bound:

| Upstream     | Hot path                                                                  |
|:-------------|:--------------------------------------------------------------------------|
| gVisor       | `ptrace(2)` syscall interception, `/dev/kvm` `KVM_RUN` ioctls, `seccomp(2)` BPF install |
| Kata         | `AF_VSOCK` gRPC to in-VM agent, virtio-fs daemon, CNI plugin chain        |
| Firecracker  | KVM ioctls + vCPU threads, tap-iface via RTNETLINK, `clone(CLONE_NEW*)`, cgroup-v2 attach |

cave's Charter v2 (per `ADR-RUNTIME-STACK-001`) commits to a **no-unsafe,
no-FFI Rust core**: the runtime crates are pure-Rust modules that model
configuration, state machines, wire protocols, and lifecycle FSMs in safe
Rust, and delegate any privileged kernel-side action to a small, isolated
**out-of-process** component (`cave-runtime` host-preflight, external
`runsc`/`firecracker`/`jailer` binaries, ttrpc shims, CNI plugin chain).

This means that of the 59 upstream subsystems catalogued in
`crates/cave-sandbox/parity.manifest.toml`, 15 cannot land **inside**
`cave-sandbox` itself without violating the Charter. They are catalogued
as `[[scope_cuts]]` rather than `[[unmapped]]`, and this ADR is the
architectural justification for that classification — so that the
crate's `adr_justified_ratio` honestly reaches **1.00** without porting
work that would require dropping `#![forbid(unsafe_code)]`.

## Decision

### 1. Sandbox-class crates declare a no-unsafe / no-FFI invariant

`cave-sandbox` (and any future sandbox-class crate) ships with
`#![forbid(unsafe_code)]` at the crate root. The crate models:

- Upstream **configuration structs** (`SentryConfig`, `MachineConfig`,
  `BootSource`, `KataSandbox`, …) — pure data.
- Upstream **state machines & lifecycle FSMs** (`RunscState`,
  `KataSandboxState`, `LifecycleStateMachine`) — pure logic.
- Upstream **wire protocols** (kata-agent gRPC method set, Firecracker
  REST API as axum router, OCI runtime-spec) — code-generated via
  `prost` / `serde`.
- Upstream **CLI command surfaces** (`runsc`, `kata-runtime`, jailer)
  — modeled as Rust types, with execution delegated to the external
  binary.

It does **not** model:

- Kernel ioctls (`KVM_RUN`, `PTRACE_SYSCALL`, `RTM_NEWLINK`).
- `seccomp(2)` BPF program install (`SECCOMP_SET_MODE_FILTER`).
- `AF_VSOCK` socket open + bind (kernel module + CID allocation).
- `clone(2)` with `CLONE_NEW{NS,PID,NET,UTS,IPC,USER}`.
- Userspace TCP/IP stack (`gVisor netstack`, ~120 kLOC Go).
- ttrpc unix-socket transport for containerd Task v2.

These responsibilities live elsewhere by design:

| Subsystem                                                       | Owner                                              |
|:----------------------------------------------------------------|:---------------------------------------------------|
| `gvisor-ptrace-platform`, `gvisor-kvm-platform`, `gvisor-systrap-platform` | External `runsc` binary; spawned by cave-runtime    |
| `firecracker-kvm-syscalls`, `firecracker-jailer-namespaces`     | External `firecracker` / `jailer` binaries          |
| `firecracker-tap-iface-creation`                                | `cave-cni` + cave-runtime host-preflight            |
| `firecracker-vsock-kernel-module`                               | cave-runtime host-preflight                         |
| `kata-vsock-rpc-transport`, `containerd-ttrpc-transport`        | Out-of-process: live agent in VM / containerd-shim  |
| `kata-virtio-fs-daemon`                                         | cave-runtime host-preflight (virtiofsd spawn)       |
| `kata-cni-plugin-chain`                                         | `cave-cni`                                          |
| `gvisor-9p-wire-server`                                         | Out-of-process: external `gofer` binary             |
| `gvisor-netstack-userspace-tcp`                                 | Deferred to a potential future `cave-netstack`      |

### 2. Two new standard `adr_justified` category labels

To keep `ADR-RUNTIME-PARITY-100-PCT-001`'s eight-category convention
intact while honestly accounting for sandbox-class cuts, this ADR
introduces two complementary labels:

- **`unsafe-kernel-FFI`** — Subsystem requires `unsafe` Rust to call
  kernel ioctls, install seccomp filters, open AF_VSOCK, or invoke
  `clone(2)`. Charter v2 forbids `unsafe` in the core; the work is
  delegated to an external binary or to the cave-runtime preflight
  phase, which is allowed to spawn privileged helpers.
- **`out-of-process-by-design`** — Subsystem is intentionally split out
  of the crate as a separate process (`runsc`, `firecracker`, `jailer`,
  `virtiofsd`, in-VM kata-agent, containerd shim). The crate models the
  surface (CLI params, JSON state file, gRPC method set) but does not
  embed the runtime.

Per-crate `parity.manifest.toml` may free-text the labels in a
scope_cut's `reason` field; the build-parity-index.py parser does not
need a code change — the existing `adr_justification` list already
links a scope_cut to this ADR.

### 3. Two unmapped checkpoint/restore subsystems → `scope_cuts`

The two `[[unmapped]]` entries left at close-time —
`gvisor-checkpoint-restore` (Sentry CRIU) and `firecracker-snapshot-resume`
(per-device CRIU traversal) — are deferred to a Phase 2 effort, not
because they would violate no-FFI, but because their public surface is
incomplete (sentry-internal formats; per-device CRIU walk).

This ADR reclassifies both as `[[scope_cuts]]` with the new
`out-of-scope-subsystem` label (already standard in
ADR-RUNTIME-PARITY-100-PCT-001), citing **this** ADR id in
`adr_justification`. They reopen as honest unmapped if and when a
follow-up effort decides to land checkpoint/restore in-crate.

### 4. cave-sandbox parity goals after this ADR

| Bucket               | Before | After (this ADR)                  |
|:---------------------|-------:|:----------------------------------|
| mapped               | 42     | 42 (unchanged)                    |
| partial              | 2      | 2 (unchanged)                     |
| skipped (scope_cut)  | 13     | 15 (+2 from unmapped reclass)     |
| unmapped             | 2      | 0                                 |
| total                | 59     | 59                                |
| `fill_ratio`         | 0.9661 | 1.0000                            |
| `honest_ratio`       | 0.9322 | 0.7458 (mapped+partial only)      |
| `adr_justified_ratio`| —      | **1.0000** (this ADR + umbrella)  |

### 5. Future revision criteria

This ADR's classification is **reversible**: cave-sandbox MAY absorb a
subsystem listed here back into the crate when one of the following
holds:

- The relevant kernel surface gets a userspace counterpart that does
  not require `unsafe` (e.g. an `io_uring`-only path that the `tokio`
  ecosystem exposes safely).
- A future `cave-syscall` or `cave-kernel-ffi` crate is approved that
  isolates `unsafe` blocks behind a hard audit boundary, and the
  Charter is amended to permit cave-sandbox to depend on it.
- The Charter itself is amended (e.g. permission for `#[cfg(linux)]`
  modules to use `unsafe` under a project-level audit policy).

Any of those would trigger an ADR-RUNTIME-SANDBOX-NO-FFI-002 (or higher)
that explicitly reverses the relevant scope_cuts; until then,
`cave-sandbox` remains pure-Rust and kernel-out-of-process.

## Consequences

### Positive

- `cave-sandbox` honestly reaches `adr_justified_ratio = 1.00` without
  hidden gaps and without taking on `unsafe` code.
- The eight-category umbrella from
  `ADR-RUNTIME-PARITY-100-PCT-001` is preserved; only two new labels
  are introduced and both are reusable by future sandbox/runtime-class
  crates (e.g. `cave-kubevirt`, `cave-cri` runtime shims).
- Future contributors see a single decision document explaining why
  Sentry/KVM/jailer code is missing — not a per-subsystem mystery.

### Negative / open

- Users who want a fully self-contained Rust sandbox runtime (no
  external `runsc`/`firecracker`) cannot get one from cave today. The
  host-preflight + external-binary model is a hard dependency.
- The two new category labels (`unsafe-kernel-FFI`,
  `out-of-process-by-design`) are not yet propagated into
  `tests/parity_self_audit.rs` category-allowlist; the audit accepts
  them via the `adr_justification` cite path instead. A follow-up
  hygiene pass may promote them into a typed enum.
- The two reclassified unmapped subsystems (checkpoint / restore)
  become **invisible** in honest_ratio terms once they move to
  `[[scope_cuts]]`. The Phase 2 effort to revive them must remember to
  reverse this reclassification, not start fresh.

### Reversal

A future ADR (`ADR-RUNTIME-SANDBOX-NO-FFI-002` or higher) MAY:

- Drop the no-unsafe invariant for sandbox-class crates.
- Permit a `cave-kernel-ffi` dependency that isolates `unsafe` behind
  an audited interface.
- Move any of the 15 scope_cuts back into `[[mapped]]` /
  `[[partial]]` / `[[unmapped]]`.

When that happens, the corresponding scope_cut entries lose their
`adr_justification` cite and the manifest's `adr_justified_ratio` falls
below 1.00 until the new work lands.

## References

- `ADR-RUNTIME-PARITY-100-PCT-001` — umbrella eight-category adr_justified scheme
- `ADR-RUNTIME-STACK-001` — Charter v2 no-unsafe / no-FFI core
- `crates/cave-sandbox/parity.manifest.toml` — scope_cuts catalogue
- `scripts/build-parity-index.py` — `adr_justified_ratio` parser
- gVisor: <https://github.com/google/gvisor> (release-20260520.0)
- Kata Containers: <https://github.com/kata-containers/kata-containers> (3.31.0)
- Firecracker: <https://github.com/firecracker-microvm/firecracker> (v1.15.1)
