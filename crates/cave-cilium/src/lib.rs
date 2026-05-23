// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-cilium` — Cilium reimplementation in pure Rust.
//!
//! Upstream: cilium/cilium v1.19.4 (Apache-2.0).
//! Source commit pin: see `parity.manifest.toml`.
//!
//! ## Surface
//!
//! cave-cilium is a userspace approximation of the Cilium CNI / datapath
//! stack. The real Cilium agent runs an eBPF datapath inside the Linux
//! kernel — we ship the userspace control plane and policy enforcement,
//! and explicitly carry the kernel eBPF C code as a Phase 2 cut. Every
//! subsystem documented as `userspace policy enforcement` here is a real
//! Rust implementation of the same algorithms (LB rendezvous hashing,
//! conntrack 5-tuple table, NetworkPolicy L3/L4/L7, identity allocator,
//! BGP UPDATE construction) — it is not a stub.
//!
//! * **Datapath**: load balancer (Maglev rendezvous), conntrack, NAT,
//!   policy enforcement (L3/L4 + L7 redirect decisions).
//! * **CNI plugin**: ADD / DEL / CHECK verbs, cluster-pool + k8s IPAM,
//!   per-node CIDR allocation.
//! * **BGP control plane**: peer config, route announcement, OPEN +
//!   KEEPALIVE + UPDATE message construction.
//! * **ClusterMesh**: multi-cluster service routing (global services,
//!   shared endpoints).
//! * **Hubble**: flow observability + L7 visibility + flow filters.
//! * **NetworkPolicy**: K8s NetworkPolicy + CiliumNetworkPolicy
//!   (L3/L4/L7 — DNS, HTTP, Kafka).
//! * **Envoy bridge**: sidecarless ClusterMesh-style L7 injection
//!   (CEC + CCEC envoy configs).
//! * **Identity**: pod identity (label-set → 16-bit ID), SPIFFE bridge.
//! * **WireGuard**: transparent node-to-node encryption.
//! * **CRDs**: CNP, CCNP, Endpoint, Identity, Node, BGPPeeringPolicy.

use std::sync::Arc;

pub mod bgp;
pub mod cep;
pub mod clustermesh;
pub mod cni;
pub mod conntrack;
pub mod crd;
pub mod datapath;
pub mod endpoint;
pub mod envoy;
pub mod error;
pub mod fqdn;
pub mod hubble;
pub mod identity;
pub mod ipam;
pub mod ipcache;
pub mod loadbalancer;
pub mod metrics;
pub mod models;
pub mod nat;
pub mod node;
pub mod policy;
pub mod routes;
pub mod state;
pub mod wireguard;

use axum::Router;
pub use state::State;

pub const MODULE_NAME: &str = "cilium";

/// Build the HTTP router (cave-net mounts this under `/api/cilium`).
pub fn router(state: Arc<State>) -> Router {
    routes::router(state)
}
