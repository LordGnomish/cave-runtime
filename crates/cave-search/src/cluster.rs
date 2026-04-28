//! Minimal cluster-state model — used by /_cluster/health and /_cat/indices.

use std::collections::BTreeMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClusterStatus {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterHealth {
    pub cluster_name: String,
    pub status: ClusterStatus,
    pub timed_out: bool,
    pub number_of_nodes: u32,
    pub number_of_data_nodes: u32,
    pub active_primary_shards: u32,
    pub active_shards: u32,
    pub relocating_shards: u32,
    pub initializing_shards: u32,
    pub unassigned_shards: u32,
    pub number_of_pending_tasks: u32,
}

#[derive(Debug)]
pub struct ClusterState {
    pub name: String,
    health: RwLock<ClusterHealth>,
    pub indices: RwLock<BTreeMap<String, IndexShardCounts>>,
}

#[derive(Debug, Clone, Default)]
pub struct IndexShardCounts {
    pub primary_shards: u32,
    pub replica_shards: u32,
    pub doc_count: u64,
}

impl ClusterState {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            health: RwLock::new(ClusterHealth {
                cluster_name: name.into(),
                status: ClusterStatus::Green,
                timed_out: false,
                number_of_nodes: 1,
                number_of_data_nodes: 1,
                active_primary_shards: 0,
                active_shards: 0,
                relocating_shards: 0,
                initializing_shards: 0,
                unassigned_shards: 0,
                number_of_pending_tasks: 0,
            }),
            indices: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn upsert_index(&self, name: &str, primaries: u32, replicas: u32, doc_count: u64) {
        let mut idx = self.indices.write();
        idx.insert(
            name.into(),
            IndexShardCounts {
                primary_shards: primaries,
                replica_shards: replicas,
                doc_count,
            },
        );
        drop(idx);
        self.recompute_health();
    }

    pub fn drop_index(&self, name: &str) -> bool {
        let removed = self.indices.write().remove(name).is_some();
        if removed {
            self.recompute_health();
        }
        removed
    }

    pub fn health(&self) -> ClusterHealth {
        self.health.read().clone()
    }

    pub fn list_indices(&self) -> Vec<(String, IndexShardCounts)> {
        self.indices
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn recompute_health(&self) {
        let idx = self.indices.read();
        let active_primary: u32 = idx.values().map(|i| i.primary_shards).sum();
        let active_replicas: u32 = idx.values().map(|i| i.replica_shards).sum();
        let mut h = self.health.write();
        h.active_primary_shards = active_primary;
        h.active_shards = active_primary + active_replicas;
        // Single-node deployment: replicas are unassigned.
        if h.number_of_data_nodes == 1 {
            h.unassigned_shards = active_replicas;
            h.status = if active_replicas == 0 {
                ClusterStatus::Green
            } else {
                ClusterStatus::Yellow
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cluster_starts_green() {
        let c = ClusterState::new("cave");
        assert_eq!(c.health().status, ClusterStatus::Green);
    }

    #[test]
    fn upsert_with_no_replicas_keeps_green() {
        let c = ClusterState::new("cave");
        c.upsert_index("a", 1, 0, 0);
        assert_eq!(c.health().status, ClusterStatus::Green);
    }

    #[test]
    fn upsert_with_replicas_becomes_yellow_on_single_node() {
        let c = ClusterState::new("cave");
        c.upsert_index("a", 1, 1, 0);
        assert_eq!(c.health().status, ClusterStatus::Yellow);
    }

    #[test]
    fn active_primary_shards_sums_across_indices() {
        let c = ClusterState::new("cave");
        c.upsert_index("a", 1, 0, 0);
        c.upsert_index("b", 3, 0, 0);
        assert_eq!(c.health().active_primary_shards, 4);
    }

    #[test]
    fn drop_index_recomputes_health() {
        let c = ClusterState::new("cave");
        c.upsert_index("a", 1, 1, 0);
        assert!(c.drop_index("a"));
        assert_eq!(c.health().status, ClusterStatus::Green);
    }

    #[test]
    fn list_indices_returns_all() {
        let c = ClusterState::new("cave");
        c.upsert_index("a", 1, 0, 5);
        c.upsert_index("b", 1, 0, 10);
        let list = c.list_indices();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn drop_unknown_index_returns_false() {
        let c = ClusterState::new("cave");
        assert!(!c.drop_index("missing"));
    }

    #[test]
    fn cluster_status_serializes_lowercase() {
        let s = serde_json::to_string(&ClusterStatus::Yellow).unwrap();
        assert_eq!(s, "\"yellow\"");
    }
}
