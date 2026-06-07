// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cilium — a control-plane port of cilium/cilium (pinned v1.19.4).
//!
//! Where [`cave-net`](../cave_net) reimplements Cilium's eBPF *datapath*
//! (LB, DSR, conntrack, source-range) as a userspace state-machine sim,
//! `cave-cilium` ports the **agent / operator control-plane** that drives
//! that datapath:
//!
//! - [`ebpf`]        — eBPF object loader: ELF section parse, map/program
//!                     extraction, relocations, license gate, a verifier
//!                     model, and tc/xdp/cgroup attach points.
//! - [`policy`]      — `CiliumNetworkPolicy` CRD types, label-based numeric
//!                     security identities, and the reconciler that lowers
//!                     rules into policy-map entries (default-deny).
//! - [`ipam`]        — cluster-pool IPAM: per-node PodCIDR carve-out plus
//!                     in-pool allocate / release / GC.
//! - [`hubble`]      — flow observability: L3/L4/L7 records, drop reasons,
//!                     `FlowFilter` include/exclude, ring buffer.
//! - [`mesh`]        — no-sidecar L7 proxy: HTTP route matching + verdict.
//! - [`encryption`]  — WireGuard / IPsec, PQC-ready (ML-KEM/ML-DSA) hybrid.
//!
//! Upstream traceability: each module cites the cilium Go file(s) it ports,
//! and `parity.manifest.toml` enumerates the mapping.

pub mod ebpf;
