// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Kubernetes cluster scanner.
//!
//! Mirrors trivy's `pkg/k8s` for cave-trivy MVP: given a pre-collected list
//! of `K8sResource` (apiVersion + kind + namespace + manifest bytes), the
//! scanner runs the misconfig registry against each and aggregates the
//! per-resource findings into a single Report. Live `kubectl` collection
//! is delegated to cave-k8s tooling — cave-trivy ingests the resource
//! list.

use crate::error::TrivyResult;
use crate::misconf::MisconfRegistry;
use crate::models::{Report, ScanResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K8sResource {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub manifest_yaml: String,
}

#[derive(Debug, Clone)]
pub struct K8sClusterSnapshot {
    pub context: String,
    pub resources: Vec<K8sResource>,
}

pub fn scan_cluster(snap: &K8sClusterSnapshot, reg: &MisconfRegistry) -> TrivyResult<Report> {
    let mut report = Report::new(&snap.context, "k8s_cluster");
    for r in &snap.resources {
        let f = reg.evaluate("kubernetes", &resource_path(r), &r.manifest_yaml);
        if !f.is_empty() {
            report.results.push(ScanResult {
                target: resource_path(r),
                class: "config".into(),
                misconfigurations: f,
                ..Default::default()
            });
        }
    }
    Ok(report)
}

pub fn resource_path(r: &K8sResource) -> String {
    if r.namespace.is_empty() {
        format!("{}/{}", r.kind, r.name)
    } else {
        format!("{}/{}/{}", r.kind, r.namespace, r.name)
    }
}

/// Quick filter used by the cavectl client to limit scan scope.
pub fn filter_by_kinds<'a>(s: &'a K8sClusterSnapshot, kinds: &[&str]) -> Vec<&'a K8sResource> {
    s.resources
        .iter()
        .filter(|r| kinds.iter().any(|k| k.eq_ignore_ascii_case(&r.kind)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> K8sClusterSnapshot {
        K8sClusterSnapshot {
            context: "kind-cave".into(),
            resources: vec![
                K8sResource {
                    kind: "Pod".into(),
                    namespace: "default".into(),
                    name: "p1".into(),
                    manifest_yaml: "spec:\n  containers:\n  - name: c\n    securityContext:\n      privileged: true\n".into(),
                },
                K8sResource {
                    kind: "Deployment".into(),
                    namespace: "kube-system".into(),
                    name: "d1".into(),
                    manifest_yaml: "spec:\n  template:\n    spec:\n      hostNetwork: true\n".into(),
                },
            ],
        }
    }

    #[test]
    fn finds_privileged_pod() {
        let r = scan_cluster(&snap(), &MisconfRegistry::builtin()).unwrap();
        assert!(r
            .results
            .iter()
            .any(|s| s.misconfigurations.iter().any(|m| m.id == "AVD-KSV-0017")));
    }

    #[test]
    fn finds_host_network() {
        let r = scan_cluster(&snap(), &MisconfRegistry::builtin()).unwrap();
        assert!(r
            .results
            .iter()
            .any(|s| s.misconfigurations.iter().any(|m| m.id == "AVD-KSV-0044")));
    }

    #[test]
    fn resource_path_format() {
        let r = K8sResource {
            kind: "Pod".into(),
            namespace: "ns".into(),
            name: "p".into(),
            manifest_yaml: "".into(),
        };
        assert_eq!(resource_path(&r), "Pod/ns/p");
        let r2 = K8sResource {
            kind: "ClusterRole".into(),
            namespace: "".into(),
            name: "admin".into(),
            manifest_yaml: "".into(),
        };
        assert_eq!(resource_path(&r2), "ClusterRole/admin");
    }

    #[test]
    fn filter_by_kinds_works() {
        let s = snap();
        let f = filter_by_kinds(&s, &["pod"]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "Pod");
    }

    #[test]
    fn report_artifact_type() {
        let r = scan_cluster(&snap(), &MisconfRegistry::builtin()).unwrap();
        assert_eq!(r.artifact_type, "k8s_cluster");
        assert_eq!(r.artifact_name, "kind-cave");
    }
}
