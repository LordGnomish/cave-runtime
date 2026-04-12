//! Cross-datacenter replication with configurable consistency levels.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::raft::LogEntry;

/// Consistency guarantees for cross-DC replication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsistencyLevel {
    /// All replicas must acknowledge before a write is confirmed.
    Strong,
    /// Writes are acknowledged locally; replication happens asynchronously.
    Eventual,
    /// The writer will always read its own writes, even across DCs.
    ReadYourWrites,
}

/// Configuration for the cross-DC replicator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    pub local_datacenter: String,
    pub remote_datacenters: Vec<String>,
    pub consistency: ConsistencyLevel,
    /// Alert if replication lag for any DC exceeds this threshold.
    pub max_replication_lag_ms: u64,
}

/// Current replication status of a single remote datacenter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationStatus {
    pub datacenter: String,
    pub last_replicated_index: u64,
    pub lag_ms: u64,
    pub healthy: bool,
}

/// Manages asynchronous replication to remote datacenters.
pub struct CrossDcReplicator {
    config: ReplicationConfig,
    status: Arc<RwLock<HashMap<String, ReplicationStatus>>>,
}

impl CrossDcReplicator {
    pub fn new(config: ReplicationConfig) -> Self {
        let mut initial: HashMap<String, ReplicationStatus> = HashMap::new();
        for dc in &config.remote_datacenters {
            initial.insert(
                dc.clone(),
                ReplicationStatus {
                    datacenter: dc.clone(),
                    last_replicated_index: 0,
                    lag_ms: 0,
                    healthy: true,
                },
            );
        }
        Self {
            config,
            status: Arc::new(RwLock::new(initial)),
        }
    }

    /// Replicate `entry` to `target_dc`.
    ///
    /// In production this would open a connection to the remote DC's leader.
    /// Here we update the in-memory status to simulate a successful replication.
    pub async fn replicate(&self, entry: &LogEntry, target_dc: &str) -> Result<(), String> {
        if !self.config.remote_datacenters.iter().any(|d| d == target_dc) {
            return Err(format!("unknown datacenter: {target_dc}"));
        }

        debug!(
            target_dc,
            index = entry.index,
            term = entry.term,
            "replicating entry to remote DC",
        );

        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(target_dc) {
            if entry.index > s.last_replicated_index {
                s.last_replicated_index = entry.index;
            }
            s.healthy = true;
            // Simulated zero lag for in-memory replication.
            s.lag_ms = 0;
        }

        Ok(())
    }

    /// Return the current status of all remote datacenters.
    pub async fn status(&self) -> Vec<ReplicationStatus> {
        self.status.read().await.values().cloned().collect()
    }

    /// Returns `true` if all remote DCs are healthy and within lag threshold.
    pub async fn is_healthy(&self) -> bool {
        let status = self.status.read().await;
        for s in status.values() {
            if !s.healthy || s.lag_ms > self.config.max_replication_lag_ms {
                return false;
            }
        }
        true
    }

    /// Return the replication lag in milliseconds for the given DC, if known.
    pub async fn lag_for(&self, dc: &str) -> Option<u64> {
        self.status.read().await.get(dc).map(|s| s.lag_ms)
    }

    /// Update the lag and last replicated index for a DC (called from replication driver).
    pub async fn update_lag(&self, dc: &str, lag_ms: u64, last_index: u64) {
        let mut status = self.status.write().await;
        if let Some(s) = status.get_mut(dc) {
            s.lag_ms = lag_ms;
            s.last_replicated_index = last_index;
            s.healthy = lag_ms <= self.config.max_replication_lag_ms;
            if !s.healthy {
                warn!(dc, lag_ms, "replication lag exceeds threshold");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ReplicationConfig {
        ReplicationConfig {
            local_datacenter: "us-east-1".to_string(),
            remote_datacenters: vec!["us-west-2".to_string(), "eu-central-1".to_string()],
            consistency: ConsistencyLevel::Eventual,
            max_replication_lag_ms: 500,
        }
    }

    #[tokio::test]
    async fn test_replication_status_tracking() {
        let replicator = CrossDcReplicator::new(default_config());

        let entry = LogEntry {
            index: 42,
            term: 3,
            data: b"some-data".to_vec(),
        };

        replicator.replicate(&entry, "us-west-2").await.unwrap();

        let status = replicator.status().await;
        let west = status
            .iter()
            .find(|s| s.datacenter == "us-west-2")
            .unwrap();
        assert_eq!(west.last_replicated_index, 42);
        assert!(west.healthy);
    }

    #[tokio::test]
    async fn test_replication_to_unknown_dc_fails() {
        let replicator = CrossDcReplicator::new(default_config());
        let entry = LogEntry { index: 1, term: 1, data: vec![] };
        let result = replicator.replicate(&entry, "ap-southeast-1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_consistency_level_config() {
        let mut config = default_config();
        config.consistency = ConsistencyLevel::Strong;
        let replicator = CrossDcReplicator::new(config.clone());
        // Config is accessible via the struct.
        assert_eq!(replicator.config.consistency, ConsistencyLevel::Strong);
        assert_eq!(replicator.config.local_datacenter, "us-east-1");
    }

    #[tokio::test]
    async fn test_is_healthy_with_high_lag() {
        let replicator = CrossDcReplicator::new(default_config());

        // Initially healthy.
        assert!(replicator.is_healthy().await);

        // Simulate high lag on one DC.
        replicator.update_lag("us-west-2", 1000, 10).await;
        assert!(!replicator.is_healthy().await);
    }

    #[tokio::test]
    async fn test_lag_for() {
        let replicator = CrossDcReplicator::new(default_config());
        replicator.update_lag("eu-central-1", 123, 5).await;
        assert_eq!(replicator.lag_for("eu-central-1").await, Some(123));
        assert_eq!(replicator.lag_for("nonexistent").await, None);
    }
}
