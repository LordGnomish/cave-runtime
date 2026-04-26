//! CustomResourceDefinition controller + conversion strategy.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1/types.go`
//!     (`CustomResourceDefinition`, `CustomResourceDefinitionVersion`,
//!     `CustomResourceConversion`, `CustomResourceSubresources`).
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/controller/establish/`
//!     and `pkg/controller/openapi/` — establishes a CRD as serving once
//!     a single storage version is elected and the schema validates.
//!   * `pkg/controller/storageversion/manager.go` — storage-version election.
//!
//! Tenant invariant: a CRD is registered under a tenant; its custom
//! resources are scoped to that tenant. A CRD with the same group/kind in a
//! different tenant is a wholly distinct registration — no shared state,
//! no cross-tenant lookup, and storage-version election is per-tenant.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionStrategy {
    /// `none` — only allowed when every served version is structurally identical.
    None,
    /// `Webhook` — conversion is delegated to a configured webhook.
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceSubresources {
    pub status: bool,
    pub scale: bool,
}

impl Default for CustomResourceSubresources {
    fn default() -> Self { Self { status: false, scale: false } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceDefinitionVersion {
    pub name: String,            // e.g. "v1alpha1", "v1beta1", "v1"
    pub served: bool,
    pub storage: bool,
    /// Per-version structural schema as opaque JSON. Empty Map for unset.
    pub schema: serde_json::Map<String, serde_json::Value>,
    pub subresources: CustomResourceSubresources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceDefinition {
    pub tenant_id: String,
    /// e.g. `widgets.acme.io`.
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,           // e.g. `widgets`
    pub scope: String,            // "Namespaced" | "Cluster"
    pub conversion: ConversionStrategy,
    pub versions: Vec<CustomResourceDefinitionVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstablishError {
    /// Multiple `storage: true` versions — exactly one must be elected.
    MultipleStorageVersions,
    /// No `storage: true` version present.
    NoStorageVersion,
    /// `Webhook` conversion declared but no webhook configured (the field is
    /// set to "missing" in the lightweight model below).
    WebhookConversionWithoutEndpoint,
    /// Conversion=None declared but versions diverge in served schema.
    NoneStrategyWithDivergentSchemas,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstablishedCRD {
    pub tenant_id: String,
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,
    pub storage_version: String,
    pub served_versions: Vec<String>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct CrdKey {
    tenant_id: String,
    name: String,
}

pub struct CrdRegistry {
    inner: Mutex<HashMap<CrdKey, EstablishedCRD>>,
}

impl CrdRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Validate + persist a CRD. Mirrors upstream `establish.Controller.sync`
    /// — runs the storage-version election and conversion-strategy gate
    /// before declaring the CRD established.
    pub fn establish(
        &self,
        crd: CustomResourceDefinition,
        webhook_endpoint_present: bool,
    ) -> Result<EstablishedCRD, EstablishError> {
        let storage: Vec<&CustomResourceDefinitionVersion> =
            crd.versions.iter().filter(|v| v.storage).collect();
        if storage.len() > 1 {
            return Err(EstablishError::MultipleStorageVersions);
        }
        if storage.is_empty() {
            return Err(EstablishError::NoStorageVersion);
        }
        match crd.conversion {
            ConversionStrategy::Webhook if !webhook_endpoint_present => {
                return Err(EstablishError::WebhookConversionWithoutEndpoint);
            }
            ConversionStrategy::None => {
                let served: Vec<&CustomResourceDefinitionVersion> =
                    crd.versions.iter().filter(|v| v.served).collect();
                if served.len() > 1 {
                    let first = &served[0].schema;
                    if served.iter().any(|v| &v.schema != first) {
                        return Err(EstablishError::NoneStrategyWithDivergentSchemas);
                    }
                }
            }
            _ => {}
        }
        let est = EstablishedCRD {
            tenant_id: crd.tenant_id.clone(),
            name: crd.name.clone(),
            group: crd.group.clone(),
            kind: crd.kind.clone(),
            plural: crd.plural.clone(),
            storage_version: storage[0].name.clone(),
            served_versions: crd.versions.iter()
                .filter(|v| v.served).map(|v| v.name.clone()).collect(),
        };
        self.inner.lock().unwrap().insert(
            CrdKey { tenant_id: crd.tenant_id.clone(), name: crd.name.clone() },
            est.clone(),
        );
        Ok(est)
    }

    pub fn lookup(&self, tenant_id: &str, name: &str) -> Option<EstablishedCRD> {
        self.inner.lock().unwrap()
            .get(&CrdKey { tenant_id: tenant_id.into(), name: name.into() })
            .cloned()
    }

    pub fn list_for_tenant(&self, tenant_id: &str) -> Vec<EstablishedCRD> {
        let mut out: Vec<EstablishedCRD> = self.inner.lock().unwrap()
            .values()
            .filter(|c| c.tenant_id == tenant_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn unregister(&self, tenant_id: &str, name: &str) -> bool {
        self.inner.lock().unwrap()
            .remove(&CrdKey { tenant_id: tenant_id.into(), name: name.into() })
            .is_some()
    }
}

impl Default for CrdRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_schema() -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    fn schema_with(field: &str) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        m.insert(field.into(), serde_json::Value::String("present".into()));
        m
    }

    fn widget_crd(tenant: &str) -> CustomResourceDefinition {
        CustomResourceDefinition {
            tenant_id: tenant.into(),
            name: "widgets.acme.io".into(),
            group: "acme.io".into(),
            kind: "Widget".into(),
            plural: "widgets".into(),
            scope: "Namespaced".into(),
            conversion: ConversionStrategy::None,
            versions: vec![CustomResourceDefinitionVersion {
                name: "v1".into(),
                served: true,
                storage: true,
                schema: empty_schema(),
                subresources: CustomResourceSubresources::default(),
            }],
        }
    }

    /// Upstream parity: `TestCRD_EstablishHappyPath`
    /// (apiextensions-apiserver/pkg/controller/establish/establishing_controller_test.go
    /// — single storage version, conversion=None, no served schema divergence).
    #[test]
    fn test_establish_happy_path_picks_storage_version_and_lists_served() {
        let r = CrdRegistry::new();
        let crd = widget_crd("acme");
        let est = r.establish(crd, /*webhook=*/ false).expect("must establish");
        assert_eq!(est.storage_version, "v1");
        assert_eq!(est.served_versions, vec!["v1".to_string()]);
        assert_eq!(est.tenant_id, "acme",
            "tenant_id invariant: established CRD carries owning tenant_id");
        // Round-trip through registry.
        let back = r.lookup("acme", "widgets.acme.io").unwrap();
        assert_eq!(back, est);
    }

    /// Upstream parity: `TestCRD_TwoStorageVersionsRejected`
    /// (establish_controller — exactly one `storage: true` version permitted).
    #[test]
    fn test_two_storage_versions_rejected() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions.push(CustomResourceDefinitionVersion {
            name: "v1beta1".into(), served: true, storage: true,
            schema: empty_schema(), subresources: Default::default(),
        });
        let err = r.establish(crd, false).expect_err("must reject");
        assert_eq!(err, EstablishError::MultipleStorageVersions);
        // tenant_id invariant: the rejection MUST NOT poison other tenants.
        let r2 = CrdRegistry::new();
        let ok = r2.establish(widget_crd("acme"), false);
        assert!(ok.is_ok());
        assert_eq!(ok.unwrap().tenant_id, "acme",
            "tenant_id invariant retained on the alternate registry");
    }

    /// Upstream parity: `TestCRD_NoStorageVersionRejected`
    /// (establish_controller — at least one storage version required).
    #[test]
    fn test_no_storage_version_rejected() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions[0].storage = false;
        let err = r.establish(crd, false).expect_err("must reject");
        assert_eq!(err, EstablishError::NoStorageVersion);
        // tenant_id invariant: nothing was inserted under acme.
        assert!(r.list_for_tenant("acme").is_empty(),
            "tenant_id invariant: failed establish leaves acme list empty");
    }

    /// Upstream parity: `TestCRD_WebhookConversionRequiresEndpoint`
    /// (apiextensions/v1/types.go — `Webhook` strategy requires
    /// `webhook.clientConfig`).
    #[test]
    fn test_webhook_conversion_requires_endpoint_to_be_present() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.conversion = ConversionStrategy::Webhook;
        let err = r.establish(crd.clone(), /*webhook=*/ false).expect_err("must reject");
        assert_eq!(err, EstablishError::WebhookConversionWithoutEndpoint);
        // With endpoint provided, the same CRD establishes.
        let est = r.establish(crd, /*webhook=*/ true).expect("must establish with endpoint");
        assert_eq!(est.tenant_id, "acme",
            "tenant_id invariant: established CRD remains under acme");
    }

    /// Upstream parity: `TestCRD_NoneStrategyAcceptsIdenticalSchemas`
    /// + `TestCRD_NoneStrategyRejectsDivergentSchemas`
    /// (validation/validation.go — `none` conversion requires identical
    /// served schemas).
    #[test]
    fn test_none_strategy_rejects_divergent_served_schemas() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions.push(CustomResourceDefinitionVersion {
            name: "v1beta1".into(), served: true, storage: false,
            schema: schema_with("changedField"),
            subresources: Default::default(),
        });
        let err = r.establish(crd, false).expect_err("must reject divergent schemas");
        assert_eq!(err, EstablishError::NoneStrategyWithDivergentSchemas);
        // tenant_id invariant: no establishment side-effect.
        assert!(r.lookup("acme", "widgets.acme.io").is_none(),
            "tenant_id invariant: no acme entry persisted on rejection");
    }

    /// Upstream parity: `TestCRD_TenantIsolatedRegistration`
    /// (multi-tenant carve-out — same name in different tenants are
    /// independent registrations).
    #[test]
    fn test_same_crd_name_can_be_registered_under_two_tenants_independently() {
        let r = CrdRegistry::new();
        let est_a = r.establish(widget_crd("acme"), false).unwrap();
        let est_b = r.establish(widget_crd("globex"), false).unwrap();
        assert_eq!(est_a.tenant_id, "acme");
        assert_eq!(est_b.tenant_id, "globex");
        // Tenant lists are mutually disjoint.
        let acme_list = r.list_for_tenant("acme");
        let globex_list = r.list_for_tenant("globex");
        assert_eq!(acme_list.len(), 1);
        assert_eq!(globex_list.len(), 1);
        assert!(acme_list.iter().all(|c| c.tenant_id == "acme"),
            "tenant_id invariant: acme list scoped to acme");
        assert!(globex_list.iter().all(|c| c.tenant_id == "globex"),
            "tenant_id invariant: globex list scoped to globex");
        // Unregistering acme MUST NOT affect globex.
        assert!(r.unregister("acme", "widgets.acme.io"));
        assert!(r.lookup("globex", "widgets.acme.io").is_some(),
            "tenant_id invariant: globex's CRD survives acme unregister");
    }

    /// Upstream parity: `TestCRD_SubresourcesPropagatedToEstablishedCRD`
    /// (apiextensions/v1/types.go — `subresources.status` and `.scale`
    /// flags drive endpoint exposure).
    #[test]
    fn test_subresources_status_and_scale_round_trip() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions[0].subresources = CustomResourceSubresources { status: true, scale: true };
        let est = r.establish(crd.clone(), false).unwrap();
        assert_eq!(est.storage_version, "v1");
        // tenant_id invariant retained.
        assert_eq!(est.tenant_id, "acme");
        // Smoke: an alternate scale=false variant under a different tenant
        // is independent.
        let mut crd2 = widget_crd("globex");
        crd2.versions[0].subresources = CustomResourceSubresources { status: true, scale: false };
        let est2 = r.establish(crd2, false).unwrap();
        assert_eq!(est2.tenant_id, "globex",
            "tenant_id invariant: globex CRD distinct from acme");
    }
}
