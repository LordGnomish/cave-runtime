// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAPI v3 schema aggregator.
//!
//! cave-apiserver ships its own built-in v3 OpenAPI schemas; cave-k8s
//! merges them with the per-CRD schemas pulled from the
//! [`crd::CrdRegistry`] so that the `/openapi/v3` endpoint serves a
//! single composed document.  Mirrors
//! `staging/src/k8s.io/apiserver/pkg/endpoints/openapi`.

use crate::crd::CrdRegistry;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiDoc {
    pub openapi: String,
    pub info: serde_json::Value,
    pub paths: serde_json::Value,
    pub components: serde_json::Value,
}

pub struct OpenApiAggregator {
    pub crds: Arc<CrdRegistry>,
}

impl OpenApiAggregator {
    pub fn new(crds: Arc<CrdRegistry>) -> Self {
        Self { crds }
    }

    /// Compose the top-level v3 document from builtin schemas + CRDs.
    pub fn compose(&self) -> OpenApiDoc {
        let mut schemas = base_component_schemas();
        for crd in self.crds.list() {
            for v in crd.served_versions() {
                let key = format!("{}.{}.{}", crd.group, v.name, crd.kind);
                if v.schema.is_object() {
                    schemas[key] = v.schema.clone();
                }
            }
        }
        OpenApiDoc {
            openapi: "3.0.0".into(),
            info: json!({
                "title": "cave-k8s",
                "version": "v1.32.0",
                "description": "cave-runtime Kubernetes-compatible control plane"
            }),
            paths: base_paths(),
            components: json!({"schemas": schemas}),
        }
    }
}

fn base_component_schemas() -> Value {
    json!({
        "io.k8s.api.core.v1.Pod": {"type": "object"},
        "io.k8s.api.core.v1.Service": {"type": "object"},
        "io.k8s.api.core.v1.ConfigMap": {"type": "object"},
        "io.k8s.api.core.v1.Secret": {"type": "object"},
        "io.k8s.api.core.v1.Namespace": {"type": "object"},
        "io.k8s.api.core.v1.Node": {"type": "object"},
        "io.k8s.api.core.v1.PersistentVolume": {"type": "object"},
        "io.k8s.api.core.v1.PersistentVolumeClaim": {"type": "object"},
        "io.k8s.api.apps.v1.Deployment": {"type": "object"},
        "io.k8s.api.apps.v1.StatefulSet": {"type": "object"},
        "io.k8s.api.apps.v1.DaemonSet": {"type": "object"},
        "io.k8s.api.apps.v1.ReplicaSet": {"type": "object"},
        "io.k8s.api.batch.v1.Job": {"type": "object"},
        "io.k8s.api.batch.v1.CronJob": {"type": "object"},
        "io.k8s.api.networking.v1.Ingress": {"type": "object"},
        "io.k8s.api.rbac.v1.Role": {"type": "object"},
        "io.k8s.api.rbac.v1.RoleBinding": {"type": "object"},
        "io.k8s.api.rbac.v1.ClusterRole": {"type": "object"},
        "io.k8s.api.rbac.v1.ClusterRoleBinding": {"type": "object"},
    })
}

fn base_paths() -> Value {
    json!({
        "/api/v1": {"get": {"operationId": "getCoreV1ApiDocs", "responses": {"200": {"description": "OK"}}}},
        "/apis/apps/v1": {"get": {"operationId": "getAppsV1ApiDocs", "responses": {"200": {"description": "OK"}}}},
        "/apis/batch/v1": {"get": {"operationId": "getBatchV1ApiDocs", "responses": {"200": {"description": "OK"}}}},
        "/apis/rbac.authorization.k8s.io/v1": {"get": {"operationId": "getRbacV1ApiDocs", "responses": {"200": {"description": "OK"}}}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_emits_v3_openapi() {
        let a = OpenApiAggregator::new(Arc::new(CrdRegistry::new()));
        let d = a.compose();
        assert_eq!(d.openapi, "3.0.0");
        assert!(d.components["schemas"]["io.k8s.api.core.v1.Pod"].is_object());
    }

    #[test]
    fn compose_includes_crd_schemas() {
        let crds = Arc::new(CrdRegistry::new());
        crds.install(crate::crd::Crd {
            group: "cave.example.com".into(),
            plural: "widgets".into(),
            kind: "Widget".into(),
            scope: crate::crd::Scope::Namespaced,
            versions: vec![crate::crd::CrdVersion {
                name: "v1".into(),
                served: true,
                storage: true,
                schema: serde_json::json!({"type": "object", "properties": {"size": {"type": "integer"}}}),
            }],
        })
        .unwrap();
        let a = OpenApiAggregator::new(crds);
        let d = a.compose();
        let key = "cave.example.com.v1.Widget";
        assert!(d.components["schemas"][key].is_object());
        assert_eq!(d.components["schemas"][key]["type"], "object");
    }

    #[test]
    fn paths_include_core_and_apps() {
        let a = OpenApiAggregator::new(Arc::new(CrdRegistry::new()));
        let d = a.compose();
        assert!(d.paths["/api/v1"].is_object());
        assert!(d.paths["/apis/apps/v1"].is_object());
    }

    #[test]
    fn base_schemas_cover_nineteen_kinds() {
        let s = base_component_schemas();
        let obj = s.as_object().unwrap();
        assert_eq!(obj.len(), 19);
    }
}
