// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for a single Raft node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// This node's ID.
    pub id: u64,
    /// Minimum election timeout ticks (each tick ~100ms).
    pub election_timeout_min: u64,
    /// Maximum election timeout ticks.
    pub election_timeout_max: u64,
    /// Heartbeat interval in ticks.
    pub heartbeat_interval: u64,
    /// Maximum log entries per AppendEntries RPC.
    pub max_append_entries: usize,
    /// Max in-flight AppendEntries RPCs per peer (pipelining).
    pub pipeline_depth: usize,
    /// Enable check-quorum: leader steps down if it cannot contact quorum.
    pub check_quorum: bool,
    /// Check quorum interval in ticks.
    pub check_quorum_interval: u64,
    /// Enable pre-vote: candidate solicits pre-vote before incrementing term.
    pub pre_vote: bool,
    /// Trigger log compaction when log exceeds this many entries.
    pub log_compaction_threshold: u64,
    /// Snapshot chunk size for transfer (bytes).
    pub snapshot_chunk_size: usize,
    /// Tick duration.
    #[serde(skip, default = "default_tick_duration")]
    pub tick_duration: Duration,
    /// HTTP API listen address.
    pub api_addr: String,
    /// gRPC transport listen address.
    pub grpc_addr: String,
    /// Advertised gRPC address (for peers to connect to).
    pub grpc_advertise_addr: String,
    /// Maximum number of pending proposals before backpressure.
    pub max_pending_proposals: usize,
    /// Leadership transfer timeout in ticks.
    pub leadership_transfer_timeout: u64,
    /// ReadIndex lease duration (fraction of election timeout).
    pub lease_fraction: f64,
}

fn default_tick_duration() -> Duration {
    Duration::from_millis(100)
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: 1,
            election_timeout_min: 10,
            election_timeout_max: 20,
            heartbeat_interval: 2,
            max_append_entries: 64,
            pipeline_depth: 4,
            check_quorum: true,
            check_quorum_interval: 10,
            pre_vote: true,
            log_compaction_threshold: 10_000,
            snapshot_chunk_size: 1024 * 1024, // 1 MiB
            tick_duration: Duration::from_millis(100),
            api_addr: "0.0.0.0:8080".into(),
            grpc_addr: "0.0.0.0:9090".into(),
            grpc_advertise_addr: "127.0.0.1:9090".into(),
            max_pending_proposals: 1000,
            leadership_transfer_timeout: 10,
            lease_fraction: 0.9,
        }
    }
}

/// Configuration for DR replication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrConfig {
    /// Remote site address (gRPC).
    pub remote_addr: String,
    /// Async replication: don't wait for DR ack on each commit.
    pub async_mode: bool,
    /// Maximum replication lag before alerting (log entries).
    pub max_lag_entries: u64,
    /// Maximum replication lag before alerting (time).
    pub max_lag_duration: Duration,
    /// RPO target in seconds.
    pub rpo_seconds: u64,
    /// RTO target in seconds.
    pub rto_seconds: u64,
    /// Automatic failback enabled.
    pub auto_failback: bool,
    /// Failback delay after primary recovery (seconds).
    pub failback_delay_seconds: u64,
}

impl Default for DrConfig {
    fn default() -> Self {
        Self {
            remote_addr: String::new(),
            async_mode: true,
            max_lag_entries: 1000,
            max_lag_duration: Duration::from_secs(30),
            rpo_seconds: 60,
            rto_seconds: 300,
            auto_failback: false,
            failback_delay_seconds: 300,
        }
    }
}
