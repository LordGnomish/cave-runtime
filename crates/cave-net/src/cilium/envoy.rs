// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Envoy XDS shapes — what cilium-agent pushes to the proxy.
//!
//! Mirrors `pkg/proxy/envoy/listener.go`, `pkg/proxy/envoy/cluster.go`,
//! `pkg/proxy/envoy/route.go`, `pkg/proxy/envoy/endpoint.go`, and the
//! NPDS (Network Policy Discovery Service) message in
//! `pkg/proxy/envoy/xds/np_resources.go`.
//!
//! Cilium drives envoy via xDS over a Unix domain socket. We model the
//! resource shapes (Listener / RouteConfiguration / Cluster /
//! ClusterLoadAssignment / TlsContext) plus the per-endpoint NPDS
//! snapshot — enough to assert that an L7 proxy redirect compiles into
//! the right resources for a given policy.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum L4ProtocolXds {
    Tcp,
    Udp,
}

/// Envoy listener — a port the proxy binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listener {
    pub name: String,
    pub address: IpAddr,
    pub port: u16,
    pub protocol: L4ProtocolXds,
    pub filter_chains: Vec<FilterChain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterChain {
    pub name: String,
    /// SNI hostnames the chain applies to. Empty = any SNI.
    pub server_names: Vec<String>,
    pub filters: Vec<NetworkFilter>,
    pub tls_context: Option<TlsContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkFilter {
    HttpConnectionManager { route_config: String },
    Kafka { rules: usize },
    TcpProxy { cluster: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsContext {
    pub sni: Option<String>,
    pub trust_domain: String,
    /// SDS secret reference (`namespace/secret-name`).
    pub sds_secret: Option<String>,
}

/// Envoy route configuration — host/path → cluster mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConfig {
    pub name: String,
    pub virtual_hosts: Vec<VirtualHost>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualHost {
    pub name: String,
    pub domains: Vec<String>,
    pub routes: Vec<RouteRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRule {
    pub match_prefix: Option<String>,
    pub match_path: Option<String>,
    pub cluster: String,
}

/// Envoy cluster — backend pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    pub name: String,
    pub lb_policy: ClusterLbPolicy,
    pub endpoints: Vec<ClusterEndpoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterLbPolicy {
    RoundRobin,
    LeastRequest,
    Maglev,
    Random,
    /// Cilium-specific consistent-ring like RingHash (used for affinity).
    RingHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterEndpoint {
    pub address: IpAddr,
    pub port: u16,
    pub weight: u32,
}

/// Per-endpoint NPDS snapshot — Cilium's policy push to envoy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpdsSnapshot {
    pub tenant: TenantId,
    pub endpoint_id: u64,
    pub listeners: Vec<Listener>,
    pub clusters: Vec<Cluster>,
    pub route_configs: Vec<RouteConfig>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum XdsError {
    #[error("listener `{0}` already exists")]
    DuplicateListener(String),
    #[error("cluster `{0}` already exists")]
    DuplicateCluster(String),
    #[error("listener `{0}` references unknown route_config `{1}`")]
    DanglingRouteConfig(String, String),
    #[error("route_config `{0}` references unknown cluster `{1}`")]
    DanglingCluster(String, String),
    #[error("tenant {tenant} cannot mutate XDS snapshot owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

impl NpdsSnapshot {
    pub fn new(tenant: TenantId, endpoint_id: u64) -> Self {
        Self {
            tenant, endpoint_id,
            listeners: Vec::new(),
            clusters: Vec::new(),
            route_configs: Vec::new(),
        }
    }

    pub fn add_listener(&mut self, l: Listener) -> Result<(), XdsError> {
        if self.listeners.iter().any(|x| x.name == l.name) {
            return Err(XdsError::DuplicateListener(l.name));
        }
        self.listeners.push(l);
        Ok(())
    }

    pub fn add_cluster(&mut self, c: Cluster) -> Result<(), XdsError> {
        if self.clusters.iter().any(|x| x.name == c.name) {
            return Err(XdsError::DuplicateCluster(c.name));
        }
        self.clusters.push(c);
        Ok(())
    }

    pub fn add_route_config(&mut self, r: RouteConfig) {
        // RouteConfig is treated as upsert — Envoy receives the latest.
        if let Some(idx) = self.route_configs.iter().position(|x| x.name == r.name) {
            self.route_configs[idx] = r;
        } else {
            self.route_configs.push(r);
        }
    }

    /// Verify that every listener references a known route-config and every
    /// route-config references a known cluster. Mirrors envoy's xDS
    /// validation pass.
    pub fn validate(&self) -> Result<(), XdsError> {
        for l in &self.listeners {
            for fc in &l.filter_chains {
                for f in &fc.filters {
                    if let NetworkFilter::HttpConnectionManager { route_config } = f {
                        if !self.route_configs.iter().any(|r| &r.name == route_config) {
                            return Err(XdsError::DanglingRouteConfig(l.name.clone(), route_config.clone()));
                        }
                    }
                    if let NetworkFilter::TcpProxy { cluster } = f {
                        if !self.clusters.iter().any(|c| &c.name == cluster) {
                            return Err(XdsError::DanglingCluster(l.name.clone(), cluster.clone()));
                        }
                    }
                }
            }
        }
        for r in &self.route_configs {
            for vh in &r.virtual_hosts {
                for rt in &vh.routes {
                    if !self.clusters.iter().any(|c| c.name == rt.cluster) {
                        return Err(XdsError::DanglingCluster(r.name.clone(), rt.cluster.clone()));
                    }
                }
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/proxy/envoy/listener.go", "GetListener");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn make_listener(name: &str, port: u16, route_config: &str) -> Listener {
        Listener {
            name: name.into(), address: ip(127, 0, 0, 1), port, protocol: L4ProtocolXds::Tcp,
            filter_chains: vec![FilterChain {
                name: format!("{name}-fc"),
                server_names: vec![],
                filters: vec![NetworkFilter::HttpConnectionManager { route_config: route_config.into() }],
                tls_context: None,
            }],
        }
    }

    fn make_cluster(name: &str) -> Cluster {
        Cluster {
            name: name.into(),
            lb_policy: ClusterLbPolicy::RoundRobin,
            endpoints: vec![ClusterEndpoint { address: ip(10, 0, 1, 1), port: 8080, weight: 1 }],
        }
    }

    fn make_route_config(name: &str, cluster: &str) -> RouteConfig {
        RouteConfig {
            name: name.into(),
            virtual_hosts: vec![VirtualHost {
                name: "default".into(),
                domains: vec!["*".into()],
                routes: vec![RouteRule { match_prefix: Some("/".into()), match_path: None, cluster: cluster.into() }],
            }],
        }
    }

    // ── add + uniqueness ─────────────────────────────────────────────────────

    #[test]
    fn xds_add_listener_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/listener.go", "AddListener", "tenant-xds-addl");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        assert_eq!(s.listeners.len(), 1);
    }

    #[test]
    fn xds_duplicate_listener_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/listener.go", "AddListener.Duplicate", "tenant-xds-dupl");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        let err = s.add_listener(make_listener("l1", 10002, "rc1")).unwrap_err();
        assert_eq!(err, XdsError::DuplicateListener("l1".into()));
    }

    #[test]
    fn xds_add_cluster_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/cluster.go", "AddCluster", "tenant-xds-addc");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_cluster(make_cluster("c1")).unwrap();
        assert_eq!(s.clusters.len(), 1);
    }

    #[test]
    fn xds_duplicate_cluster_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/cluster.go", "AddCluster.Duplicate", "tenant-xds-dupc");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_cluster(make_cluster("c1")).unwrap();
        let err = s.add_cluster(make_cluster("c1")).unwrap_err();
        assert_eq!(err, XdsError::DuplicateCluster("c1".into()));
    }

    #[test]
    fn xds_route_config_upserts_on_repeat_add() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/route.go", "UpsertRouteConfig", "tenant-xds-rcup");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_route_config(make_route_config("rc1", "c1"));
        let mut updated = make_route_config("rc1", "c2");
        updated.virtual_hosts[0].name = "updated".into();
        s.add_route_config(updated.clone());
        assert_eq!(s.route_configs.len(), 1);
        assert_eq!(s.route_configs[0], updated);
    }

    // ── validation ───────────────────────────────────────────────────────────

    #[test]
    fn xds_validate_passes_with_consistent_resources() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/xds.go", "Validate", "tenant-xds-val");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_cluster(make_cluster("c1")).unwrap();
        s.add_route_config(make_route_config("rc1", "c1"));
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        s.validate().unwrap();
    }

    #[test]
    fn xds_validate_rejects_listener_referencing_unknown_route_config() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/xds.go", "Validate.DanglingRC", "tenant-xds-drc");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_cluster(make_cluster("c1")).unwrap();
        s.add_listener(make_listener("l1", 10001, "rc-missing")).unwrap();
        let err = s.validate().unwrap_err();
        assert_eq!(err, XdsError::DanglingRouteConfig("l1".into(), "rc-missing".into()));
    }

    #[test]
    fn xds_validate_rejects_route_referencing_unknown_cluster() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/xds.go", "Validate.DanglingCluster", "tenant-xds-dc");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_route_config(make_route_config("rc1", "c-missing"));
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        let err = s.validate().unwrap_err();
        assert_eq!(err, XdsError::DanglingCluster("rc1".into(), "c-missing".into()));
    }

    // ── Filter chains ────────────────────────────────────────────────────────

    #[test]
    fn xds_filter_chain_with_sni_records_server_names() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/listener.go", "FilterChain.SNI", "tenant-xds-sni");
        let mut s = NpdsSnapshot::new(tenant, 1);
        let mut l = make_listener("l1", 10001, "rc1");
        l.filter_chains[0].server_names = vec!["api.example.com".into()];
        l.filter_chains[0].tls_context = Some(TlsContext {
            sni: Some("api.example.com".into()),
            trust_domain: "spiffe://cluster.local".into(),
            sds_secret: Some("default/api-cert".into()),
        });
        s.add_listener(l).unwrap();
        assert_eq!(s.listeners[0].filter_chains[0].server_names, vec!["api.example.com".to_string()]);
    }

    #[test]
    fn xds_listener_with_kafka_filter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/kafka.go", "KafkaFilter", "tenant-xds-kafka");
        let mut s = NpdsSnapshot::new(tenant, 1);
        let l = Listener {
            name: "kafka-l".into(), address: ip(127, 0, 0, 1), port: 9092, protocol: L4ProtocolXds::Tcp,
            filter_chains: vec![FilterChain {
                name: "kafka-fc".into(), server_names: vec![],
                filters: vec![NetworkFilter::Kafka { rules: 3 }],
                tls_context: None,
            }],
        };
        s.add_listener(l).unwrap();
        match &s.listeners[0].filter_chains[0].filters[0] {
            NetworkFilter::Kafka { rules } => assert_eq!(*rules, 3),
            _ => panic!("expected kafka filter"),
        }
    }

    #[test]
    fn xds_listener_with_tcp_proxy_filter_validates_cluster() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/listener.go", "TcpProxy", "tenant-xds-tcp");
        let mut s = NpdsSnapshot::new(tenant, 1);
        let l = Listener {
            name: "tcp-l".into(), address: ip(127, 0, 0, 1), port: 5432, protocol: L4ProtocolXds::Tcp,
            filter_chains: vec![FilterChain {
                name: "tcp-fc".into(), server_names: vec![],
                filters: vec![NetworkFilter::TcpProxy { cluster: "missing".into() }],
                tls_context: None,
            }],
        };
        s.add_listener(l).unwrap();
        let err = s.validate().unwrap_err();
        assert_eq!(err, XdsError::DanglingCluster("tcp-l".into(), "missing".into()));
    }

    // ── LB policy ────────────────────────────────────────────────────────────

    #[test]
    fn xds_cluster_with_maglev_lb_policy() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/cluster.go", "Cluster.LbPolicy.Maglev", "tenant-xds-mg");
        let mut s = NpdsSnapshot::new(tenant, 1);
        let mut c = make_cluster("c1");
        c.lb_policy = ClusterLbPolicy::Maglev;
        s.add_cluster(c).unwrap();
        assert_eq!(s.clusters[0].lb_policy, ClusterLbPolicy::Maglev);
    }

    #[test]
    fn xds_cluster_endpoints_carry_weight() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/endpoint.go", "ClusterEndpoint.Weight", "tenant-xds-wt");
        let mut s = NpdsSnapshot::new(tenant, 1);
        let mut c = make_cluster("c1");
        c.endpoints = vec![
            ClusterEndpoint { address: ip(10, 0, 1, 1), port: 80, weight: 9 },
            ClusterEndpoint { address: ip(10, 0, 1, 2), port: 80, weight: 1 },
        ];
        s.add_cluster(c).unwrap();
        assert_eq!(s.clusters[0].endpoints[0].weight, 9);
    }

    #[test]
    fn xds_cluster_lb_policy_serializes() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/envoy/cluster.go", "ClusterLbPolicy.Serde", "tenant-xds-lbserde");
        for p in [
            ClusterLbPolicy::RoundRobin,
            ClusterLbPolicy::LeastRequest,
            ClusterLbPolicy::Maglev,
            ClusterLbPolicy::Random,
            ClusterLbPolicy::RingHash,
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: ClusterLbPolicy = serde_json::from_str(&s).unwrap();
            assert_eq!(back, p);
        }
    }

    // ── Snapshot serde ───────────────────────────────────────────────────────

    #[test]
    fn xds_npds_snapshot_round_trips_serde() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/xds/np_resources.go", "Snapshot", "tenant-xds-serde");
        let mut s = NpdsSnapshot::new(tenant, 7);
        s.add_cluster(make_cluster("c1")).unwrap();
        s.add_route_config(make_route_config("rc1", "c1"));
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: NpdsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ── Default route ────────────────────────────────────────────────────────

    #[test]
    fn xds_route_config_with_multiple_virtual_hosts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/envoy/route.go", "RouteConfig.MultipleVH", "tenant-xds-mvh");
        let mut s = NpdsSnapshot::new(tenant, 1);
        s.add_cluster(make_cluster("c-api")).unwrap();
        s.add_cluster(make_cluster("c-www")).unwrap();
        let rc = RouteConfig {
            name: "rc1".into(),
            virtual_hosts: vec![
                VirtualHost {
                    name: "api".into(), domains: vec!["api.example.com".into()],
                    routes: vec![RouteRule { match_prefix: Some("/".into()), match_path: None, cluster: "c-api".into() }],
                },
                VirtualHost {
                    name: "www".into(), domains: vec!["www.example.com".into()],
                    routes: vec![RouteRule { match_prefix: Some("/".into()), match_path: None, cluster: "c-www".into() }],
                },
            ],
        };
        s.add_route_config(rc);
        s.add_listener(make_listener("l1", 10001, "rc1")).unwrap();
        s.validate().unwrap();
        assert_eq!(s.route_configs[0].virtual_hosts.len(), 2);
    }

    #[test]
    fn xds_route_rule_with_exact_path() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/envoy/route.go", "RouteRule.ExactPath", "tenant-xds-exact");
        let r = RouteRule { match_prefix: None, match_path: Some("/health".into()), cluster: "c-health".into() };
        let json = serde_json::to_string(&r).unwrap();
        let back: RouteRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn xds_filter_chain_tls_context_with_sds() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/envoy/listener.go", "TlsContext.SDS", "tenant-xds-sds");
        let tls = TlsContext {
            sni: None, trust_domain: "spiffe://cluster.local".into(),
            sds_secret: Some("default/api-cert".into()),
        };
        let json = serde_json::to_string(&tls).unwrap();
        let back: TlsContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tls);
        assert_eq!(back.sds_secret.as_deref(), Some("default/api-cert"));
    }
}
