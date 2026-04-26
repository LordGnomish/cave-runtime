//! API discovery + OpenAPI v3 schema generation.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/discovery/group.go`
//!     (`/apis/<group>` discovery doc).
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/discovery/aggregated/aggregated.go`
//!     (KEP-3352 — aggregated discovery).
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/`
//!     and `staging/src/k8s.io/kube-openapi/pkg/handler3/handler.go`
//!     (OpenAPI v3 per-group-version document layout under `/openapi/v3`).
//!
//! Discovery doc surface:
//!   - `/api/v1`, `/apis/<group>/<version>` → APIResourceList
//!   - `/openapi/v3` → group-version → OpenAPI v3 JSON URL map
//!   - `/openapi/v3/<group>/<version>` → OpenAPI v3 schema doc
//!
//! Tenant invariant: discovery is normally cluster-scoped, but in cave-apiserver
//! a *tenant* is the unit of API surface ownership. A tenant may register CRDs
//! (or aggregated APIServices) that the discovery doc MUST surface ONLY to its
//! own callers — listing for tenant A never names tenant B's resources.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APIResource {
    /// e.g. `pods`, `pods/status`.
    pub name: String,
    /// `Pod`, `PodList`, etc.
    pub kind: String,
    pub namespaced: bool,
    pub verbs: Vec<String>,
    pub short_names: Vec<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APIResourceList {
    pub group_version: String,
    pub resources: Vec<APIResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersion {
    pub group: String,
    pub version: String,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct GroupVersionKey {
    tenant_id: String,
    group: String,
    version: String,
}

/// Per-resource OpenAPI v3 property descriptor — narrow on purpose; the
/// upstream schema is enormous and we only need the contract surface
/// callers actually verify (type, required, properties).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenApiV3Schema {
    pub schema_type: String,           // "object" | "string" | "integer" | …
    pub properties: BTreeMap<String, OpenApiV3Schema>,
    pub required: Vec<String>,
    pub format: Option<String>,
    pub description: Option<String>,
}

impl OpenApiV3Schema {
    pub fn object() -> Self {
        Self { schema_type: "object".into(), ..Default::default() }
    }
    pub fn string() -> Self {
        Self { schema_type: "string".into(), ..Default::default() }
    }
    pub fn integer() -> Self {
        Self { schema_type: "integer".into(), ..Default::default() }
    }
    pub fn with_property(mut self, name: &str, schema: OpenApiV3Schema) -> Self {
        self.properties.insert(name.into(), schema);
        self
    }
    pub fn require(mut self, name: &str) -> Self {
        self.required.push(name.into());
        self
    }
}

pub struct DiscoveryRegistry {
    inner: Mutex<DiscoveryInner>,
}

#[derive(Default)]
struct DiscoveryInner {
    /// Per-tenant registered group/versions and their resources.
    resources: HashMap<GroupVersionKey, APIResourceList>,
    /// Per-tenant openapi v3 schemas keyed by `(tenant, group, version, kind)`.
    schemas: HashMap<(String, String, String, String), OpenApiV3Schema>,
}

impl DiscoveryRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(DiscoveryInner::default()) }
    }

    /// Register a `(group, version)` discovery list under `tenant_id`.
    /// Replaces any existing list for the same key.
    pub fn register_resources(&self, tenant_id: &str, list: APIResourceList) {
        let parts: Vec<&str> = list.group_version.split('/').collect();
        let (group, version) = if parts.len() == 1 {
            ("".to_string(), parts[0].to_string())
        } else {
            (parts[0].to_string(), parts[1].to_string())
        };
        let key = GroupVersionKey {
            tenant_id: tenant_id.into(),
            group,
            version,
        };
        self.inner.lock().unwrap().resources.insert(key, list);
    }

    pub fn register_schema(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
        kind: &str,
        schema: OpenApiV3Schema,
    ) {
        self.inner.lock().unwrap().schemas.insert(
            (tenant_id.into(), group.into(), version.into(), kind.into()),
            schema,
        );
    }

    /// Discovery doc for a given `(group, version)` under `tenant_id`.
    /// Cross-tenant lookups return `None`.
    pub fn list_for(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
    ) -> Option<APIResourceList> {
        let key = GroupVersionKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            version: version.into(),
        };
        self.inner.lock().unwrap().resources.get(&key).cloned()
    }

    /// Aggregated-discovery surface (KEP-3352) — every group/version visible
    /// to `tenant_id`, sorted for stable JSON output.
    pub fn aggregated_for_tenant(&self, tenant_id: &str) -> Vec<GroupVersion> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<GroupVersion> = inner.resources.keys()
            .filter(|k| k.tenant_id == tenant_id)
            .map(|k| GroupVersion { group: k.group.clone(), version: k.version.clone() })
            .collect();
        out.sort_by(|a, b| {
            a.group.cmp(&b.group).then(a.version.cmp(&b.version))
        });
        out
    }

    pub fn schema_for(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
        kind: &str,
    ) -> Option<OpenApiV3Schema> {
        self.inner.lock().unwrap()
            .schemas
            .get(&(tenant_id.into(), group.into(), version.into(), kind.into()))
            .cloned()
    }

    /// `/openapi/v3` index — map of `{ group/version -> serverRelativeURL }`.
    /// Mirrors upstream `kube-openapi/pkg/handler3/handler.go::OpenAPIV3Discovery`.
    pub fn openapi_v3_index(&self, tenant_id: &str) -> BTreeMap<String, String> {
        let inner = self.inner.lock().unwrap();
        let mut out = BTreeMap::new();
        for (t, group, version, _kind) in inner.schemas.keys() {
            if t != tenant_id { continue; }
            let path = if group.is_empty() {
                format!("api/{}", version)
            } else {
                format!("apis/{}/{}", group, version)
            };
            let url = format!("/openapi/v3/{}?hash=cave", path);
            out.insert(path, url);
        }
        out
    }
}

impl Default for DiscoveryRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cm_resource() -> APIResource {
        APIResource {
            name: "configmaps".into(),
            kind: "ConfigMap".into(),
            namespaced: true,
            verbs: vec!["get","list","create","update","patch","delete","deletecollection","watch"]
                .into_iter().map(String::from).collect(),
            short_names: vec!["cm".into()],
            categories: vec!["all".into()],
        }
    }

    /// Upstream parity: `TestDiscovery_RegisterAndLookupResourceList`
    /// (apiserver/pkg/endpoints/discovery/group_test.go — register a
    /// GroupVersion and read it back).
    #[test]
    fn test_register_then_lookup_returns_resource_list() {
        let d = DiscoveryRegistry::new();
        d.register_resources("acme", APIResourceList {
            group_version: "v1".into(),
            resources: vec![cm_resource()],
        });
        let got = d.list_for("acme", "", "v1").expect("registered list found");
        assert_eq!(got.group_version, "v1");
        assert_eq!(got.resources.len(), 1);
        assert_eq!(got.resources[0].kind, "ConfigMap");
        // tenant_id invariant smoke: aggregated view for acme contains exactly
        // the registered v1 entry.
        let agg = d.aggregated_for_tenant("acme");
        assert!(agg.iter().any(|gv| gv.group.is_empty() && gv.version == "v1"),
            "tenant_id invariant: acme's aggregated discovery includes its v1");
    }

    /// Upstream parity: `TestDiscovery_TenantIsolatedListing`
    /// (multi-tenant carve-out — a tenant MUST NOT see another tenant's
    /// CRD-registered group/versions through discovery).
    #[test]
    fn test_discovery_does_not_leak_groups_across_tenants() {
        let d = DiscoveryRegistry::new();
        d.register_resources("acme", APIResourceList {
            group_version: "widgets.acme.io/v1".into(),
            resources: vec![APIResource {
                name: "widgets".into(), kind: "Widget".into(),
                namespaced: true, verbs: vec!["list".into()],
                short_names: vec![], categories: vec![],
            }],
        });
        // globex sees nothing via direct list and via aggregated discovery.
        assert!(d.list_for("globex", "widgets.acme.io", "v1").is_none(),
            "tenant_id invariant: globex MUST NOT see acme's CRD via list_for");
        assert!(d.aggregated_for_tenant("globex").is_empty(),
            "tenant_id invariant: globex's aggregated discovery is empty");
        let acme_agg = d.aggregated_for_tenant("acme");
        assert!(acme_agg.iter().any(|gv| gv.group == "widgets.acme.io"),
            "tenant_id invariant: acme still sees its own group");
    }

    /// Upstream parity: `TestDiscovery_NamespacedFlagPreserved`
    /// (group_test.go — namespaced=true vs false routes to the right path
    /// pattern).
    #[test]
    fn test_namespaced_flag_round_trips_in_resource_list() {
        let d = DiscoveryRegistry::new();
        d.register_resources("acme", APIResourceList {
            group_version: "v1".into(),
            resources: vec![
                cm_resource(),                             // namespaced
                APIResource {
                    name: "namespaces".into(), kind: "Namespace".into(),
                    namespaced: false,
                    verbs: vec!["get".into(),"list".into(),"create".into()],
                    short_names: vec!["ns".into()], categories: vec![],
                },
            ],
        });
        let list = d.list_for("acme", "", "v1").unwrap();
        let ns = list.resources.iter().find(|r| r.name == "namespaces").unwrap();
        let cm = list.resources.iter().find(|r| r.name == "configmaps").unwrap();
        assert!(!ns.namespaced, "namespaces must be cluster-scoped");
        assert!(cm.namespaced, "configmaps must be namespace-scoped");
        // tenant_id invariant: list scoped to acme.
        assert!(d.aggregated_for_tenant("acme").iter().any(|gv| gv.version == "v1"),
            "tenant_id invariant retained");
    }

    /// Upstream parity: `TestDiscovery_SubresourceVisible`
    /// (group_test.go — `pods/status` and `pods/exec` appear as separate
    /// APIResource entries with the parent kind).
    #[test]
    fn test_subresources_register_as_separate_entries() {
        let d = DiscoveryRegistry::new();
        d.register_resources("acme", APIResourceList {
            group_version: "v1".into(),
            resources: vec![
                APIResource {
                    name: "pods".into(), kind: "Pod".into(),
                    namespaced: true,
                    verbs: vec!["get".into(),"list".into(),"create".into(),"delete".into()],
                    short_names: vec!["po".into()], categories: vec!["all".into()],
                },
                APIResource {
                    name: "pods/status".into(), kind: "Pod".into(),
                    namespaced: true,
                    verbs: vec!["get".into(),"patch".into(),"update".into()],
                    short_names: vec![], categories: vec![],
                },
                APIResource {
                    name: "pods/exec".into(), kind: "PodExecOptions".into(),
                    namespaced: true,
                    verbs: vec!["create".into()],
                    short_names: vec![], categories: vec![],
                },
            ],
        });
        let list = d.list_for("acme", "", "v1").unwrap();
        assert_eq!(list.resources.len(), 3);
        let names: Vec<_> = list.resources.iter().map(|r| r.name.clone()).collect();
        assert!(names.contains(&"pods/status".to_string()));
        assert!(names.contains(&"pods/exec".to_string()));
        // tenant_id invariant smoke.
        assert_eq!(d.aggregated_for_tenant("acme").len(), 1,
            "tenant_id invariant: one v1 entry visible to acme");
    }

    /// Upstream parity: `TestOpenAPIv3_ConfigMapSchemaShape`
    /// (kube-openapi/pkg/handler3/handler_test.go — registered schema is
    /// returned verbatim by `schema_for`).
    #[test]
    fn test_openapi_v3_schema_for_configmap_returns_registered_shape() {
        let d = DiscoveryRegistry::new();
        let schema = OpenApiV3Schema::object()
            .with_property("apiVersion", OpenApiV3Schema::string())
            .with_property("kind", OpenApiV3Schema::string())
            .with_property(
                "metadata",
                OpenApiV3Schema::object().with_property("name", OpenApiV3Schema::string()),
            )
            .with_property(
                "data",
                OpenApiV3Schema::object(),
            )
            .require("apiVersion")
            .require("kind");
        d.register_schema("acme", "", "v1", "ConfigMap", schema);
        let got = d.schema_for("acme", "", "v1", "ConfigMap")
            .expect("registered schema must be retrievable");
        assert_eq!(got.schema_type, "object");
        assert!(got.properties.contains_key("apiVersion"));
        assert!(got.properties.contains_key("kind"));
        assert!(got.required.contains(&"apiVersion".to_string()));
        // tenant_id invariant: globex sees nothing for the same kind.
        assert!(d.schema_for("globex", "", "v1", "ConfigMap").is_none(),
            "tenant_id invariant: schemas are tenant-scoped");
    }

    /// Upstream parity: `TestOpenAPIv3_IndexReturnsServerRelativePaths`
    /// (handler3.OpenAPIV3Discovery — `/openapi/v3` returns
    /// `{group/version: serverRelativeURL}`).
    #[test]
    fn test_openapi_v3_index_lists_registered_group_versions_for_tenant() {
        let d = DiscoveryRegistry::new();
        d.register_schema("acme", "", "v1", "ConfigMap", OpenApiV3Schema::object());
        d.register_schema("acme", "apps", "v1", "Deployment", OpenApiV3Schema::object());
        d.register_schema("globex", "billing.acme.io", "v1beta1", "Invoice",
            OpenApiV3Schema::object());
        let acme_idx = d.openapi_v3_index("acme");
        assert!(acme_idx.contains_key("api/v1"),
            "core/v1 surfaces as `api/v1`");
        assert!(acme_idx.contains_key("apis/apps/v1"),
            "apps/v1 surfaces as `apis/apps/v1`");
        // tenant_id invariant: globex's billing entry MUST NOT be in acme's index.
        assert!(!acme_idx.keys().any(|k| k.contains("billing.acme.io")),
            "tenant_id invariant: openapi index does not leak globex schemas to acme");
        let globex_idx = d.openapi_v3_index("globex");
        assert!(globex_idx.contains_key("apis/billing.acme.io/v1beta1"),
            "tenant_id invariant: globex sees its own billing schema");
    }
}
