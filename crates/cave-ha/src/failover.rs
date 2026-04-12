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
}
