//! Azure provider — deeper v2 surface.
//!
//! Upstream: `kubernetes-sigs/cloud-provider-azure` @
//! [`super::azure::PROVIDER_VERSION`]. Covers:
//!
//! * **AvailabilitySet** — legacy non-VMSS deployment model.
//! * **DiskEncryptionSet** — customer-managed key wrapper.
//! * **DiskSnapshot** — point-in-time copy.
//! * **AzureFiles** — SMB / NFS file share.
//! * **AGICIngressClass** — Application Gateway Ingress Controller class.
//! * **PrivateCluster** — private API-server endpoint config.
//! * **AadAdminGroup** — RBAC integration.
//! * **VnetPeering** — cross-VNet routing.

use crate::providers::azure_extras::AvailabilityZone;
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── AvailabilitySet ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilitySet {
    pub name: String,
    pub resource_group: String,
    pub location: String,
    /// Update domains. Azure permits 1..=20.
    pub platform_update_domain_count: u8,
    /// Fault domains. Azure permits 1..=3 in most regions.
    pub platform_fault_domain_count: u8,
    pub vm_ids: Vec<String>,
}

impl AvailabilitySet {
    pub const MAX_VMS: usize = 200;

    pub fn new(name: &str, rg: &str, location: &str) -> Self {
        Self {
            name: name.into(),
            resource_group: rg.into(),
            location: location.into(),
            platform_update_domain_count: 5,
            platform_fault_domain_count: 2,
            vm_ids: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(1..=20).contains(&self.platform_update_domain_count) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "availability set {}: platformUpdateDomainCount {} outside [1, 20]",
                    self.name, self.platform_update_domain_count
                ),
            });
        }
        if !(1..=3).contains(&self.platform_fault_domain_count) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "availability set {}: platformFaultDomainCount {} outside [1, 3]",
                    self.name, self.platform_fault_domain_count
                ),
            });
        }
        Ok(())
    }

    pub fn add_vm(&mut self, vm_id: &str) -> Result<(), CloudError> {
        if self.vm_ids.iter().any(|v| v == vm_id) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("vm {vm_id} already in availability set {}", self.name),
            });
        }
        if self.vm_ids.len() >= Self::MAX_VMS {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("availability set {} reached its 200-VM cap", self.name),
            });
        }
        self.vm_ids.push(vm_id.into());
        Ok(())
    }
}

// ─── DiskEncryptionSet ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncryptionType {
    EncryptionAtRestWithPlatformKey,
    EncryptionAtRestWithCustomerKey,
    EncryptionAtRestWithPlatformAndCustomerKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskEncryptionSet {
    pub name: String,
    pub resource_group: String,
    pub key_vault_uri: String,
    pub key_name: String,
    pub key_version: Option<String>,
    pub encryption_type: EncryptionType,
}

impl DiskEncryptionSet {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.key_vault_uri.starts_with("https://") || !self.key_vault_uri.contains(".vault.azure.net") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "disk encryption set {}: key_vault_uri {:?} must be an https Azure Key Vault URI",
                    self.name, self.key_vault_uri
                ),
            });
        }
        if self.key_name.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("disk encryption set {}: key_name required", self.name),
            });
        }
        Ok(())
    }
}

// ─── DiskSnapshot ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapshotState {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSnapshot {
    pub name: String,
    pub source_disk: String,
    pub size_gb: u32,
    pub state: SnapshotState,
    pub incremental: bool,
}

impl DiskSnapshot {
    pub fn pending(name: &str, source: &str, size_gb: u32) -> Self {
        Self {
            name: name.into(),
            source_disk: source.into(),
            size_gb,
            state: SnapshotState::Pending,
            incremental: false,
        }
    }

    pub fn finish(&mut self) -> Result<(), CloudError> {
        if self.state != SnapshotState::Pending {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("snapshot {} is not pending", self.name),
            });
        }
        self.state = SnapshotState::Ready;
        Ok(())
    }

    pub fn fail(&mut self) {
        self.state = SnapshotState::Failed;
    }
}

// ─── AzureFiles ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilesProtocol {
    Smb,
    Nfs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureFilesShare {
    pub name: String,
    pub storage_account: String,
    pub protocol: FilesProtocol,
    pub quota_gb: u32,
    pub access_tier: AccessTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessTier {
    TransactionOptimized,
    Hot,
    Cool,
    Premium,
}

impl AzureFilesShare {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !(1..=102_400).contains(&self.quota_gb) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "azure files share {}: quota {} outside [1, 102400] GiB",
                    self.name, self.quota_gb
                ),
            });
        }
        if self.protocol == FilesProtocol::Nfs && self.access_tier != AccessTier::Premium {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "azure files share {}: NFS protocol requires Premium access tier",
                    self.name
                ),
            });
        }
        Ok(())
    }
}

// ─── AGIC Ingress Class ──────────────────────────────────────────────────────

pub const AGIC_INGRESS_CONTROLLER_ANNOTATION: &str =
    "kubernetes.io/ingress.class";
pub const AGIC_DEFAULT_CLASS: &str = "azure/application-gateway";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgicIngressClass {
    pub name: String,
    pub controller: String,
    pub app_gateway_id: String,
}

impl AgicIngressClass {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.controller != AGIC_DEFAULT_CLASS && !self.controller.contains('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "ingress class {}: controller must be a domain-prefixed string",
                    self.name
                ),
            });
        }
        if !self.app_gateway_id.starts_with("/subscriptions/") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "ingress class {}: app_gateway_id must be a full ARM resource id",
                    self.name
                ),
            });
        }
        Ok(())
    }
}

// ─── Private cluster ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateClusterConfig {
    pub enable_private_cluster: bool,
    pub private_dns_zone: PrivateDnsZoneMode,
    /// Enable the public FQDN as a fallback alongside the private endpoint.
    pub enable_public_fqdn: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivateDnsZoneMode {
    System,
    None,
    Custom(String),
}

impl PrivateClusterConfig {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.enable_private_cluster {
            return Ok(());
        }
        if let PrivateDnsZoneMode::Custom(zone) = &self.private_dns_zone {
            if !zone.starts_with("/subscriptions/") {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!("private DNS zone {zone:?} must be an ARM resource id"),
                });
            }
        }
        if matches!(self.private_dns_zone, PrivateDnsZoneMode::None) && self.enable_public_fqdn {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "private DNS zone None cannot be combined with enable_public_fqdn".into(),
            });
        }
        Ok(())
    }
}

// ─── AAD admin group ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AadProfile {
    pub managed: bool,
    pub admin_group_object_ids: Vec<String>,
    pub tenant_id: String,
}

impl AadProfile {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.managed {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "legacy AAD integration is deprecated; managed must be true".into(),
            });
        }
        if self.tenant_id.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "AAD tenant_id must not be empty".into(),
            });
        }
        for id in &self.admin_group_object_ids {
            // Azure object IDs are GUIDs.
            if id.len() != 36 || id.matches('-').count() != 4 {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!("admin group object id {id:?} is not a GUID"),
                });
            }
        }
        Ok(())
    }
}

// ─── VNet peering ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VnetPeering {
    pub name: String,
    pub source_vnet: String,
    pub remote_vnet: String,
    pub allow_forwarded_traffic: bool,
    pub allow_gateway_transit: bool,
    pub use_remote_gateways: bool,
}

impl VnetPeering {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.source_vnet == self.remote_vnet {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "vnet peering {}: source and remote vnets are identical",
                    self.name
                ),
            });
        }
        if self.use_remote_gateways && self.allow_gateway_transit {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "vnet peering {}: cannot both use_remote_gateways and allow_gateway_transit",
                    self.name
                ),
            });
        }
        Ok(())
    }
}

// ─── Helper: zone count ──────────────────────────────────────────────────────

pub fn zone_count(zones: &[AvailabilityZone]) -> usize {
    let mut seen: Vec<AvailabilityZone> = Vec::new();
    for z in zones {
        if !seen.contains(z) {
            seen.push(*z);
        }
    }
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::azure::PROVIDER_VERSION;
    use crate::test_ctx;
    use crate::types::TenantId;

    const REPO: &str = "kubernetes-sigs/cloud-provider-azure";

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, sym, tenant);
        assert_eq!(cite.repo, REPO);
        t
    }

    // ─── AvailabilitySet ─────────────────────────────────────────────────────

    #[test]
    fn availability_set_default_values_validate() {
        let _ = ctx("acme", "pkg/provider/azure_availabilityset.go", "AvailabilitySet");
        let s = AvailabilitySet::new("as", "rg", "westeurope");
        assert!(s.validate().is_ok());
    }

    #[test]
    fn availability_set_update_domain_outside_1_20_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_availabilityset.go", "AvailabilitySet");
        let mut s = AvailabilitySet::new("as", "rg", "westeurope");
        s.platform_update_domain_count = 0;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        s.platform_update_domain_count = 21;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn availability_set_fault_domain_outside_1_3_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_availabilityset.go", "AvailabilitySet");
        let mut s = AvailabilitySet::new("as", "rg", "westeurope");
        s.platform_fault_domain_count = 4;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn availability_set_add_vm_caps_at_two_hundred() {
        let _ = ctx("acme", "pkg/provider/azure_availabilityset.go", "AvailabilitySet");
        let mut s = AvailabilitySet::new("as", "rg", "westeurope");
        for i in 0..AvailabilitySet::MAX_VMS {
            s.add_vm(&format!("vm-{i}")).unwrap();
        }
        let err = s.add_vm("vm-extra").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn availability_set_rejects_duplicate_vm() {
        let _ = ctx("acme", "pkg/provider/azure_availabilityset.go", "AvailabilitySet");
        let mut s = AvailabilitySet::new("as", "rg", "westeurope");
        s.add_vm("vm-1").unwrap();
        let err = s.add_vm("vm-1").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── DiskEncryptionSet ───────────────────────────────────────────────────

    fn des(name: &str) -> DiskEncryptionSet {
        DiskEncryptionSet {
            name: name.into(),
            resource_group: "rg-sec".into(),
            key_vault_uri: "https://kv-acme.vault.azure.net".into(),
            key_name: "k".into(),
            key_version: None,
            encryption_type: EncryptionType::EncryptionAtRestWithCustomerKey,
        }
    }

    #[test]
    fn disk_encryption_set_validates_minimum_config() {
        let _ = ctx("acme", "pkg/provider/azure_disk_encryption.go", "DiskEncryptionSet");
        assert!(des("d1").validate().is_ok());
    }

    #[test]
    fn disk_encryption_set_rejects_non_kv_uri() {
        let _ = ctx("acme", "pkg/provider/azure_disk_encryption.go", "DiskEncryptionSet");
        let mut d = des("d1");
        d.key_vault_uri = "https://example.com".into();
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn disk_encryption_set_rejects_http_uri() {
        let _ = ctx("acme", "pkg/provider/azure_disk_encryption.go", "DiskEncryptionSet");
        let mut d = des("d1");
        d.key_vault_uri = "http://kv-acme.vault.azure.net".into();
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn disk_encryption_set_requires_key_name() {
        let _ = ctx("acme", "pkg/provider/azure_disk_encryption.go", "DiskEncryptionSet");
        let mut d = des("d1");
        d.key_name.clear();
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── DiskSnapshot ────────────────────────────────────────────────────────

    #[test]
    fn snapshot_starts_pending_and_finishes_ready() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "Snapshot");
        let mut s = DiskSnapshot::pending("s", "d", 100);
        assert_eq!(s.state, SnapshotState::Pending);
        s.finish().unwrap();
        assert_eq!(s.state, SnapshotState::Ready);
    }

    #[test]
    fn snapshot_finish_rejects_non_pending() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "Snapshot");
        let mut s = DiskSnapshot::pending("s", "d", 100);
        s.finish().unwrap();
        assert!(matches!(s.finish().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn snapshot_fail_marks_state_failed() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "Snapshot");
        let mut s = DiskSnapshot::pending("s", "d", 100);
        s.fail();
        assert_eq!(s.state, SnapshotState::Failed);
    }

    // ─── AzureFiles ──────────────────────────────────────────────────────────

    fn afs(protocol: FilesProtocol, tier: AccessTier, quota: u32) -> AzureFilesShare {
        AzureFilesShare {
            name: "share".into(),
            storage_account: "stacme".into(),
            protocol,
            quota_gb: quota,
            access_tier: tier,
        }
    }

    #[test]
    fn azure_files_validates_quota_range() {
        let _ = ctx("acme", "pkg/provider/azure_files.go", "FileShare");
        assert!(matches!(
            afs(FilesProtocol::Smb, AccessTier::Hot, 0).validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        assert!(matches!(
            afs(FilesProtocol::Smb, AccessTier::Hot, 200_000).validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        assert!(afs(FilesProtocol::Smb, AccessTier::Hot, 100).validate().is_ok());
    }

    #[test]
    fn azure_files_nfs_requires_premium_tier() {
        let _ = ctx("acme", "pkg/provider/azure_files.go", "FileShare");
        let bad = afs(FilesProtocol::Nfs, AccessTier::Hot, 100);
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let good = afs(FilesProtocol::Nfs, AccessTier::Premium, 100);
        assert!(good.validate().is_ok());
    }

    // ─── AGIC Ingress Class ──────────────────────────────────────────────────

    fn agic() -> AgicIngressClass {
        AgicIngressClass {
            name: "agic".into(),
            controller: AGIC_DEFAULT_CLASS.into(),
            app_gateway_id: "/subscriptions/aaa/resourceGroups/rg/providers/Microsoft.Network/applicationGateways/agw"
                .into(),
        }
    }

    #[test]
    fn agic_default_constants_match_upstream() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "IngressClass");
        assert_eq!(AGIC_DEFAULT_CLASS, "azure/application-gateway");
        assert_eq!(AGIC_INGRESS_CONTROLLER_ANNOTATION, "kubernetes.io/ingress.class");
    }

    #[test]
    fn agic_default_class_validates() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "IngressClass");
        assert!(agic().validate().is_ok());
    }

    #[test]
    fn agic_app_gateway_id_must_be_arm_resource_id() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "IngressClass");
        let mut c = agic();
        c.app_gateway_id = "agw".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agic_custom_controller_must_be_domain_prefixed() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "IngressClass");
        let mut c = agic();
        c.controller = "custom".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        c.controller = "example.com/custom".into();
        assert!(c.validate().is_ok());
    }

    // ─── Private cluster ─────────────────────────────────────────────────────

    #[test]
    fn private_cluster_disabled_skips_validation() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "PrivateCluster");
        let p = PrivateClusterConfig {
            enable_private_cluster: false,
            private_dns_zone: PrivateDnsZoneMode::System,
            enable_public_fqdn: false,
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn private_cluster_custom_zone_must_be_arm_id() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "PrivateCluster");
        let p = PrivateClusterConfig {
            enable_private_cluster: true,
            private_dns_zone: PrivateDnsZoneMode::Custom("my-zone".into()),
            enable_public_fqdn: false,
        };
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn private_cluster_none_zone_cannot_combine_with_public_fqdn() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "PrivateCluster");
        let p = PrivateClusterConfig {
            enable_private_cluster: true,
            private_dns_zone: PrivateDnsZoneMode::None,
            enable_public_fqdn: true,
        };
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn private_cluster_system_zone_validates() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "PrivateCluster");
        let p = PrivateClusterConfig {
            enable_private_cluster: true,
            private_dns_zone: PrivateDnsZoneMode::System,
            enable_public_fqdn: false,
        };
        assert!(p.validate().is_ok());
    }

    // ─── AAD profile ─────────────────────────────────────────────────────────

    fn aad() -> AadProfile {
        AadProfile {
            managed: true,
            admin_group_object_ids: vec!["00000000-0000-0000-0000-000000000001".into()],
            tenant_id: "00000000-0000-0000-0000-000000000002".into(),
        }
    }

    #[test]
    fn aad_managed_profile_with_guid_validates() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "AADProfile");
        assert!(aad().validate().is_ok());
    }

    #[test]
    fn aad_legacy_unmanaged_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "AADProfile");
        let mut a = aad();
        a.managed = false;
        assert!(matches!(a.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn aad_admin_group_must_be_guid() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "AADProfile");
        let mut a = aad();
        a.admin_group_object_ids = vec!["not-a-guid".into()];
        assert!(matches!(a.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn aad_tenant_id_must_be_non_empty() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "AADProfile");
        let mut a = aad();
        a.tenant_id.clear();
        assert!(matches!(a.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── VNet peering ────────────────────────────────────────────────────────

    fn peering() -> VnetPeering {
        VnetPeering {
            name: "p".into(),
            source_vnet: "vnet-aks".into(),
            remote_vnet: "vnet-hub".into(),
            allow_forwarded_traffic: true,
            allow_gateway_transit: false,
            use_remote_gateways: false,
        }
    }

    #[test]
    fn vnet_peering_valid_two_vnets_validates() {
        let _ = ctx("acme", "pkg/provider/azure_vnet_peering.go", "VnetPeering");
        assert!(peering().validate().is_ok());
    }

    #[test]
    fn vnet_peering_self_peering_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_vnet_peering.go", "VnetPeering");
        let mut p = peering();
        p.remote_vnet = p.source_vnet.clone();
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn vnet_peering_gateway_transit_and_use_remote_are_mutually_exclusive() {
        let _ = ctx("acme", "pkg/provider/azure_vnet_peering.go", "VnetPeering");
        let mut p = peering();
        p.allow_gateway_transit = true;
        p.use_remote_gateways = true;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── zone_count ──────────────────────────────────────────────────────────

    #[test]
    fn zone_count_dedupes_repeated_entries() {
        let _ = ctx("acme", "pkg/provider/azure_zones.go", "AvailabilityZone");
        let zs = vec![AvailabilityZone::Zone1, AvailabilityZone::Zone1, AvailabilityZone::Zone2];
        assert_eq!(zone_count(&zs), 2);
    }

    #[test]
    fn zone_count_returns_zero_for_empty() {
        let _ = ctx("acme", "pkg/provider/azure_zones.go", "AvailabilityZone");
        assert_eq!(zone_count(&[]), 0);
    }
}
