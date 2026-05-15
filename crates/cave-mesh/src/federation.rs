// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-network mesh federation. Sits on top of
//! [`crate::multicluster`] (which manages cross-cluster service
//! discovery) and adds the *network* dimension — east-west
//! gateways that route traffic between Istio meshes spanning
//! different L3 networks.
//!
//! Mirrors `pilot/pkg/networking/federation/` from upstream
//! Istio:
//!
//! * `MeshNetwork` — one L3 network the mesh covers.
//! * `EastWestGateway` — the workload gateway through which
//!   inter-network traffic flows (Istio's `istio-eastwestgateway`
//!   deployment).
//! * `NetworkEndpoint` — a workload reachable from another
//!   network, addressed via its serving network's east-west
//!   gateway.
//! * `TrustBundle` — the CA cert bundle that proves the mesh
//!   identity; distributed across networks so SPIFFE IDs
//!   validate end-to-end.
//! * `FederationRegistry` — top-level state machine that
//!   enrols / lists / removes networks + endpoints + trust
//!   bundles and produces a `FederationView` snapshot for the
//!   xDS layer.
//!
//! ## Scope
//!
//! * Federation control-plane state — no actual data-plane
//!   forwarding; that lives in the bound Envoy / ambient proxy.
//! * Single-mesh, multi-network. Cross-mesh federation (where
//!   two separate Istio installs federate) is tracked, not in
//!   this batch.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use crate::error::MeshError;

/// One L3 network the federated mesh covers. Networks are the
/// boundary that requires explicit east-west routing; workloads
/// within a single network are reachable directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshNetwork {
    /// Stable name — e.g. "network1" / "us-east" / "vpc-prod".
    pub name: String,
    /// The cluster names belonging to this network (one
    /// network can span multiple clusters, e.g. control-plane
    /// + workload clusters on the same VPC).
    pub clusters: BTreeSet<String>,
    /// East-west gateway addresses (host:port) workloads in
    /// remote networks dial. Multi-address supported for HA.
    pub gateway_addresses: Vec<String>,
}

impl MeshNetwork {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            clusters: BTreeSet::new(),
            gateway_addresses: Vec::new(),
        }
    }

    pub fn with_cluster(mut self, cluster: impl Into<String>) -> Self {
        self.clusters.insert(cluster.into());
        self
    }

    pub fn with_gateway(mut self, address: impl Into<String>) -> Self {
        let addr = address.into();
        if !addr.is_empty() {
            self.gateway_addresses.push(addr);
        }
        self
    }
}

/// East-west gateway descriptor — one gateway workload sitting
/// at the boundary of a mesh network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EastWestGateway {
    pub network: String,
    pub address: String,
    /// Default Istio east-west port is 15443.
    pub port: u16,
    /// Marker for whether this gateway is the active load-
    /// balancer entry or a standby — `false` means the federation
    /// view should skip it for new connections.
    pub healthy: bool,
}

/// Endpoint a workload in network A advertises so workloads in
/// network B can route to it. Mirrors the upstream
/// `WorkloadEntry` flattened with the east-west gateway
/// resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEndpoint {
    /// Service name as seen by clients.
    pub service: String,
    /// Network this endpoint sits in.
    pub network: String,
    /// SPIFFE identity (`spiffe://<trust-domain>/ns/<ns>/sa/<sa>`).
    pub spiffe_id: String,
    /// Stable address inside the serving network — what clients
    /// in the *same* network would dial. Cross-network clients
    /// dial the east-west gateway, which then forwards here.
    pub workload_address: String,
    pub port: u16,
    /// Optional labels — used by traffic-shaping policy.
    pub labels: BTreeMap<String, String>,
}

/// Trust bundle for one trust domain — the set of root + sub
/// CAs that validate SPIFFE IDs from that domain. Federations
/// exchange these so end-to-end mTLS works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustBundle {
    pub trust_domain: String,
    /// PEM-encoded root certificates.
    pub root_pems: Vec<String>,
    /// Generation counter — incremented on every rotation.
    pub generation: u64,
}

impl TrustBundle {
    pub fn new(trust_domain: impl Into<String>) -> Self {
        Self {
            trust_domain: trust_domain.into(),
            root_pems: Vec::new(),
            generation: 0,
        }
    }

    pub fn rotate(&mut self, new_root_pems: Vec<String>) {
        self.root_pems = new_root_pems;
        self.generation = self.generation.saturating_add(1);
    }
}

/// Snapshot of the federation state — what the xDS / proxy
/// layer reads to program its routing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FederationView {
    pub networks: BTreeMap<String, MeshNetwork>,
    pub gateways: BTreeMap<String, EastWestGateway>,
    /// Keyed by `(service, network)` so the xDS builder can
    /// project per-network per-service routes.
    pub endpoints: BTreeMap<(String, String), Vec<NetworkEndpoint>>,
    pub trust_bundles: BTreeMap<String, TrustBundle>,
}

/// Top-level state machine. `&self` everywhere; interior
/// mutability via `RwLock` so multiple xDS push loops can read
/// without serialising.
#[derive(Default)]
pub struct FederationRegistry {
    inner: RwLock<FederationView>,
}

impl FederationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Networks ───────────────────────────────────────────────

    pub fn enroll_network(&self, network: MeshNetwork) -> Result<(), MeshError> {
        if network.name.is_empty() {
            return Err(MeshError::InvalidConfig("network name must not be empty".into()));
        }
        let mut g = self.inner.write().expect("poisoned");
        g.networks.insert(network.name.clone(), network);
        Ok(())
    }

    pub fn drop_network(&self, name: &str) {
        let mut g = self.inner.write().expect("poisoned");
        g.networks.remove(name);
        g.gateways.retain(|_, gw| gw.network != name);
        g.endpoints.retain(|(_, n), _| n != name);
    }

    pub fn list_networks(&self) -> Vec<MeshNetwork> {
        self.inner
            .read()
            .expect("poisoned")
            .networks
            .values()
            .cloned()
            .collect()
    }

    // ── Gateways ───────────────────────────────────────────────

    pub fn upsert_gateway(&self, gw: EastWestGateway) -> Result<(), MeshError> {
        if gw.network.is_empty() {
            return Err(MeshError::InvalidConfig("gateway network must not be empty".into()));
        }
        if gw.address.is_empty() {
            return Err(MeshError::InvalidConfig("gateway address must not be empty".into()));
        }
        let mut g = self.inner.write().expect("poisoned");
        if !g.networks.contains_key(&gw.network) {
            return Err(MeshError::InvalidConfig(format!(
                "gateway references unknown network: {}",
                gw.network
            )));
        }
        let key = format!("{}|{}", gw.network, gw.address);
        g.gateways.insert(key, gw);
        Ok(())
    }

    pub fn healthy_gateways_for(&self, network: &str) -> Vec<EastWestGateway> {
        self.inner
            .read()
            .expect("poisoned")
            .gateways
            .values()
            .filter(|gw| gw.network == network && gw.healthy)
            .cloned()
            .collect()
    }

    pub fn mark_gateway_unhealthy(&self, network: &str, address: &str) {
        let mut g = self.inner.write().expect("poisoned");
        let key = format!("{network}|{address}");
        if let Some(gw) = g.gateways.get_mut(&key) {
            gw.healthy = false;
        }
    }

    // ── Endpoints ──────────────────────────────────────────────

    pub fn publish_endpoint(&self, ep: NetworkEndpoint) -> Result<(), MeshError> {
        if ep.service.is_empty() {
            return Err(MeshError::InvalidConfig("endpoint service must not be empty".into()));
        }
        if ep.network.is_empty() {
            return Err(MeshError::InvalidConfig("endpoint network must not be empty".into()));
        }
        let mut g = self.inner.write().expect("poisoned");
        if !g.networks.contains_key(&ep.network) {
            return Err(MeshError::InvalidConfig(format!(
                "endpoint references unknown network: {}",
                ep.network
            )));
        }
        let key = (ep.service.clone(), ep.network.clone());
        let entry = g.endpoints.entry(key).or_default();
        // Replace any prior endpoint with the same SPIFFE id —
        // matches `WorkloadEntry`'s "one identity, one slot"
        // semantics.
        entry.retain(|e| e.spiffe_id != ep.spiffe_id);
        entry.push(ep);
        Ok(())
    }

    pub fn endpoints_for(&self, service: &str) -> Vec<NetworkEndpoint> {
        let g = self.inner.read().expect("poisoned");
        let mut out = Vec::new();
        for ((s, _), eps) in g.endpoints.iter() {
            if s == service {
                out.extend(eps.iter().cloned());
            }
        }
        out
    }

    pub fn retract_endpoint(&self, service: &str, network: &str, spiffe_id: &str) {
        let mut g = self.inner.write().expect("poisoned");
        let key = (service.to_string(), network.to_string());
        if let Some(eps) = g.endpoints.get_mut(&key) {
            eps.retain(|e| e.spiffe_id != spiffe_id);
            if eps.is_empty() {
                g.endpoints.remove(&key);
            }
        }
    }

    // ── Trust bundles ──────────────────────────────────────────

    pub fn upsert_trust_bundle(&self, bundle: TrustBundle) -> Result<(), MeshError> {
        if bundle.trust_domain.is_empty() {
            return Err(MeshError::InvalidConfig("trust domain must not be empty".into()));
        }
        let mut g = self.inner.write().expect("poisoned");
        g.trust_bundles
            .insert(bundle.trust_domain.clone(), bundle);
        Ok(())
    }

    pub fn trust_bundle(&self, trust_domain: &str) -> Option<TrustBundle> {
        self.inner
            .read()
            .expect("poisoned")
            .trust_bundles
            .get(trust_domain)
            .cloned()
    }

    /// Distribute (project) trust bundles to a destination
    /// network — returns the bundles the destination should
    /// know about. Today every bundle is shared with every
    /// network; in the future this can scope via opt-in trust
    /// policy.
    pub fn project_trust_bundles_to(&self, _destination_network: &str) -> Vec<TrustBundle> {
        self.inner
            .read()
            .expect("poisoned")
            .trust_bundles
            .values()
            .cloned()
            .collect()
    }

    // ── Snapshot ───────────────────────────────────────────────

    pub fn snapshot(&self) -> FederationView {
        self.inner.read().expect("poisoned").clone()
    }

    /// Cross-network endpoints — the xDS layer feeds these
    /// through the east-west gateway resolution. Returns each
    /// endpoint paired with the gateway address its consumers
    /// should dial.
    pub fn cross_network_endpoints(
        &self,
        consuming_network: &str,
    ) -> Vec<(NetworkEndpoint, Vec<String>)> {
        let g = self.inner.read().expect("poisoned");
        let mut out = Vec::new();
        for ((_, net), eps) in g.endpoints.iter() {
            if net == consuming_network {
                continue;
            }
            let gw_addresses: Vec<String> = g
                .gateways
                .values()
                .filter(|gw| gw.network == *net && gw.healthy)
                .map(|gw| format!("{}:{}", gw.address, gw.port))
                .collect();
            if gw_addresses.is_empty() {
                continue;
            }
            for ep in eps {
                out.push((ep.clone(), gw_addresses.clone()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ew_gw(network: &str, address: &str, healthy: bool) -> EastWestGateway {
        EastWestGateway {
            network: network.into(),
            address: address.into(),
            port: 15443,
            healthy,
        }
    }

    fn endpoint(service: &str, network: &str, id: &str) -> NetworkEndpoint {
        NetworkEndpoint {
            service: service.into(),
            network: network.into(),
            spiffe_id: format!("spiffe://example.com/ns/default/sa/{id}"),
            workload_address: format!("10.0.0.{}", id.bytes().last().unwrap_or(b'1')),
            port: 8080,
            labels: BTreeMap::new(),
        }
    }

    fn enroll(reg: &FederationRegistry, name: &str) {
        reg.enroll_network(MeshNetwork::new(name).with_cluster(format!("{name}-c1")))
            .unwrap();
    }

    #[test]
    fn enroll_network_round_trips() {
        let r = FederationRegistry::new();
        enroll(&r, "network1");
        let nets = r.list_networks();
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].name, "network1");
    }

    #[test]
    fn enroll_rejects_empty_name() {
        let r = FederationRegistry::new();
        assert!(r.enroll_network(MeshNetwork::new("")).is_err());
    }

    #[test]
    fn drop_network_cascades_to_gateways_and_endpoints() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.upsert_gateway(ew_gw("n1", "10.1.0.1", true)).unwrap();
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.drop_network("n1");
        assert!(r.list_networks().is_empty());
        assert!(r.healthy_gateways_for("n1").is_empty());
        assert!(r.endpoints_for("svc").is_empty());
    }

    #[test]
    fn upsert_gateway_requires_known_network() {
        let r = FederationRegistry::new();
        assert!(r.upsert_gateway(ew_gw("missing", "1.2.3.4", true)).is_err());
    }

    #[test]
    fn healthy_gateways_filters_by_health() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.upsert_gateway(ew_gw("n1", "10.1.0.1", true)).unwrap();
        r.upsert_gateway(ew_gw("n1", "10.1.0.2", false)).unwrap();
        let live = r.healthy_gateways_for("n1");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].address, "10.1.0.1");
    }

    #[test]
    fn mark_gateway_unhealthy_flips_state() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.upsert_gateway(ew_gw("n1", "10.1.0.1", true)).unwrap();
        r.mark_gateway_unhealthy("n1", "10.1.0.1");
        assert!(r.healthy_gateways_for("n1").is_empty());
    }

    #[test]
    fn publish_endpoint_round_trips() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        let eps = r.endpoints_for("svc");
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].network, "n1");
    }

    #[test]
    fn publish_endpoint_replaces_same_spiffe_id() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        let mut ep = endpoint("svc", "n1", "a");
        r.publish_endpoint(ep.clone()).unwrap();
        ep.workload_address = "10.0.0.99".into();
        r.publish_endpoint(ep).unwrap();
        let eps = r.endpoints_for("svc");
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].workload_address, "10.0.0.99");
    }

    #[test]
    fn publish_endpoint_keeps_distinct_spiffe_ids() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.publish_endpoint(endpoint("svc", "n1", "b")).unwrap();
        assert_eq!(r.endpoints_for("svc").len(), 2);
    }

    #[test]
    fn publish_endpoint_rejects_empty_fields() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        let mut ep = endpoint("svc", "n1", "a");
        ep.service = String::new();
        assert!(r.publish_endpoint(ep).is_err());
        let mut ep = endpoint("svc", "n1", "a");
        ep.network = String::new();
        assert!(r.publish_endpoint(ep).is_err());
    }

    #[test]
    fn publish_endpoint_unknown_network_errors() {
        let r = FederationRegistry::new();
        let ep = endpoint("svc", "ghost", "a");
        assert!(r.publish_endpoint(ep).is_err());
    }

    #[test]
    fn retract_endpoint_removes_only_matching_id() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.publish_endpoint(endpoint("svc", "n1", "b")).unwrap();
        r.retract_endpoint("svc", "n1", "spiffe://example.com/ns/default/sa/a");
        let remaining = r.endpoints_for("svc");
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].spiffe_id.ends_with("/sa/b"));
    }

    #[test]
    fn endpoints_for_collects_across_networks() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        enroll(&r, "n2");
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.publish_endpoint(endpoint("svc", "n2", "b")).unwrap();
        let eps = r.endpoints_for("svc");
        assert_eq!(eps.len(), 2);
        let networks: BTreeSet<_> = eps.iter().map(|e| e.network.clone()).collect();
        assert!(networks.contains("n1"));
        assert!(networks.contains("n2"));
    }

    #[test]
    fn upsert_trust_bundle_round_trips() {
        let r = FederationRegistry::new();
        r.upsert_trust_bundle(TrustBundle::new("example.com")).unwrap();
        let b = r.trust_bundle("example.com").unwrap();
        assert_eq!(b.generation, 0);
    }

    #[test]
    fn rotate_trust_bundle_bumps_generation() {
        let mut b = TrustBundle::new("example.com");
        b.rotate(vec!["-----BEGIN CERT----- ... -----END CERT-----".into()]);
        assert_eq!(b.generation, 1);
        b.rotate(vec!["new".into()]);
        assert_eq!(b.generation, 2);
    }

    #[test]
    fn trust_bundle_rejects_empty_domain() {
        let r = FederationRegistry::new();
        assert!(r.upsert_trust_bundle(TrustBundle::new("")).is_err());
    }

    #[test]
    fn project_trust_bundles_returns_every_bundle() {
        let r = FederationRegistry::new();
        for td in ["a.com", "b.com", "c.com"] {
            r.upsert_trust_bundle(TrustBundle::new(td)).unwrap();
        }
        let projected = r.project_trust_bundles_to("n1");
        assert_eq!(projected.len(), 3);
    }

    #[test]
    fn cross_network_endpoints_includes_other_networks_only() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        enroll(&r, "n2");
        r.upsert_gateway(ew_gw("n2", "10.2.0.1", true)).unwrap();
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.publish_endpoint(endpoint("svc", "n2", "b")).unwrap();
        let cross = r.cross_network_endpoints("n1");
        // n1 consumes — only n2-resident endpoints, dialled via
        // n2's east-west gateway.
        assert_eq!(cross.len(), 1);
        assert_eq!(cross[0].0.network, "n2");
        assert_eq!(cross[0].1, vec!["10.2.0.1:15443".to_string()]);
    }

    #[test]
    fn cross_network_endpoints_skips_networks_without_healthy_gateway() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        enroll(&r, "n2");
        // n2 has a gateway but it's unhealthy.
        r.upsert_gateway(ew_gw("n2", "10.2.0.1", false)).unwrap();
        r.publish_endpoint(endpoint("svc", "n2", "b")).unwrap();
        let cross = r.cross_network_endpoints("n1");
        assert!(cross.is_empty(), "no healthy gateway → no cross routing");
    }

    #[test]
    fn snapshot_reflects_state() {
        let r = FederationRegistry::new();
        enroll(&r, "n1");
        r.upsert_gateway(ew_gw("n1", "10.1.0.1", true)).unwrap();
        r.publish_endpoint(endpoint("svc", "n1", "a")).unwrap();
        r.upsert_trust_bundle(TrustBundle::new("example.com")).unwrap();
        let snap = r.snapshot();
        assert_eq!(snap.networks.len(), 1);
        assert_eq!(snap.gateways.len(), 1);
        assert_eq!(snap.endpoints.len(), 1);
        assert_eq!(snap.trust_bundles.len(), 1);
    }
}
