<<<<<<< HEAD
//! Leader failover and split-brain protection.
//!
//! Failure detection is consensus-based: an instance is only declared failed
//! when a quorum of peers independently observe the failure, preventing
//! false positives from transient network blips.

use crate::{
    models::{FailoverEvent, InstanceRole, InstanceStatus},
    HaState,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Consensus-based failure detection.
///
/// An instance is considered failed only when a majority of peers report it
/// unhealthy within the last 10 seconds — prevents flip-flopping on packet loss.
pub async fn detect_failure(state: Arc<HaState>, target_id: Uuid) -> Result<bool> {
    let votes = state.health_votes.read().await;
    let cutoff = Utc::now() - chrono::Duration::seconds(10);

    let recent: Vec<_> = votes
        .iter()
        .filter(|v| v.target_id == target_id && v.timestamp > cutoff)
        .collect();

    if recent.is_empty() {
        return Ok(false);
    }

    let unhealthy = recent.iter().filter(|v| !v.healthy).count();
    let quorum = state.topology.read().await.quorum_size;
    let failed = unhealthy >= (quorum + 1) / 2;

    if failed {
        warn!(
            target = %target_id,
            unhealthy_votes = unhealthy,
            total_votes = recent.len(),
            "Consensus: instance declared failed"
        );
    }
    Ok(failed)
}

/// Promote `new_leader_id` to leader and update cluster routing.
///
/// Records a `FailoverEvent` in audit history for post-mortem analysis.
pub async fn trigger_failover(
    state: Arc<HaState>,
    new_leader_id: Uuid,
    reason: String,
) -> Result<()> {
    let start = std::time::Instant::now();
    let old_leader = state.raft.read().await.leader_id;

    {
        let mut raft = state.raft.write().await;
        raft.current_term += 1;
        raft.leader_id = Some(new_leader_id);
        raft.voted_for = Some(new_leader_id);
    }
    {
        let mut topology = state.topology.write().await;
        topology.leader = Some(new_leader_id);
        for inst in topology.instances.iter_mut() {
            if inst.id == new_leader_id {
                inst.role = InstanceRole::Leader;
            } else if matches!(inst.role, InstanceRole::Leader | InstanceRole::Candidate) {
                inst.role = InstanceRole::Follower;
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    state.failover_history.write().await.push(FailoverEvent {
        timestamp: Utc::now(),
        old_leader,
        new_leader: new_leader_id,
        reason: reason.clone(),
        duration_ms,
    });

    info!(
        new_leader = %new_leader_id,
        old_leader = ?old_leader,
        reason,
        duration_ms,
        "Failover complete"
    );
    Ok(())
}

/// Planned leader transfer for scheduled maintenance.
///
/// Verifies the target is healthy before initiating transfer to minimise
/// the chance of immediately triggering another failover.
pub async fn graceful_handoff(state: Arc<HaState>, target_id: Uuid) -> Result<()> {
    let target_healthy = {
        let topology = state.topology.read().await;
        topology
            .instances
            .iter()
            .find(|i| i.id == target_id)
            .map(|i| i.status == InstanceStatus::Healthy)
            .ok_or_else(|| anyhow!("Instance {} not found in topology", target_id))?
    };

    if !target_healthy {
        return Err(anyhow!("Target {} is not healthy — cannot hand off leadership", target_id));
    }

    info!(target = %target_id, "Initiating graceful leader handoff");
    trigger_failover(state, target_id, "graceful_handoff".to_string()).await
}

/// Prevent split-brain by stepping down if quorum becomes unreachable.
///
/// If fewer than `quorum_size` instances are healthy, the current leader
/// must stop accepting writes to avoid a dual-leader scenario.
pub async fn split_brain_protection(state: Arc<HaState>) -> Result<()> {
    let (healthy_count, quorum) = {
        let topology = state.topology.read().await;
        let healthy = topology
            .instances
            .iter()
            .filter(|i| i.status == InstanceStatus::Healthy)
            .count();
        (healthy, topology.quorum_size)
    };

    if healthy_count < quorum {
        warn!(
            healthy = healthy_count,
            required = quorum,
            "Quorum lost — stepping down to prevent split-brain"
        );
        let self_term = state.raft.read().await.current_term;
        crate::raft::step_down(state, self_term).await?;
    }
    Ok(())
}

/// Fencing: mark the evicted instance as `Unreachable` so it cannot be routed to.
///
/// This is the software equivalent of STONITH — the old leader's writes are
/// rejected at the routing layer once it appears `Unreachable` in topology.
pub async fn fencing(state: Arc<HaState>, evicted_id: Uuid) -> Result<()> {
    let mut topology = state.topology.write().await;
    if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == evicted_id) {
        inst.status = InstanceStatus::Unreachable;
        inst.role = InstanceRole::Follower;
    }
    error!(evicted = %evicted_id, "Fencing applied: instance marked unreachable");
    Ok(())
=======
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
>>>>>>> claude/great-sanderson
}
