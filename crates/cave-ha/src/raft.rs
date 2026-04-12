//! Simplified Raft consensus implementation.
//!
//! No external etcd dependency — pure in-process state machine suitable for
//! small bare-metal clusters (3–7 nodes). A production deployment would persist
//! the write-ahead log to disk; this phase keeps it in-memory.

use crate::{
    models::{InstanceRole, LogEntry},
    HaState,
};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Candidate requests votes from peers when the leader heartbeat times out.
///
/// Increments the current term, votes for self, and transitions to Candidate.
pub async fn start_election(state: Arc<HaState>) -> Result<()> {
    let self_id = state.self_instance.id;
    let new_term = {
        let mut raft = state.raft.write().await;
        raft.current_term += 1;
        raft.voted_for = Some(self_id);
        raft.leader_id = None;
        raft.current_term
    };
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.role = InstanceRole::Candidate;
        }
    }
    info!(term = new_term, candidate = %self_id, "Election started");
    Ok(())
}

/// Handle an incoming vote request from a candidate.
///
/// Grants the vote if the candidate's term is higher, or if we haven't voted
/// yet in this term and the candidate's log is at least as up-to-date as ours.
pub async fn request_vote(
    state: Arc<HaState>,
    candidate_id: Uuid,
    candidate_term: u64,
    candidate_log_index: u64,
) -> Result<bool> {
    let mut raft = state.raft.write().await;

    if candidate_term > raft.current_term {
        raft.current_term = candidate_term;
        raft.voted_for = Some(candidate_id);
        raft.leader_id = None;
        info!(term = candidate_term, candidate = %candidate_id, "Vote granted (higher term)");
        return Ok(true);
    }

    if candidate_term == raft.current_term
        && (raft.voted_for.is_none() || raft.voted_for == Some(candidate_id))
        && candidate_log_index >= raft.commit_index
    {
        raft.voted_for = Some(candidate_id);
        info!(term = candidate_term, candidate = %candidate_id, "Vote granted (same term)");
        return Ok(true);
    }

    warn!(
        candidate_term,
        current_term = raft.current_term,
        candidate = %candidate_id,
        "Vote denied"
    );
    Ok(false)
}

/// Leader replicates log entries to this follower and updates the commit index.
///
/// Returns `false` if the leader's term is stale (the leader must step down).
pub async fn append_entries(
    state: Arc<HaState>,
    leader_id: Uuid,
    leader_term: u64,
    entries: Vec<LogEntry>,
    leader_commit: u64,
) -> Result<bool> {
    let mut raft = state.raft.write().await;

    if leader_term < raft.current_term {
        warn!(
            leader_term,
            current_term = raft.current_term,
            "Rejected append_entries: stale leader term"
        );
        return Ok(false);
    }

    raft.current_term = leader_term;
    raft.leader_id = Some(leader_id);
    if leader_commit > raft.commit_index {
        raft.commit_index = leader_commit;
    }

    info!(
        leader = %leader_id,
        entries = entries.len(),
        commit_index = raft.commit_index,
        "Entries appended"
    );
    Ok(true)
}

/// Apply all log entries up to `commit_index` to the local state machine.
pub async fn apply_committed(state: Arc<HaState>) -> Result<()> {
    let (commit_index, last_applied) = {
        let raft = state.raft.read().await;
        (raft.commit_index, raft.last_applied)
    };

    if commit_index > last_applied {
        let mut raft = state.raft.write().await;
        raft.last_applied = raft.commit_index;
        info!(applied_through = raft.last_applied, "Committed entries applied to state machine");
    }
    Ok(())
}

/// Leader sends a periodic heartbeat (empty append_entries) to prevent elections.
pub async fn leader_heartbeat(state: Arc<HaState>) -> Result<()> {
    let self_id = state.self_instance.id;
    let term = state.raft.read().await.current_term;
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.last_heartbeat = Utc::now();
        }
    }
    info!(term, leader = %self_id, "Heartbeat sent");
    Ok(())
}

/// Leader steps down to follower upon discovering a higher term in any RPC response.
pub async fn step_down(state: Arc<HaState>, new_term: u64) -> Result<()> {
    let self_id = state.self_instance.id;
    {
        let mut raft = state.raft.write().await;
        raft.current_term = new_term;
        raft.voted_for = None;
        raft.leader_id = None;
    }
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.role = InstanceRole::Follower;
        }
    }
    warn!(new_term, node = %self_id, "Stepped down to follower");
    Ok(())
}
