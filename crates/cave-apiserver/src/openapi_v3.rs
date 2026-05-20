// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Live OpenAPI v3 handler.
//!
//! Mirrors `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/` from
//! kubernetes/kubernetes v1.36.0 — upstream synthesises a per-group
//! OpenAPI v3 document by walking the registered REST stores. Each
//! registered resource contributes a schema fragment under
//! `components.schemas.<group>.<kind>` and a paths block.
//!
//! cave-apiserver previously served a static OpenAPI document via
//! [`crate::discovery_v2`]. This module computes the document from a
//! [`Registry`] of resources at request time, so newly-registered CRDs
//! show up without restarting the server.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenApiV3Document {
    pub openapi: String,
    pub info: OpenApiInfo,
    pub paths: BTreeMap<String, OpenApiPathItem>,
    pub components: OpenApiComponents,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiInfo {
    pub title: String,
    pub version: String,
}

impl Default for OpenApiInfo {
    fn default() -> Self {
        Self {
            title: "Kubernetes".into(),
            version: "v1.36.0".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenApiPathItem {
    pub get: Option<OpenApiOperation>,
    pub post: Option<OpenApiOperation>,
    pub put: Option<OpenApiOperation>,
    pub delete: Option<OpenApiOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiOperation {
    pub operation_id: String,
    pub tags: Vec<String>,
    pub responses: BTreeMap<String, OpenApiResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiResponse {
    pub description: String,
    pub content: BTreeMap<String, OpenApiMediaType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiMediaType {
    pub schema: OpenApiSchemaRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSchemaRef {
    #[serde(rename = "$ref")]
    pub r#ref: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenApiComponents {
    pub schemas: BTreeMap<String, OpenApiSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenApiSchema {
    #[serde(rename = "type")]
    pub r#type: String,
    pub description: Option<String>,
    pub properties: BTreeMap<String, OpenApiSchema>,
    pub required: Vec<String>,
    /// `x-kubernetes-group-version-kind` — KEP-3962 type identification.
    #[serde(
        rename = "x-kubernetes-group-version-kind",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub gvk: Vec<GroupVersionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionKind {
    pub group: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct ResourceRegistration {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespaced: bool,
    pub schema: OpenApiSchema,
}

#[derive(Default)]
pub struct Registry {
    resources: RwLock<Vec<ResourceRegistration>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, r: ResourceRegistration) {
        self.resources.write().unwrap().push(r);
    }

    pub fn len(&self) -> usize {
        self.resources.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Synthesise the full v3 document.
    pub fn build(&self) -> OpenApiV3Document {
        let mut doc = OpenApiV3Document {
            openapi: "3.0.0".into(),
            ..Default::default()
        };

        for r in self.resources.read().unwrap().iter() {
            let schema_key = format!("{}.{}.{}", r.group, r.version, r.kind);
            let mut s = r.schema.clone();
            s.gvk.push(GroupVersionKind {
                group: r.group.clone(),
                version: r.version.clone(),
                kind: r.kind.clone(),
            });
            doc.components.schemas.insert(schema_key.clone(), s);

            let list_path = if r.group.is_empty() {
                if r.namespaced {
                    format!(
                        "/api/{ver}/namespaces/{{namespace}}/{plural}",
                        ver = r.version,
                        plural = kind_to_plural(&r.kind),
                    )
                } else {
                    format!(
                        "/api/{ver}/{plural}",
                        ver = r.version,
                        plural = kind_to_plural(&r.kind)
                    )
                }
            } else if r.namespaced {
                format!(
                    "/apis/{group}/{ver}/namespaces/{{namespace}}/{plural}",
                    group = r.group,
                    ver = r.version,
                    plural = kind_to_plural(&r.kind),
                )
            } else {
                format!(
                    "/apis/{group}/{ver}/{plural}",
                    group = r.group,
                    ver = r.version,
                    plural = kind_to_plural(&r.kind),
                )
            };

            let item_path = format!("{list_path}/{{name}}");

            let list_op = OpenApiOperation {
                operation_id: format!("list{}{}{}", r.group, r.version, r.kind),
                tags: vec![if r.group.is_empty() {
                    "core".into()
                } else {
                    r.group.clone()
                }],
                responses: BTreeMap::from([(
                    "200".to_string(),
                    OpenApiResponse {
                        description: "OK".into(),
                        content: BTreeMap::from([(
                            "application/json".into(),
                            OpenApiMediaType {
                                schema: OpenApiSchemaRef {
                                    r#ref: format!("#/components/schemas/{schema_key}List"),
                                },
                            },
                        )]),
                    },
                )]),
            };

            let get_op = OpenApiOperation {
                operation_id: format!("read{}{}{}", r.group, r.version, r.kind),
                tags: list_op.tags.clone(),
                responses: BTreeMap::from([(
                    "200".to_string(),
                    OpenApiResponse {
                        description: "OK".into(),
                        content: BTreeMap::from([(
                            "application/json".into(),
                            OpenApiMediaType {
                                schema: OpenApiSchemaRef {
                                    r#ref: format!("#/components/schemas/{schema_key}"),
                                },
                            },
                        )]),
                    },
                )]),
            };

            doc.paths.insert(
                list_path.clone(),
                OpenApiPathItem {
                    get: Some(list_op),
                    post: Some(OpenApiOperation {
                        operation_id: format!("create{}{}{}", r.group, r.version, r.kind),
                        tags: vec![if r.group.is_empty() {
                            "core".into()
                        } else {
                            r.group.clone()
                        }],
                        responses: BTreeMap::from([(
                            "201".to_string(),
                            OpenApiResponse {
                                description: "Created".into(),
                                content: BTreeMap::from([(
                                    "application/json".into(),
                                    OpenApiMediaType {
                                        schema: OpenApiSchemaRef {
                                            r#ref: format!("#/components/schemas/{schema_key}"),
                                        },
                                    },
                                )]),
                            },
                        )]),
                    }),
                    ..Default::default()
                },
            );

            doc.paths.insert(
                item_path,
                OpenApiPathItem {
                    get: Some(get_op),
                    delete: Some(OpenApiOperation {
                        operation_id: format!("delete{}{}{}", r.group, r.version, r.kind),
                        tags: vec![if r.group.is_empty() {
                            "core".into()
                        } else {
                            r.group.clone()
                        }],
                        responses: BTreeMap::from([(
                            "200".to_string(),
                            OpenApiResponse {
                                description: "OK".into(),
                                content: BTreeMap::new(),
                            },
                        )]),
                    }),
                    ..Default::default()
                },
            );
        }

        doc
    }
}

/// Naive pluraliser — matches upstream `meta/v1.Kind.Plural` lower-case
/// rules for the handful of irregulars cave-apiserver supports
/// (endpoints, ingresses, …) and otherwise lower-cases + appends `s`.
pub fn kind_to_plural(kind: &str) -> String {
    let l = kind.to_lowercase();
    match l.as_str() {
        "endpoints" => "endpoints".into(),
        "ingress" => "ingresses".into(),
        "policy" => "policies".into(),
        "networkpolicy" => "networkpolicies".into(),
        "podsecuritypolicy" => "podsecuritypolicies".into(),
        "storageclass" => "storageclasses".into(),
        _ if l.ends_with('s') => format!("{l}es"),
        _ if l.ends_with('y') => format!("{}ies", &l[..l.len() - 1]),
        _ => format!("{l}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_reg() -> ResourceRegistration {
        ResourceRegistration {
            group: "".into(),
            version: "v1".into(),
            kind: "Pod".into(),
            namespaced: true,
            schema: OpenApiSchema {
                r#type: "object".into(),
                description: Some("Pod".into()),
                ..Default::default()
            },
        }
    }

    fn node_reg() -> ResourceRegistration {
        ResourceRegistration {
            group: "".into(),
            version: "v1".into(),
            kind: "Node".into(),
            namespaced: false,
            schema: OpenApiSchema {
                r#type: "object".into(),
                ..Default::default()
            },
        }
    }

    fn deployment_reg() -> ResourceRegistration {
        ResourceRegistration {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            namespaced: true,
            schema: OpenApiSchema {
                r#type: "object".into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn empty_registry_yields_empty_doc() {
        let r = Registry::new();
        let d = r.build();
        assert_eq!(d.openapi, "3.0.0");
        assert!(d.paths.is_empty());
        assert!(d.components.schemas.is_empty());
    }

    #[test]
    fn pod_registration_emits_namespaced_list_and_item_paths() {
        let r = Registry::new();
        r.register(pod_reg());
        let d = r.build();
        assert!(d.paths.contains_key("/api/v1/namespaces/{namespace}/pods"));
        assert!(d.paths.contains_key("/api/v1/namespaces/{namespace}/pods/{name}"));
    }

    #[test]
    fn node_registration_emits_cluster_scoped_path() {
        let r = Registry::new();
        r.register(node_reg());
        let d = r.build();
        assert!(d.paths.contains_key("/api/v1/nodes"));
        assert!(!d.paths.keys().any(|p| p.contains("namespaces")));
    }

    #[test]
    fn group_registration_emits_apis_prefix() {
        let r = Registry::new();
        r.register(deployment_reg());
        let d = r.build();
        assert!(d
            .paths
            .contains_key("/apis/apps/v1/namespaces/{namespace}/deployments"));
    }

    #[test]
    fn schema_carries_gvk_annotation() {
        let r = Registry::new();
        r.register(pod_reg());
        let d = r.build();
        let s = d.components.schemas.get(".v1.Pod").unwrap();
        assert_eq!(s.gvk.len(), 1);
        assert_eq!(s.gvk[0].kind, "Pod");
        assert_eq!(s.gvk[0].version, "v1");
    }

    #[test]
    fn list_op_has_correct_operation_id_and_tag() {
        let r = Registry::new();
        r.register(pod_reg());
        let d = r.build();
        let list = d.paths.get("/api/v1/namespaces/{namespace}/pods").unwrap();
        let get = list.get.as_ref().unwrap();
        assert_eq!(get.operation_id, "listv1Pod");
        assert_eq!(get.tags, vec!["core".to_string()]);
    }

    #[test]
    fn kind_to_plural_handles_irregulars() {
        assert_eq!(kind_to_plural("Pod"), "pods");
        assert_eq!(kind_to_plural("Endpoints"), "endpoints");
        assert_eq!(kind_to_plural("Ingress"), "ingresses");
        assert_eq!(kind_to_plural("NetworkPolicy"), "networkpolicies");
        assert_eq!(kind_to_plural("StorageClass"), "storageclasses");
    }

    #[test]
    fn registry_len_tracks_registrations() {
        let r = Registry::new();
        assert!(r.is_empty());
        r.register(pod_reg());
        r.register(node_reg());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn registered_after_initial_build_is_visible_on_next_build() {
        let r = Registry::new();
        r.register(pod_reg());
        let d1 = r.build();
        assert_eq!(d1.paths.len(), 2);
        r.register(deployment_reg());
        let d2 = r.build();
        assert!(d2.paths.len() > d1.paths.len());
    }
}
