// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hetzner Cloud provider scaffold.
//!
//! Upstream: `hetznercloud/hcloud-cloud-controller-manager` @
//! [`PROVIDER_VERSION`]. Models the bits the controllers actually touch:
//!
//! * **Server type** — e.g. `cpx21`, `cax11`. Used for `instance_type`.
//! * **Location** — e.g. `fsn1` (Falkenstein). Used for `region`/`zone`.
//! * **Network** — private network UUID for `Routes`.
//! * **LoadBalancer** — `lb11` etc., one per Service.
//! * **FloatingIP** — public IPv4 attached to an LB or to a server directly.

use crate::provider::{
    CloudConfig, CloudProvider, InstancesIface, LoadBalancerIface, RoutesIface, ZonesIface,
};
use crate::types::{Cite, CloudError, ProviderName, TenantId};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// Pinned upstream provider release.
pub const PROVIDER_VERSION: &str = "v1.30.1";

/// Provider-id scheme used by upstream — `hcloud://<server-id>`.
pub const PROVIDER_ID_SCHEME: &str = "hcloud";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HetznerServer {
    pub id: u64,
    pub name: String,
    /// e.g. `"cpx21"`. Used as `node.kubernetes.io/instance-type`.
    pub server_type: String,
    /// e.g. `"fsn1"`. Used as `topology.kubernetes.io/region`.
    pub location: String,
    /// Hetzner exposes a single zone per location; mirror that.
    pub zone: String,
    pub network_id: Option<u64>,
}

impl HetznerServer {
    pub fn provider_id(&self) -> String {
        format!("{}://{}", PROVIDER_ID_SCHEME, self.id)
    }
}

#[derive(Debug)]
pub struct HetznerProvider {
    cfg: CloudConfig,
    servers: HashMap<String, HetznerServer>,
    routes: RefCell<Vec<String>>,
    lbs: RefCell<HashMap<String, String>>,
    floating_ip_pool: RefCell<Vec<String>>,
}

impl HetznerProvider {
    pub fn new(cfg: CloudConfig) -> Result<Self, CloudError> {
        if cfg.provider != ProviderName::Hetzner {
            return Err(CloudError::InvalidConfig {
                provider: cfg.provider,
                reason: "HetznerProvider requires ProviderName::Hetzner".into(),
            });
        }
        cfg.validate()?;
        Ok(Self {
            cfg,
            servers: HashMap::new(),
            routes: RefCell::new(Vec::new()),
            lbs: RefCell::new(HashMap::new()),
            floating_ip_pool: RefCell::new(vec![
                "203.0.113.10".into(),
                "203.0.113.11".into(),
                "203.0.113.12".into(),
            ]),
        })
    }

    pub fn upsert_server(&mut self, s: HetznerServer) {
        self.servers.insert(s.name.clone(), s);
    }

    pub fn server_count(&self) -> usize {
        self.servers.len()
    }
}

impl CloudProvider for HetznerProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Hetzner
    }
    fn config(&self) -> &CloudConfig {
        &self.cfg
    }
}

impl InstancesIface for HetznerProvider {
    fn provider_id(&self, tenant: &TenantId, node_name: &str) -> Result<String, CloudError> {
        self.authorise(tenant, "Server", node_name)?;
        self.servers
            .get(node_name)
            .map(|s| s.provider_id())
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("server {node_name} not found"),
            })
    }
    fn zone_for(&self, tenant: &TenantId, node_name: &str) -> Result<(String, String), CloudError> {
        self.authorise(tenant, "Server", node_name)?;
        let s = self.servers.get(node_name).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("server {node_name} not found"),
        })?;
        Ok((s.zone.clone(), s.location.clone()))
    }
}

impl LoadBalancerIface for HetznerProvider {
    fn ensure_lb(&self, tenant: &TenantId, service: &str) -> Result<String, CloudError> {
        self.authorise(tenant, "LoadBalancer", service)?;
        let mut lbs = self.lbs.borrow_mut();
        if let Some(ip) = lbs.get(service) {
            return Ok(ip.clone());
        }
        let mut pool = self.floating_ip_pool.borrow_mut();
        let ip = pool.pop().ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: "floating IP pool exhausted".into(),
        })?;
        lbs.insert(service.to_string(), ip.clone());
        Ok(ip)
    }
    fn delete_lb(&self, tenant: &TenantId, service: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "LoadBalancer", service)?;
        let mut lbs = self.lbs.borrow_mut();
        if let Some(ip) = lbs.remove(service) {
            self.floating_ip_pool.borrow_mut().push(ip);
        }
        Ok(())
    }
}

impl RoutesIface for HetznerProvider {
    fn list_routes(&self, tenant: &TenantId) -> Result<Vec<String>, CloudError> {
        self.authorise(tenant, "Network", &self.cfg.region)?;
        Ok(self.routes.borrow().clone())
    }
    fn create_route(&self, tenant: &TenantId, name: &str, _cidr: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "Network", name)?;
        self.routes.borrow_mut().push(name.into());
        Ok(())
    }
    fn delete_route(&self, tenant: &TenantId, name: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "Network", name)?;
        self.routes.borrow_mut().retain(|n| n != name);
        Ok(())
    }
}

impl ZonesIface for HetznerProvider {
    fn current_zone(&self, tenant: &TenantId) -> Result<String, CloudError> {
        self.authorise(tenant, "Zone", &self.cfg.region)?;
        Ok(self.cfg.region.clone())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::ext(
    "hetznercloud/hcloud-cloud-controller-manager",
    "hcloud/instances.go",
    "InstanceMetadata",
    PROVIDER_VERSION,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cfg(tenant: &str) -> CloudConfig {
        CloudConfig {
            tenant: TenantId::new(tenant).expect("test fixture"),
            provider: ProviderName::Hetzner,
            region: "fsn1".into(),
            credential_ref: "vault://kv/hcloud".into(),
        }
    }

    fn server(id: u64, name: &str) -> HetznerServer {
        HetznerServer {
            id,
            name: name.into(),
            server_type: "cpx21".into(),
            location: "fsn1".into(),
            zone: "fsn1-dc14".into(),
            network_id: Some(42),
        }
    }

    #[test]
    fn provider_id_uses_hcloud_scheme() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/instances.go",
            "InstanceMetadata",
            "acme"
        );
        let mut p = HetznerProvider::new(cfg("acme")).unwrap();
        p.upsert_server(server(7, "node-a"));
        assert_eq!(p.provider_id(&tenant, "node-a").unwrap(), "hcloud://7");
    }

    #[test]
    fn zone_returns_zone_and_location_pair() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/zones.go",
            "GetZone",
            "acme"
        );
        let mut p = HetznerProvider::new(cfg("acme")).unwrap();
        p.upsert_server(server(7, "node-a"));
        let (z, r) = p.zone_for(&tenant, "node-a").unwrap();
        assert_eq!(z, "fsn1-dc14");
        assert_eq!(r, "fsn1");
    }

    #[test]
    fn ensure_lb_pops_floating_ip_from_pool() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/load_balancers.go",
            "EnsureLoadBalancer",
            "acme"
        );
        let p = HetznerProvider::new(cfg("acme")).unwrap();
        let ip = p.ensure_lb(&tenant, "web").unwrap();
        assert!(ip.starts_with("203.0.113."));
        // Calling again is idempotent.
        assert_eq!(p.ensure_lb(&tenant, "web").unwrap(), ip);
    }

    #[test]
    fn delete_lb_returns_floating_ip_to_pool() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/load_balancers.go",
            "EnsureLoadBalancerDeleted",
            "acme"
        );
        let p = HetznerProvider::new(cfg("acme")).unwrap();
        let before = p.floating_ip_pool.borrow().len();
        let _ = p.ensure_lb(&tenant, "web").unwrap();
        p.delete_lb(&tenant, "web").unwrap();
        assert_eq!(p.floating_ip_pool.borrow().len(), before);
    }

    #[test]
    fn routes_round_trip_through_provider() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/routes.go",
            "CreateRoute",
            "acme"
        );
        let p = HetznerProvider::new(cfg("acme")).unwrap();
        p.create_route(&tenant, "acme-n1", "10.0.0.0/24").unwrap();
        p.create_route(&tenant, "acme-n2", "10.0.1.0/24").unwrap();
        assert_eq!(p.list_routes(&tenant).unwrap().len(), 2);
        p.delete_route(&tenant, "acme-n1").unwrap();
        assert_eq!(p.list_routes(&tenant).unwrap(), vec!["acme-n2".to_string()]);
    }

    #[test]
    fn cross_tenant_calls_are_refused() {
        let (_cite, attacker) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/load_balancers.go",
            "EnsureLoadBalancer",
            "tenant-attacker"
        );
        let p = HetznerProvider::new(cfg("acme")).unwrap();
        let err = p.ensure_lb(&attacker, "web").unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    #[test]
    fn rejects_construction_with_wrong_provider_name() {
        let (_cite, tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            PROVIDER_VERSION,
            "hcloud/cloud.go",
            "newCloud",
            "acme"
        );
        let _ = tenant;
        let mut bad = cfg("acme");
        bad.provider = ProviderName::Azure;
        assert!(HetznerProvider::new(bad).is_err());
    }
}
