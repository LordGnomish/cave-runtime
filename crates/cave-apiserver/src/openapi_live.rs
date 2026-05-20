// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Live OpenAPI v3 synthesis.
//!
//! Upstream: `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/`
//!
//! Upstream synthesises an OpenAPI v3 document on demand by walking the
//! registered API types in each group, emitting one document per
//! `<group>/<version>`. The cave port previously served a static-ish
//! OpenAPI shape from `discovery_v2::OpenAPIV3Index` — the schema list
//! came from the discovery layer but the per-type schemas were not
//! re-synthesised from live type registrations.
//!
//! This module ports the missing piece: `LiveOpenApiBuilder` accepts
//! registered resource schemas (one per group/version/resource) and
//! emits `OpenAPIV3Document` instances on demand. The builder reuses
//! the discovery layer's existing v3-schema shape (see
//! `discovery_v2.rs`), so callers can plug the live builder in place
//! of the static index without breaking existing clients.
//!
//! Tenant invariant: the document itself is tenant-agnostic (OpenAPI
//! schemas describe shape, not data). Tenant scoping happens at the
//! handler layer that serves the route.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::RwLock;

/// One OpenAPI v3 schema definition — the shape that gets emitted in
/// the document under `components.schemas`. Mirrors
/// `staging/src/k8s.io/kube-openapi/pkg/spec3.Schema` minus features
/// cave does not currently exercise (e.g. discriminators).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenApiSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, OpenApiSchema>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,
}

impl OpenApiSchema {
    pub fn object() -> Self {
        Self {
            schema_type: "object".into(),
            ..Default::default()
        }
    }

    pub fn string() -> Self {
        Self {
            schema_type: "string".into(),
            ..Default::default()
        }
    }

    pub fn integer() -> Self {
        Self {
            schema_type: "integer".into(),
            ..Default::default()
        }
    }

    pub fn with_required<I: IntoIterator<Item = impl Into<String>>>(mut self, fields: I) -> Self {
        self.required = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_property(mut self, name: impl Into<String>, schema: OpenApiSchema) -> Self {
        self.properties.insert(name.into(), schema);
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
}

/// Group/version/resource identifier — the key under which a schema is
/// registered.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct GroupVersionResource {
    pub group: String,
    pub version: String,
    pub resource: String,
}

impl GroupVersionResource {
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            resource: resource.into(),
        }
    }

    /// Path under which the live document is served, e.g.
    /// `openapi/v3/apis/apps/v1` or `openapi/v3/api/v1` (core group).
    pub fn document_path(&self) -> String {
        if self.group.is_empty() {
            format!("openapi/v3/api/{}", self.version)
        } else {
            format!("openapi/v3/apis/{}/{}", self.group, self.version)
        }
    }

    /// Components-schemas key, e.g. `io.k8s.api.apps.v1.Deployment`.
    pub fn components_key(&self) -> String {
        if self.group.is_empty() {
            format!("io.k8s.api.core.{}.{}", self.version, capitalise(&self.resource))
        } else {
            format!(
                "io.k8s.api.{}.{}.{}",
                self.group.replace('.', "_"),
                self.version,
                capitalise(&self.resource)
            )
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// One OpenAPI v3 info block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenApiInfo {
    pub title: String,
    pub version: String,
}

/// A single OpenAPI v3 document covering one group/version. Mirrors the
/// upstream `spec3.OpenAPI` shape (minimal subset).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAPIV3Document {
    pub openapi: String,
    pub info: OpenApiInfo,
    pub components: OpenApiComponents,
    pub paths: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenApiComponents {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, OpenApiSchema>,
}

/// Live OpenAPI v3 builder.
///
/// Resources are registered once at startup (or on CRD admission) and
/// the builder emits a fresh per-group document on every `build_for`
/// call. The implementation keeps everything in-memory; the `RwLock`
/// allows hot-reload (CRD create/delete) without rebuilding the whole
/// server.
pub struct LiveOpenApiBuilder {
    /// Registered schemas, keyed by GVR for stable group iteration.
    schemas: RwLock<BTreeMap<GroupVersionResource, OpenApiSchema>>,
    title: String,
    version: String,
}

impl LiveOpenApiBuilder {
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            schemas: RwLock::new(BTreeMap::new()),
            title: title.into(),
            version: version.into(),
        }
    }

    /// Register or replace the schema for a GVR.
    pub async fn register(&self, gvr: GroupVersionResource, schema: OpenApiSchema) {
        self.schemas.write().await.insert(gvr, schema);
    }

    /// Drop the schema for a GVR. Returns `true` if it was registered.
    pub async fn unregister(&self, gvr: &GroupVersionResource) -> bool {
        self.schemas.write().await.remove(gvr).is_some()
    }

    /// Total registered schemas.
    pub async fn len(&self) -> usize {
        self.schemas.read().await.len()
    }

    /// List of `(group, version)` pairs that currently have at least
    /// one registered resource.
    pub async fn groups(&self) -> Vec<(String, String)> {
        let mut groups: Vec<(String, String)> = self
            .schemas
            .read()
            .await
            .keys()
            .map(|k| (k.group.clone(), k.version.clone()))
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    /// Build the OpenAPI v3 document for a single `(group, version)`.
    /// Returns `None` if no resources are registered for that pair.
    pub async fn build_for(&self, group: &str, version: &str) -> Option<OpenAPIV3Document> {
        let g = self.schemas.read().await;
        let mut components = OpenApiComponents::default();
        let mut paths: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let mut any = false;
        for (gvr, schema) in g.iter() {
            if gvr.group != group || gvr.version != version {
                continue;
            }
            any = true;
            components
                .schemas
                .insert(gvr.components_key(), schema.clone());
            // Emit a minimal path stub so consumers see the resource is real.
            let list_path = if gvr.group.is_empty() {
                format!("/api/{}/{}", gvr.version, gvr.resource)
            } else {
                format!("/apis/{}/{}/{}", gvr.group, gvr.version, gvr.resource)
            };
            paths.insert(
                list_path,
                serde_json::json!({
                    "get": {
                        "operationId": format!("list{}", capitalise(&gvr.resource)),
                        "responses": {
                            "200": {
                                "description": "ok",
                            }
                        }
                    }
                }),
            );
        }
        if !any {
            return None;
        }
        Some(OpenAPIV3Document {
            openapi: "3.0.0".into(),
            info: OpenApiInfo {
                title: self.title.clone(),
                version: self.version.clone(),
            },
            components,
            paths,
        })
    }

    /// Build the full OpenAPI v3 set (every registered group/version),
    /// keyed by `document_path()`.
    pub async fn build_all(&self) -> BTreeMap<String, OpenAPIV3Document> {
        let mut out = BTreeMap::new();
        for (group, version) in self.groups().await {
            if let Some(doc) = self.build_for(&group, &version).await {
                let gvr = GroupVersionResource::new(group, version, "_");
                let mut path = gvr.document_path();
                // strip the trailing `/_` we used as a sentinel
                if path.ends_with("/_") {
                    path.truncate(path.len() - 2);
                }
                out.insert(path, doc);
            }
        }
        out
    }
}

impl Default for LiveOpenApiBuilder {
    fn default() -> Self {
        Self::new("Cave APIServer", "v1.36.0")
    }
}

/// HTTP route helper — returns the path under which the live document
/// for a `(group, version)` should be served.
///
/// Examples:
///
/// ```ignore
/// assert_eq!(openapi_v3_route("apps", "v1"), "/openapi/v3/apis/apps/v1");
/// assert_eq!(openapi_v3_route("", "v1"), "/openapi/v3/api/v1");
/// ```
pub fn openapi_v3_route(group: &str, version: &str) -> String {
    if group.is_empty() {
        format!("/openapi/v3/api/{}", version)
    } else {
        format!("/openapi/v3/apis/{}/{}", group, version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_schema() -> OpenApiSchema {
        OpenApiSchema::object()
            .with_required(["apiVersion", "kind", "metadata", "spec"])
            .with_property("apiVersion", OpenApiSchema::string())
            .with_property("kind", OpenApiSchema::string())
            .with_property(
                "metadata",
                OpenApiSchema::object()
                    .with_property("name", OpenApiSchema::string())
                    .with_property("namespace", OpenApiSchema::string()),
            )
            .with_property("spec", OpenApiSchema::object())
            .with_description("Pod is a collection of containers that share resources")
    }

    fn deploy_schema() -> OpenApiSchema {
        OpenApiSchema::object()
            .with_required(["apiVersion", "kind", "metadata", "spec"])
            .with_property("apiVersion", OpenApiSchema::string())
            .with_property("kind", OpenApiSchema::string())
            .with_property(
                "spec",
                OpenApiSchema::object()
                    .with_property("replicas", OpenApiSchema::integer()),
            )
    }

    #[tokio::test]
    async fn register_and_build_for_core_v1_emits_a_document_with_pod_schema() {
        let b = LiveOpenApiBuilder::new("test", "v1.36.0");
        let gvr = GroupVersionResource::new("", "v1", "pods");
        b.register(gvr.clone(), pod_schema()).await;
        assert_eq!(b.len().await, 1);
        let doc = b.build_for("", "v1").await.expect("doc emitted");
        assert_eq!(doc.openapi, "3.0.0");
        assert_eq!(doc.info.version, "v1.36.0");
        // The component key for core/v1/Pod must follow the kube-openapi
        // convention: io.k8s.api.core.v1.Pods (best-effort capitalise).
        assert!(doc.components.schemas.contains_key(&gvr.components_key()));
        let pod = &doc.components.schemas[&gvr.components_key()];
        assert_eq!(pod.schema_type, "object");
        assert!(pod.required.contains(&"apiVersion".to_string()));
        // /api/v1/pods GET stub was emitted.
        assert!(doc.paths.contains_key("/api/v1/pods"));
    }

    #[tokio::test]
    async fn build_for_unknown_group_returns_none() {
        let b = LiveOpenApiBuilder::new("test", "v1.36.0");
        b.register(
            GroupVersionResource::new("", "v1", "pods"),
            pod_schema(),
        )
        .await;
        assert!(b.build_for("apps", "v1").await.is_none());
        assert!(b.build_for("", "v2").await.is_none());
    }

    #[tokio::test]
    async fn separate_groups_emit_separate_documents() {
        let b = LiveOpenApiBuilder::new("t", "v1");
        b.register(
            GroupVersionResource::new("", "v1", "pods"),
            pod_schema(),
        )
        .await;
        b.register(
            GroupVersionResource::new("apps", "v1", "deployments"),
            deploy_schema(),
        )
        .await;
        let core = b.build_for("", "v1").await.unwrap();
        let apps = b.build_for("apps", "v1").await.unwrap();
        // Each document carries only its own group's schemas.
        assert_eq!(core.components.schemas.len(), 1);
        assert_eq!(apps.components.schemas.len(), 1);
        assert!(core.paths.contains_key("/api/v1/pods"));
        assert!(apps.paths.contains_key("/apis/apps/v1/deployments"));
    }

    #[tokio::test]
    async fn unregister_drops_schema_and_subsequent_build_is_none() {
        let b = LiveOpenApiBuilder::default();
        let gvr = GroupVersionResource::new("apps", "v1", "deployments");
        b.register(gvr.clone(), deploy_schema()).await;
        assert!(b.unregister(&gvr).await);
        assert_eq!(b.len().await, 0);
        assert!(b.build_for("apps", "v1").await.is_none());
        // Second unregister returns false (already removed).
        assert!(!b.unregister(&gvr).await);
    }

    #[tokio::test]
    async fn build_all_returns_one_doc_per_group_version() {
        let b = LiveOpenApiBuilder::default();
        b.register(
            GroupVersionResource::new("", "v1", "pods"),
            pod_schema(),
        )
        .await;
        b.register(
            GroupVersionResource::new("apps", "v1", "deployments"),
            deploy_schema(),
        )
        .await;
        let all = b.build_all().await;
        assert_eq!(all.len(), 2);
        assert!(all.contains_key("openapi/v3/api/v1"));
        assert!(all.contains_key("openapi/v3/apis/apps/v1"));
    }

    #[test]
    fn openapi_v3_route_handles_core_and_named_groups() {
        assert_eq!(openapi_v3_route("", "v1"), "/openapi/v3/api/v1");
        assert_eq!(
            openapi_v3_route("apps", "v1"),
            "/openapi/v3/apis/apps/v1"
        );
        assert_eq!(
            openapi_v3_route("apiextensions.k8s.io", "v1"),
            "/openapi/v3/apis/apiextensions.k8s.io/v1"
        );
    }

    #[tokio::test]
    async fn document_round_trips_through_json_serde() {
        let b = LiveOpenApiBuilder::default();
        b.register(
            GroupVersionResource::new("", "v1", "pods"),
            pod_schema(),
        )
        .await;
        let doc = b.build_for("", "v1").await.unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        let parsed: OpenAPIV3Document = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, doc);
    }

    #[tokio::test]
    async fn groups_listing_is_deduplicated_and_sorted() {
        let b = LiveOpenApiBuilder::default();
        b.register(
            GroupVersionResource::new("", "v1", "pods"),
            pod_schema(),
        )
        .await;
        b.register(
            GroupVersionResource::new("", "v1", "services"),
            OpenApiSchema::object(),
        )
        .await;
        b.register(
            GroupVersionResource::new("apps", "v1", "deployments"),
            deploy_schema(),
        )
        .await;
        let groups = b.groups().await;
        assert_eq!(
            groups,
            vec![
                ("".to_string(), "v1".to_string()),
                ("apps".to_string(), "v1".to_string()),
            ]
        );
    }
}
