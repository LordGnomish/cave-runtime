// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cilium — a control-plane port of cilium/cilium (pinned v1.19.4).
//!
//! Where [`cave-net`](../cave_net) reimplements Cilium's eBPF *datapath*
//! (LB, DSR, conntrack, source-range) as a userspace state-machine sim,
//! `cave-cilium` ports the **agent / operator control-plane** that drives
//! that datapath:
//!
//! - [`ebpf`] — eBPF object loader: ELF parse, map/program extraction,
//!   relocations, license gate, a verifier model, tc/xdp/cgroup attach.
//! - [`policy`] — `CiliumNetworkPolicy` CRD types, label-based numeric
//!   security identities, and the default-deny reconciler.
//! - [`ipam`] — cluster-pool IPAM: per-node PodCIDR carve plus
//!   in-pool allocate / release / GC.
//! - [`hubble`] — flow observability: L3/L4/L7 records, drop reasons,
//!   `FlowFilter` include/exclude, ring buffer.
//! - [`mesh`] — no-sidecar L7 proxy: HTTP route matching + verdict.
//! - [`encryption`] — WireGuard / IPsec, PQC-ready (ML-KEM/ML-DSA) hybrid.
//!
//! Upstream traceability: each module cites the cilium Go file(s) it ports,
//! and `parity.manifest.toml` enumerates the mapping.

pub mod ebpf;
pub mod encryption;
pub mod hubble;
pub mod ipam;
pub mod mesh;
pub mod policy;
pub mod routes;

use std::sync::Mutex;

/// Aggregate control-plane state shared across the HTTP surface.
#[derive(Default)]
pub struct CiliumState {
    pub ipam: Mutex<ipam::ClusterPoolState>,
    pub policy: Mutex<policy::PolicyRepository>,
    pub identities: Mutex<policy::IdentityAllocator>,
    pub hubble: Mutex<hubble::FlowBuffer>,
}

impl CiliumState {
    pub fn new() -> Self {
        CiliumState {
            ipam: Mutex::new(ipam::ClusterPoolState::default()),
            policy: Mutex::new(policy::PolicyRepository::default()),
            identities: Mutex::new(policy::IdentityAllocator::new()),
            hubble: Mutex::new(hubble::FlowBuffer::default()),
        }
    }
}

use std::sync::Arc;

pub fn new_state() -> Arc<CiliumState> {
    Arc::new(CiliumState::new())
}

pub fn router(state: Arc<CiliumState>) -> axum::Router {
    routes::create_router(state)
}
