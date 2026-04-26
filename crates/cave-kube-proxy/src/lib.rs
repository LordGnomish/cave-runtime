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

pub mod endpoints;
pub mod error;
pub mod healthcheck;
pub mod iptables;
pub mod nftables;
pub mod nodeport;
pub mod service;

pub use endpoints::{EndpointInfo, EndpointSliceMap};
pub use error::{KubeProxyError, KubeProxyResult};
pub use healthcheck::HealthCheckServer;
pub use iptables::IptablesProxier;
pub use nftables::NftablesProxier;
pub use nodeport::{NodePortAllocator, DEFAULT_MAX_NODE_PORT, DEFAULT_MIN_NODE_PORT};
pub use service::{
    Cidr, Protocol, ServiceChangeTracker, ServicePortInfo, ServicePortName, SessionAffinity,
    TrafficPolicy,
};

pub const MODULE_NAME: &str = "kube-proxy";
