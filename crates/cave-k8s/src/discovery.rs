// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! /api + /apis discovery surface.
//!
//! Mirrors `pkg/api/server/handlers/discovery` plus the aggregator
//! layer in `pkg/server/genericapiserver/discovery`.  cave-k8s merges
//! three sources:
//!
//!   1. built-in core / apps / batch / networking / rbac / storage
//!   2. CRD registry (`crd::CrdRegistry`)
//!   3. APIService registry (`aggregator::AggregatorRegistry`)

use crate::aggregator::AggregatorRegistry;
use crate::crd::CrdRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiGroup {
    pub name: String,
    pub versions: Vec<String>,
    pub preferred_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryDoc {
    pub api_versions: Vec<String>,
    pub groups: Vec<ApiGroup>,
}

pub struct Discovery {
    pub crds: Arc<CrdRegistry>,
    pub aggregator: Arc<AggregatorRegistry>,
}

impl Discovery {
    pub fn new(crds: Arc<CrdRegistry>, aggregator: Arc<AggregatorRegistry>) -> Self {
        Self { crds, aggregator }
    }

    pub fn builtin_groups() -> Vec<ApiGroup> {
        vec![
            ApiGroup {
                name: "apps".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "batch".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "discovery.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "networking.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "rbac.authorization.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "storage.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "policy".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "autoscaling".into(),
                versions: vec!["v2".into()],
                preferred_version: "v2".into(),
            },
            ApiGroup {
                name: "admissionregistration.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "apiextensions.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "apiregistration.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "scheduling.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "coordination.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "events.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
            ApiGroup {
                name: "node.k8s.io".into(),
                versions: vec!["v1".into()],
                preferred_version: "v1".into(),
            },
        ]
    }

    pub fn doc(&self) -> DiscoveryDoc {
        let mut groups = Self::builtin_groups();
        // Add CRD-backed groups.
        let mut crd_groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for crd in self.crds.list() {
            let entry = crd_groups.entry(crd.group.clone()).or_default();
            for v in crd.served_versions() {
                if !entry.contains(&v.name) {
                    entry.push(v.name.clone());
                }
            }
        }
        for (g, versions) in crd_groups {
            if let Some(preferred) = versions.first().cloned() {
                groups.push(ApiGroup {
                    name: g,
                    versions,
                    preferred_version: preferred,
                });
            }
        }
        // Aggregator-backed groups.
        for ag in self.aggregator.available_groups() {
            if !groups.iter().any(|g| g.name == ag) {
                groups.push(ApiGroup {
                    name: ag.clone(),
                    versions: vec!["v1".into()],
                    preferred_version: "v1".into(),
                });
            }
        }
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        DiscoveryDoc {
            api_versions: vec!["v1".into()],
            groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_fifteen_groups() {
        assert_eq!(Discovery::builtin_groups().len(), 15);
    }

    #[test]
    fn doc_includes_builtin_only_when_no_extras() {
        let d = Discovery::new(
            Arc::new(CrdRegistry::new()),
            Arc::new(AggregatorRegistry::new()),
        );
        assert_eq!(d.doc().groups.len(), 15);
    }

    #[test]
    fn doc_includes_crd_groups() {
        let crds = Arc::new(CrdRegistry::new());
        crds.install(crate::crd::Crd {
            group: "cave.example.com".into(),
            plural: "widgets".into(),
            kind: "Widget".into(),
            scope: crate::crd::Scope::Namespaced,
            versions: vec![crate::crd::CrdVersion {
                name: "v1alpha1".into(),
                served: true,
                storage: true,
                schema: serde_json::json!({}),
            }],
        })
        .unwrap();
        let d = Discovery::new(crds, Arc::new(AggregatorRegistry::new()));
        let doc = d.doc();
        assert!(doc.groups.iter().any(|g| g.name == "cave.example.com"));
    }

    #[test]
    fn doc_includes_available_aggregator_groups() {
        let aggr = Arc::new(AggregatorRegistry::new());
        aggr.register(crate::aggregator::ApiService {
            name: "v1.metrics.k8s.io".into(),
            group: "metrics.k8s.io".into(),
            version: "v1".into(),
            service: "kube-system/metrics:443".into(),
            insecure_skip_tls_verify: false,
            group_priority_minimum: 100,
            version_priority: 10,
        });
        aggr.mark_available("v1.metrics.k8s.io");
        let d = Discovery::new(Arc::new(CrdRegistry::new()), aggr);
        let doc = d.doc();
        assert!(doc.groups.iter().any(|g| g.name == "metrics.k8s.io"));
    }

    #[test]
    fn doc_sorted_alphabetically() {
        let d = Discovery::new(
            Arc::new(CrdRegistry::new()),
            Arc::new(AggregatorRegistry::new()),
        );
        let doc = d.doc();
        let names: Vec<_> = doc.groups.iter().map(|g| &g.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn aggregator_pending_groups_hidden() {
        let aggr = Arc::new(AggregatorRegistry::new());
        aggr.register(crate::aggregator::ApiService {
            name: "v1.pending.io".into(),
            group: "pending.io".into(),
            version: "v1".into(),
            service: "ns/svc:443".into(),
            insecure_skip_tls_verify: false,
            group_priority_minimum: 100,
            version_priority: 10,
        });
        let d = Discovery::new(Arc::new(CrdRegistry::new()), aggr);
        let doc = d.doc();
        assert!(!doc.groups.iter().any(|g| g.name == "pending.io"));
    }
}
