//! Resource syncer — propagates host cluster resources into vclusters.

use crate::models::SyncedResource;
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct ResourceSyncer {
    synced: DashMap<String, SyncedResource>,
    history: std::sync::Mutex<Vec<SyncedResource>>,
}

impl ResourceSyncer {
    pub fn new() -> Self {
        Self {
            synced: DashMap::new(),
            history: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn key(cluster: &str, ns: &str, kind: &str, name: &str) -> String {
        format!("{cluster}/{ns}/{kind}/{name}")
    }

    pub fn sync(&self, cluster_name: &str, namespace: &str, kind: &str, name: &str, data: &str) -> SyncedResource {
        let hash = format!("{:x}", data.len()); // simplified hash
        let resource = SyncedResource {
            id: Uuid::new_v4(),
            cluster_name: cluster_name.to_owned(),
            namespace: namespace.to_owned(),
            resource_kind: kind.to_owned(),
            resource_name: name.to_owned(),
            synced_at: Utc::now(),
            hash: hash.clone(),
        };
        let k = Self::key(cluster_name, namespace, kind, name);
        self.synced.insert(k, resource.clone());
        let mut hist = self.history.lock().unwrap();
        hist.push(resource.clone());
        let len = hist.len();
        if len > 500 {
            let excess = len - 500;
            hist.drain(0..excess);
        }
        resource
    }

    pub fn list_for_cluster(&self, cluster_name: &str) -> Vec<SyncedResource> {
        self.synced.iter()
            .filter(|r| r.value().cluster_name == cluster_name)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_for_cluster(&self, cluster_name: &str) {
        self.synced.retain(|_, v| v.cluster_name != cluster_name);
    }
}

impl Default for ResourceSyncer {
    fn default() -> Self { Self::new() }
}
