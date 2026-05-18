// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cluster CRUD and state machine.

use crate::error::{ClusterError, ClusterResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Cluster state ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ClusterStatus {
    Provisioning,
    Running,
    Upgrading,
    Scaling,
    Deleting,
    Failed,
    Hibernated,
}

// ── Network config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub pod_cidr: String,
    pub service_cidr: String,
    pub dns_service_ip: String,
    pub network_plugin: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            pod_cidr: "10.244.0.0/16".into(),
            service_cidr: "10.96.0.0/12".into(),
            dns_service_ip: "10.96.0.10".into(),
            network_plugin: "calico".into(),
        }
    }
}

// ── Cluster definition ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSpec {
    pub name: String,
    pub kubernetes_version: String,
    pub region: String,
    pub network: NetworkConfig,
    pub tags: HashMap<String, String>,
    /// Whether to enable RBAC (always true)
    pub enable_rbac: bool,
    /// Whether to enable audit logging
    pub audit_logging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: Uuid,
    pub spec: ClusterSpec,
    pub status: ClusterStatus,
    pub api_endpoint: String,
    pub ca_data: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
    pub error_message: Option<String>,
}

impl Cluster {
    pub fn new(spec: ClusterSpec, created_by: String) -> Self {
        let id = Uuid::new_v4();
        let api_endpoint = format!(
            "https://{}.cave-cluster.internal:6443",
            spec.name
        );
        Self {
            id,
            spec,
            status: ClusterStatus::Provisioning,
            api_endpoint,
            ca_data: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("CAVE-CA-{id}").as_bytes(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by,
            error_message: None,
        }
    }

    pub fn transition(&mut self, new_status: ClusterStatus) {
        tracing::info!(
            cluster = %self.spec.name,
            from = ?self.status,
            to = ?new_status,
            "cluster state transition"
        );
        self.status = new_status;
        self.updated_at = Utc::now();
    }

    pub fn fail(&mut self, message: String) {
        self.status = ClusterStatus::Failed;
        self.error_message = Some(message);
        self.updated_at = Utc::now();
    }

    pub fn is_mutable(&self) -> bool {
        matches!(self.status, ClusterStatus::Running | ClusterStatus::Failed)
    }
}

// ── Create / scale request ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateClusterRequest {
    pub name: String,
    pub kubernetes_version: String,
    pub region: String,
    #[serde(default)]
    pub network: Option<NetworkConfig>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub audit_logging: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ScaleClusterRequest {
    /// If set, scale the default node pool to this size
    pub node_count: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpgradeClusterRequest {
    pub kubernetes_version: String,
}

// ── Cluster store ─────────────────────────────────────────────────────────────

pub struct ClusterStore {
    /// cluster_name → Cluster
    clusters: DashMap<String, Cluster>,
}

impl ClusterStore {
    pub fn new() -> Self {
        Self {
            clusters: DashMap::new(),
        }
    }

    pub fn create(&self, req: CreateClusterRequest, created_by: &str) -> ClusterResult<Cluster> {
        Self::validate_name(&req.name)?;
        if self.clusters.contains_key(&req.name) {
            return Err(ClusterError::AlreadyExists(req.name));
        }
        // Validate the Kubernetes version
        crate::version::validate_k8s_version(&req.kubernetes_version)?;

        let spec = ClusterSpec {
            name: req.name.clone(),
            kubernetes_version: req.kubernetes_version,
            region: req.region,
            network: req.network.unwrap_or_default(),
            tags: req.tags,
            enable_rbac: true,
            audit_logging: req.audit_logging,
        };
        let mut cluster = Cluster::new(spec, created_by.to_string());
        // Simulate synchronous provisioning for in-memory store
        cluster.transition(ClusterStatus::Running);
        let result = cluster.clone();
        self.clusters.insert(req.name, cluster);
        Ok(result)
    }

    pub fn get(&self, name: &str) -> ClusterResult<Cluster> {
        self.clusters
            .get(name)
            .map(|c| c.clone())
            .ok_or_else(|| ClusterError::NotFound(name.to_string()))
    }

    pub fn list(&self) -> Vec<Cluster> {
        self.clusters.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete(&self, name: &str) -> ClusterResult<()> {
        let mut cluster = self
            .clusters
            .get_mut(name)
            .ok_or_else(|| ClusterError::NotFound(name.to_string()))?;
        cluster.transition(ClusterStatus::Deleting);
        drop(cluster);
        self.clusters.remove(name);
        Ok(())
    }

    pub fn upgrade(
        &self,
        name: &str,
        target_version: &str,
    ) -> ClusterResult<Cluster> {
        let mut cluster = self
            .clusters
            .get_mut(name)
            .ok_or_else(|| ClusterError::NotFound(name.to_string()))?;

        if cluster.status != ClusterStatus::Running {
            return Err(ClusterError::InvalidState {
                cluster: name.to_string(),
                expected: "Running".into(),
                actual: format!("{:?}", cluster.status),
            });
        }

        let current = cluster.spec.kubernetes_version.clone();
        crate::version::validate_upgrade(&current, target_version)?;

        cluster.transition(ClusterStatus::Upgrading);
        cluster.spec.kubernetes_version = target_version.to_string();
        cluster.transition(ClusterStatus::Running);

        Ok(cluster.clone())
    }

    pub fn set_tags(&self, name: &str, tags: HashMap<String, String>) -> ClusterResult<Cluster> {
        let mut cluster = self
            .clusters
            .get_mut(name)
            .ok_or_else(|| ClusterError::NotFound(name.to_string()))?;
        cluster.spec.tags = tags;
        cluster.updated_at = Utc::now();
        Ok(cluster.clone())
    }

    fn validate_name(name: &str) -> ClusterResult<()> {
        if name.is_empty() || name.len() > 63 {
            return Err(ClusterError::InvalidName {
                name: name.to_string(),
                reason: "must be 1-63 characters".into(),
            });
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err(ClusterError::InvalidName {
                name: name.to_string(),
                reason: "must be alphanumeric or hyphen".into(),
            });
        }
        if name.starts_with('-') || name.ends_with('-') {
            return Err(ClusterError::InvalidName {
                name: name.to_string(),
                reason: "must not start or end with hyphen".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ClusterStore {
        ClusterStore::new()
    }

    fn create_req(name: &str) -> CreateClusterRequest {
        CreateClusterRequest {
            name: name.to_string(),
            kubernetes_version: "1.29".to_string(),
            region: "eu-west-1".to_string(),
            network: None,
            tags: HashMap::new(),
            audit_logging: true,
        }
    }

    #[test]
    fn create_and_get_cluster() {
        let s = store();
        let c = s.create(create_req("my-cluster"), "alice").unwrap();
        assert_eq!(c.spec.name, "my-cluster");
        assert_eq!(c.status, ClusterStatus::Running);
        let got = s.get("my-cluster").unwrap();
        assert_eq!(got.id, c.id);
    }

    #[test]
    fn duplicate_cluster_fails() {
        let s = store();
        s.create(create_req("dup"), "alice").unwrap();
        assert!(matches!(s.create(create_req("dup"), "bob"), Err(ClusterError::AlreadyExists(_))));
    }

    #[test]
    fn delete_cluster() {
        let s = store();
        s.create(create_req("delete-me"), "alice").unwrap();
        s.delete("delete-me").unwrap();
        assert!(matches!(s.get("delete-me"), Err(ClusterError::NotFound(_))));
    }

    #[test]
    fn upgrade_cluster() {
        let s = store();
        s.create(create_req("upgrade-me"), "alice").unwrap();
        let upgraded = s.upgrade("upgrade-me", "1.30").unwrap();
        assert_eq!(upgraded.spec.kubernetes_version, "1.30");
        assert_eq!(upgraded.status, ClusterStatus::Running);
    }

    #[test]
    fn invalid_cluster_name() {
        let s = store();
        assert!(s.create(create_req(""), "a").is_err());
        assert!(s.create(create_req("-bad"), "a").is_err());
        assert!(s.create(create_req("bad!name"), "a").is_err());
    }
}
