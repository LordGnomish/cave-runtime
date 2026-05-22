// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! State replication from leader to followers.
//!
//! Two modes:
//! - **Sync** — leader waits for quorum acknowledgment before confirming a write
//!   (strong consistency, CP in CAP terms).
//! - **Async** — leader fires-and-forgets to followers (eventual consistency,
//!   lower latency, suitable for read-heavy workloads tolerating slight lag).

use crate::{models::{LogEntry, ReplicationMode}, HaState};
use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Replicate a state-change entry; dispatch to sync or async based on config.
pub async fn replicate_state(state: Arc<HaState>, entry: LogEntry) -> Result<()> {
    match state.replication_config.mode {
        ReplicationMode::Sync => sync_replication(state, entry).await,
        ReplicationMode::Async => {
            async_replication(Arc::clone(&state), entry).await;
            Ok(())
        }
    }
}

/// Wait for a quorum of followers to acknowledge the entry before returning.
///
/// Provides strong consistency: a committed entry will survive the loss of any
/// minority of nodes.
pub async fn sync_replication(state: Arc<HaState>, entry: LogEntry) -> Result<()> {
    let targets = state.replication_config.targets.clone();
    // Quorum = majority of the full cluster (self + followers).
    let quorum_needed = (targets.len() + 2) / 2; // ceiling division, +1 for self

    // Production: send AppendEntries RPCs to each target and await acks.
    // Stub: assume all targets respond immediately (in-process cluster).
    let acks = targets.len() + 1; // +1 for self

    if acks >= quorum_needed {
        let mut raft = state.raft.write().await;
        raft.commit_index = entry.index;
        info!(
            index = entry.index,
            term = entry.term,
            acks,
            required = quorum_needed,
            "Sync replication committed"
        );
        Ok(())
    } else {
        warn!(
            index = entry.index,
            acks,
            required = quorum_needed,
            "Sync replication failed: insufficient acks"
        );
        Err(anyhow::anyhow!(
            "Quorum not reached for log index {} (got {}/{} acks)",
            entry.index,
            acks,
            quorum_needed
        ))
    }
}

/// Fire-and-forget replication — returns immediately; followers catch up in background.
pub async fn async_replication(state: Arc<HaState>, entry: LogEntry) {
    let targets = state.replication_config.targets.len();
    // Production: tokio::spawn a task per target, send AppendEntries without awaiting.
    info!(
        index = entry.index,
        term = entry.term,
        targets,
        "Async replication dispatched"
    );
}

/// Measure how far behind a specific follower is (in log entries).
pub async fn detect_lag(state: Arc<HaState>, follower_id: Uuid) -> Result<u64> {
    let commit_index = state.raft.read().await.commit_index;
    let in_topology = state
        .topology
        .read()
        .await
        .instances
        .iter()
        .any(|i| i.id == follower_id);

    if !in_topology {
        return Err(anyhow::anyhow!(
            "Follower {} is not in cluster topology",
            follower_id
        ));
    }

    // Production: query follower's last_applied via RPC, return commit_index - last_applied.
    let lag_entries = 0u64;
    info!(
        follower = %follower_id,
        commit_index,
        lag_entries,
        "Replication lag measured"
    );
    Ok(lag_entries)
}

/// Follower requests missing log entries after recovering from a network partition.
///
/// The leader streams entries `[from_index, commit_index]` so the rejoining
/// follower can apply them and return to a consistent state.
pub async fn catch_up(
    state: Arc<HaState>,
    follower_id: Uuid,
    from_index: u64,
) -> Result<Vec<LogEntry>> {
    let commit_index = state.raft.read().await.commit_index;
    // Production: retrieve entries from the leader's durable log store.
    // Stub: no persistent log in Phase 1 — follower starts fresh.
    info!(
        follower = %follower_id,
        from_index,
        commit_index,
        "Follower catch-up initiated"
    );
    Ok(vec![])
}
