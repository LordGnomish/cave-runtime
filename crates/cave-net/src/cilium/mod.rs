//! Cilium-parity batch.
//!
//! Ports the Cilium model surface that ztunnel/waypoint/HBONE in
//! `cave-mesh` *integrate with* but don't themselves implement:
//!
//! * [`identity`] — numeric per-label-set identity allocator
//!   (`pkg/identity/cache/local.go`).
//! * [`l7policy`] — `CiliumNetworkPolicy` L7 evaluator covering HTTP
//!   (`pkg/policy/api/l7.go`), gRPC, DNS allow-lists and FQDN policy.
//! * [`clustermesh`] — multi-cluster identity exchange + service announce
//!   (`pkg/clustermesh/clustermesh.go`).
//! * [`hubble`] — flow log emission + drop-reason classification +
//!   topology graph (`pkg/hubble/parser/parser.go`).
//! * [`l7proxy`] — HTTP filter chain + mTLS terminate via SPIFFE
//!   (`pkg/proxy/proxy.go`).
//!
//! Every type carries a [`Cite`] pointing at the upstream symbol it ports;
//! every test in this subtree carries a `Cite` and a `TenantId` via the
//! module-local [`crate::cilium_test_ctx`] macro.

pub mod types;

pub mod auth;
pub mod bandwidth;
pub mod bgp;
pub mod bpfmaps;
pub mod cilium_node;
pub mod clustermesh;
pub mod clustermesh_ext;
pub mod conntrack;
pub mod dns_proxy;
pub mod egress;
pub mod endpoint;
pub mod endpoint_regen;
pub mod envoy;
pub mod external_workload;
pub mod fqdn;
pub mod gateway_filters;
pub mod health;
pub mod hubble;
pub mod hubble_ext;
pub mod hubble_metrics;
pub mod identity;
pub mod ingress;
pub mod ipam;
pub mod ipcache;
pub mod ipsec;
pub mod ipv6;
pub mod kafka;
pub mod l2_announce;
pub mod l7policy;
pub mod l7proxy;
pub mod lb;
pub mod lrp;
pub mod lb_ext;
pub mod maglev;
pub mod nat;
pub mod operator;
pub mod policy;
pub mod recorder;
pub mod services;
pub mod sock_lb;
pub mod srv6;
pub mod status;
pub mod tunnel;
pub mod wireguard;

pub use types::{Cite, TenantId, UPSTREAM_VERSION};
