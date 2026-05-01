//! Azure provider resource catalog + tenant-scoped inventory.
//!
//! Extends [`super::azure::AzureProvider`] with the resource shapes the
//! controllers actually touch beyond the four-trait core surface, all pinned
//! to `kubernetes-sigs/cloud-provider-azure` @
//! [`super::azure::PROVIDER_VERSION`].
//!
//! * [`VmSku`] — Standard_D / E / F / B-series catalog with vCPU/memory.
//! * [`VmTier`] — Spot / Standard / Premium pricing tier.
//! * [`ManagedCluster`] + [`AgentPool`] — AKS shape with system / user pools.
//! * [`NsgRuleSpec`] — full NSG rule (priority / direction / action / proto).
//! * [`PublicIp`] — allocation method × SKU pair.
//! * [`StandardLoadBalancer`] — frontend / backend / rule trio.
//! * [`ManagedIdentity`] — System-assigned + User-assigned variants.
//! * [`ResourceGroup`] — name / location / tags lifecycle.
//!
//! Multi-tenancy: [`AzureInventory`] holds a [`TenantId`] and refuses any call
//! whose caller doesn't match.

use crate::providers::azure::LbSku;
use crate::types::{CloudError, ProviderName, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ─── VM SKUs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VmTier {
    Spot,
    Standard,
    Premium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkuFamily {
    GeneralPurpose,
    MemoryOptimized,
    ComputeOptimized,
    Burstable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VmSku {
    StandardD2sV5,
    StandardD4sV5,
    StandardD8sV5,
    StandardE4sV5,
    StandardE8sV5,
    StandardF4sV2,
    StandardF8sV2,
    StandardB2s,
    StandardB4ms,
}

impl VmSku {
    pub const fn name(self) -> &'static str {
        match self {
            VmSku::StandardD2sV5 => "Standard_D2s_v5",
            VmSku::StandardD4sV5 => "Standard_D4s_v5",
            VmSku::StandardD8sV5 => "Standard_D8s_v5",
            VmSku::StandardE4sV5 => "Standard_E4s_v5",
            VmSku::StandardE8sV5 => "Standard_E8s_v5",
            VmSku::StandardF4sV2 => "Standard_F4s_v2",
            VmSku::StandardF8sV2 => "Standard_F8s_v2",
            VmSku::StandardB2s => "Standard_B2s",
            VmSku::StandardB4ms => "Standard_B4ms",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "Standard_D2s_v5" => Some(VmSku::StandardD2sV5),
            "Standard_D4s_v5" => Some(VmSku::StandardD4sV5),
            "Standard_D8s_v5" => Some(VmSku::StandardD8sV5),
            "Standard_E4s_v5" => Some(VmSku::StandardE4sV5),
            "Standard_E8s_v5" => Some(VmSku::StandardE8sV5),
            "Standard_F4s_v2" => Some(VmSku::StandardF4sV2),
            "Standard_F8s_v2" => Some(VmSku::StandardF8sV2),
            "Standard_B2s" => Some(VmSku::StandardB2s),
            "Standard_B4ms" => Some(VmSku::StandardB4ms),
            _ => None,
        }
    }

    pub const fn vcpus(self) -> u32 {
        match self {
            VmSku::StandardD2sV5 | VmSku::StandardB2s => 2,
            VmSku::StandardD4sV5
            | VmSku::StandardE4sV5
            | VmSku::StandardF4sV2
            | VmSku::StandardB4ms => 4,
            VmSku::StandardD8sV5 | VmSku::StandardE8sV5 | VmSku::StandardF8sV2 => 8,
        }
    }

    pub const fn memory_gb(self) -> u32 {
        match self {
            VmSku::StandardD2sV5 => 8,
            VmSku::StandardD4sV5 => 16,
            VmSku::StandardD8sV5 => 32,
            VmSku::StandardE4sV5 => 32,
            VmSku::StandardE8sV5 => 64,
            VmSku::StandardF4sV2 => 8,
            VmSku::StandardF8sV2 => 16,
            VmSku::StandardB2s => 4,
            VmSku::StandardB4ms => 16,
        }
    }

    pub const fn family(self) -> SkuFamily {
        match self {
            VmSku::StandardD2sV5 | VmSku::StandardD4sV5 | VmSku::StandardD8sV5 => {
                SkuFamily::GeneralPurpose
            }
            VmSku::StandardE4sV5 | VmSku::StandardE8sV5 => SkuFamily::MemoryOptimized,
            VmSku::StandardF4sV2 | VmSku::StandardF8sV2 => SkuFamily::ComputeOptimized,
            VmSku::StandardB2s | VmSku::StandardB4ms => SkuFamily::Burstable,
        }
    }

    /// Spot is supported on every D / E / F SKU but not on the burstable
    /// B-series (Azure rejects creating a Spot VMSS over B-series).
    pub const fn supports_spot(self) -> bool {
        !matches!(self.family(), SkuFamily::Burstable)
    }
}

// ─── AKS managed cluster ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentPoolMode {
    System,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPool {
    pub name: String,
    pub vm_size: VmSku,
    pub tier: VmTier,
    pub mode: AgentPoolMode,
    pub node_count: u32,
    pub min_count: u32,
    pub max_count: u32,
}

impl AgentPool {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.min_count > self.max_count {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "agent pool {}: min_count {} > max_count {}",
                    self.name, self.min_count, self.max_count
                ),
            });
        }
        if self.node_count < self.min_count || self.node_count > self.max_count {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "agent pool {}: node_count {} outside [{}, {}]",
                    self.name, self.node_count, self.min_count, self.max_count
                ),
            });
        }
        if self.tier == VmTier::Spot && !self.vm_size.supports_spot() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "agent pool {}: SKU {} does not support Spot tier",
                    self.name,
                    self.vm_size.name()
                ),
            });
        }
        if self.mode == AgentPoolMode::System && self.tier == VmTier::Spot {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "agent pool {}: System mode pools must use Standard tier",
                    self.name
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCluster {
    pub name: String,
    pub resource_group: String,
    pub location: String,
    pub kubernetes_version: String,
    pub node_pools: Vec<AgentPool>,
}

impl ManagedCluster {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.kubernetes_version.starts_with('v') && !self.kubernetes_version.starts_with(char::is_numeric)
        {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("kubernetes_version {:?} not in vX.Y.Z form", self.kubernetes_version),
            });
        }
        if !self.node_pools.iter().any(|p| p.mode == AgentPoolMode::System) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("managed cluster {} has no System node pool", self.name),
            });
        }
        for p in &self.node_pools {
            p.validate()?;
        }
        Ok(())
    }
}

// ─── NSG rules ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NsgDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NsgAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NsgProtocol {
    Tcp,
    Udp,
    Icmp,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NsgRuleSpec {
    pub name: String,
    pub priority: u16,
    pub direction: NsgDirection,
    pub action: NsgAction,
    pub protocol: NsgProtocol,
    pub source_prefix: String,
    pub dest_port: String,
}

impl NsgRuleSpec {
    /// Azure permits priorities in `[100, 4096]` per network security group.
    pub fn validate(&self) -> Result<(), CloudError> {
        if !(100..=4096).contains(&self.priority) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "nsg rule {}: priority {} outside [100, 4096]",
                    self.name, self.priority
                ),
            });
        }
        if self.source_prefix.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("nsg rule {}: source_prefix must not be empty", self.name),
            });
        }
        if self.dest_port.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("nsg rule {}: dest_port must not be empty", self.name),
            });
        }
        Ok(())
    }
}

// ─── Public IP ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AllocationMethod {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicIp {
    pub id: String,
    pub address: String,
    pub sku: LbSku,
    pub allocation: AllocationMethod,
    pub resource_group: String,
}

// ─── Standard Load Balancer ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbBackendPool {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbRule {
    pub name: String,
    pub frontend_port: u16,
    pub backend_port: u16,
    pub protocol: NsgProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardLoadBalancer {
    pub name: String,
    pub resource_group: String,
    pub frontend_ip_id: String,
    pub backend_pools: Vec<LbBackendPool>,
    pub rules: Vec<LbRule>,
}

// ─── Managed Identity ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagedIdentity {
    SystemAssigned { principal_id: String },
    UserAssigned { resource_id: String, client_id: String },
}

impl ManagedIdentity {
    pub fn kind(&self) -> &'static str {
        match self {
            ManagedIdentity::SystemAssigned { .. } => "SystemAssigned",
            ManagedIdentity::UserAssigned { .. } => "UserAssigned",
        }
    }
}

// ─── Resource Group ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceGroup {
    pub name: String,
    pub location: String,
    pub tags: BTreeMap<String, String>,
}

// ─── Inventory ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct AzureInventory {
    tenant: TenantId,
    resource_groups: HashMap<String, ResourceGroup>,
    public_ips: HashMap<String, PublicIp>,
    nsg_rules: HashMap<String, NsgRuleSpec>,
    managed_clusters: HashMap<String, ManagedCluster>,
    user_identities: HashMap<String, ManagedIdentity>,
    load_balancers: HashMap<String, StandardLoadBalancer>,
}

impl AzureInventory {
    pub fn for_tenant(tenant: TenantId) -> Self {
        Self {
            tenant,
            resource_groups: HashMap::new(),
            public_ips: HashMap::new(),
            nsg_rules: HashMap::new(),
            managed_clusters: HashMap::new(),
            user_identities: HashMap::new(),
            load_balancers: HashMap::new(),
        }
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    fn check_tenant(
        &self,
        caller: &TenantId,
        kind: &'static str,
        name: &str,
    ) -> Result<(), CloudError> {
        if caller != &self.tenant {
            return Err(CloudError::TenantDenied {
                tenant: caller.clone(),
                kind,
                name: name.to_string(),
            });
        }
        Ok(())
    }

    // Resource groups

    pub fn create_resource_group(
        &mut self,
        caller: &TenantId,
        name: &str,
        location: &str,
        tags: BTreeMap<String, String>,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "ResourceGroup", name)?;
        if self.resource_groups.contains_key(name) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("resource group {name} already exists"),
            });
        }
        self.resource_groups.insert(
            name.to_string(),
            ResourceGroup { name: name.into(), location: location.into(), tags },
        );
        Ok(())
    }

    pub fn delete_resource_group(
        &mut self,
        caller: &TenantId,
        name: &str,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "ResourceGroup", name)?;
        self.public_ips.retain(|_, ip| ip.resource_group != name);
        self.load_balancers.retain(|_, lb| lb.resource_group != name);
        self.managed_clusters.retain(|_, c| c.resource_group != name);
        self.resource_groups.remove(name);
        Ok(())
    }

    pub fn resource_group(&self, name: &str) -> Option<&ResourceGroup> {
        self.resource_groups.get(name)
    }

    // Public IPs

    pub fn allocate_public_ip(
        &mut self,
        caller: &TenantId,
        id: &str,
        rg: &str,
        sku: LbSku,
        allocation: AllocationMethod,
    ) -> Result<String, CloudError> {
        self.check_tenant(caller, "PublicIP", id)?;
        if !self.resource_groups.contains_key(rg) {
            return Err(CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("resource group {rg} not found"),
            });
        }
        if sku == LbSku::Standard && allocation == AllocationMethod::Dynamic {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "Standard SKU public IPs require Static allocation".into(),
            });
        }
        let address = format!("203.0.115.{}", (self.public_ips.len() % 250) + 1);
        self.public_ips.insert(
            id.to_string(),
            PublicIp {
                id: id.into(),
                address: address.clone(),
                sku,
                allocation,
                resource_group: rg.into(),
            },
        );
        Ok(address)
    }

    pub fn public_ip(&self, id: &str) -> Option<&PublicIp> {
        self.public_ips.get(id)
    }

    // NSG rules

    pub fn add_nsg_rule(
        &mut self,
        caller: &TenantId,
        rule: NsgRuleSpec,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "NsgRule", &rule.name)?;
        rule.validate()?;
        if self.nsg_rules.values().any(|r| r.priority == rule.priority) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("nsg rule priority {} already in use", rule.priority),
            });
        }
        self.nsg_rules.insert(rule.name.clone(), rule);
        Ok(())
    }

    pub fn nsg_rule(&self, name: &str) -> Option<&NsgRuleSpec> {
        self.nsg_rules.get(name)
    }

    // AKS clusters

    pub fn create_managed_cluster(
        &mut self,
        caller: &TenantId,
        cluster: ManagedCluster,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "ManagedCluster", &cluster.name)?;
        if !self.resource_groups.contains_key(&cluster.resource_group) {
            return Err(CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("resource group {} not found", cluster.resource_group),
            });
        }
        cluster.validate()?;
        self.managed_clusters.insert(cluster.name.clone(), cluster);
        Ok(())
    }

    pub fn managed_cluster(&self, name: &str) -> Option<&ManagedCluster> {
        self.managed_clusters.get(name)
    }

    pub fn scale_node_pool(
        &mut self,
        caller: &TenantId,
        cluster: &str,
        pool: &str,
        node_count: u32,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "AgentPool", pool)?;
        let c = self.managed_clusters.get_mut(cluster).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Azure,
            reason: format!("managed cluster {cluster} not found"),
        })?;
        let p = c
            .node_pools
            .iter_mut()
            .find(|p| p.name == pool)
            .ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("agent pool {pool} not found in {cluster}"),
            })?;
        if node_count < p.min_count || node_count > p.max_count {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "node_count {node_count} outside [{}, {}] for pool {pool}",
                    p.min_count, p.max_count
                ),
            });
        }
        p.node_count = node_count;
        Ok(())
    }

    // Managed identities

    pub fn create_user_identity(
        &mut self,
        caller: &TenantId,
        name: &str,
        resource_id: &str,
        client_id: &str,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "ManagedIdentity", name)?;
        self.user_identities.insert(
            name.to_string(),
            ManagedIdentity::UserAssigned {
                resource_id: resource_id.into(),
                client_id: client_id.into(),
            },
        );
        Ok(())
    }

    pub fn user_identity(&self, name: &str) -> Option<&ManagedIdentity> {
        self.user_identities.get(name)
    }

    // Standard load balancers

    pub fn create_load_balancer(
        &mut self,
        caller: &TenantId,
        lb: StandardLoadBalancer,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "LoadBalancer", &lb.name)?;
        if !self.resource_groups.contains_key(&lb.resource_group) {
            return Err(CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("resource group {} not found", lb.resource_group),
            });
        }
        if !self.public_ips.contains_key(&lb.frontend_ip_id) {
            return Err(CloudError::Upstream {
                provider: ProviderName::Azure,
                reason: format!("frontend public ip {} not found", lb.frontend_ip_id),
            });
        }
        self.load_balancers.insert(lb.name.clone(), lb);
        Ok(())
    }

    pub fn load_balancer(&self, name: &str) -> Option<&StandardLoadBalancer> {
        self.load_balancers.get(name)
    }
}

// Stable read-only aggregates for tests / introspection.
impl AzureInventory {
    pub fn resource_group_count(&self) -> usize {
        self.resource_groups.len()
    }
    pub fn public_ip_count(&self) -> usize {
        self.public_ips.len()
    }
    pub fn nsg_rule_count(&self) -> usize {
        self.nsg_rules.len()
    }
    pub fn managed_cluster_count(&self) -> usize {
        self.managed_clusters.len()
    }
    pub fn user_identity_count(&self) -> usize {
        self.user_identities.len()
    }
    pub fn load_balancer_count(&self) -> usize {
        self.load_balancers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::azure::PROVIDER_VERSION;
    use crate::test_ctx;

    const REPO: &str = "kubernetes-sigs/cloud-provider-azure";

    fn tenant_ctx(tenant: &'static str, path: &'static str, symbol: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, symbol, tenant);
        assert_eq!(cite.repo, REPO);
        assert_eq!(cite.version, PROVIDER_VERSION);
        t
    }

    fn rg_with(inv: &mut AzureInventory, tenant: &TenantId, name: &str) {
        inv.create_resource_group(tenant, name, "westeurope", BTreeMap::new()).unwrap();
    }

    // ─── VM SKU tests ────────────────────────────────────────────────────────

    #[test]
    fn d_series_is_general_purpose() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        for sku in [VmSku::StandardD2sV5, VmSku::StandardD4sV5, VmSku::StandardD8sV5] {
            assert_eq!(sku.family(), SkuFamily::GeneralPurpose);
        }
    }

    #[test]
    fn e_series_is_memory_optimized() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        assert_eq!(VmSku::StandardE4sV5.family(), SkuFamily::MemoryOptimized);
        assert_eq!(VmSku::StandardE8sV5.family(), SkuFamily::MemoryOptimized);
        assert!(VmSku::StandardE4sV5.memory_gb() > VmSku::StandardD4sV5.memory_gb());
    }

    #[test]
    fn f_series_is_compute_optimized() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        assert_eq!(VmSku::StandardF4sV2.family(), SkuFamily::ComputeOptimized);
        assert_eq!(VmSku::StandardF8sV2.family(), SkuFamily::ComputeOptimized);
    }

    #[test]
    fn b_series_is_burstable_and_does_not_support_spot() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        assert_eq!(VmSku::StandardB2s.family(), SkuFamily::Burstable);
        assert!(!VmSku::StandardB2s.supports_spot());
        assert!(!VmSku::StandardB4ms.supports_spot());
        assert!(VmSku::StandardD2sV5.supports_spot());
    }

    #[test]
    fn vm_sku_round_trips_through_canonical_name() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        for sku in [
            VmSku::StandardD2sV5,
            VmSku::StandardE4sV5,
            VmSku::StandardF8sV2,
            VmSku::StandardB2s,
        ] {
            assert_eq!(VmSku::from_name(sku.name()), Some(sku));
        }
        assert!(VmSku::from_name("Standard_X1_v9").is_none());
    }

    #[test]
    fn vm_sku_vcpu_and_memory_match_family_expectations() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_vmss.go", "VMSize");
        assert_eq!(VmSku::StandardD2sV5.vcpus(), 2);
        assert_eq!(VmSku::StandardD2sV5.memory_gb(), 8);
        assert_eq!(VmSku::StandardE4sV5.memory_gb(), 32);
        assert_eq!(VmSku::StandardF4sV2.memory_gb(), 8);
    }

    // ─── Agent pool tests ────────────────────────────────────────────────────

    fn pool(name: &str, mode: AgentPoolMode, tier: VmTier, size: VmSku) -> AgentPool {
        AgentPool {
            name: name.into(),
            vm_size: size,
            tier,
            mode,
            node_count: 2,
            min_count: 1,
            max_count: 4,
        }
    }

    #[test]
    fn agent_pool_node_count_must_lie_within_min_max() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "AgentPool");
        let mut p = pool("system", AgentPoolMode::System, VmTier::Standard, VmSku::StandardD2sV5);
        p.node_count = 5;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agent_pool_min_must_be_lte_max() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "AgentPool");
        let mut p = pool("system", AgentPoolMode::System, VmTier::Standard, VmSku::StandardD2sV5);
        p.min_count = 8;
        p.max_count = 4;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn system_pool_cannot_use_spot_tier() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "AgentPoolMode");
        let p = pool("system", AgentPoolMode::System, VmTier::Spot, VmSku::StandardD2sV5);
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn user_pool_with_burstable_sku_rejects_spot_tier() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "AgentPool");
        let p = pool("workers", AgentPoolMode::User, VmTier::Spot, VmSku::StandardB2s);
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn user_pool_with_d_series_accepts_spot_tier() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "AgentPool");
        let p = pool("spot-workers", AgentPoolMode::User, VmTier::Spot, VmSku::StandardD4sV5);
        assert!(p.validate().is_ok());
    }

    // ─── Managed cluster tests ───────────────────────────────────────────────

    fn make_cluster(name: &str, rg: &str, with_system: bool) -> ManagedCluster {
        let mut pools = vec![pool(
            "workers",
            AgentPoolMode::User,
            VmTier::Standard,
            VmSku::StandardD4sV5,
        )];
        if with_system {
            pools.push(pool(
                "system",
                AgentPoolMode::System,
                VmTier::Standard,
                VmSku::StandardD2sV5,
            ));
        }
        ManagedCluster {
            name: name.into(),
            resource_group: rg.into(),
            location: "westeurope".into(),
            kubernetes_version: "1.30.4".into(),
            node_pools: pools,
        }
    }

    #[test]
    fn managed_cluster_requires_system_pool() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "ManagedCluster");
        let c = make_cluster("aks-acme", "rg-aks", false);
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_cluster_with_system_pool_validates() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "ManagedCluster");
        let c = make_cluster("aks-acme", "rg-aks", true);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn create_managed_cluster_requires_existing_rg() {
        let tenant = tenant_ctx(
            "acme",
            "pkg/provider/azure_managedclusters.go",
            "CreateOrUpdate",
        );
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        let c = make_cluster("aks-acme", "rg-aks", true);
        assert!(matches!(
            inv.create_managed_cluster(&tenant, c).unwrap_err(),
            CloudError::Upstream { .. }
        ));
    }

    #[test]
    fn create_managed_cluster_succeeds_when_rg_exists() {
        let tenant = tenant_ctx(
            "acme",
            "pkg/provider/azure_managedclusters.go",
            "CreateOrUpdate",
        );
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-aks");
        let c = make_cluster("aks-acme", "rg-aks", true);
        inv.create_managed_cluster(&tenant, c).unwrap();
        assert_eq!(inv.managed_cluster_count(), 1);
        assert_eq!(inv.managed_cluster("aks-acme").unwrap().location, "westeurope");
    }

    #[test]
    fn scale_node_pool_writes_through_to_cluster() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "Scale");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-aks");
        inv.create_managed_cluster(&tenant, make_cluster("aks-acme", "rg-aks", true)).unwrap();
        inv.scale_node_pool(&tenant, "aks-acme", "workers", 4).unwrap();
        let c = inv.managed_cluster("aks-acme").unwrap();
        let p = c.node_pools.iter().find(|p| p.name == "workers").unwrap();
        assert_eq!(p.node_count, 4);
    }

    #[test]
    fn scale_node_pool_outside_min_max_is_refused() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_managedclusters.go", "Scale");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-aks");
        inv.create_managed_cluster(&tenant, make_cluster("aks-acme", "rg-aks", true)).unwrap();
        let err = inv.scale_node_pool(&tenant, "aks-acme", "workers", 99).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── NSG rule tests ──────────────────────────────────────────────────────

    fn nsg(name: &str, prio: u16) -> NsgRuleSpec {
        NsgRuleSpec {
            name: name.into(),
            priority: prio,
            direction: NsgDirection::Inbound,
            action: NsgAction::Allow,
            protocol: NsgProtocol::Tcp,
            source_prefix: "0.0.0.0/0".into(),
            dest_port: "443".into(),
        }
    }

    #[test]
    fn nsg_priority_outside_100_4096_is_refused() {
        let _ = tenant_ctx(
            "acme",
            "pkg/provider/azure_securitygroup_repo.go",
            "ReconcileSecurityGroup",
        );
        assert!(matches!(nsg("low", 50).validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        assert!(matches!(
            nsg("high", 5000).validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        assert!(nsg("ok", 200).validate().is_ok());
    }

    #[test]
    fn nsg_rule_requires_source_prefix_and_port() {
        let _ = tenant_ctx(
            "acme",
            "pkg/provider/azure_securitygroup_repo.go",
            "ReconcileSecurityGroup",
        );
        let mut r = nsg("r", 200);
        r.source_prefix = String::new();
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let mut r = nsg("r", 200);
        r.dest_port = String::new();
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn nsg_priorities_must_be_unique_per_inventory() {
        let tenant = tenant_ctx(
            "acme",
            "pkg/provider/azure_securitygroup_repo.go",
            "ReconcileSecurityGroup",
        );
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        inv.add_nsg_rule(&tenant, nsg("a", 200)).unwrap();
        let err = inv.add_nsg_rule(&tenant, nsg("b", 200)).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── Public IP tests ─────────────────────────────────────────────────────

    #[test]
    fn standard_sku_public_ip_requires_static_allocation() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "Create");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-net");
        let err = inv
            .allocate_public_ip(&tenant, "ip-web", "rg-net", LbSku::Standard, AllocationMethod::Dynamic)
            .unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn standard_static_public_ip_returns_address() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "Create");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-net");
        let addr = inv
            .allocate_public_ip(&tenant, "ip-web", "rg-net", LbSku::Standard, AllocationMethod::Static)
            .unwrap();
        assert!(addr.starts_with("203.0.115."));
        assert_eq!(inv.public_ip("ip-web").unwrap().resource_group, "rg-net");
    }

    #[test]
    fn public_ip_creation_requires_existing_rg() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "Create");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        let err = inv
            .allocate_public_ip(&tenant, "ip", "rg-missing", LbSku::Standard, AllocationMethod::Static)
            .unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    // ─── Resource group tests ────────────────────────────────────────────────

    #[test]
    fn resource_group_cannot_be_created_twice() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_resourcegroupclient.go", "CreateOrUpdate");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-aks");
        let err = inv
            .create_resource_group(&tenant, "rg-aks", "westeurope", BTreeMap::new())
            .unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn resource_group_delete_cascades_owned_resources() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_resourcegroupclient.go", "Delete");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-aks");
        inv.allocate_public_ip(&tenant, "ip", "rg-aks", LbSku::Standard, AllocationMethod::Static)
            .unwrap();
        inv.create_managed_cluster(&tenant, make_cluster("aks-acme", "rg-aks", true)).unwrap();
        inv.delete_resource_group(&tenant, "rg-aks").unwrap();
        assert_eq!(inv.public_ip_count(), 0);
        assert_eq!(inv.managed_cluster_count(), 0);
        assert!(inv.resource_group("rg-aks").is_none());
    }

    #[test]
    fn resource_group_cross_tenant_create_is_refused() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_resourcegroupclient.go", "CreateOrUpdate");
        let attacker = TenantId::new("attacker").expect("test fixture");
        let mut inv = AzureInventory::for_tenant(tenant);
        let err = inv
            .create_resource_group(&attacker, "rg-aks", "westeurope", BTreeMap::new())
            .unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    #[test]
    fn resource_group_tags_round_trip() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_resourcegroupclient.go", "CreateOrUpdate");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        let mut tags = BTreeMap::new();
        tags.insert("env".into(), "prod".into());
        tags.insert("owner".into(), "platform".into());
        inv.create_resource_group(&tenant, "rg-aks", "westeurope", tags).unwrap();
        let rg = inv.resource_group("rg-aks").unwrap();
        assert_eq!(rg.tags.get("env").map(String::as_str), Some("prod"));
        assert_eq!(rg.tags.get("owner").map(String::as_str), Some("platform"));
    }

    // ─── Managed identity tests ──────────────────────────────────────────────

    #[test]
    fn user_assigned_identity_round_trips() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_identity.go", "UserAssignedIdentity");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        inv.create_user_identity(
            &tenant,
            "uami-aks",
            "/subscriptions/.../uami-aks",
            "00000000-0000-0000-0000-000000000001",
        )
        .unwrap();
        let id = inv.user_identity("uami-aks").unwrap();
        assert_eq!(id.kind(), "UserAssigned");
        match id {
            ManagedIdentity::UserAssigned { client_id, .. } => {
                assert!(client_id.starts_with("00000000-"));
            }
            _ => panic!("expected UserAssigned"),
        }
    }

    #[test]
    fn system_assigned_identity_reports_kind() {
        let _ = tenant_ctx("acme", "pkg/provider/azure_identity.go", "SystemAssignedIdentity");
        let id = ManagedIdentity::SystemAssigned { principal_id: "pid".into() };
        assert_eq!(id.kind(), "SystemAssigned");
    }

    #[test]
    fn managed_identity_cross_tenant_create_is_refused() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_identity.go", "UserAssignedIdentity");
        let attacker = TenantId::new("attacker").expect("test fixture");
        let mut inv = AzureInventory::for_tenant(tenant);
        let err = inv
            .create_user_identity(&attacker, "uami", "rid", "cid")
            .unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    // ─── Standard LB tests ───────────────────────────────────────────────────

    #[test]
    fn standard_lb_requires_existing_frontend_ip() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_loadbalancer.go", "EnsureLoadBalancer");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-net");
        let lb = StandardLoadBalancer {
            name: "lb-web".into(),
            resource_group: "rg-net".into(),
            frontend_ip_id: "ip-missing".into(),
            backend_pools: vec![LbBackendPool { name: "bp".into(), members: vec![] }],
            rules: vec![LbRule {
                name: "r".into(),
                frontend_port: 443,
                backend_port: 8443,
                protocol: NsgProtocol::Tcp,
            }],
        };
        let err = inv.create_load_balancer(&tenant, lb).unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn standard_lb_with_full_chain_succeeds() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure_loadbalancer.go", "EnsureLoadBalancer");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-net");
        inv.allocate_public_ip(&tenant, "ip-web", "rg-net", LbSku::Standard, AllocationMethod::Static)
            .unwrap();
        let lb = StandardLoadBalancer {
            name: "lb-web".into(),
            resource_group: "rg-net".into(),
            frontend_ip_id: "ip-web".into(),
            backend_pools: vec![LbBackendPool {
                name: "bp".into(),
                members: vec!["vmss-app_0".into(), "vmss-app_1".into()],
            }],
            rules: vec![LbRule {
                name: "https".into(),
                frontend_port: 443,
                backend_port: 8443,
                protocol: NsgProtocol::Tcp,
            }],
        };
        inv.create_load_balancer(&tenant, lb).unwrap();
        assert_eq!(inv.load_balancer_count(), 1);
        let lb = inv.load_balancer("lb-web").unwrap();
        assert_eq!(lb.backend_pools[0].members.len(), 2);
    }

    // ─── Misc multi-tenant tests ─────────────────────────────────────────────

    #[test]
    fn inventory_counts_track_inserts_across_kinds() {
        let tenant = tenant_ctx("acme", "pkg/provider/azure.go", "Cloud");
        let mut inv = AzureInventory::for_tenant(tenant.clone());
        rg_with(&mut inv, &tenant, "rg-net");
        inv.allocate_public_ip(&tenant, "ip", "rg-net", LbSku::Standard, AllocationMethod::Static)
            .unwrap();
        inv.add_nsg_rule(&tenant, nsg("r1", 200)).unwrap();
        inv.create_user_identity(&tenant, "uami", "rid", "cid").unwrap();
        assert_eq!(inv.resource_group_count(), 1);
        assert_eq!(inv.public_ip_count(), 1);
        assert_eq!(inv.nsg_rule_count(), 1);
        assert_eq!(inv.user_identity_count(), 1);
    }
}
