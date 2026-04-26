//! VCluster lifecycle store.

use crate::error::{VClusterError, VClusterResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use tracing::info;
use uuid::Uuid;

pub struct ClusterStore {
    clusters: DashMap<String, VCluster>,
}

impl ClusterStore {
    pub fn new() -> Self {
        Self { clusters: DashMap::new() }
    }

    fn ns_key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    pub fn create(&self, req: CreateClusterRequest, max_per_ns: u32) -> VClusterResult<VCluster> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.clusters.contains_key(&key) {
            return Err(VClusterError::AlreadyExists(key));
        }
        let count = self.count_in_namespace(&req.namespace);
        if count >= max_per_ns {
            return Err(VClusterError::QuotaExceeded { max: max_per_ns });
        }
        let spec = req.spec.unwrap_or_default();
        let ttl = spec.ttl_secs;
        let cluster = VCluster {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            pr_number: req.pr_number,
            branch: req.branch,
            spec,
            status: VClusterStatus::Pending,
            kubeconfig: None,
            api_server_url: Some(format!("https://{}.{}.vcluster.local:6443", req.name, req.namespace)),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::try_seconds(ttl as i64).unwrap_or_default(),
            labels: req.labels.unwrap_or_default(),
        };
        self.clusters.insert(key, cluster.clone());
        info!(name = %req.name, namespace = %req.namespace, "vcluster created");
        Ok(cluster)
    }

    pub fn get(&self, namespace: &str, name: &str) -> VClusterResult<VCluster> {
        let key = Self::ns_key(namespace, name);
        self.clusters.get(&key).map(|r| r.clone()).ok_or_else(|| VClusterError::ClusterNotFound(key))
    }

    pub fn list(&self, namespace: &str) -> Vec<VCluster> {
        self.clusters.iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn update_status(&self, namespace: &str, name: &str, status: VClusterStatus) -> VClusterResult<VCluster> {
        let key = Self::ns_key(namespace, name);
        let mut cluster = self.clusters.get(&key).map(|r| r.clone())
            .ok_or_else(|| VClusterError::ClusterNotFound(key.clone()))?;
        cluster.status = status;
        if cluster.status == VClusterStatus::Running && cluster.kubeconfig.is_none() {
            cluster.kubeconfig = Some(format!("# kubeconfig for {}/{}", namespace, name));
        }
        self.clusters.insert(key, cluster.clone());
        Ok(cluster)
    }

    pub fn delete(&self, namespace: &str, name: &str) -> VClusterResult<()> {
        let key = Self::ns_key(namespace, name);
        self.clusters.remove(&key).ok_or_else(|| VClusterError::ClusterNotFound(key))?;
        Ok(())
    }

    pub fn count_in_namespace(&self, namespace: &str) -> u32 {
        self.clusters.iter().filter(|r| r.value().namespace == namespace).count() as u32
    }

    pub fn expire_stale(&self) -> Vec<String> {
        let now = Utc::now();
        let expired: Vec<String> = self.clusters.iter()
            .filter(|r| r.value().expires_at < now && r.value().status != VClusterStatus::Expired)
            .map(|r| r.key().clone())
            .collect();
        for key in &expired {
            if let Some(mut entry) = self.clusters.get_mut(key) {
                entry.status = VClusterStatus::Expired;
            }
        }
        expired
    }
}

impl Default for ClusterStore {
    fn default() -> Self { Self::new() }
}
