// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-kube-proxy — kube-proxy parity for Cave Runtime.
//!
//! Tracks Service / EndpointSlice events from cave-apiserver and renders them
//! into iptables (legacy) or nftables (preferred — Linux ≥ 7.1) datapath
//! rules. The userspace proxier is intentionally NOT supported: this is a
//! greenfield deployment, no upgrade path from the legacy data plane.
//!
//! Multi-tenancy: every store, change tracker, allocator and proxier is
//! constructed against a `tenant_id`, mirroring cave-runtime's namespace
//! isolation model. Cross-tenant Service / NodePort visibility is forbidden.
//!
//! Upstream parity target: kubernetes/kubernetes v1.36.0.

pub mod conntrack;
pub mod endpoints;
pub mod error;
pub mod healthcheck;
pub mod iptables;
pub mod metrics;
pub mod nftables;
pub mod nodeport;
pub mod proxy_config;
pub mod service;
pub mod sync_runner;
pub mod topology;

pub use conntrack::{
    apply_conntrack_sysctls, flush_stale_cluster_ips, flush_stale_node_ports, CapturedConntrack,
    ConntrackBackend,
};
pub use endpoints::{EndpointInfo, EndpointSliceMap};
pub use error::{KubeProxyError, KubeProxyResult};
pub use healthcheck::HealthCheckServer;
pub use iptables::IptablesProxier;
pub use metrics::{KubeProxyMetrics, LatencyHistogram};
pub use nftables::NftablesProxier;
pub use nodeport::{DEFAULT_MAX_NODE_PORT, DEFAULT_MIN_NODE_PORT, NodePortAllocator};
pub use proxy_config::{DetectLocal, ProxyConfig, ProxyMode};
pub use service::{
    Cidr, IpCidr, Protocol, ServiceChangeTracker, ServicePortInfo, ServicePortName,
    SessionAffinity, TrafficPolicy,
};
pub use sync_runner::{BoundedFrequencyRunner, ProxySyncRunner};
pub use topology::{can_use_topology, categorize_endpoints, EndpointCategories};

pub const MODULE_NAME: &str = "kube-proxy";
