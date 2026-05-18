// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cluster gateway — wraps a kube::Client and provides the primitives
//! that the sync engine needs: apply (SSA), get, delete, list.
//!
//! In test / dry-run mode (`client = None`) every mutating call is a no-op
//! and reads return `None` / `[]`.  This lets the full sync pipeline run
//! without a real cluster, which is critical for unit tests.

use crate::error::DeployError;
use crate::models::Manifest;
use crate::sync::CAVE_MANAGER;
use kube::Client;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{debug, info};

/// Maps common Kubernetes kinds to their plural resource names.
/// Used for constructing REST API paths.
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
            // Naive pluralisation: lowercase + "s"
            let lower = other.to_lowercase();
            return if lower.ends_with('s') { lower } else { format!("{lower}s") };
        }
    }
    .to_string()
}

/// Gateway to a single Kubernetes cluster.
pub struct ClusterGateway {
    pub(crate) client: Option<Client>,
    pub server: String,
}

impl ClusterGateway {
    /// Connect using the default kubeconfig / in-cluster service account.
    pub async fn try_connect(server: &str) -> Result<Self, DeployError> {
        let client = Client::try_default()
            .await
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;
        info!(%server, "Connected to Kubernetes cluster");
        Ok(Self { client: Some(client), server: server.to_string() })
    }

    /// Mock gateway — all mutating operations are no-ops, reads return empty.
    pub fn mock(server: &str) -> Self {
        Self { client: None, server: server.to_string() }
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Server-Side Apply a manifest.  Returns the applied object as JSON.
    pub async fn apply_ssa(
        &self,
        manifest: &mut Manifest,
        app_name: &str,
        dry_run: bool,
    ) -> Result<Value, DeployError> {
        // Inject tracking labels before apply
        crate::sync::inject_tracking(&mut manifest.raw, app_name);

        if dry_run || self.client.is_none() {
            debug!(kind = %manifest.kind, name = %manifest.name, "dry-run / mock apply");
            return Ok(manifest.raw.clone());
        }

        // Real SSA via kube Client — build URL and use raw HTTP PATCH
        let client = self.client.as_ref().unwrap();
        let url = self.build_resource_url(
            &manifest.api_version,
            &manifest.kind,
            &manifest.name,
            manifest.namespace.as_deref(),
        );
        let query = format!("{url}?fieldManager={CAVE_MANAGER}&force=true");

        let body = serde_json::to_vec(&manifest.raw)
            .map_err(|e| DeployError::ManifestParse(e.to_string()))?;

        let req = http::Request::builder()
            .method("PATCH")
            .uri(&query)
            .header("Content-Type", "application/apply-patch+yaml")
            .body(body)
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;

        let result: Value = client
            .request(req)
            .await
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;

        info!(kind = %manifest.kind, name = %manifest.name, "SSA apply succeeded");
        Ok(result)
    }

    /// Get the live state of a resource. Returns `None` if not found.
    pub async fn get_live(
        &self,
        api_version: &str,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<Option<Value>, DeployError> {
        let Some(client) = &self.client else {
            return Ok(None);
        };
        let url = self.build_resource_url(api_version, kind, name, namespace);
        let req = http::Request::builder()
            .method("GET")
            .uri(&url)
            .body(vec![])
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;

        match client.request::<Value>(req).await {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("404") || msg.contains("NotFound") {
                    Ok(None)
                } else {
                    Err(DeployError::Kubernetes(msg))
                }
            }
        }
    }

    /// Delete a resource (prune).
    pub async fn delete(
        &self,
        api_version: &str,
        kind: &str,
        name: &str,
        namespace: Option<&str>,
    ) -> Result<(), DeployError> {
        let Some(client) = &self.client else {
            return Ok(());
        };
        let url = self.build_resource_url(api_version, kind, name, namespace);
        let req = http::Request::builder()
            .method("DELETE")
            .uri(&url)
            .body(vec![])
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;
        client
            .request::<Value>(req)
            .await
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;
        info!(kind, name, "Resource pruned");
        Ok(())
    }

    /// List all resources of a given kind managed by cave-deploy (label selector).
    pub async fn list_managed(
        &self,
        api_version: &str,
        kind: &str,
        namespace: Option<&str>,
        app_name: &str,
    ) -> Result<Vec<Value>, DeployError> {
        let Some(client) = &self.client else {
            return Ok(vec![]);
        };
        let base = self.build_list_url(api_version, kind, namespace);
        let url = format!(
            "{base}?labelSelector=argocd.argoproj.io%2Fapp-name%3D{app_name}"
        );
        let req = http::Request::builder()
            .method("GET")
            .uri(&url)
            .body(vec![])
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;
        let list: Value = client
            .request(req)
            .await
            .map_err(|e| DeployError::Kubernetes(e.to_string()))?;
        let items = list["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(items)
    }

    // ─── URL builders ─────────────────────────────────────────────────────────

    fn build_resource_url(
        &self,
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

    fn build_list_url(
        &self,
        api_version: &str,
        kind: &str,
        namespace: Option<&str>,
    ) -> String {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_to_plural_known() {
        assert_eq!(kind_to_plural("Deployment"), "deployments");
        assert_eq!(kind_to_plural("Service"), "services");
        assert_eq!(kind_to_plural("ConfigMap"), "configmaps");
    }

    #[test]
    fn test_kind_to_plural_unknown_appends_s() {
        assert_eq!(kind_to_plural("FooBar"), "foobars");
    }

    #[test]
    fn test_build_resource_url_namespaced() {
        let gw = ClusterGateway::mock("https://k8s.example.com");
        let url = gw.build_resource_url("apps/v1", "Deployment", "myapp", Some("default"));
        assert_eq!(url, "/apis/apps/v1/namespaces/default/deployments/myapp");
    }

    #[test]
    fn test_build_resource_url_cluster_scoped() {
        let gw = ClusterGateway::mock("https://k8s.example.com");
        let url =
            gw.build_resource_url("v1", "Namespace", "kube-system", None);
        assert_eq!(url, "/api/v1/namespaces/kube-system");
    }
}
