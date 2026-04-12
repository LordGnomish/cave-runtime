//! Automatic and manual failover with split-brain protection.

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::raft::NodeId;

/// Why a failover was triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailoverReason {
    LeaderTimeout,
    ManualTrigger,
    HealthCheckFailed,
    SplitBrain,
}

/// Record of a single failover event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverEvent {
    pub id: uuid::Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub old_leader: NodeId,
    pub new_leader: NodeId,
    pub reason: FailoverReason,
    /// How long the failover took (approximation).
    pub duration_ms: u64,
}

/// Manages failover events and split-brain protection for a Raft cluster.
pub struct FailoverManager {
    history: Arc<RwLock<Vec<FailoverEvent>>>,
    /// Minimum number of nodes required to form a quorum.
    split_brain_threshold: u64,
}

impl FailoverManager {
    /// Create a new manager for a cluster of `cluster_size` nodes.
    /// Sets `split_brain_threshold = cluster_size / 2 + 1`.
    pub fn new(cluster_size: u64) -> Self {
        let split_brain_threshold = cluster_size / 2 + 1;
        Self {
            history: Arc::new(RwLock::new(Vec::new())),
            split_brain_threshold,
        }
    }

    /// Persist a failover event in the in-memory history log.
    pub async fn record_failover(&self, event: FailoverEvent) {
        info!(
            id = %event.id,
            old_leader = event.old_leader,
            new_leader = event.new_leader,
            reason = ?event.reason,
            duration_ms = event.duration_ms,
            "recording failover event",
        );
        self.history.write().await.push(event);
    }

    /// Return a snapshot of all recorded failover events (oldest first).
    pub async fn history(&self) -> Vec<FailoverEvent> {
        self.history.read().await.clone()
    }

    /// Returns `true` if `active_nodes` constitutes a quorum.
    pub fn has_quorum(&self, active_nodes: u64) -> bool {
        active_nodes >= self.split_brain_threshold
    }

    /// Initiate a manual leadership transfer from `current_leader` to `target`.
    /// Creates and records a `FailoverEvent` and returns it.
    pub async fn trigger_manual_failover(
        &self,
        current_leader: NodeId,
        target: NodeId,
    ) -> FailoverEvent {
        let start = std::time::Instant::now();

        // Simulate a brief handoff delay.
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        let event = FailoverEvent {
            id: uuid::Uuid::new_v4(),
            timestamp: Utc::now(),
            old_leader: current_leader,
            new_leader: target,
            reason: FailoverReason::ManualTrigger,
            duration_ms,
        };

        self.record_failover(event.clone()).await;
        event
    }

    /// Returns `true` when both network partitions have enough nodes to
    /// independently elect a leader (i.e. a genuine split-brain situation).
    pub fn detect_split_brain(&self, partition_a: u64, partition_b: u64) -> bool {
        let a_has_quorum = partition_a >= self.split_brain_threshold;
        let b_has_quorum = partition_b >= self.split_brain_threshold;
        if a_has_quorum && b_has_quorum {
            warn!(
                partition_a,
                partition_b,
                threshold = self.split_brain_threshold,
                "split-brain detected: both partitions could elect a leader",
            );
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manual_failover() {
        let manager = FailoverManager::new(5);
        let event = manager.trigger_manual_failover(1, 3).await;

        assert_eq!(event.old_leader, 1);
        assert_eq!(event.new_leader, 3);
        assert_eq!(event.reason, FailoverReason::ManualTrigger);

        let history = manager.history().await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, event.id);
    }

    #[tokio::test]
    async fn test_split_brain_detection() {
        // 5-node cluster: threshold = 3
        let manager = FailoverManager::new(5);

        // Both partitions >= 3 → split brain.
        assert!(manager.detect_split_brain(3, 3));

        // Only one partition has quorum → no split brain.
        assert!(!manager.detect_split_brain(3, 2));
        assert!(!manager.detect_split_brain(2, 3));

        // Neither partition has quorum.
        assert!(!manager.detect_split_brain(2, 2));
    }

    #[tokio::test]
    async fn test_quorum_calculation() {
        // 3-node cluster: quorum = 2
        let m3 = FailoverManager::new(3);
        assert!(m3.has_quorum(2));
        assert!(m3.has_quorum(3));
        assert!(!m3.has_quorum(1));

        // 5-node cluster: quorum = 3
        let m5 = FailoverManager::new(5);
        assert!(m5.has_quorum(3));
        assert!(!m5.has_quorum(2));

        // 1-node cluster: quorum = 1
        let m1 = FailoverManager::new(1);
        assert!(m1.has_quorum(1));
    }

    #[tokio::test]
    async fn test_failover_history_accumulates() {
        let manager = FailoverManager::new(3);
        manager.trigger_manual_failover(1, 2).await;
        manager.trigger_manual_failover(2, 1).await;
        manager.trigger_manual_failover(1, 3).await;

        let history = manager.history().await;
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn test_record_failover_leader_timeout() {
        let manager = FailoverManager::new(3);
        let event = FailoverEvent {
            id: uuid::Uuid::new_v4(),
            timestamp: Utc::now(),
            old_leader: 1,
            new_leader: 2,
            reason: FailoverReason::LeaderTimeout,
            duration_ms: 250,
        };
        manager.record_failover(event.clone()).await;

        let history = manager.history().await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].reason, FailoverReason::LeaderTimeout);
        assert_eq!(history[0].duration_ms, 250);
    }
}
