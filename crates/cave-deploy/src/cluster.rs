// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster registry + Kubernetes REST URL builders.
//!
//! The MVP scope does not include a kube::Client binding (deferred to Phase 2
//! `cave-deploy-runtime`). This module exposes the deterministic pieces the
//! sync engine needs: kind-to-plural mapping and URL construction.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Plural resource names for the well-known Kubernetes kinds.
pub fn kind_to_plural(kind: &str) -> String {
    match kind {
        "Deployment" => "deployments",
        "StatefulSet" => "statefulsets",
        "DaemonSet" => "daemonsets",
        "ReplicaSet" => "replicasets",
        "Pod" => "pods",
        "Service" => "services",
        "ServiceAccount" => "serviceaccounts",
        "ConfigMap" => "configmaps",
        "Secret" => "secrets",
        "Namespace" => "namespaces",
        "PersistentVolume" => "persistentvolumes",
        "PersistentVolumeClaim" => "persistentvolumeclaims",
        "Ingress" => "ingresses",
        "Job" => "jobs",
        "CronJob" => "cronjobs",
        "HorizontalPodAutoscaler" => "horizontalpodautoscalers",
        "NetworkPolicy" => "networkpolicies",
        "Role" => "roles",
        "RoleBinding" => "rolebindings",
        "ClusterRole" => "clusterroles",
        "ClusterRoleBinding" => "clusterrolebindings",
        "CustomResourceDefinition" => "customresourcedefinitions",
        "StorageClass" => "storageclasses",
        "ResourceQuota" => "resourcequotas",
        "LimitRange" => "limitranges",
        other => {
            let lower = other.to_lowercase();
            return if lower.ends_with('s') { lower } else { format!("{lower}s") };
        }
    }
    .to_string()
}

/// Build the namespaced/cluster-scoped Kubernetes REST path for a single resource.
pub fn build_resource_url(
    api_version: &str,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> String {
    let plural = kind_to_plural(kind);
    let api_path = if api_version.contains('/') {
        format!("apis/{api_version}")
    } else {
        format!("api/{api_version}")
    };
    match namespace {
        Some(ns) => format!("/{api_path}/namespaces/{ns}/{plural}/{name}"),
        None => format!("/{api_path}/{plural}/{name}"),
    }
}

/// Build the list path for a kind (optionally namespaced).
pub fn build_list_url(api_version: &str, kind: &str, namespace: Option<&str>) -> String {
    let plural = kind_to_plural(kind);
    let api_path = if api_version.contains('/') {
        format!("apis/{api_version}")
    } else {
        format!("api/{api_version}")
    };
    match namespace {
        Some(ns) => format!("/{api_path}/namespaces/{ns}/{plural}"),
        None => format!("/{api_path}/{plural}"),
    }
}

// ─── Cluster CRD ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub id: Uuid,
    pub name: String,
    /// Kubernetes API server URL (e.g. https://kubernetes.default.svc).
    pub server: String,
    /// Bearer-token / kubeconfig credential reference (keychain key — never
    /// inlined).
    pub credential_ref: Option<String>,
    /// Free-form labels for selector targeting.
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    /// Project that owns this cluster (empty = global).
    pub project: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Cluster {
    pub fn matches_labels(&self, selector: &HashMap<String, String>) -> bool {
        selector.iter().all(|(k, v)| self.labels.get(k) == Some(v))
    }
}

/// Tracking label key used by cave-deploy. Mirrors ArgoCD's well-known key so
/// that imports of an existing fleet do not need to be re-stamped.
pub const TRACKING_LABEL: &str = "argocd.argoproj.io/instance";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_to_plural_known() {
        assert_eq!(kind_to_plural("Deployment"), "deployments");
        assert_eq!(kind_to_plural("Service"), "services");
        assert_eq!(kind_to_plural("ConfigMap"), "configmaps");
        assert_eq!(kind_to_plural("Ingress"), "ingresses");
    }

    #[test]
    fn test_kind_to_plural_unknown_appends_s() {
        assert_eq!(kind_to_plural("FooBar"), "foobars");
        assert_eq!(kind_to_plural("Things"), "things"); // already plural
    }

    #[test]
    fn test_build_resource_url_namespaced() {
        let url = build_resource_url("apps/v1", "Deployment", "myapp", Some("default"));
        assert_eq!(url, "/apis/apps/v1/namespaces/default/deployments/myapp");
    }

    #[test]
    fn test_build_resource_url_cluster_scoped() {
        let url = build_resource_url("v1", "Namespace", "kube-system", None);
        assert_eq!(url, "/api/v1/namespaces/kube-system");
    }

    #[test]
    fn list_url_namespaced() {
        let url = build_list_url("v1", "ConfigMap", Some("kube-system"));
        assert_eq!(url, "/api/v1/namespaces/kube-system/configmaps");
    }

    #[test]
    fn cluster_matches_labels() {
        let now = Utc::now();
        let cluster = Cluster {
            id: Uuid::new_v4(),
            name: "prod".into(),
            server: "https://k.example".into(),
            credential_ref: None,
            labels: [("env".to_string(), "prod".to_string())].into(),
            annotations: Default::default(),
            project: None,
            created_at: now,
            updated_at: now,
        };
        let selector = [("env".to_string(), "prod".to_string())].into();
        assert!(cluster.matches_labels(&selector));
        let mismatch = [("env".to_string(), "staging".to_string())].into();
        assert!(!cluster.matches_labels(&mismatch));
    }
}
