// SPDX-License-Identifier: AGPL-3.0-or-later
//! Azure provider — networking, autoscaler, BYO-VNet, CNI / NetworkPolicy.
//!
//! Upstream: `kubernetes-sigs/cloud-provider-azure` @
//! [`super::azure::PROVIDER_VERSION`]. Covers:
//!
//! * **CNI / NetworkPolicy** — Azure CNI vs kubenet, NPM / Calico /
//!   Cilium policy modes.
//! * **Cluster Autoscaler** — node pool min/max, scale-down delay.
//! * **BYO VNet** — preexisting subnet IDs the cluster joins.
//! * **AGIC Ingress** — backend pool fan-out per-listener match rules.
//! * **API server VNet integration** — vnet integration subnet, private
//!   API server address pool.

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── CNI / NetworkPolicy ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkPlugin {
    /// Azure CNI — pod IPs are real VNet IPs.
    Azure,
    /// kubenet — pod IPs are NATed via the node IP.
    Kubenet,
    /// Azure CNI overlay — VNet IPs assigned from a private CIDR.
    AzureOverlay,
    /// No plugin — bring-your-own.
    None,
}

impl NetworkPlugin {
    pub const fn key(self) -> &'static str {
        match self {
            NetworkPlugin::Azure => "azure",
            NetworkPlugin::Kubenet => "kubenet",
            NetworkPlugin::AzureOverlay => "azure-overlay",
            NetworkPlugin::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkPolicy {
    Calico,
    Azure,
    Cilium,
    None,
}

impl NetworkPolicy {
    pub const fn key(self) -> &'static str {
        match self {
            NetworkPolicy::Calico => "calico",
            NetworkPolicy::Azure => "azure",
            NetworkPolicy::Cilium => "cilium",
            NetworkPolicy::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkProfile {
    pub plugin: NetworkPlugin,
    pub policy: NetworkPolicy,
    pub service_cidr: String,
    pub dns_service_ip: String,
    pub pod_cidr: Option<String>,
}

impl NetworkProfile {
    pub fn validate(&self) -> Result<(), CloudError> {
        // kubenet always uses pod CIDR; Azure CNI doesn't.
        match (self.plugin, &self.pod_cidr) {
            (NetworkPlugin::Kubenet, None) => {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: "kubenet plugin requires pod_cidr".into(),
                });
            }
            (NetworkPlugin::Azure, Some(_)) => {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: "azure CNI does not accept pod_cidr".into(),
                });
            }
            _ => {}
        }
        if self.policy == NetworkPolicy::Azure
            && !matches!(self.plugin, NetworkPlugin::Azure | NetworkPlugin::AzureOverlay)
        {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "azure NetworkPolicy requires the azure CNI plugin".into(),
            });
        }
        if self.service_cidr.is_empty() || !self.service_cidr.contains('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("service_cidr {:?} must be a CIDR", self.service_cidr),
            });
        }
        if self.dns_service_ip.is_empty() || !self.dns_service_ip.contains('.') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("dns_service_ip {:?} must be IPv4", self.dns_service_ip),
            });
        }
        Ok(())
    }
}

// ─── Cluster autoscaler ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoscalerProfile {
    pub balance_similar_node_groups: bool,
    pub expander: String,
    pub max_empty_bulk_delete: u32,
    pub max_graceful_termination_seconds: u32,
    pub max_node_provision_time_minutes: u32,
    pub scale_down_delay_after_add_minutes: u32,
    pub scale_down_unneeded_time_minutes: u32,
    pub scan_interval_seconds: u32,
}

impl AutoscalerProfile {
    /// AKS defaults — mirrors `cluster-autoscaler/cloudprovider/azure/azure.go`.
    pub fn defaults() -> Self {
        Self {
            balance_similar_node_groups: false,
            expander: "random".into(),
            max_empty_bulk_delete: 10,
            max_graceful_termination_seconds: 600,
            max_node_provision_time_minutes: 15,
            scale_down_delay_after_add_minutes: 10,
            scale_down_unneeded_time_minutes: 10,
            scan_interval_seconds: 10,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        let allowed_expanders =
            ["least-waste", "most-pods", "priority", "random", "price"];
        if !allowed_expanders.iter().any(|e| *e == self.expander) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "autoscaler expander {:?} not in {:?}",
                    self.expander, allowed_expanders
                ),
            });
        }
        if !(1..=100).contains(&self.max_empty_bulk_delete) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "max_empty_bulk_delete {} outside [1, 100]",
                    self.max_empty_bulk_delete
                ),
            });
        }
        if !(60..=3_600).contains(&self.max_graceful_termination_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "max_graceful_termination_seconds {} outside [60, 3600]",
                    self.max_graceful_termination_seconds
                ),
            });
        }
        if !(1..=120).contains(&self.scan_interval_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "scan_interval_seconds {} outside [1, 120]",
                    self.scan_interval_seconds
                ),
            });
        }
        Ok(())
    }
}

// ─── BYO VNet ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByoVnetConfig {
    pub vnet_subnet_id: String,
    pub pod_subnet_id: Option<String>,
    pub service_cidr: String,
}

impl ByoVnetConfig {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.vnet_subnet_id.starts_with("/subscriptions/")
            || !self.vnet_subnet_id.contains("/virtualNetworks/")
            || !self.vnet_subnet_id.contains("/subnets/")
        {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!(
                    "vnet_subnet_id {:?} must be an ARM subnet resource id",
                    self.vnet_subnet_id
                ),
            });
        }
        if let Some(pod_subnet_id) = &self.pod_subnet_id {
            if !pod_subnet_id.contains("/subnets/") {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!(
                        "pod_subnet_id {pod_subnet_id:?} must be an ARM subnet resource id"
                    ),
                });
            }
        }
        if !self.service_cidr.contains('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("service_cidr {:?} must be a CIDR", self.service_cidr),
            });
        }
        Ok(())
    }
}

// ─── AGIC backend match ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgicBackendMatch {
    pub host: Option<String>,
    pub path_prefix: Option<String>,
    pub backend_pool: String,
}

impl AgicBackendMatch {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.host.is_none() && self.path_prefix.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "AGIC backend match requires host or path_prefix".into(),
            });
        }
        if let Some(p) = &self.path_prefix {
            if !p.starts_with('/') {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!("path_prefix {p:?} must start with /"),
                });
            }
        }
        if let Some(h) = &self.host {
            if !h.contains('.') {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!("host {h:?} must be a fully qualified DNS name"),
                });
            }
        }
        if self.backend_pool.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "AGIC backend_pool must not be empty".into(),
            });
        }
        Ok(())
    }
}

// ─── API server VNet integration ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiServerVnetIntegration {
    pub enabled: bool,
    pub subnet_id: Option<String>,
}

impl ApiServerVnetIntegration {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.enabled && self.subnet_id.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: "api server VNet integration requires subnet_id".into(),
            });
        }
        if let Some(s) = &self.subnet_id {
            if !s.starts_with("/subscriptions/") {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Azure,
                    reason: format!("subnet_id {s:?} must be an ARM resource id"),
                });
            }
        }
        Ok(())
    }
}

// ─── Disk CSI snapshotting ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CsiDriverState {
    Available,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskCsiDriver {
    pub name: String,
    pub state: CsiDriverState,
    pub volume_count: u32,
}

impl DiskCsiDriver {
    pub const NAME: &'static str = "disk.csi.azure.com";
    pub const FILE_NAME: &'static str = "file.csi.azure.com";

    pub fn azure_disk() -> Self {
        Self { name: Self::NAME.into(), state: CsiDriverState::Available, volume_count: 0 }
    }
    pub fn azure_file() -> Self {
        Self { name: Self::FILE_NAME.into(), state: CsiDriverState::Available, volume_count: 0 }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !matches!(
            self.name.as_str(),
            DiskCsiDriver::NAME | DiskCsiDriver::FILE_NAME
        ) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Azure,
                reason: format!("unknown CSI driver name {:?}", self.name),
            });
        }
        Ok(())
    }
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

    // ─── Network plugin / policy ─────────────────────────────────────────────

    #[test]
    fn network_plugin_keys_match_aks_cli_strings() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkPlugin");
        assert_eq!(NetworkPlugin::Azure.key(), "azure");
        assert_eq!(NetworkPlugin::Kubenet.key(), "kubenet");
        assert_eq!(NetworkPlugin::AzureOverlay.key(), "azure-overlay");
        assert_eq!(NetworkPlugin::None.key(), "none");
    }

    #[test]
    fn network_policy_keys_match_aks_cli_strings() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkPolicy");
        assert_eq!(NetworkPolicy::Calico.key(), "calico");
        assert_eq!(NetworkPolicy::Azure.key(), "azure");
        assert_eq!(NetworkPolicy::Cilium.key(), "cilium");
        assert_eq!(NetworkPolicy::None.key(), "none");
    }

    fn np(plugin: NetworkPlugin, policy: NetworkPolicy, pod_cidr: Option<&str>) -> NetworkProfile {
        NetworkProfile {
            plugin,
            policy,
            service_cidr: "10.0.0.0/16".into(),
            dns_service_ip: "10.0.0.10".into(),
            pod_cidr: pod_cidr.map(String::from),
        }
    }

    #[test]
    fn kubenet_profile_requires_pod_cidr() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let bad = np(NetworkPlugin::Kubenet, NetworkPolicy::None, None);
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let good = np(NetworkPlugin::Kubenet, NetworkPolicy::None, Some("10.244.0.0/16"));
        assert!(good.validate().is_ok());
    }

    #[test]
    fn azure_cni_rejects_pod_cidr() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let bad = np(NetworkPlugin::Azure, NetworkPolicy::None, Some("10.244.0.0/16"));
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn azure_network_policy_requires_azure_cni() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let bad = np(NetworkPlugin::Kubenet, NetworkPolicy::Azure, Some("10.244.0.0/16"));
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let good = np(NetworkPlugin::Azure, NetworkPolicy::Azure, None);
        assert!(good.validate().is_ok());
    }

    #[test]
    fn calico_policy_works_with_kubenet_or_cni() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let p1 = np(NetworkPlugin::Kubenet, NetworkPolicy::Calico, Some("10.244.0.0/16"));
        let p2 = np(NetworkPlugin::Azure, NetworkPolicy::Calico, None);
        assert!(p1.validate().is_ok());
        assert!(p2.validate().is_ok());
    }

    #[test]
    fn network_profile_service_cidr_must_be_cidr() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let mut p = np(NetworkPlugin::Azure, NetworkPolicy::None, None);
        p.service_cidr = "garbage".into();
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn network_profile_dns_service_ip_must_be_ipv4() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "NetworkProfile");
        let mut p = np(NetworkPlugin::Azure, NetworkPolicy::None, None);
        p.dns_service_ip = "no-dots".into();
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── Autoscaler profile ──────────────────────────────────────────────────

    #[test]
    fn autoscaler_defaults_validate() {
        let _ = ctx("acme", "cluster-autoscaler/cloudprovider/azure/azure.go", "AutoscalerProfile");
        assert!(AutoscalerProfile::defaults().validate().is_ok());
    }

    #[test]
    fn autoscaler_expander_must_be_known() {
        let _ = ctx("acme", "cluster-autoscaler/cloudprovider/azure/azure.go", "AutoscalerProfile");
        let mut p = AutoscalerProfile::defaults();
        p.expander = "weighted".into();
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        for e in ["least-waste", "most-pods", "priority", "random", "price"] {
            p.expander = e.into();
            assert!(p.validate().is_ok(), "expander {e}");
        }
    }

    #[test]
    fn autoscaler_max_empty_bulk_delete_is_capped() {
        let _ = ctx("acme", "cluster-autoscaler/cloudprovider/azure/azure.go", "AutoscalerProfile");
        let mut p = AutoscalerProfile::defaults();
        p.max_empty_bulk_delete = 0;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        p.max_empty_bulk_delete = 200;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn autoscaler_max_graceful_termination_outside_60_3600_is_rejected() {
        let _ = ctx("acme", "cluster-autoscaler/cloudprovider/azure/azure.go", "AutoscalerProfile");
        let mut p = AutoscalerProfile::defaults();
        p.max_graceful_termination_seconds = 30;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn autoscaler_scan_interval_outside_1_120_is_rejected() {
        let _ = ctx("acme", "cluster-autoscaler/cloudprovider/azure/azure.go", "AutoscalerProfile");
        let mut p = AutoscalerProfile::defaults();
        p.scan_interval_seconds = 0;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        p.scan_interval_seconds = 200;
        assert!(matches!(p.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── BYO VNet ────────────────────────────────────────────────────────────

    fn byo() -> ByoVnetConfig {
        ByoVnetConfig {
            vnet_subnet_id:
                "/subscriptions/aaa/resourceGroups/rg/providers/Microsoft.Network/virtualNetworks/v/subnets/s".into(),
            pod_subnet_id: None,
            service_cidr: "10.0.0.0/16".into(),
        }
    }

    #[test]
    fn byo_vnet_default_validates() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ByoVnet");
        assert!(byo().validate().is_ok());
    }

    #[test]
    fn byo_vnet_requires_arm_subnet_id() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ByoVnet");
        let mut c = byo();
        c.vnet_subnet_id = "vnet-1/subnets/s".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn byo_vnet_pod_subnet_must_be_subnet_resource() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ByoVnet");
        let mut c = byo();
        c.pod_subnet_id = Some("not-a-subnet".into());
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        c.pod_subnet_id = Some(
            "/subscriptions/aaa/resourceGroups/rg/providers/Microsoft.Network/virtualNetworks/v/subnets/pods".into(),
        );
        assert!(c.validate().is_ok());
    }

    #[test]
    fn byo_vnet_service_cidr_must_be_cidr() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ByoVnet");
        let mut c = byo();
        c.service_cidr = "no-mask".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── AGIC backend match ──────────────────────────────────────────────────

    #[test]
    fn agic_backend_match_requires_host_or_path() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "BackendMatch");
        let m = AgicBackendMatch { host: None, path_prefix: None, backend_pool: "bp".into() };
        assert!(matches!(m.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agic_backend_match_path_must_start_with_slash() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "BackendMatch");
        let m = AgicBackendMatch {
            host: None,
            path_prefix: Some("api".into()),
            backend_pool: "bp".into(),
        };
        assert!(matches!(m.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agic_backend_match_host_must_be_fqdn() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "BackendMatch");
        let m = AgicBackendMatch {
            host: Some("nodot".into()),
            path_prefix: None,
            backend_pool: "bp".into(),
        };
        assert!(matches!(m.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agic_backend_match_backend_pool_must_be_non_empty() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "BackendMatch");
        let m = AgicBackendMatch {
            host: Some("api.example.com".into()),
            path_prefix: None,
            backend_pool: String::new(),
        };
        assert!(matches!(m.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn agic_backend_match_full_spec_validates() {
        let _ = ctx("acme", "pkg/provider/azure_app_gateway.go", "BackendMatch");
        let m = AgicBackendMatch {
            host: Some("api.example.com".into()),
            path_prefix: Some("/v1".into()),
            backend_pool: "bp".into(),
        };
        assert!(m.validate().is_ok());
    }

    // ─── API server VNet integration ─────────────────────────────────────────

    #[test]
    fn api_server_vnet_integration_disabled_skips_validation() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ApiServerVnetIntegration");
        let v = ApiServerVnetIntegration { enabled: false, subnet_id: None };
        assert!(v.validate().is_ok());
    }

    #[test]
    fn api_server_vnet_integration_requires_subnet_id_when_enabled() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ApiServerVnetIntegration");
        let v = ApiServerVnetIntegration { enabled: true, subnet_id: None };
        assert!(matches!(v.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn api_server_vnet_integration_subnet_id_must_be_arm_id() {
        let _ = ctx("acme", "pkg/provider/azure_managedclusters.go", "ApiServerVnetIntegration");
        let v = ApiServerVnetIntegration { enabled: true, subnet_id: Some("subnet-1".into()) };
        assert!(matches!(v.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let v = ApiServerVnetIntegration {
            enabled: true,
            subnet_id: Some("/subscriptions/aaa/resourceGroups/rg/providers/Microsoft.Network/virtualNetworks/v/subnets/api".into()),
        };
        assert!(v.validate().is_ok());
    }

    // ─── Disk CSI driver ─────────────────────────────────────────────────────

    #[test]
    fn disk_csi_driver_constants_match_upstream_canonical_names() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "CSIDriver");
        assert_eq!(DiskCsiDriver::NAME, "disk.csi.azure.com");
        assert_eq!(DiskCsiDriver::FILE_NAME, "file.csi.azure.com");
    }

    #[test]
    fn disk_csi_driver_constructors_validate() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "CSIDriver");
        assert!(DiskCsiDriver::azure_disk().validate().is_ok());
        assert!(DiskCsiDriver::azure_file().validate().is_ok());
    }

    #[test]
    fn disk_csi_driver_unknown_name_is_rejected() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "CSIDriver");
        let d = DiskCsiDriver {
            name: "disk.example.com".into(),
            state: CsiDriverState::Available,
            volume_count: 0,
        };
        assert!(matches!(d.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn disk_csi_driver_states_match_csi_spec() {
        let _ = ctx("acme", "pkg/provider/azure_managed_disk.go", "CSIDriver");
        // Type-level — confirm the variants exist and are distinct.
        let a = CsiDriverState::Available;
        let b = CsiDriverState::Degraded;
        let c = CsiDriverState::Unavailable;
        assert_ne!(a, b);
        assert_ne!(b, c);
    }
}
