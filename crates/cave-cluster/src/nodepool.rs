// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node pool management — add, remove, scale, labels, taints.

use crate::error::{ClusterError, ClusterResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Node pool types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum NodePoolStatus {
    Provisioning,
    Running,
    Scaling,
    Upgrading,
    Deleting,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: TaintEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePool {
    pub name: String,
    pub cluster_name: String,
    pub vm_size: String,
    pub node_count: i32,
    pub min_count: Option<i32>,
    pub max_count: Option<i32>,
    pub autoscaling_enabled: bool,
    pub os_disk_size_gb: i32,
    pub labels: HashMap<String, String>,
    pub taints: Vec<Taint>,
    pub kubernetes_version: Option<String>,
    pub status: NodePoolStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl NodePool {
    pub fn new(
        cluster_name: String,
        name: String,
        vm_size: String,
        node_count: i32,
    ) -> ClusterResult<Self> {
        Self::validate_name(&name)?;
        if node_count < 0 {
            return Err(ClusterError::InvalidName {
                name,
                reason: "node_count must be >= 0".into(),
            });
        }
        Ok(Self {
            name,
            cluster_name,
            vm_size,
            node_count,
            min_count: None,
            max_count: None,
            autoscaling_enabled: false,
            os_disk_size_gb: 100,
            labels: HashMap::new(),
            taints: vec![],
            kubernetes_version: None,
            status: NodePoolStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    fn validate_name(name: &str) -> ClusterResult<()> {
        if name.is_empty() || name.len() > 12 {
            return Err(ClusterError::InvalidName {
                name: name.to_string(),
                reason: "node pool name must be 1-12 characters".into(),
            });
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err(ClusterError::InvalidName {
                name: name.to_string(),
                reason: "must be alphanumeric or hyphen".into(),
            });
        }
        Ok(())
    }

    pub fn scale(&mut self, node_count: i32) -> ClusterResult<()> {
        if node_count < 0 {
            return Err(ClusterError::InvalidName {
                name: self.name.clone(),
                reason: "node_count must be >= 0".into(),
            });
        }
        if let (Some(min), Some(max)) = (self.min_count, self.max_count) {
            if node_count < min || node_count > max {
                return Err(ClusterError::InvalidName {
                    name: self.name.clone(),
                    reason: format!("node_count {node_count} out of autoscaling range [{min}, {max}]"),
                });
            }
        }
        self.status = NodePoolStatus::Scaling;
        self.node_count = node_count;
        self.status = NodePoolStatus::Running;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn set_autoscaling(&mut self, min: i32, max: i32) -> ClusterResult<()> {
        if min < 0 || max < min {
            return Err(ClusterError::InvalidName {
                name: self.name.clone(),
                reason: format!("invalid autoscaling range [{min}, {max}]"),
            });
        }
        self.autoscaling_enabled = true;
        self.min_count = Some(min);
        self.max_count = Some(max);
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn set_labels(&mut self, labels: HashMap<String, String>) {
        self.labels = labels;
        self.updated_at = Utc::now();
    }

    pub fn set_taints(&mut self, taints: Vec<Taint>) {
        self.taints = taints;
        self.updated_at = Utc::now();
    }
}

// ── Node pool store ───────────────────────────────────────────────────────────

pub struct NodePoolStore {
    /// (cluster_name, pool_name) → NodePool
    pools: DashMap<(String, String), NodePool>,
}

impl NodePoolStore {
    pub fn new() -> Self {
        Self {
            pools: DashMap::new(),
        }
    }

    pub fn create(
        &self,
        cluster_name: &str,
        req: CreateNodePoolRequest,
    ) -> ClusterResult<NodePool> {
        let key = (cluster_name.to_string(), req.name.clone());
        if self.pools.contains_key(&key) {
            return Err(ClusterError::NodePoolAlreadyExists(req.name));
        }
        let mut pool = NodePool::new(
            cluster_name.to_string(),
            req.name,
            req.vm_size,
            req.node_count,
        )?;
        if let (Some(min), Some(max)) = (req.min_count, req.max_count) {
            pool.set_autoscaling(min, max)?;
        }
        if let Some(labels) = req.labels {
            pool.set_labels(labels);
        }
        if let Some(taints) = req.taints {
            pool.set_taints(taints);
        }
        let result = pool.clone();
        self.pools.insert(key, pool);
        Ok(result)
    }

    pub fn get(&self, cluster_name: &str, pool_name: &str) -> ClusterResult<NodePool> {
        self.pools
            .get(&(cluster_name.to_string(), pool_name.to_string()))
            .map(|p| p.clone())
            .ok_or_else(|| ClusterError::NodePoolNotFound {
                cluster: cluster_name.to_string(),
                pool: pool_name.to_string(),
            })
    }

    pub fn list(&self, cluster_name: &str) -> Vec<NodePool> {
        self.pools
            .iter()
            .filter(|e| e.key().0 == cluster_name)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn scale(
        &self,
        cluster_name: &str,
        pool_name: &str,
        node_count: i32,
    ) -> ClusterResult<NodePool> {
        let mut pool = self
            .pools
            .get_mut(&(cluster_name.to_string(), pool_name.to_string()))
            .ok_or_else(|| ClusterError::NodePoolNotFound {
                cluster: cluster_name.to_string(),
                pool: pool_name.to_string(),
            })?;
        pool.scale(node_count)?;
        Ok(pool.clone())
    }

    pub fn update_labels(
        &self,
        cluster_name: &str,
        pool_name: &str,
        labels: HashMap<String, String>,
    ) -> ClusterResult<NodePool> {
        let mut pool = self
            .pools
            .get_mut(&(cluster_name.to_string(), pool_name.to_string()))
            .ok_or_else(|| ClusterError::NodePoolNotFound {
                cluster: cluster_name.to_string(),
                pool: pool_name.to_string(),
            })?;
        pool.set_labels(labels);
        Ok(pool.clone())
    }

    pub fn update_taints(
        &self,
        cluster_name: &str,
        pool_name: &str,
        taints: Vec<Taint>,
    ) -> ClusterResult<NodePool> {
        let mut pool = self
            .pools
            .get_mut(&(cluster_name.to_string(), pool_name.to_string()))
            .ok_or_else(|| ClusterError::NodePoolNotFound {
                cluster: cluster_name.to_string(),
                pool: pool_name.to_string(),
            })?;
        pool.set_taints(taints);
        Ok(pool.clone())
    }

    pub fn delete(&self, cluster_name: &str, pool_name: &str) -> ClusterResult<()> {
        // Ensure not the last pool
        let pool_count = self
            .pools
            .iter()
            .filter(|e| e.key().0 == cluster_name)
            .count();
        if pool_count <= 1 {
            return Err(ClusterError::LastNodePool(cluster_name.to_string()));
        }
        let key = (cluster_name.to_string(), pool_name.to_string());
        self.pools
            .remove(&key)
            .ok_or_else(|| ClusterError::NodePoolNotFound {
                cluster: cluster_name.to_string(),
                pool: pool_name.to_string(),
            })?;
        Ok(())
    }

    pub fn delete_all(&self, cluster_name: &str) {
        self.pools.retain(|k, _| k.0 != cluster_name);
    }
}

// ── Request DTOs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateNodePoolRequest {
    pub name: String,
    pub vm_size: String,
    pub node_count: i32,
    pub min_count: Option<i32>,
    pub max_count: Option<i32>,
    pub labels: Option<HashMap<String, String>>,
    pub taints: Option<Vec<Taint>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> NodePoolStore {
        NodePoolStore::new()
    }

    fn pool_req(name: &str) -> CreateNodePoolRequest {
        CreateNodePoolRequest {
            name: name.to_string(),
            vm_size: "Standard_D4s_v3".to_string(),
            node_count: 3,
            min_count: None,
            max_count: None,
            labels: None,
            taints: None,
        }
    }

    #[test]
    fn create_and_scale() {
        let s = store();
        s.create("cluster-1", pool_req("system")).unwrap();
        let scaled = s.scale("cluster-1", "system", 5).unwrap();
        assert_eq!(scaled.node_count, 5);
    }

    #[test]
    fn cannot_delete_last_pool() {
        let s = store();
        s.create("cluster-1", pool_req("system")).unwrap();
        assert!(matches!(
            s.delete("cluster-1", "system"),
            Err(ClusterError::LastNodePool(_))
        ));
    }

    #[test]
    fn can_delete_when_multiple_pools() {
        let s = store();
        s.create("c1", pool_req("system")).unwrap();
        s.create("c1", pool_req("user")).unwrap();
        s.delete("c1", "user").unwrap();
        assert_eq!(s.list("c1").len(), 1);
    }

    #[test]
    fn autoscaling_range_enforced() {
        let s = store();
        let mut req = pool_req("auto");
        req.min_count = Some(2);
        req.max_count = Some(10);
        let pool = s.create("c1", req).unwrap();
        assert!(pool.autoscaling_enabled);

        // Scale within range
        s.scale("c1", "auto", 5).unwrap();
        // Scale outside range should fail
        assert!(s.scale("c1", "auto", 15).is_err());
    }
}
