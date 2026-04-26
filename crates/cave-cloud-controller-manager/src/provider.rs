//! Cloud-provider trait surface.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/cloud.go::Interface` and its
//! sub-interfaces (`Instances`, `LoadBalancer`, `Routes`, `Zones`,
//! `Clusters`). The Rust side splits these into one trait each so providers
//! can opt out of capabilities they don't support.

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

/// Subset of `cloudprovider.Zones`.
pub trait ZonesIface: CloudProvider {
    fn current_zone(&self, tenant: &TenantId) -> Result<String, CloudError>;
}

/// Subset of `cloudprovider.Clusters` — list+rename of managed clusters.
pub trait ClustersIface: CloudProvider {
    fn list_clusters(&self, tenant: &TenantId) -> Result<Vec<String>, CloudError>;
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s("staging/src/k8s.io/cloud-provider/cloud.go", "Interface");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cfg(tenant: &str, region: &str, cred: &str) -> CloudConfig {
        CloudConfig {
            tenant: TenantId::new(tenant),
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
}
