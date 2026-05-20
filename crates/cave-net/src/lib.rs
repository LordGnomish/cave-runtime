// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-net — eBPF-based pod networking.
//!
//! Reimplements Cilium CNI functionality:
//! - Pod IP allocation from CIDR
//! - Service discovery (ClusterIP routing)
//! - Network policy enforcement (default-deny, allow rules)
//! - Flow recording (Hubble-style observability)
//!
//! On Linux kernel 7.0+: uses eBPF programs for kernel-level enforcement.
//! On other platforms: userspace simulation for development.

pub mod dataplane;
pub mod models;
pub mod routes;

/// Cilium-parity batch (numeric identity allocator, L7 policy,
/// ClusterMesh, Hubble flow log + topology, L7 proxy redirect).
/// Pinned to cilium/cilium v1.19.3.
pub mod cilium;

/// 2026-05-14 eBPF userspace simulation. Closes the `behavioral_parity`
/// audit's deliberately-skipped BPF datapath test by providing a
/// deterministic userspace state-machine sim of Cilium's
/// `bpf_lxc.c` / `bpf_host.c` / `bpf/lib/conntrack.h` control
/// surfaces. Not a packet emulator — see `ebpf_sim/mod.rs`.
pub mod ebpf_sim;

use dataplane::NetState;
use std::sync::Arc;

pub fn new_state() -> Arc<NetState> {
    Arc::new(NetState::new())
}

pub fn router(state: Arc<NetState>) -> axum::Router {
    routes::create_router(state)
}
