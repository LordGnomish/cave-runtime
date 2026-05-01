//! Cloud-provider trait surface.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/cloud.go::Interface` and its
//! sub-interfaces (`Instances`, `LoadBalancer`, `Routes`, `Zones`,
//! `Clusters`). The Rust side splits these into one trait each so providers
//! can opt out of capabilities they don't support.
//!
//! `InstancesV2Iface` mirrors the v2 surface introduced in upstream's
//! `cloud-provider/cloud.go` to replace the legacy v1 `Instances` interface.

use crate::node_controller::NodeAddress;
use crate::types::{Cite, CloudError, ProviderName, TenantId};
use serde::{Deserialize, Serialize};

/// Per-tenant cloud configuration. Loaded from a YAML / TOML file at
/// controller startup; carries credentials and region defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
    pub tenant: TenantId,
    pub provider: ProviderName,
    pub region: String,
    /// Opaque credential reference (a vault/secret URI, never raw secrets).
    pub credential_ref: String,
}

impl CloudConfig {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.tenant.as_str().is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: self.provider,
                reason: "tenant must not be empty".into(),
            });
        }
        if self.region.trim().is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: self.provider,
                reason: "region must not be empty".into(),
            });
        }
        if !self.credential_ref.starts_with("vault://") && !self.credential_ref.starts_with("secret://") {
            return Err(CloudError::InvalidConfig {
                provider: self.provider,
                reason: format!("credential_ref must use vault:// or secret://, got {:?}", self.credential_ref),
            });
        }
        Ok(())
    }
}

/// Top-level provider trait. Mirrors `cloud.Interface`.
pub trait CloudProvider {
    fn name(&self) -> ProviderName;
    fn config(&self) -> &CloudConfig;

    /// Authorise a (tenant, kind, name) triple before any sub-trait method is
    /// invoked. Default implementation refuses cross-tenant access.
    fn authorise(&self, tenant: &TenantId, kind: &'static str, name: &str) -> Result<(), CloudError> {
        if tenant != &self.config().tenant {
            return Err(CloudError::TenantDenied {
                tenant: tenant.clone(),
                kind,
                name: name.to_string(),
            });
        }
        Ok(())
    }
}

/// Subset of `cloudprovider.Instances` (v1.36 dropped the Pre-Initializer
/// surface; `InstanceMetadata` is the only modern entry point).
pub trait InstancesIface: CloudProvider {
    /// Return the `<scheme>://<id>` provider-id for `node_name`.
    fn provider_id(&self, tenant: &TenantId, node_name: &str) -> Result<String, CloudError>;

    /// Return the (zone, region) pair for `node_name`.
    fn zone_for(&self, tenant: &TenantId, node_name: &str) -> Result<(String, String), CloudError>;
}

/// Mirrors `cloudprovider.InstanceMetadata` — the v2 instance struct returned
/// by `InstanceMetadataByProviderID`. Centralises the things the node
/// controller needs for one node so a single RPC can populate them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceMetadata {
    pub provider_id: String,
    pub instance_type: String,
    pub region: String,
    pub zone: String,
    pub node_addresses: Vec<NodeAddress>,
    /// Whether the cloud reports the instance as `Shutdown` (powered off but
    /// not deleted). Mirrors the upstream `InstanceShutdownByProviderID`
    /// auxiliary call that v2 folds into the metadata struct.
    #[serde(default)]
    pub shutdown: bool,
    /// `True` when the cloud has no record of the instance — node should be
    /// deleted. Mirrors `cloudprovider.InstanceNotFound`.
    #[serde(default)]
    pub not_found: bool,
}

impl InstanceMetadata {
    pub fn new(
        provider_id: impl Into<String>,
        instance_type: impl Into<String>,
        region: impl Into<String>,
        zone: impl Into<String>,
        node_addresses: Vec<NodeAddress>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            instance_type: instance_type.into(),
            region: region.into(),
            zone: zone.into(),
            node_addresses,
            shutdown: false,
            not_found: false,
        }
    }

    /// Validate the metadata. Mirrors the assertions in
    /// `Controller.syncNode` upstream.
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.not_found {
            return Ok(());
        }
        if self.provider_id.is_empty() {
            return Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: "InstanceMetadata.provider_id must not be empty".into(),
            });
        }
        if self.instance_type.is_empty() {
            return Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: "InstanceMetadata.instance_type must not be empty".into(),
            });
        }
        if self.region.is_empty() || self.zone.is_empty() {
            return Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: "InstanceMetadata zone/region must not be empty".into(),
            });
        }
        Ok(())
    }
}

/// V2 instance interface — superset of `InstancesIface`. Mirrors
/// `cloudprovider.InstancesV2`. Implementations should return everything in
/// one shot to avoid the multi-RPC fan-out of the v1 interface.
pub trait InstancesV2Iface: CloudProvider {
    /// Whether the underlying cloud has metadata for this node. Mirrors
    /// `cloudprovider.InstanceExists`.
    fn instance_exists(&self, tenant: &TenantId, node_name: &str) -> Result<bool, CloudError>;

    /// Whether the cloud reports the instance as currently shut down.
    /// Mirrors `cloudprovider.InstanceShutdown`.
    fn instance_shutdown(&self, tenant: &TenantId, node_name: &str) -> Result<bool, CloudError>;

    /// Return the full metadata for `node_name`. Mirrors
    /// `cloudprovider.InstanceMetadata`.
    fn instance_metadata(
        &self,
        tenant: &TenantId,
        node_name: &str,
    ) -> Result<InstanceMetadata, CloudError>;
}

/// Subset of `cloudprovider.LoadBalancer`.
pub trait LoadBalancerIface: CloudProvider {
    /// Ensure an LB exists for `service`, returning its external IP.
    fn ensure_lb(&self, tenant: &TenantId, service: &str) -> Result<String, CloudError>;
    fn delete_lb(&self, tenant: &TenantId, service: &str) -> Result<(), CloudError>;
}

/// Subset of `cloudprovider.Routes`.
pub trait RoutesIface: CloudProvider {
    fn list_routes(&self, tenant: &TenantId) -> Result<Vec<String>, CloudError>;
    fn create_route(&self, tenant: &TenantId, name: &str, cidr: &str) -> Result<(), CloudError>;
    fn delete_route(&self, tenant: &TenantId, name: &str) -> Result<(), CloudError>;
}

/// Detailed (zone, region) pair returned by `cloudprovider.Zones.GetZone()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub region: String,
    pub failure_domain: String,
}

impl ZoneInfo {
    pub fn new(region: impl Into<String>, failure_domain: impl Into<String>) -> Self {
        Self { region: region.into(), failure_domain: failure_domain.into() }
    }
}

/// Subset of `cloudprovider.Zones`.
pub trait ZonesIface: CloudProvider {
    fn current_zone(&self, tenant: &TenantId) -> Result<String, CloudError>;
}

/// Subset of `cloudprovider.Clusters` — list+rename of managed clusters.
pub trait ClustersIface: CloudProvider {
    fn list_clusters(&self, tenant: &TenantId) -> Result<Vec<String>, CloudError>;
}

/// Identifier for a managed cluster. Mirrors `cloudprovider.ClusterID`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClusterId(pub String);

impl ClusterId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s("staging/src/k8s.io/cloud-provider/cloud.go", "Interface");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_controller::NodeAddressType;
    use crate::test_ctx;

    fn cfg(tenant: &str, region: &str, cred: &str) -> CloudConfig {
        CloudConfig {
            tenant: TenantId::new(tenant).expect("test fixture"),
            provider: ProviderName::Hetzner,
            region: region.into(),
            credential_ref: cred.into(),
        }
    }

    /// Tiny in-memory provider used to exercise default trait methods.
    struct StubProvider {
        cfg: CloudConfig,
    }
    impl CloudProvider for StubProvider {
        fn name(&self) -> ProviderName {
            self.cfg.provider
        }
        fn config(&self) -> &CloudConfig {
            &self.cfg
        }
    }

    #[test]
    fn config_requires_non_empty_region() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "ProviderConfig",
            "tenant-cfg-region"
        );
        let _ = tenant;
        let bad = cfg("acme", "  ", "vault://hcloud-token");
        assert!(bad.validate().is_err());
    }

    #[test]
    fn config_requires_uri_credential() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "ProviderConfig",
            "tenant-cfg-cred"
        );
        let _ = tenant;
        let bad = cfg("acme", "fsn1", "raw-token-here");
        assert!(bad.validate().is_err());
        let good = cfg("acme", "fsn1", "vault://kv/hcloud");
        assert!(good.validate().is_ok());
    }

    #[test]
    fn authorise_rejects_cross_tenant_calls() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "Interface",
            "tenant-attacker"
        );
        let p = StubProvider { cfg: cfg("acme", "fsn1", "vault://kv/hcloud") };
        let err = p.authorise(&attacker, "LoadBalancer", "web").unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    #[test]
    fn authorise_allows_same_tenant_calls() {
        let (_cite, owner) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "Interface",
            "acme"
        );
        let p = StubProvider { cfg: cfg("acme", "fsn1", "vault://kv/hcloud") };
        assert!(p.authorise(&owner, "LoadBalancer", "web").is_ok());
    }

    // ─── InstanceMetadata ────────────────────────────────────────────────────

    #[test]
    fn instance_metadata_constructor_starts_alive() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-im-new"
        );
        let m = InstanceMetadata::new("hcloud://7", "cpx21", "fsn1", "fsn1-dc14", vec![]);
        assert!(!m.shutdown);
        assert!(!m.not_found);
        assert_eq!(m.provider_id, "hcloud://7");
        assert_eq!(m.instance_type, "cpx21");
    }

    #[test]
    fn instance_metadata_validates_required_fields() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-im-validate"
        );
        let mut m = InstanceMetadata::new("hcloud://7", "cpx21", "fsn1", "fsn1-dc14", vec![]);
        assert!(m.validate().is_ok());
        m.provider_id.clear();
        assert!(m.validate().is_err());
    }

    #[test]
    fn instance_metadata_not_found_skips_validation() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceNotFound",
            "tenant-im-nf"
        );
        let mut m = InstanceMetadata::new("", "", "", "", vec![]);
        m.not_found = true;
        assert!(m.validate().is_ok());
    }

    #[test]
    fn instance_metadata_validation_rejects_missing_zone() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-im-zone"
        );
        let mut m = InstanceMetadata::new("hcloud://7", "cpx21", "fsn1", "fsn1-dc14", vec![]);
        m.zone.clear();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn instance_metadata_carries_addresses() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-im-addrs"
        );
        let m = InstanceMetadata::new(
            "hcloud://7",
            "cpx21",
            "fsn1",
            "fsn1-dc14",
            vec![NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1")],
        );
        assert_eq!(m.node_addresses.len(), 1);
        assert_eq!(m.node_addresses[0].kind, NodeAddressType::InternalIP);
    }

    // ─── ZoneInfo ────────────────────────────────────────────────────────────

    #[test]
    fn zone_info_round_trips_region_and_failure_domain() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "Zones",
            "tenant-zoneinfo"
        );
        let z = ZoneInfo::new("fsn1", "fsn1-dc14");
        assert_eq!(z.region, "fsn1");
        assert_eq!(z.failure_domain, "fsn1-dc14");
    }

    // ─── ClusterId ───────────────────────────────────────────────────────────

    #[test]
    fn cluster_id_constructor_round_trips() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "ClusterID",
            "tenant-cluster-id"
        );
        let c = ClusterId::new("acme-prod");
        assert_eq!(c.as_str(), "acme-prod");
    }

    // ─── InstancesV2Iface stub conformance ───────────────────────────────────

    struct StubV2 {
        cfg: CloudConfig,
        meta: std::collections::HashMap<String, InstanceMetadata>,
    }
    impl CloudProvider for StubV2 {
        fn name(&self) -> ProviderName {
            self.cfg.provider
        }
        fn config(&self) -> &CloudConfig {
            &self.cfg
        }
    }
    impl InstancesV2Iface for StubV2 {
        fn instance_exists(&self, t: &TenantId, n: &str) -> Result<bool, CloudError> {
            self.authorise(t, "Instance", n)?;
            Ok(self.meta.get(n).map(|m| !m.not_found).unwrap_or(false))
        }
        fn instance_shutdown(&self, t: &TenantId, n: &str) -> Result<bool, CloudError> {
            self.authorise(t, "Instance", n)?;
            Ok(self.meta.get(n).map(|m| m.shutdown).unwrap_or(false))
        }
        fn instance_metadata(&self, t: &TenantId, n: &str) -> Result<InstanceMetadata, CloudError> {
            self.authorise(t, "Instance", n)?;
            self.meta.get(n).cloned().ok_or_else(|| CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("instance {n} not found"),
            })
        }
    }

    fn stub_v2(tenant: &str) -> StubV2 {
        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "alive".into(),
            InstanceMetadata::new("hcloud://1", "cpx21", "fsn1", "fsn1-dc14", vec![]),
        );
        let mut down = InstanceMetadata::new("hcloud://2", "cpx21", "fsn1", "fsn1-dc14", vec![]);
        down.shutdown = true;
        meta.insert("down".into(), down);
        let mut nf = InstanceMetadata::new("", "", "", "", vec![]);
        nf.not_found = true;
        meta.insert("ghost".into(), nf);
        StubV2 { cfg: cfg(tenant, "fsn1", "vault://kv/hcloud"), meta }
    }

    #[test]
    fn instances_v2_exists_returns_true_for_alive_instance() {
        let (_cite, t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceExists",
            "acme"
        );
        let p = stub_v2("acme");
        assert!(p.instance_exists(&t, "alive").unwrap());
    }

    #[test]
    fn instances_v2_exists_returns_false_for_not_found() {
        let (_cite, t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceExists",
            "acme"
        );
        let p = stub_v2("acme");
        assert!(!p.instance_exists(&t, "ghost").unwrap());
    }

    #[test]
    fn instances_v2_shutdown_reports_state() {
        let (_cite, t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceShutdown",
            "acme"
        );
        let p = stub_v2("acme");
        assert!(p.instance_shutdown(&t, "down").unwrap());
        assert!(!p.instance_shutdown(&t, "alive").unwrap());
    }

    #[test]
    fn instances_v2_metadata_returns_full_struct() {
        let (_cite, t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "acme"
        );
        let p = stub_v2("acme");
        let m = p.instance_metadata(&t, "alive").unwrap();
        assert_eq!(m.provider_id, "hcloud://1");
        assert_eq!(m.instance_type, "cpx21");
    }

    #[test]
    fn instances_v2_methods_authorise_first() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-attacker"
        );
        let p = stub_v2("acme");
        assert!(matches!(
            p.instance_metadata(&attacker, "alive").unwrap_err(),
            CloudError::TenantDenied { .. }
        ));
        assert!(matches!(
            p.instance_exists(&attacker, "alive").unwrap_err(),
            CloudError::TenantDenied { .. }
        ));
        assert!(matches!(
            p.instance_shutdown(&attacker, "alive").unwrap_err(),
            CloudError::TenantDenied { .. }
        ));
    }
}
