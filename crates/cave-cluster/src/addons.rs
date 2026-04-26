//! Cluster add-ons management.

use crate::error::{ClusterError, ClusterResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Add-on types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AddonStatus {
    NotInstalled,
    Installing,
    Running,
    Upgrading,
    Failed,
    Uninstalling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonSpec {
    pub name: String,
    pub description: String,
    pub current_version: String,
    pub available_versions: Vec<String>,
    pub config: HashMap<String, String>,
    pub status: AddonStatus,
    pub namespace: String,
}

/// Well-known add-ons.
pub fn available_addons() -> Vec<(&'static str, &'static str)> {
    vec![
        ("cert-manager", "Certificate manager for Kubernetes"),
        ("ingress-nginx", "NGINX Ingress Controller"),
        ("metrics-server", "Resource metrics API server"),
        ("cluster-autoscaler", "Automatic node scaling"),
        ("external-dns", "Automates DNS records from Kubernetes resources"),
        ("velero", "Backup and restore for Kubernetes resources and volumes"),
        ("keda", "Kubernetes Event-driven Autoscaling"),
        ("prometheus-stack", "Prometheus + Grafana monitoring stack"),
        ("loki", "Log aggregation system"),
        ("argo-cd", "GitOps continuous deployment"),
        ("tekton", "Cloud-native CI/CD pipelines"),
        ("istio", "Service mesh"),
    ]
}

pub fn addon_versions(name: &str) -> Vec<String> {
    match name {
        "cert-manager" => vec!["v1.13.0".into(), "v1.14.0".into(), "v1.15.0".into()],
        "ingress-nginx" => vec!["4.8.0".into(), "4.9.0".into(), "4.10.0".into()],
        "metrics-server" => vec!["0.6.4".into(), "0.7.0".into(), "0.7.1".into()],
        "prometheus-stack" => vec!["55.0.0".into(), "58.0.0".into(), "60.0.0".into()],
        _ => vec!["latest".into()],
    }
}

pub fn addon_namespace(name: &str) -> &'static str {
    match name {
        "cert-manager" => "cert-manager",
        "ingress-nginx" => "ingress-nginx",
        "metrics-server" => "kube-system",
        "cluster-autoscaler" => "kube-system",
        "prometheus-stack" => "monitoring",
        "loki" => "monitoring",
        "argo-cd" => "argocd",
        "tekton" => "tekton-pipelines",
        "istio" => "istio-system",
        _ => "kube-addons",
    }
}

// ── Add-on manager ────────────────────────────────────────────────────────────

pub struct AddonManager {
    /// (cluster_name, addon_name) → AddonSpec
    addons: DashMap<(String, String), AddonSpec>,
}

impl AddonManager {
    pub fn new() -> Self {
        Self {
            addons: DashMap::new(),
        }
    }

    pub fn install(
        &self,
        cluster_name: &str,
        addon_name: &str,
        version: Option<String>,
        config: HashMap<String, String>,
    ) -> ClusterResult<AddonSpec> {
        let key = (cluster_name.to_string(), addon_name.to_string());

        let available: HashMap<&str, &str> = available_addons().into_iter().collect();
        if !available.contains_key(addon_name) {
            return Err(ClusterError::AddonNotFound(addon_name.to_string()));
        }

        let versions = addon_versions(addon_name);
        let chosen_version = version
            .or_else(|| versions.last().cloned())
            .unwrap_or_else(|| "latest".into());

        let addon = AddonSpec {
            name: addon_name.to_string(),
            description: available[addon_name].to_string(),
            current_version: chosen_version,
            available_versions: versions,
            config,
            status: AddonStatus::Running,
            namespace: addon_namespace(addon_name).to_string(),
        };

        let result = addon.clone();
        self.addons.insert(key, addon);
        Ok(result)
    }

    pub fn get(&self, cluster_name: &str, addon_name: &str) -> ClusterResult<AddonSpec> {
        self.addons
            .get(&(cluster_name.to_string(), addon_name.to_string()))
            .map(|a| a.clone())
            .ok_or_else(|| ClusterError::AddonNotFound(addon_name.to_string()))
    }

    pub fn list(&self, cluster_name: &str) -> Vec<AddonSpec> {
        self.addons
            .iter()
            .filter(|e| e.key().0 == cluster_name)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn upgrade(
        &self,
        cluster_name: &str,
        addon_name: &str,
        version: String,
    ) -> ClusterResult<AddonSpec> {
        let mut addon = self
            .addons
            .get_mut(&(cluster_name.to_string(), addon_name.to_string()))
            .ok_or_else(|| ClusterError::AddonNotFound(addon_name.to_string()))?;
        addon.status = AddonStatus::Upgrading;
        addon.current_version = version;
        addon.status = AddonStatus::Running;
        Ok(addon.clone())
    }

    pub fn uninstall(&self, cluster_name: &str, addon_name: &str) -> ClusterResult<()> {
        let key = (cluster_name.to_string(), addon_name.to_string());
        self.addons
            .remove(&key)
            .ok_or_else(|| ClusterError::AddonNotFound(addon_name.to_string()))?;
        Ok(())
    }

    pub fn list_available() -> Vec<AddonInfo> {
        available_addons()
            .into_iter()
            .map(|(name, desc)| AddonInfo {
                name: name.to_string(),
                description: desc.to_string(),
                latest_version: addon_versions(name).last().cloned().unwrap_or_else(|| "latest".into()),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AddonInfo {
    pub name: String,
    pub description: String,
    pub latest_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> AddonManager {
        AddonManager::new()
    }

    #[test]
    fn install_and_get_addon() {
        let m = mgr();
        let addon = m.install("prod", "cert-manager", None, HashMap::new()).unwrap();
        assert_eq!(addon.status, AddonStatus::Running);
        assert!(!addon.current_version.is_empty());
        let got = m.get("prod", "cert-manager").unwrap();
        assert_eq!(got.name, "cert-manager");
    }

    #[test]
    fn unknown_addon_fails() {
        let m = mgr();
        assert!(matches!(
            m.install("c1", "unknown-addon", None, HashMap::new()),
            Err(ClusterError::AddonNotFound(_))
        ));
    }

    #[test]
    fn upgrade_addon() {
        let m = mgr();
        m.install("c1", "metrics-server", Some("0.6.4".into()), HashMap::new()).unwrap();
        let upgraded = m.upgrade("c1", "metrics-server", "0.7.1".into()).unwrap();
        assert_eq!(upgraded.current_version, "0.7.1");
        assert_eq!(upgraded.status, AddonStatus::Running);
    }

    #[test]
    fn uninstall_addon() {
        let m = mgr();
        m.install("c1", "ingress-nginx", None, HashMap::new()).unwrap();
        m.uninstall("c1", "ingress-nginx").unwrap();
        assert!(m.get("c1", "ingress-nginx").is_err());
    }

    #[test]
    fn list_available_addons_non_empty() {
        let available = AddonManager::list_available();
        assert!(!available.is_empty());
        assert!(available.iter().any(|a| a.name == "cert-manager"));
    }
}
