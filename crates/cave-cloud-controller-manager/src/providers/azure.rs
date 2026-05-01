//! Azure cloud provider scaffold.
//!
//! Upstream: `kubernetes-sigs/cloud-provider-azure` @ [`PROVIDER_VERSION`].
//! Models the bits the controllers actually touch:
//!
//! * **VM Scale Set** — VMSS instance backing an AKS node.
//! * **AKS node** — exposed via the VMSS instance ID.
//! * **NSG** — Network Security Group rules opened per Service.
//! * **Public IP** — Standard SKU, allocated per LoadBalancer.
//! * **Standard LB** — multi-IP load balancer.

use crate::provider::{
    CloudConfig, CloudProvider, InstancesIface, LoadBalancerIface, RoutesIface, ZonesIface,
};
use crate::types::{Cite, CloudError, ProviderName, TenantId};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// Pinned upstream provider release.
pub const PROVIDER_VERSION: &str = "v1.35.3";

/// Provider-id scheme used by upstream — `azure:///subscriptions/...` for ARM
/// resources. We keep the simpler `azure://<resource-id>` form for tests.
pub const PROVIDER_ID_SCHEME: &str = "azure";

/// SKU of the public IP / LB. Upstream supports Basic and Standard; v1.35
/// only Standard is in active maintenance — Basic is in preview deprecation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbSku {
    Standard,
    Basic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureNode {
    /// VMSS-style ID, e.g. `vmss-app_0`.
    pub vmss_instance_id: String,
    pub name: String,
    /// Azure VM size, e.g. `Standard_D4s_v5`.
    pub vm_size: String,
    /// Region, e.g. `westeurope`.
    pub location: String,
    /// Availability zone (`"1"`, `"2"`, `"3"` or empty for non-zonal regions).
    pub zone: String,
    /// Resource group of the VMSS.
    pub resource_group: String,
}

impl AzureNode {
    pub fn provider_id(&self) -> String {
        format!("{}://{}", PROVIDER_ID_SCHEME, self.vmss_instance_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NsgRule {
    pub service: String,
    pub port: u16,
    pub protocol: &'static str,
}

#[derive(Debug)]
pub struct AzureProvider {
    cfg: CloudConfig,
    nodes: HashMap<String, AzureNode>,
    routes: RefCell<Vec<String>>,
    /// Service → public-IP map.
    public_ips: RefCell<HashMap<String, String>>,
    nsg_rules: RefCell<Vec<NsgRule>>,
    pub lb_sku: LbSku,
    next_ip_octet: RefCell<u8>,
}

impl AzureProvider {
    pub fn new(cfg: CloudConfig) -> Result<Self, CloudError> {
        if cfg.provider != ProviderName::Azure {
            return Err(CloudError::InvalidConfig {
                provider: cfg.provider,
                reason: "AzureProvider requires ProviderName::Azure".into(),
            });
        }
        cfg.validate()?;
        Ok(Self {
            cfg,
            nodes: HashMap::new(),
            routes: RefCell::new(Vec::new()),
            public_ips: RefCell::new(HashMap::new()),
            nsg_rules: RefCell::new(Vec::new()),
            lb_sku: LbSku::Standard,
            next_ip_octet: RefCell::new(1),
        })
    }

    pub fn upsert_node(&mut self, n: AzureNode) {
        self.nodes.insert(n.name.clone(), n);
    }

    pub fn open_nsg_rule(&self, tenant: &TenantId, rule: NsgRule) -> Result<(), CloudError> {
        self.authorise(tenant, "NSG", &rule.service)?;
        self.nsg_rules.borrow_mut().push(rule);
        Ok(())
    }

    pub fn nsg_rules(&self) -> Vec<NsgRule> {
        self.nsg_rules.borrow().clone()
    }

    fn allocate_public_ip(&self) -> String {
        let mut octet = self.next_ip_octet.borrow_mut();
        let ip = format!("203.0.114.{}", *octet);
        *octet = octet.saturating_add(1);
        ip
    }
}

impl CloudProvider for AzureProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Azure
    }
    fn config(&self) -> &CloudConfig {
        &self.cfg
    }
}

impl InstancesIface for AzureProvider {
    fn provider_id(&self, tenant: &TenantId, node_name: &str) -> Result<String, CloudError> {
        self.authorise(tenant, "VMSSInstance", node_name)?;
        self.nodes
            .get(node_name)
            .map(|n| n.provider_id())
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("vmss instance {node_name} not found"),
            })
    }
    fn zone_for(&self, tenant: &TenantId, node_name: &str) -> Result<(String, String), CloudError> {
        self.authorise(tenant, "VMSSInstance", node_name)?;
        let n = self.nodes.get(node_name).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Azure,
            reason: format!("vmss instance {node_name} not found"),
        })?;
        Ok((n.zone.clone(), n.location.clone()))
    }
}

impl LoadBalancerIface for AzureProvider {
    fn ensure_lb(&self, tenant: &TenantId, service: &str) -> Result<String, CloudError> {
        self.authorise(tenant, "LoadBalancer", service)?;
        let mut ips = self.public_ips.borrow_mut();
        if let Some(ip) = ips.get(service) {
            return Ok(ip.clone());
        }
        if self.lb_sku != LbSku::Standard {
            // Basic is unsupported for new allocations in v1.35.
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "Basic SKU LB is not supported; use Standard".into(),
            });
        }
        let ip = self.allocate_public_ip();
        ips.insert(service.to_string(), ip.clone());
        Ok(ip)
    }
    fn delete_lb(&self, tenant: &TenantId, service: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "LoadBalancer", service)?;
        self.public_ips.borrow_mut().remove(service);
        self.nsg_rules.borrow_mut().retain(|r| r.service != service);
        Ok(())
    }
}

impl RoutesIface for AzureProvider {
    fn list_routes(&self, tenant: &TenantId) -> Result<Vec<String>, CloudError> {
        self.authorise(tenant, "RouteTable", &self.cfg.region)?;
        Ok(self.routes.borrow().clone())
    }
    fn create_route(&self, tenant: &TenantId, name: &str, _cidr: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "RouteTable", name)?;
        self.routes.borrow_mut().push(name.into());
        Ok(())
    }
    fn delete_route(&self, tenant: &TenantId, name: &str) -> Result<(), CloudError> {
        self.authorise(tenant, "RouteTable", name)?;
        self.routes.borrow_mut().retain(|n| n != name);
        Ok(())
    }
}

impl ZonesIface for AzureProvider {
    fn current_zone(&self, tenant: &TenantId) -> Result<String, CloudError> {
        self.authorise(tenant, "Zone", &self.cfg.region)?;
        Ok(self.cfg.region.clone())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::ext(
    "kubernetes-sigs/cloud-provider-azure",
    "pkg/provider/azure.go",
    "Cloud",
    PROVIDER_VERSION,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cfg(tenant: &str) -> CloudConfig {
        CloudConfig {
            tenant: TenantId::new(tenant).expect("test fixture"),
            provider: ProviderName::Azure,
            region: "westeurope".into(),
            credential_ref: "vault://kv/azure-sp".into(),
        }
    }

    fn node(id: &str, name: &str) -> AzureNode {
        AzureNode {
            vmss_instance_id: id.into(),
            name: name.into(),
            vm_size: "Standard_D4s_v5".into(),
            location: "westeurope".into(),
            zone: "1".into(),
            resource_group: "rg-aks".into(),
        }
    }

    #[test]
    fn provider_id_uses_azure_scheme() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_instances.go",
            "InstanceMetadata",
            "acme"
        );
        let mut p = AzureProvider::new(cfg("acme")).unwrap();
        p.upsert_node(node("vmss-app_0", "aks-app-0"));
        assert_eq!(p.provider_id(&tenant, "aks-app-0").unwrap(), "azure://vmss-app_0");
    }

    #[test]
    fn zone_returns_zone_and_region() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_zones.go",
            "GetZone",
            "acme"
        );
        let mut p = AzureProvider::new(cfg("acme")).unwrap();
        p.upsert_node(node("vmss-app_0", "aks-app-0"));
        let (z, r) = p.zone_for(&tenant, "aks-app-0").unwrap();
        assert_eq!(z, "1");
        assert_eq!(r, "westeurope");
    }

    #[test]
    fn ensure_lb_allocates_standard_sku_public_ip() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_loadbalancer.go",
            "EnsureLoadBalancer",
            "acme"
        );
        let p = AzureProvider::new(cfg("acme")).unwrap();
        let ip = p.ensure_lb(&tenant, "web").unwrap();
        assert!(ip.starts_with("203.0.114."));
        // Idempotent.
        assert_eq!(p.ensure_lb(&tenant, "web").unwrap(), ip);
    }

    #[test]
    fn basic_sku_lb_allocation_is_refused() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_loadbalancer.go",
            "ensureLoadBalancer",
            "acme"
        );
        let mut p = AzureProvider::new(cfg("acme")).unwrap();
        p.lb_sku = LbSku::Basic;
        let err = p.ensure_lb(&tenant, "web").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn delete_lb_removes_public_ip_and_nsg_rules() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_loadbalancer.go",
            "EnsureLoadBalancerDeleted",
            "acme"
        );
        let p = AzureProvider::new(cfg("acme")).unwrap();
        p.ensure_lb(&tenant, "web").unwrap();
        p.open_nsg_rule(&tenant, NsgRule { service: "web".into(), port: 443, protocol: "tcp" })
            .unwrap();
        assert_eq!(p.nsg_rules().len(), 1);
        p.delete_lb(&tenant, "web").unwrap();
        assert!(p.public_ips.borrow().get("web").is_none());
        assert!(p.nsg_rules().is_empty());
    }

    #[test]
    fn cross_tenant_nsg_open_is_refused() {
        let (_cite, attacker) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure_securitygroup_repo.go",
            "ReconcileSecurityGroup",
            "tenant-attacker"
        );
        let p = AzureProvider::new(cfg("acme")).unwrap();
        let err = p
            .open_nsg_rule(
                &attacker,
                NsgRule { service: "web".into(), port: 443, protocol: "tcp" },
            )
            .unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    #[test]
    fn rejects_construction_with_wrong_provider_name() {
        let (_cite, tenant) = test_ctx!(
            ext: "kubernetes-sigs/cloud-provider-azure",
            PROVIDER_VERSION,
            "pkg/provider/azure.go",
            "NewCloud",
            "acme"
        );
        let _ = tenant;
        let mut bad = cfg("acme");
        bad.provider = ProviderName::Hetzner;
        assert!(AzureProvider::new(bad).is_err());
    }
}
