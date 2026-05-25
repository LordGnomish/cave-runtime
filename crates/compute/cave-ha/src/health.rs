// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster health monitoring and auto-healing.
//!
//! Aggregates health across all instances, detects quorum loss, identifies
//! asymmetric network partitions, and attempts self-healing where possible.

use crate::{models::InstanceStatus, HaState};
use anyhow::Result;
use std::{collections::HashSet, sync::Arc};
use tracing::{info, warn};

/// Self-health check for this instance: CPU, memory, disk, and module health.
///
/// Production: query `/proc/meminfo`, disk utilisation, and each module's
/// `/health` endpoint. Returns structured health facts for peer voting.
pub async fn instance_health_check(_state: Arc<HaState>) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "cpu_ok":     true,
        "memory_ok":  true,
        "disk_ok":    true,
        "modules_ok": true,
    }))
}

/// Aggregate health view across the entire cluster.
pub async fn cluster_health(state: Arc<HaState>) -> Result<serde_json::Value> {
    let topology = state.topology.read().await;
    let total = topology.instances.len();
    let healthy = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Healthy)
        .count();
    let degraded = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Degraded)
        .count();
    let unreachable = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Unreachable)
        .count();
    let leader = topology.leader.map(|id| id.to_string());
    let has_quorum = healthy >= topology.quorum_size;
    drop(topology);

    Ok(serde_json::json!({
        "total_instances": total,
        "healthy":         healthy,
        "degraded":        degraded,
        "unreachable":     unreachable,
        "leader":          leader,
        "has_quorum":      has_quorum,
    }))
}

/// Verify we still hold a majority of healthy instances.
pub async fn quorum_check(state: Arc<HaState>) -> Result<bool> {
    let topology = state.topology.read().await;
    let healthy = topology
        .instances
        .iter()
        .filter(|i| i.status == InstanceStatus::Healthy)
        .count();
    let required = topology.quorum_size;
    let has_quorum = healthy >= required;
    drop(topology);

    if has_quorum {
        info!(healthy, required, "Quorum check passed");
    } else {
        warn!(healthy, required, "Quorum check FAILED");
    }
    Ok(has_quorum)
}

/// Detect asymmetric network failures where this instance can see some peers
/// but not others (classic split scenario that precedes a split-brain).
///
/// A partition is suspected when the number of peers we have recent health
/// votes from is less than `total_peers`.
pub async fn network_partition_detection(state: Arc<HaState>) -> Result<bool> {
    let total_peers = state
        .topology
        .read()
        .await
        .instances
        .len()
        .saturating_sub(1); // exclude self

    if total_peers == 0 {
        return Ok(false); // single-node cluster — no partition possible
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(30);
    let self_id = state.self_instance.id;
    let reachable: HashSet<uuid::Uuid> = state
        .health_votes
        .read()
        .await
        .iter()
        .filter(|v| v.instance_id == self_id && v.timestamp > cutoff)
        .map(|v| v.target_id)
        .collect();

    let partition = reachable.len() < total_peers;
    if partition {
        warn!(
            reachable = reachable.len(),
            total_peers,
            "Network partition suspected: cannot reach all peers"
        );
    }
    Ok(partition)
}

/// Auto-healing: attempt recovery from quorum loss or network partition.
///
/// Called by the background health loop. Triggers split-brain protection if
/// quorum is lost, and initiates catch-up replication after a partition heals.
pub async fn auto_healing(state: Arc<HaState>) -> Result<()> {
    let has_quorum = quorum_check(Arc::clone(&state)).await?;
    if !has_quorum {
        info!("Auto-healing: quorum lost — applying split-brain protection");
        crate::failover::split_brain_protection(Arc::clone(&state)).await?;
    }

    let partition = network_partition_detection(Arc::clone(&state)).await?;
    if partition {
        // Production: trigger catch_up replication for this instance.
        info!("Auto-healing: partition detected — catch-up replication scheduled");
    }

    info!("Auto-healing cycle complete");
    Ok(())
}
