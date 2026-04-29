//! Service topology + endpoint slice integration.
//!
//! Mirrors the upstream pieces in
//! `staging/src/k8s.io/cloud-provider/controllers/service` (topology
//! hints, endpoint-slice consumer) and `pkg/proxy` (TrafficDistribution).
//!
//! * **TrafficDistribution** — beta in v1.30, GA target v1.32.
//!   `PreferClose` keeps traffic in the same zone whenever possible.
//! * **EndpointSlice** consumer view — what addresses, ports, hints, and
//!   readiness flags the LB target list should be derived from.
//! * **Service variants** — Headless / ClusterIP / NodePort / LoadBalancer /
//!   ExternalName, with the constraints upstream enforces.
//! * **publishNotReadyAddresses** — when set, the LB targets all endpoints
//!   regardless of readiness (used by stateful workloads).

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── TrafficDistribution ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficDistribution {
    /// Keep traffic in the same topology zone whenever possible.
    PreferClose,
    /// Distribute uniformly across all endpoints (default v1.32 behaviour
    /// when the field is unset).
    Default,
}

impl TrafficDistribution {
    pub const fn key(self) -> &'static str {
        match self {
            TrafficDistribution::PreferClose => "PreferClose",
            TrafficDistribution::Default => "",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "" => Some(TrafficDistribution::Default),
            "PreferClose" => Some(TrafficDistribution::PreferClose),
            _ => None,
        }
    }
}

// ─── Topology hints ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForZone {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointHints {
    pub for_zones: Vec<ForZone>,
}

impl EndpointHints {
    pub fn empty() -> Self {
        Self { for_zones: Vec::new() }
    }

    /// True iff `caller_zone` is in the hint list, i.e. routing should
    /// keep traffic local. Mirrors `EndpointSliceCache.shouldUseHints`.
    pub fn matches_zone(&self, caller_zone: &str) -> bool {
        self.for_zones.iter().any(|z| z.name == caller_zone)
    }
}

// ─── EndpointSlice consumer view ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointConditions {
    pub ready: bool,
    pub serving: bool,
    pub terminating: bool,
}

impl EndpointConditions {
    pub fn ready() -> Self {
        Self { ready: true, serving: true, terminating: false }
    }
    pub fn terminating() -> Self {
        Self { ready: false, serving: true, terminating: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointEntry {
    pub addresses: Vec<String>,
    pub conditions: EndpointConditions,
    pub node_name: Option<String>,
    pub zone: Option<String>,
    pub hints: EndpointHints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSliceView {
    pub service: String,
    pub namespace: String,
    pub address_type: AddressType,
    pub endpoints: Vec<EndpointEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressType {
    Ipv4,
    Ipv6,
    Fqdn,
}

impl AddressType {
    pub const fn key(self) -> &'static str {
        match self {
            AddressType::Ipv4 => "IPv4",
            AddressType::Ipv6 => "IPv6",
            AddressType::Fqdn => "FQDN",
        }
    }
}

/// Filter endpoint slice entries to the set the LB should program.
/// Mirrors the loop in `endpointslicecache.GetEndpointAddresses`.
pub fn select_targets(
    slice: &EndpointSliceView,
    publish_not_ready: bool,
    traffic: TrafficDistribution,
    caller_zone: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let preferring_close = matches!(traffic, TrafficDistribution::PreferClose);
    let mut local_only = preferring_close && caller_zone.is_some();

    // First pass: when `local_only` is set, look for endpoints whose hint
    // matches the caller's zone. If we don't find any, fall back to the
    // global pool.
    if local_only {
        let any_local = slice.endpoints.iter().any(|e| {
            caller_zone
                .map(|z| e.hints.matches_zone(z))
                .unwrap_or(false)
        });
        if !any_local {
            local_only = false;
        }
    }

    for e in &slice.endpoints {
        let serving = if publish_not_ready {
            true
        } else {
            e.conditions.ready && e.conditions.serving
        };
        if !serving {
            continue;
        }
        if local_only {
            if let Some(z) = caller_zone {
                if !e.hints.matches_zone(z) {
                    continue;
                }
            }
        }
        for a in &e.addresses {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
    }
    out
}

// ─── Service variants ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIp,
    NodePort,
    LoadBalancer,
    ExternalName,
}

impl ServiceType {
    pub const fn key(self) -> &'static str {
        match self {
            ServiceType::ClusterIp => "ClusterIP",
            ServiceType::NodePort => "NodePort",
            ServiceType::LoadBalancer => "LoadBalancer",
            ServiceType::ExternalName => "ExternalName",
        }
    }
}

/// Mirrors the validation upstream's apiserver runs over a Service.
pub fn validate_service_type(
    service_type: ServiceType,
    cluster_ip: Option<&str>,
    external_name: Option<&str>,
) -> Result<(), CloudError> {
    match service_type {
        ServiceType::ClusterIp => {
            if external_name.is_some() {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: "ClusterIP service cannot set externalName".into(),
                });
            }
        }
        ServiceType::NodePort | ServiceType::LoadBalancer => {
            if cluster_ip == Some("None") {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("{:?} service cannot be headless", service_type),
                });
            }
        }
        ServiceType::ExternalName => {
            let name = external_name.unwrap_or("");
            if name.is_empty() {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: "ExternalName service requires externalName".into(),
                });
            }
            if !name.contains('.') {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!(
                        "ExternalName {name:?} must be a fully-qualified DNS name"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// True iff the Service is headless. Mirrors `IsHeadless` upstream.
pub fn is_headless(cluster_ip: Option<&str>) -> bool {
    cluster_ip == Some("None")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) {
        let (cite, _t) = test_ctx!(path, sym, tenant);
        assert_eq!(cite.repo, "kubernetes/kubernetes");
    }

    fn entry(addrs: Vec<&str>, ready: bool, zone: Option<&str>, hint_zones: Vec<&str>) -> EndpointEntry {
        EndpointEntry {
            addresses: addrs.into_iter().map(String::from).collect(),
            conditions: if ready {
                EndpointConditions::ready()
            } else {
                EndpointConditions { ready: false, serving: false, terminating: false }
            },
            node_name: Some("node-x".into()),
            zone: zone.map(String::from),
            hints: EndpointHints {
                for_zones: hint_zones
                    .into_iter()
                    .map(|n| ForZone { name: n.into() })
                    .collect(),
            },
        }
    }

    fn slice(eps: Vec<EndpointEntry>) -> EndpointSliceView {
        EndpointSliceView {
            service: "web".into(),
            namespace: "default".into(),
            address_type: AddressType::Ipv4,
            endpoints: eps,
        }
    }

    // ─── TrafficDistribution ─────────────────────────────────────────────────

    #[test]
    fn traffic_distribution_keys_match_upstream() {
        ctx("acme", "staging/src/k8s.io/api/core/v1/types.go", "TrafficDistribution");
        assert_eq!(TrafficDistribution::PreferClose.key(), "PreferClose");
        assert_eq!(TrafficDistribution::Default.key(), "");
    }

    #[test]
    fn traffic_distribution_round_trips_through_from_key() {
        ctx("acme", "staging/src/k8s.io/api/core/v1/types.go", "TrafficDistribution");
        assert_eq!(
            TrafficDistribution::from_key("PreferClose"),
            Some(TrafficDistribution::PreferClose)
        );
        assert_eq!(TrafficDistribution::from_key(""), Some(TrafficDistribution::Default));
        assert!(TrafficDistribution::from_key("nonsense").is_none());
    }

    // ─── EndpointHints ───────────────────────────────────────────────────────

    #[test]
    fn endpoint_hints_match_zone_returns_true_for_listed_zone() {
        ctx("acme", "staging/src/k8s.io/api/discovery/v1/types.go", "EndpointHints");
        let h = EndpointHints {
            for_zones: vec![ForZone { name: "fsn1".into() }, ForZone { name: "nbg1".into() }],
        };
        assert!(h.matches_zone("fsn1"));
        assert!(!h.matches_zone("hel1"));
    }

    #[test]
    fn empty_endpoint_hints_match_no_zone() {
        ctx("acme", "staging/src/k8s.io/api/discovery/v1/types.go", "EndpointHints");
        assert!(!EndpointHints::empty().matches_zone("fsn1"));
    }

    // ─── select_targets ──────────────────────────────────────────────────────

    #[test]
    fn select_targets_returns_ready_addresses_only() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, Some("fsn1"), vec![]),
            entry(vec!["10.0.0.2"], false, Some("fsn1"), vec![]),
        ]);
        let out = select_targets(&s, false, TrafficDistribution::Default, None);
        assert_eq!(out, vec!["10.0.0.1".to_string()]);
    }

    #[test]
    fn select_targets_with_publish_not_ready_returns_all() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, Some("fsn1"), vec![]),
            entry(vec!["10.0.0.2"], false, Some("fsn1"), vec![]),
        ]);
        let out = select_targets(&s, true, TrafficDistribution::Default, None);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn select_targets_dedupes_addresses() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, None, vec![]),
            entry(vec!["10.0.0.1"], true, None, vec![]),
        ]);
        let out = select_targets(&s, false, TrafficDistribution::Default, None);
        assert_eq!(out, vec!["10.0.0.1".to_string()]);
    }

    #[test]
    fn select_targets_prefer_close_keeps_zone_local_when_hinted() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "shouldUseHints");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, Some("fsn1"), vec!["fsn1"]),
            entry(vec!["10.0.0.2"], true, Some("nbg1"), vec!["nbg1"]),
        ]);
        let out = select_targets(&s, false, TrafficDistribution::PreferClose, Some("fsn1"));
        assert_eq!(out, vec!["10.0.0.1".to_string()]);
    }

    #[test]
    fn select_targets_prefer_close_falls_back_to_global_when_no_local_endpoint() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "shouldUseHints");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, Some("fsn1"), vec!["fsn1"]),
            entry(vec!["10.0.0.2"], true, Some("nbg1"), vec!["nbg1"]),
        ]);
        let out = select_targets(&s, false, TrafficDistribution::PreferClose, Some("hel1"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn select_targets_default_distribution_ignores_zone_hint() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let s = slice(vec![
            entry(vec!["10.0.0.1"], true, Some("fsn1"), vec!["fsn1"]),
            entry(vec!["10.0.0.2"], true, Some("nbg1"), vec!["nbg1"]),
        ]);
        let out = select_targets(&s, false, TrafficDistribution::Default, Some("fsn1"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn select_targets_drops_terminating_when_publish_not_ready_is_false() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let term = EndpointEntry {
            addresses: vec!["10.0.0.3".into()],
            conditions: EndpointConditions::terminating(),
            node_name: None,
            zone: None,
            hints: EndpointHints::empty(),
        };
        let s = slice(vec![entry(vec!["10.0.0.1"], true, None, vec![]), term]);
        let out = select_targets(&s, false, TrafficDistribution::Default, None);
        assert_eq!(out, vec!["10.0.0.1".to_string()]);
    }

    #[test]
    fn select_targets_publish_not_ready_includes_terminating() {
        ctx("acme", "pkg/proxy/endpointslicecache.go", "GetEndpointAddresses");
        let term = EndpointEntry {
            addresses: vec!["10.0.0.3".into()],
            conditions: EndpointConditions::terminating(),
            node_name: None,
            zone: None,
            hints: EndpointHints::empty(),
        };
        let s = slice(vec![term]);
        let out = select_targets(&s, true, TrafficDistribution::Default, None);
        assert_eq!(out, vec!["10.0.0.3".to_string()]);
    }

    // ─── AddressType ─────────────────────────────────────────────────────────

    #[test]
    fn address_type_keys_match_upstream() {
        ctx("acme", "staging/src/k8s.io/api/discovery/v1/types.go", "AddressType");
        assert_eq!(AddressType::Ipv4.key(), "IPv4");
        assert_eq!(AddressType::Ipv6.key(), "IPv6");
        assert_eq!(AddressType::Fqdn.key(), "FQDN");
    }

    // ─── Service type validation ─────────────────────────────────────────────

    #[test]
    fn service_type_keys_match_upstream() {
        ctx("acme", "staging/src/k8s.io/api/core/v1/types.go", "ServiceType");
        assert_eq!(ServiceType::ClusterIp.key(), "ClusterIP");
        assert_eq!(ServiceType::NodePort.key(), "NodePort");
        assert_eq!(ServiceType::LoadBalancer.key(), "LoadBalancer");
        assert_eq!(ServiceType::ExternalName.key(), "ExternalName");
    }

    #[test]
    fn cluster_ip_with_external_name_is_invalid() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "validateService");
        let err = validate_service_type(ServiceType::ClusterIp, None, Some("api.example.com"))
            .unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn nodeport_service_cannot_be_headless() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "validateService");
        let err = validate_service_type(ServiceType::NodePort, Some("None"), None).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn loadbalancer_service_cannot_be_headless() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "validateService");
        let err = validate_service_type(ServiceType::LoadBalancer, Some("None"), None).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn external_name_service_requires_dns_name() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "validateService");
        let err = validate_service_type(ServiceType::ExternalName, None, None).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        let err = validate_service_type(ServiceType::ExternalName, None, Some("nodot")).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        assert!(validate_service_type(ServiceType::ExternalName, None, Some("api.example.com"))
            .is_ok());
    }

    #[test]
    fn cluster_ip_headless_is_valid() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "validateService");
        assert!(validate_service_type(ServiceType::ClusterIp, Some("None"), None).is_ok());
    }

    #[test]
    fn is_headless_recognises_none_cluster_ip() {
        ctx("acme", "pkg/apis/core/validation/validation.go", "IsHeadless");
        assert!(is_headless(Some("None")));
        assert!(!is_headless(Some("10.0.0.1")));
        assert!(!is_headless(None));
    }

    // ─── EndpointConditions constructors ─────────────────────────────────────

    #[test]
    fn endpoint_conditions_ready_constructor_sets_flags() {
        ctx("acme", "staging/src/k8s.io/api/discovery/v1/types.go", "EndpointConditions");
        let c = EndpointConditions::ready();
        assert!(c.ready && c.serving && !c.terminating);
    }

    #[test]
    fn endpoint_conditions_terminating_constructor_sets_flags() {
        ctx("acme", "staging/src/k8s.io/api/discovery/v1/types.go", "EndpointConditions");
        let c = EndpointConditions::terminating();
        assert!(!c.ready && c.serving && c.terminating);
    }
}
