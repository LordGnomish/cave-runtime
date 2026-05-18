// SPDX-License-Identifier: AGPL-3.0-or-later
//! Azure provider extras — Application Gateway, AvailabilityZones,
//! Disk CSI shape, AAD Workload Identity.
//!
//! Upstream: `kubernetes-sigs/cloud-provider-azure` @
//! [`super::azure::PROVIDER_VERSION`]. These types are split out from
//! `azure_resources.rs` to keep that file readable; they cover the
//! "deeper" surface the user asked for in the parity sprint:
//!
//! * **AvailabilityZones** — `1` / `2` / `3`, plus the per-region table.
//! * **Application Gateway Ingress** — backend pools, listeners, rules,
//!   plus a public-IP front-end requirement.
//! * **NSG rule reconciliation** — a diff that turns a desired rule set
//!   into add / remove operations against an existing inventory.
//! * **Public IP standard tier** — extends the SKU model with idle timeout
//!   and zone-redundancy.
//! * **AvailabilityZones for the LB** — applied at allocate time.
//! * **Disk CSI side** — `ManagedDisk` / `DiskSku` / encryption mode.
//! * **AAD Workload Identity** — federated credential subjects (the
//!   replacement for the deprecated AAD Pod Identity).

use crate::providers::azure::LbSku;
use crate::providers::azure_resources::NsgRuleSpec;
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Availability Zones ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AvailabilityZone {
    Zone1,
    Zone2,
    Zone3,
}

impl AvailabilityZone {
    pub const fn key(self) -> &'static str {
        match self {
            AvailabilityZone::Zone1 => "1",
            AvailabilityZone::Zone2 => "2",
            AvailabilityZone::Zone3 => "3",
        }
    }
    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "1" => Some(AvailabilityZone::Zone1),
            "2" => Some(AvailabilityZone::Zone2),
            "3" => Some(AvailabilityZone::Zone3),
            _ => None,
        }
    }
}

/// Return the set of Availability Zones a region supports. Mirrors the
/// region-table upstream pulls from the ARM `ListLocations` reply.
/// Single-zone regions (e.g. `southindia`) return an empty Vec.
pub fn zones_for_region(region: &str) -> Vec<AvailabilityZone> {
    match region {
        "westeurope" | "northeurope" | "eastus" | "eastus2" | "westus2" | "westus3"
        | "centralus" | "uksouth" | "southeastasia" | "japaneast" => {
            vec![AvailabilityZone::Zone1, AvailabilityZone::Zone2, AvailabilityZone::Zone3]
        }
        "ukwest" | "japanwest" | "northcentralus" => vec![],
        _ => vec![],
    }
}

// ─── Application Gateway ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppGatewaySku {
    StandardV2,
    WafV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppGatewayProtocol {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppGwBackendPool {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppGwListener {
    pub name: String,
    pub protocol: AppGatewayProtocol,
    pub port: u16,
    pub host: Option<String>,
    pub tls_certificate_ref: Option<String>,
}

impl AppGwListener {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.protocol == AppGatewayProtocol::Https && self.tls_certificate_ref.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("listener {}: HTTPS requires tls_certificate_ref", self.name),
            });
        }
        if self.port == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("listener {}: port must be non-zero", self.name),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppGwRule {
    pub name: String,
    pub listener: String,
    pub backend_pool: String,
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppGateway {
    pub name: String,
    pub resource_group: String,
    pub sku: AppGatewaySku,
    pub frontend_public_ip_id: String,
    pub backend_pools: Vec<AppGwBackendPool>,
    pub listeners: Vec<AppGwListener>,
    pub rules: Vec<AppGwRule>,
}

impl AppGateway {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.frontend_public_ip_id.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("app gateway {}: frontend_public_ip_id required", self.name),
            });
        }
        if self.backend_pools.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("app gateway {}: at least one backend pool required", self.name),
            });
        }
        for l in &self.listeners {
            l.validate()?;
        }
        for r in &self.rules {
            if !self.listeners.iter().any(|l| l.name == r.listener) {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!(
                        "rule {} refers to unknown listener {}",
                        r.name, r.listener
                    ),
                });
            }
            if !self.backend_pools.iter().any(|b| b.name == r.backend_pool) {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!(
                        "rule {} refers to unknown backend pool {}",
                        r.name, r.backend_pool
                    ),
                });
            }
        }
        Ok(())
    }
}

// ─── NSG reconcile ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NsgDiff {
    pub add: Vec<NsgRuleSpec>,
    pub remove: Vec<String>,
    pub update: Vec<NsgRuleSpec>,
}

impl NsgDiff {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty() && self.update.is_empty()
    }
    pub fn write_count(&self) -> u32 {
        (self.add.len() + self.remove.len() + self.update.len()) as u32
    }
}

/// Compute the NSG diff between the current rule set and the desired state.
/// Mirrors the diff loop in `azure_securitygroup_repo.go::reconcileSecurityGroup`.
pub fn diff_nsg_rules(current: &[NsgRuleSpec], desired: &[NsgRuleSpec]) -> NsgDiff {
    let mut add = Vec::new();
    let mut update = Vec::new();
    for d in desired {
        match current.iter().find(|c| c.name == d.name) {
            None => add.push(d.clone()),
            Some(c) if c != d => update.push(d.clone()),
            Some(_) => {}
        }
    }
    let remove: Vec<String> = current
        .iter()
        .filter(|c| !desired.iter().any(|d| d.name == c.name))
        .map(|c| c.name.clone())
        .collect();
    NsgDiff { add, remove, update }
}

// ─── Public IP standard-tier extras ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardPublicIpProps {
    pub idle_timeout_minutes: u32,
    pub zone_redundant: bool,
    pub zones: Vec<AvailabilityZone>,
}

impl StandardPublicIpProps {
    pub fn default_for_zonal_region() -> Self {
        Self {
            idle_timeout_minutes: 4,
            zone_redundant: true,
            zones: vec![
                AvailabilityZone::Zone1,
                AvailabilityZone::Zone2,
                AvailabilityZone::Zone3,
            ],
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(4..=30).contains(&self.idle_timeout_minutes) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "public IP idle_timeout_minutes {} outside [4, 30]",
                    self.idle_timeout_minutes
                ),
            });
        }
        if self.zone_redundant && self.zones.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "zone_redundant=true requires at least one zone".into(),
            });
        }
        if !self.zones.is_empty() && self.zones.iter().any(|z| z.key().is_empty()) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "zone keys must be non-empty".into(),
            });
        }
        Ok(())
    }
}

// ─── Managed Disk (CSI side) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiskSku {
    StandardLrs,
    StandardSsdLrs,
    PremiumLrs,
    UltraSsdLrs,
}

impl DiskSku {
    pub const fn name(self) -> &'static str {
        match self {
            DiskSku::StandardLrs => "Standard_LRS",
            DiskSku::StandardSsdLrs => "StandardSSD_LRS",
            DiskSku::PremiumLrs => "Premium_LRS",
            DiskSku::UltraSsdLrs => "UltraSSD_LRS",
        }
    }
    pub const fn supports_shared_disk(self) -> bool {
        matches!(self, DiskSku::PremiumLrs | DiskSku::UltraSsdLrs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiskEncryption {
    PlatformManaged,
    CustomerManaged,
    DoubleEncryption,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedDisk {
    pub name: String,
    pub resource_group: String,
    pub size_gb: u32,
    pub sku: DiskSku,
    pub encryption: DiskEncryption,
    pub zone: Option<AvailabilityZone>,
    pub attached_node: Option<String>,
}

impl ManagedDisk {
    pub fn new(name: &str, rg: &str, size_gb: u32, sku: DiskSku) -> Self {
        Self {
            name: name.into(),
            resource_group: rg.into(),
            size_gb,
            sku,
            encryption: DiskEncryption::PlatformManaged,
            zone: None,
            attached_node: None,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(1..=65536).contains(&self.size_gb) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("disk size {} outside [1, 65536] GiB", self.size_gb),
            });
        }
        if self.sku == DiskSku::UltraSsdLrs && self.zone.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "UltraSSD disks must be created in an Availability Zone".into(),
            });
        }
        Ok(())
    }

    /// `True` iff the disk can be attached to multiple nodes simultaneously
    /// (`maxShares > 1`). Mirrors the `enableSharedDisk` upstream toggle.
    pub fn supports_multi_attach(&self) -> bool {
        self.sku.supports_shared_disk()
    }

    pub fn attach(&mut self, node: &str) -> Result<(), CloudError> {
        if let Some(existing) = &self.attached_node {
            if existing != node && !self.supports_multi_attach() {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!(
                        "disk {} already attached to {}",
                        self.name, existing
                    ),
                });
            }
        }
        self.attached_node = Some(node.into());
        Ok(())
    }

    pub fn detach(&mut self) {
        self.attached_node = None;
    }
}

// ─── AAD Workload Identity ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadIdentityFederation {
    pub name: String,
    pub user_assigned_identity: String,
    /// `system:serviceaccount:<ns>:<sa>` — the federation subject.
    pub service_account_subject: String,
    /// OIDC issuer URL of the AKS cluster.
    pub issuer_url: String,
    pub audience: String,
}

impl WorkloadIdentityFederation {
    pub fn for_service_account(
        name: &str,
        identity: &str,
        namespace: &str,
        service_account: &str,
        issuer_url: &str,
    ) -> Self {
        Self {
            name: name.into(),
            user_assigned_identity: identity.into(),
            service_account_subject: format!(
                "system:serviceaccount:{namespace}:{service_account}"
            ),
            issuer_url: issuer_url.into(),
            audience: "api://AzureADTokenExchange".into(),
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.service_account_subject.starts_with("system:serviceaccount:") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "workload identity {}: subject must start with system:serviceaccount:",
                    self.name
                ),
            });
        }
        let parts: Vec<&str> = self.service_account_subject.splitn(4, ':').collect();
        if parts.len() != 4 || parts[2].is_empty() || parts[3].is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "workload identity {}: subject must include namespace and SA",
                    self.name
                ),
            });
        }
        if !self.issuer_url.starts_with("https://") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "workload identity {}: issuer must be HTTPS",
                    self.name
                ),
            });
        }
        if self.audience.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("workload identity {}: audience required", self.name),
            });
        }
        Ok(())
    }
}

/// Helper that exposes the LB SKU check so callers can import everything
/// they need from this module without reaching into `azure.rs`.
pub fn lb_sku_is_standard(sku: LbSku) -> bool {
    sku == LbSku::Standard
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::azure::PROVIDER_VERSION;
    use crate::providers::azure_resources::{NsgAction, NsgDirection, NsgProtocol, NsgRuleSpec};
    use crate::test_ctx;
    use crate::types::TenantId;

    const REPO: &str = "kubernetes-sigs/cloud-provider-azure";

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, sym, tenant);
        assert_eq!(cite.repo, REPO);
        t
    }

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

    // ─── Availability Zones ──────────────────────────────────────────────────

    #[test]
    fn availability_zone_keys_match_arm_strings() {
        let _ = ctx("acme", "pkg/provider/azure_zones.go", "AvailabilityZone");
        assert_eq!(AvailabilityZone::Zone1.key(), "1");
        assert_eq!(AvailabilityZone::Zone2.key(), "2");
        assert_eq!(AvailabilityZone::Zone3.key(), "3");
        assert_eq!(AvailabilityZone::from_key("2"), Some(AvailabilityZone::Zone2));
        assert!(AvailabilityZone::from_key("5").is_none());
    }

    #[test]
    fn westeurope_supports_three_zones() {
        let _ = ctx("acme", "pkg/provider/azure_zones.go", "AvailabilityZone");
        let z = zones_for_region("westeurope");
        assert_eq!(z.len(), 3);
    }

    #[test]
    fn single_zone_regions_return_empty_zone_set() {
        let _ = ctx("acme", "pkg/provider/azure_zones.go", "AvailabilityZone");
        assert!(zones_for_region("ukwest").is_empty());
        assert!(zones_for_region("japanwest").is_empty());
        assert!(zones_for_region("unknown-region").is_empty());
    }

    // ─── Application Gateway ─────────────────────────────────────────────────

    fn appgw(rg: &str) -> AppGateway {
        AppGateway {
            name: "agw".into(),
            resource_group: rg.into(),
            sku: AppGatewaySku::WafV2,
            frontend_public_ip_id: "ip-agw".into(),
            backend_pools: vec![AppGwBackendPool { name: "bp".into(), members: vec![] }],
            listeners: vec![AppGwListener {
                name: "http".into(),
                protocol: AppGatewayProtocol::Http,
                port: 80,
                host: None,
                tls_certificate_ref: None,
            }],
            rules: vec![AppGwRule {
                name: "r".into(),
                listener: "http".into(),
                backend_pool: "bp".into(),
                path_prefix: None,
            }],
        }
    }

    #[test]
    fn app_gateway_minimum_config_validates() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGateway");
        assert!(appgw("rg").validate().is_ok());
    }

    #[test]
    fn app_gateway_https_listener_requires_tls_ref() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGatewayHttpListener");
        let mut g = appgw("rg");
        g.listeners[0].protocol = AppGatewayProtocol::Https;
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        g.listeners[0].tls_certificate_ref = Some("cert-id".into());
        assert!(g.validate().is_ok());
    }

    #[test]
    fn app_gateway_listener_zero_port_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGatewayHttpListener");
        let mut g = appgw("rg");
        g.listeners[0].port = 0;
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn app_gateway_rule_must_reference_known_listener() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGatewayRequestRoutingRule");
        let mut g = appgw("rg");
        g.rules[0].listener = "missing".into();
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn app_gateway_rule_must_reference_known_backend_pool() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGatewayRequestRoutingRule");
        let mut g = appgw("rg");
        g.rules[0].backend_pool = "missing".into();
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn app_gateway_must_have_at_least_one_backend_pool() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGateway");
        let mut g = appgw("rg");
        g.backend_pools.clear();
        g.rules.clear();
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn app_gateway_must_have_frontend_public_ip() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "ApplicationGateway");
        let mut g = appgw("rg");
        g.frontend_public_ip_id.clear();
        assert!(matches!(g.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── NSG reconcile ───────────────────────────────────────────────────────

    #[test]
    fn nsg_diff_returns_empty_for_identical_sets() {
        let _ = ctx("acme", "pkg/provider/azure_securitygroup_repo.go", "reconcileSecurityGroup");
        let r = vec![nsg("a", 200), nsg("b", 300)];
        let d = diff_nsg_rules(&r, &r);
        assert!(d.is_empty());
        assert_eq!(d.write_count(), 0);
    }

    #[test]
    fn nsg_diff_emits_adds_for_new_rules() {
        let _ = ctx("acme", "pkg/provider/azure_securitygroup_repo.go", "reconcileSecurityGroup");
        let cur = vec![nsg("a", 200)];
        let want = vec![nsg("a", 200), nsg("b", 300)];
        let d = diff_nsg_rules(&cur, &want);
        assert_eq!(d.add.len(), 1);
        assert_eq!(d.add[0].name, "b");
    }

    #[test]
    fn nsg_diff_emits_removes_for_stale_rules() {
        let _ = ctx("acme", "pkg/provider/azure_securitygroup_repo.go", "reconcileSecurityGroup");
        let cur = vec![nsg("a", 200), nsg("b", 300)];
        let want = vec![nsg("a", 200)];
        let d = diff_nsg_rules(&cur, &want);
        assert_eq!(d.remove, vec!["b".to_string()]);
    }

    #[test]
    fn nsg_diff_emits_updates_for_priority_change() {
        let _ = ctx("acme", "pkg/provider/azure_securitygroup_repo.go", "reconcileSecurityGroup");
        let cur = vec![nsg("a", 200)];
        let mut want = vec![nsg("a", 200)];
        want[0].priority = 250;
        let d = diff_nsg_rules(&cur, &want);
        assert_eq!(d.update.len(), 1);
        assert_eq!(d.update[0].priority, 250);
    }

    #[test]
    fn nsg_diff_write_count_sums_all_three_categories() {
        let _ = ctx("acme", "pkg/provider/azure_securitygroup_repo.go", "reconcileSecurityGroup");
        let cur = vec![nsg("a", 200), nsg("b", 300)];
        let mut want = vec![nsg("a", 250), nsg("c", 400)];
        let mut x = nsg("a", 250);
        x.priority = 250;
        want[0] = x;
        let d = diff_nsg_rules(&cur, &want);
        assert_eq!(d.write_count(), 3); // 1 update + 1 remove + 1 add
    }

    // ─── Standard public IP props ────────────────────────────────────────────

    #[test]
    fn standard_public_ip_default_validates() {
        let _ = ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "PublicIPAddress");
        let p = StandardPublicIpProps::default_for_zonal_region();
        assert!(p.validate().is_ok());
        assert!(p.zone_redundant);
        assert_eq!(p.zones.len(), 3);
    }

    #[test]
    fn public_ip_idle_timeout_outside_4_to_30_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "PublicIPAddress");
        let mut p = StandardPublicIpProps::default_for_zonal_region();
        p.idle_timeout_minutes = 1;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        p.idle_timeout_minutes = 60;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn zone_redundant_public_ip_requires_zones() {
        let _ = ctx("acme", "pkg/provider/azure_publicipaddressclient.go", "PublicIPAddress");
        let mut p = StandardPublicIpProps::default_for_zonal_region();
        p.zones.clear();
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lb_sku_helper_recognises_standard() {
        let _ = ctx("acme", "pkg/provider/azure_loadbalancer.go", "LoadBalancerSku");
        assert!(lb_sku_is_standard(LbSku::Standard));
        assert!(!lb_sku_is_standard(LbSku::Basic));
    }

    // ─── Managed Disks ───────────────────────────────────────────────────────

    #[test]
    fn disk_sku_names_match_azure_api_strings() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "DiskSku");
        assert_eq!(DiskSku::StandardLrs.name(), "Standard_LRS");
        assert_eq!(DiskSku::StandardSsdLrs.name(), "StandardSSD_LRS");
        assert_eq!(DiskSku::PremiumLrs.name(), "Premium_LRS");
        assert_eq!(DiskSku::UltraSsdLrs.name(), "UltraSSD_LRS");
    }

    #[test]
    fn disk_sku_shared_disk_support_is_premium_or_ultra_only() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "DiskSku");
        assert!(DiskSku::PremiumLrs.supports_shared_disk());
        assert!(DiskSku::UltraSsdLrs.supports_shared_disk());
        assert!(!DiskSku::StandardLrs.supports_shared_disk());
        assert!(!DiskSku::StandardSsdLrs.supports_shared_disk());
    }

    #[test]
    fn managed_disk_ultra_ssd_requires_a_zone() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "Disk");
        let mut d = ManagedDisk::new("d", "rg", 256, DiskSku::UltraSsdLrs);
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        d.zone = Some(AvailabilityZone::Zone1);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn managed_disk_size_outside_range_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "Disk");
        let mut d = ManagedDisk::new("d", "rg", 0, DiskSku::PremiumLrs);
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        d.size_gb = 200_000;
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_disk_attach_to_second_node_is_refused_for_standard_sku() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "AttachDisk");
        let mut d = ManagedDisk::new("d", "rg", 256, DiskSku::StandardSsdLrs);
        d.attach("node-a").unwrap();
        let err = d.attach("node-b").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_disk_attach_is_idempotent_for_same_node() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "AttachDisk");
        let mut d = ManagedDisk::new("d", "rg", 256, DiskSku::StandardSsdLrs);
        d.attach("node-a").unwrap();
        d.attach("node-a").unwrap();
        assert_eq!(d.attached_node.as_deref(), Some("node-a"));
    }

    #[test]
    fn managed_disk_detach_clears_node() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "DetachDisk");
        let mut d = ManagedDisk::new("d", "rg", 256, DiskSku::StandardSsdLrs);
        d.attach("node-a").unwrap();
        d.detach();
        assert!(d.attached_node.is_none());
    }

    // ─── Workload Identity ───────────────────────────────────────────────────

    #[test]
    fn workload_identity_for_service_account_builds_subject() {
        let _ = ctx("acme", "pkg/provider/azure_workload_identity.go", "FederatedIdentityCredential");
        let f = WorkloadIdentityFederation::for_service_account(
            "fed-aks",
            "/subscriptions/.../uami",
            "kube-system",
            "ccm",
            "https://oidc.eastus.azurewebsites.net/abc",
        );
        assert_eq!(f.service_account_subject, "system:serviceaccount:kube-system:ccm");
        assert_eq!(f.audience, "api://AzureADTokenExchange");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn workload_identity_subject_must_start_with_correct_prefix() {
        let _ = ctx("acme", "pkg/provider/azure_workload_identity.go", "FederatedIdentityCredential");
        let mut f = WorkloadIdentityFederation::for_service_account(
            "fed-aks",
            "uami",
            "ns",
            "sa",
            "https://oidc.example/x",
        );
        f.service_account_subject = "user:alice".into();
        assert!(matches!(f.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn workload_identity_issuer_must_be_https() {
        let _ = ctx("acme", "pkg/provider/azure_workload_identity.go", "FederatedIdentityCredential");
        let mut f = WorkloadIdentityFederation::for_service_account(
            "fed-aks",
            "uami",
            "ns",
            "sa",
            "http://oidc.example/x",
        );
        assert!(matches!(f.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        f.issuer_url = "https://oidc.example/x".into();
        assert!(f.validate().is_ok());
    }

    #[test]
    fn workload_identity_audience_must_be_non_empty() {
        let _ = ctx("acme", "pkg/provider/azure_workload_identity.go", "FederatedIdentityCredential");
        let mut f = WorkloadIdentityFederation::for_service_account(
            "fed-aks",
            "uami",
            "ns",
            "sa",
            "https://oidc.example/x",
        );
        f.audience.clear();
        assert!(matches!(f.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn workload_identity_subject_namespace_must_be_non_empty() {
        let _ = ctx("acme", "pkg/provider/azure_workload_identity.go", "FederatedIdentityCredential");
        let mut f = WorkloadIdentityFederation::for_service_account(
            "fed-aks",
            "uami",
            "ns",
            "sa",
            "https://oidc.example/x",
        );
        f.service_account_subject = "system:serviceaccount::sa".into();
        assert!(matches!(f.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }
}
