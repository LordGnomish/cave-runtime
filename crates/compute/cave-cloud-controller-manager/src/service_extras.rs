// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service controller extras — beyond the basic ensure/update/delete loop.
//!
//! Mirrors the surface upstream's `controllers/service` reads from a Service:
//!
//! * `loadBalancerSourceRanges` — CIDR allow-list rendered as NSG / firewall
//!   rules at the cloud.
//! * `externalIPs` — extra ingress addresses kube-proxy publishes.
//! * `sessionAffinity` + `sessionAffinityConfig.timeoutSeconds`.
//! * `allocateLoadBalancerNodePorts` (beta in v1.20, GA in v1.24).
//! * `healthCheckNodePort` allocator.
//! * Internal-LB annotation (cloud-specific).
//! * `appProtocol` (per-port).
//! * Port name uniqueness.

use crate::route_controller::{CidrFamily, cidr_family, is_valid_cidr};
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Source ranges ───────────────────────────────────────────────────────────

/// CIDR list backing `loadBalancerSourceRanges`. Mirrors the upstream
/// validation in `pkg/api/service/util.go::GetLoadBalancerSourceRanges`.
pub fn validate_source_ranges(ranges: &[String]) -> Result<(), CloudError> {
    if ranges.is_empty() {
        return Ok(());
    }
    for r in ranges {
        if !is_valid_cidr(r) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("loadBalancerSourceRanges: {r:?} is not a valid CIDR"),
            });
        }
    }
    Ok(())
}

/// Returns the families present in the source-range list. Used to decide
/// whether a dual-stack LB needs both v4 and v6 firewall rules.
pub fn source_range_families(ranges: &[String]) -> (bool, bool) {
    let mut v4 = false;
    let mut v6 = false;
    for r in ranges {
        match cidr_family(r) {
            Some(CidrFamily::V4) => v4 = true,
            Some(CidrFamily::V6) => v6 = true,
            None => {}
        }
    }
    (v4, v6)
}

// ─── External IPs ────────────────────────────────────────────────────────────

/// Validate the `externalIPs` list. Upstream forbids loopback / multicast /
/// link-local addresses and de-duplicates — we mirror the dedup, but the
/// per-octet checks live here.
pub fn validate_external_ips(ips: &[String]) -> Result<(), CloudError> {
    for ip in ips {
        if ip.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "externalIPs entry must not be empty".into(),
            });
        }
        if ip.starts_with("127.") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("externalIPs entry {ip:?} is loopback"),
            });
        }
        if ip.starts_with("169.254.") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("externalIPs entry {ip:?} is link-local"),
            });
        }
        if ip.starts_with("224.") || ip.starts_with("239.") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("externalIPs entry {ip:?} is multicast"),
            });
        }
    }
    Ok(())
}

pub fn dedupe_external_ips(ips: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(ips.len());
    for ip in ips {
        if !seen.iter().any(|s| s == ip) {
            seen.push(ip.clone());
        }
    }
    seen
}

// ─── Session affinity ────────────────────────────────────────────────────────

/// Mirrors `core/v1.ServiceAffinity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionAffinity {
    None,
    ClientIP,
}

impl SessionAffinity {
    pub const fn key(self) -> &'static str {
        match self {
            SessionAffinity::None => "None",
            SessionAffinity::ClientIP => "ClientIP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAffinityConfig {
    pub kind: SessionAffinity,
    /// Seconds. Required for `ClientIP`. Upstream caps at 24h (`MaxSessionAffinityTimeoutSeconds`).
    pub timeout_seconds: Option<u32>,
}

impl SessionAffinityConfig {
    pub fn none() -> Self {
        Self {
            kind: SessionAffinity::None,
            timeout_seconds: None,
        }
    }

    pub fn client_ip(timeout_seconds: u32) -> Self {
        Self {
            kind: SessionAffinity::ClientIP,
            timeout_seconds: Some(timeout_seconds),
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        match (self.kind, self.timeout_seconds) {
            (SessionAffinity::None, _) => Ok(()),
            (SessionAffinity::ClientIP, None) => Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "ClientIP affinity requires timeout_seconds".into(),
            }),
            (SessionAffinity::ClientIP, Some(t)) if !(1..=86_400).contains(&t) => {
                Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("session affinity timeout {t} outside [1, 86400] s"),
                })
            }
            (SessionAffinity::ClientIP, Some(_)) => Ok(()),
        }
    }
}

// ─── Internal LB annotation ──────────────────────────────────────────────────

/// Annotation key for the Azure internal-LB toggle. Mirrors
/// `consts.ServiceAnnotationLoadBalancerInternal`.
pub const AZURE_INTERNAL_LB_ANNOTATION: &str =
    "service.beta.kubernetes.io/azure-load-balancer-internal";

/// Annotation key for the Hetzner private LB toggle. Mirrors
/// the `load-balancer.hetzner.cloud/private-ipv4` knob in
/// `hcloud-cloud-controller-manager`.
pub const HCLOUD_PRIVATE_LB_ANNOTATION: &str = "load-balancer.hetzner.cloud/private-ipv4";

/// True iff the given annotations request an internal-only load balancer.
pub fn is_internal_lb(annotations: &[(String, String)]) -> bool {
    annotations.iter().any(|(k, v)| {
        (k == AZURE_INTERNAL_LB_ANNOTATION && v == "true")
            || (k == HCLOUD_PRIVATE_LB_ANNOTATION && !v.is_empty())
    })
}

// ─── HealthCheckNodePort allocator ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NodePortAllocator {
    /// Allocator domain — upstream uses `--service-node-port-range`,
    /// default `30000-32767`.
    pub min: u16,
    pub max: u16,
    in_use: Vec<u16>,
}

impl NodePortAllocator {
    pub fn default_range() -> Self {
        Self {
            min: 30_000,
            max: 32_767,
            in_use: Vec::new(),
        }
    }

    pub fn with_range(min: u16, max: u16) -> Self {
        Self {
            min,
            max,
            in_use: Vec::new(),
        }
    }

    pub fn capacity(&self) -> u32 {
        if self.max < self.min {
            0
        } else {
            (self.max - self.min + 1) as u32
        }
    }

    pub fn allocate(&mut self) -> Result<u16, CloudError> {
        for p in self.min..=self.max {
            if !self.in_use.contains(&p) {
                self.in_use.push(p);
                return Ok(p);
            }
        }
        Err(CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "service-node-port range exhausted".into(),
        })
    }

    pub fn release(&mut self, port: u16) {
        self.in_use.retain(|p| *p != port);
    }

    pub fn reserve(&mut self, port: u16) -> Result<(), CloudError> {
        if !(self.min..=self.max).contains(&port) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("port {port} outside allocator range"),
            });
        }
        if self.in_use.contains(&port) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("port {port} already allocated"),
            });
        }
        self.in_use.push(port);
        Ok(())
    }

    pub fn used_count(&self) -> u32 {
        self.in_use.len() as u32
    }
}

// ─── AppProtocol ─────────────────────────────────────────────────────────────

/// Pre-defined `appProtocol` values understood by upstream. Cloud providers
/// use these to select TLS termination, HTTP routing, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppProtocol {
    Http,
    Https,
    Http2,
    Grpc,
    Tcp,
    Tls,
}

impl AppProtocol {
    pub const fn key(self) -> &'static str {
        match self {
            AppProtocol::Http => "http",
            AppProtocol::Https => "https",
            AppProtocol::Http2 => "http2",
            AppProtocol::Grpc => "kubernetes.io/grpc",
            AppProtocol::Tcp => "tcp",
            AppProtocol::Tls => "tls",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "http" => Some(AppProtocol::Http),
            "https" => Some(AppProtocol::Https),
            "http2" => Some(AppProtocol::Http2),
            "kubernetes.io/grpc" => Some(AppProtocol::Grpc),
            "tcp" => Some(AppProtocol::Tcp),
            "tls" => Some(AppProtocol::Tls),
            _ => None,
        }
    }
}

// ─── Port name uniqueness ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortName(pub String);

/// Validate a list of port names. Upstream requires distinct names across
/// all ports of a multi-port Service.
pub fn validate_port_names(names: &[String]) -> Result<(), CloudError> {
    let mut seen: Vec<&str> = Vec::new();
    for n in names {
        if n.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "port name must not be empty".into(),
            });
        }
        if n.len() > 15 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("port name {n:?} exceeds 15 characters"),
            });
        }
        if !n
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("port name {n:?} must be DNS-1123 lowercase"),
            });
        }
        if seen.contains(&n.as_str()) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("duplicate port name {n:?}"),
            });
        }
        seen.push(n);
    }
    Ok(())
}

// ─── AllocateLoadBalancerNodePorts ───────────────────────────────────────────

/// Whether upstream's optional `allocateLoadBalancerNodePorts` field is set
/// to `false`. When false, kube-proxy programs the LB target port directly
/// without going through a NodePort.
pub fn allocates_node_ports(value: Option<bool>) -> bool {
    // Default is true; only an explicit `false` disables.
    value.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
        assert!(!t.as_str().is_empty());
    }

    // ─── Source ranges ───────────────────────────────────────────────────────

    #[test]
    fn empty_source_range_list_validates() {
        ctx(
            "acme",
            "pkg/api/service/util.go",
            "GetLoadBalancerSourceRanges",
        );
        assert!(validate_source_ranges(&[]).is_ok());
    }

    #[test]
    fn source_ranges_must_all_be_valid_cidrs() {
        ctx(
            "acme",
            "pkg/api/service/util.go",
            "GetLoadBalancerSourceRanges",
        );
        let good: Vec<String> = vec!["10.0.0.0/8".into(), "192.168.0.0/16".into()];
        assert!(validate_source_ranges(&good).is_ok());
        let bad: Vec<String> = vec!["10.0.0.0/8".into(), "garbage".into()];
        assert!(matches!(
            validate_source_ranges(&bad).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn source_range_families_detects_dual_stack_input() {
        ctx(
            "acme",
            "pkg/api/service/util.go",
            "GetLoadBalancerSourceRanges",
        );
        let r = vec!["10.0.0.0/8".into(), "2001:db8::/32".into()];
        let (v4, v6) = source_range_families(&r);
        assert!(v4 && v6);
    }

    #[test]
    fn source_range_families_v4_only() {
        ctx(
            "acme",
            "pkg/api/service/util.go",
            "GetLoadBalancerSourceRanges",
        );
        let r = vec!["10.0.0.0/8".into()];
        let (v4, v6) = source_range_families(&r);
        assert!(v4 && !v6);
    }

    // ─── External IPs ────────────────────────────────────────────────────────

    #[test]
    fn external_ip_loopback_is_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateExternalIPs",
        );
        let err = validate_external_ips(&["127.0.0.1".into()]).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn external_ip_link_local_is_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateExternalIPs",
        );
        let err = validate_external_ips(&["169.254.169.254".into()]).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn external_ip_multicast_is_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateExternalIPs",
        );
        let err = validate_external_ips(&["224.0.0.1".into()]).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        let err = validate_external_ips(&["239.0.0.1".into()]).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn external_ip_global_unicast_is_accepted() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateExternalIPs",
        );
        assert!(validate_external_ips(&["203.0.113.1".into(), "198.51.100.5".into()]).is_ok());
    }

    #[test]
    fn external_ip_empty_string_is_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateExternalIPs",
        );
        let err = validate_external_ips(&["".into()]).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn dedupe_external_ips_preserves_first_occurrence() {
        ctx("acme", "pkg/api/service/util.go", "ExternalIPs");
        let out = dedupe_external_ips(&[
            "203.0.113.1".into(),
            "198.51.100.5".into(),
            "203.0.113.1".into(),
        ]);
        assert_eq!(
            out,
            vec!["203.0.113.1".to_string(), "198.51.100.5".to_string()]
        );
    }

    // ─── Session affinity ────────────────────────────────────────────────────

    #[test]
    fn session_affinity_keys_match_upstream() {
        ctx(
            "acme",
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServiceAffinity",
        );
        assert_eq!(SessionAffinity::None.key(), "None");
        assert_eq!(SessionAffinity::ClientIP.key(), "ClientIP");
    }

    #[test]
    fn none_affinity_does_not_require_timeout() {
        ctx(
            "acme",
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServiceAffinity",
        );
        assert!(SessionAffinityConfig::none().validate().is_ok());
    }

    #[test]
    fn client_ip_affinity_requires_timeout_seconds() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateSessionAffinityConfig",
        );
        let mut c = SessionAffinityConfig::client_ip(60);
        c.timeout_seconds = None;
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn client_ip_affinity_caps_timeout_at_24_hours() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateSessionAffinityConfig",
        );
        let c = SessionAffinityConfig::client_ip(86_401);
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn client_ip_affinity_zero_timeout_is_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateSessionAffinityConfig",
        );
        let c = SessionAffinityConfig::client_ip(0);
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn client_ip_affinity_within_bounds_validates() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateSessionAffinityConfig",
        );
        assert!(SessionAffinityConfig::client_ip(60).validate().is_ok());
        assert!(SessionAffinityConfig::client_ip(86_400).validate().is_ok());
    }

    // ─── Internal LB annotation ──────────────────────────────────────────────

    #[test]
    fn azure_internal_lb_annotation_matches_upstream_constant() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/api/well_known_annotations.go",
            "ServiceAnnotation",
        );
        assert_eq!(
            AZURE_INTERNAL_LB_ANNOTATION,
            "service.beta.kubernetes.io/azure-load-balancer-internal"
        );
        assert_eq!(
            HCLOUD_PRIVATE_LB_ANNOTATION,
            "load-balancer.hetzner.cloud/private-ipv4"
        );
    }

    #[test]
    fn is_internal_lb_recognises_azure_annotation() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/api/well_known_annotations.go",
            "ServiceAnnotation",
        );
        let ann = vec![(AZURE_INTERNAL_LB_ANNOTATION.into(), "true".into())];
        assert!(is_internal_lb(&ann));
    }

    #[test]
    fn is_internal_lb_ignores_false_value() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/api/well_known_annotations.go",
            "ServiceAnnotation",
        );
        let ann = vec![(AZURE_INTERNAL_LB_ANNOTATION.into(), "false".into())];
        assert!(!is_internal_lb(&ann));
    }

    #[test]
    fn is_internal_lb_recognises_hcloud_private_annotation() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/api/well_known_annotations.go",
            "ServiceAnnotation",
        );
        let ann = vec![(HCLOUD_PRIVATE_LB_ANNOTATION.into(), "10.0.0.1".into())];
        assert!(is_internal_lb(&ann));
    }

    #[test]
    fn is_internal_lb_returns_false_when_no_annotation() {
        ctx(
            "acme",
            "staging/src/k8s.io/cloud-provider/api/well_known_annotations.go",
            "ServiceAnnotation",
        );
        assert!(!is_internal_lb(&[]));
    }

    // ─── NodePort allocator ──────────────────────────────────────────────────

    #[test]
    fn node_port_allocator_default_range_matches_upstream() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "NewPortAllocator",
        );
        let a = NodePortAllocator::default_range();
        assert_eq!(a.min, 30_000);
        assert_eq!(a.max, 32_767);
        assert_eq!(a.capacity(), 2768);
    }

    #[test]
    fn node_port_allocate_returns_distinct_ports_in_order() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "AllocateNext",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_002);
        assert_eq!(a.allocate().unwrap(), 30_000);
        assert_eq!(a.allocate().unwrap(), 30_001);
        assert_eq!(a.allocate().unwrap(), 30_002);
        assert_eq!(a.used_count(), 3);
    }

    #[test]
    fn node_port_allocator_returns_error_when_range_exhausted() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "AllocateNext",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_001);
        a.allocate().unwrap();
        a.allocate().unwrap();
        let err = a.allocate().unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn node_port_release_returns_port_to_pool() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "Release",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_001);
        let p = a.allocate().unwrap();
        a.release(p);
        assert_eq!(a.allocate().unwrap(), 30_000);
        assert_eq!(a.used_count(), 1);
    }

    #[test]
    fn node_port_reserve_takes_specific_port() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "Allocate",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_010);
        a.reserve(30_005).unwrap();
        assert!(a.allocate().is_ok());
    }

    #[test]
    fn node_port_reserve_rejects_out_of_range() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "Allocate",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_010);
        let err = a.reserve(40_000).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn node_port_reserve_rejects_double_reservation() {
        ctx(
            "acme",
            "pkg/registry/core/service/portallocator/allocator.go",
            "Allocate",
        );
        let mut a = NodePortAllocator::with_range(30_000, 30_010);
        a.reserve(30_005).unwrap();
        let err = a.reserve(30_005).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── AppProtocol ─────────────────────────────────────────────────────────

    #[test]
    fn app_protocol_keys_match_well_known_strings() {
        ctx(
            "acme",
            "staging/src/k8s.io/api/core/v1/types.go",
            "AppProtocol",
        );
        assert_eq!(AppProtocol::Http.key(), "http");
        assert_eq!(AppProtocol::Https.key(), "https");
        assert_eq!(AppProtocol::Http2.key(), "http2");
        assert_eq!(AppProtocol::Grpc.key(), "kubernetes.io/grpc");
        assert_eq!(AppProtocol::Tcp.key(), "tcp");
        assert_eq!(AppProtocol::Tls.key(), "tls");
    }

    #[test]
    fn app_protocol_round_trips_through_from_key() {
        ctx(
            "acme",
            "staging/src/k8s.io/api/core/v1/types.go",
            "AppProtocol",
        );
        for p in [
            AppProtocol::Http,
            AppProtocol::Https,
            AppProtocol::Http2,
            AppProtocol::Grpc,
            AppProtocol::Tcp,
            AppProtocol::Tls,
        ] {
            assert_eq!(AppProtocol::from_key(p.key()), Some(p));
        }
        assert!(AppProtocol::from_key("nonsense").is_none());
    }

    // ─── Port names ──────────────────────────────────────────────────────────

    #[test]
    fn port_names_validate_with_distinct_dns1123_lowercase() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateServicePortName",
        );
        let names: Vec<String> = vec!["http".into(), "https".into(), "tcp-1".into()];
        assert!(validate_port_names(&names).is_ok());
    }

    #[test]
    fn port_name_must_not_be_empty() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateServicePortName",
        );
        let names: Vec<String> = vec!["".into()];
        assert!(matches!(
            validate_port_names(&names).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn port_name_must_not_exceed_15_characters() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateServicePortName",
        );
        let long = "abcdefghijklmnop".to_string(); // 16 chars
        assert!(matches!(
            validate_port_names(&[long]).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn port_names_reject_uppercase_and_underscores() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateServicePortName",
        );
        assert!(matches!(
            validate_port_names(&["HTTP".into()]).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        assert!(matches!(
            validate_port_names(&["with_underscore".into()]).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn duplicate_port_names_are_rejected() {
        ctx(
            "acme",
            "pkg/apis/core/validation/validation.go",
            "validateServicePortName",
        );
        let names: Vec<String> = vec!["http".into(), "http".into()];
        assert!(matches!(
            validate_port_names(&names).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    // ─── allocates_node_ports ────────────────────────────────────────────────

    #[test]
    fn allocates_node_ports_defaults_to_true() {
        ctx(
            "acme",
            "staging/src/k8s.io/api/core/v1/types.go",
            "AllocateLoadBalancerNodePorts",
        );
        assert!(allocates_node_ports(None));
        assert!(allocates_node_ports(Some(true)));
        assert!(!allocates_node_ports(Some(false)));
    }
}
