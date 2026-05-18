// SPDX-License-Identifier: AGPL-3.0-or-later
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use std::sync::Arc;

/// Prometheus metrics for a Raft node.
pub struct Metrics {
    /// Current term number.
    pub current_term: Gauge,
    /// Commit index.
    pub commit_index: Gauge,
    /// Last applied index.
    pub last_applied: Gauge,
    /// 1.0 if this node is leader, 0.0 otherwise.
    pub is_leader: Gauge,
    /// Leader ID (0 if unknown).
    pub leader_id: Gauge,
    /// Total proposals received.
    pub proposals_total: Counter,
    /// Proposals that failed (not leader, quorum loss, etc.).
    pub proposals_failed: Counter,
    /// Number of leader elections started.
    pub elections_started: Counter,
    /// Number of leadership transitions (leader changes).
    pub leader_changes: Counter,
    /// Heartbeats sent (leader).
    pub heartbeats_sent: Counter,
    /// AppendEntries RPCs sent.
    pub append_entries_sent: Counter,
    /// Snapshots sent to peers.
    pub snapshots_sent: Counter,
    /// Snapshots received from leader.
    pub snapshots_received: Counter,
    /// Log entries applied to state machine.
    pub entries_applied: Counter,
    /// Log compactions performed.
    pub log_compactions: Counter,
    /// Check quorum failures (leader step-down due to quorum loss).
    pub check_quorum_failures: Counter,
    /// Membership changes applied.
    pub membership_changes: Counter,
    /// DR replication lag (entries behind primary).
    pub dr_lag_entries: Gauge,
    /// ReadIndex requests served.
    pub read_index_requests: Counter,
}

impl Metrics {
    pub fn new(registry: &mut Registry) -> Arc<Self> {
        let m = Arc::new(Self {
            current_term: Gauge::default(),
            commit_index: Gauge::default(),
            last_applied: Gauge::default(),
            is_leader: Gauge::default(),
            leader_id: Gauge::default(),
            proposals_total: Counter::default(),
            proposals_failed: Counter::default(),
            elections_started: Counter::default(),
            leader_changes: Counter::default(),
            heartbeats_sent: Counter::default(),
            append_entries_sent: Counter::default(),
            snapshots_sent: Counter::default(),
            snapshots_received: Counter::default(),
            entries_applied: Counter::default(),
            log_compactions: Counter::default(),
            check_quorum_failures: Counter::default(),
            membership_changes: Counter::default(),
            dr_lag_entries: Gauge::default(),
            read_index_requests: Counter::default(),
        });
        registry.register("raft_current_term", "Current Raft term", m.current_term.clone());
        registry.register("raft_commit_index", "Commit index", m.commit_index.clone());
        registry.register("raft_last_applied", "Last applied index", m.last_applied.clone());
        registry.register("raft_is_leader", "1 if this node is leader", m.is_leader.clone());
        registry.register("raft_leader_id", "Current leader node ID", m.leader_id.clone());
        registry.register("raft_proposals_total", "Total proposals received", m.proposals_total.clone());
        registry.register("raft_proposals_failed", "Failed proposals", m.proposals_failed.clone());
        registry.register("raft_elections_started", "Elections started", m.elections_started.clone());
        registry.register("raft_leader_changes", "Leadership transitions", m.leader_changes.clone());
        registry.register("raft_heartbeats_sent", "Heartbeats sent", m.heartbeats_sent.clone());
        registry.register("raft_append_entries_sent", "AppendEntries RPCs sent", m.append_entries_sent.clone());
        registry.register("raft_snapshots_sent", "Snapshots sent", m.snapshots_sent.clone());
        registry.register("raft_snapshots_received", "Snapshots received", m.snapshots_received.clone());
        registry.register("raft_entries_applied", "Entries applied to state machine", m.entries_applied.clone());
        registry.register("raft_log_compactions", "Log compactions performed", m.log_compactions.clone());
        registry.register("raft_check_quorum_failures", "Check-quorum step-downs", m.check_quorum_failures.clone());
        registry.register("raft_membership_changes", "Membership changes", m.membership_changes.clone());
        registry.register("raft_dr_lag_entries", "DR replication lag in entries", m.dr_lag_entries.clone());
        registry.register("raft_read_index_requests", "ReadIndex requests", m.read_index_requests.clone());
        m
    }
}
