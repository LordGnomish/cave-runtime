// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Enumerations ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceRole {
    Leader,
    Follower,
    Candidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Healthy,
    Degraded,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationMode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DRReplicationMode {
    ActivePassive,
    ActiveActive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackupTarget {
    CaveStore,
    S3,
}

// ── Core cluster types ────────────────────────────────────────────────────────

/// A single bare-metal CAVE runtime instance in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInstance {
    pub id: Uuid,
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    pub role: InstanceRole,
    pub status: InstanceStatus,
    pub last_heartbeat: DateTime<Utc>,
    pub datacenter: String,
    pub started_at: DateTime<Utc>,
}

/// Raft persistent state (survives restarts in production via write-ahead log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftState {
    pub current_term: u64,
    pub voted_for: Option<Uuid>,
    pub commit_index: u64,
    pub last_applied: u64,
    pub leader_id: Option<Uuid>,
}

/// A single entry in the replicated command log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub term: u64,
    pub index: u64,
    /// Serialized state mutation command.
    pub command: String,
    pub timestamp: DateTime<Utc>,
}

/// Current view of all instances, the elected leader, and quorum parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterTopology {
    pub instances: Vec<RuntimeInstance>,
    pub leader: Option<Uuid>,
    pub quorum_size: usize,
    pub split_brain_protection: bool,
}

// ── Configuration types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// How often the leader sends heartbeats (milliseconds).
    pub interval_ms: u64,
    /// How long a follower waits before starting an election (milliseconds).
    pub timeout_ms: u64,
    /// Maximum missed heartbeats before triggering failure detection.
    pub max_missed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    pub mode: ReplicationMode,
    pub targets: Vec<Uuid>,
    /// Maximum tolerable replication lag in milliseconds before alerting.
    pub lag_tolerance: u64,
}

/// DR site pairing and recovery objectives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DRConfig {
    pub primary_site: String,
    pub secondary_site: String,
    pub replication_mode: DRReplicationMode,
    /// Recovery Point Objective: maximum tolerable data loss in seconds.
    pub rpo_seconds: u64,
    /// Recovery Time Objective: maximum tolerable downtime in seconds.
    pub rto_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    /// Interval between full backups in seconds.
    pub full_backup_interval: u64,
    /// Interval between incremental backups in seconds.
    pub incremental_interval: u64,
    pub retention_days: u32,
    pub target: BackupTarget,
}

// ── Event / status types ──────────────────────────────────────────────────────

/// Audit record of a leader election or manual failover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverEvent {
    pub timestamp: DateTime<Utc>,
    pub old_leader: Option<Uuid>,
    pub new_leader: Uuid,
    pub reason: String,
    pub duration_ms: u64,
}

/// Status snapshot of a datacenter site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteStatus {
    pub datacenter: String,
    pub instances: Vec<RuntimeInstance>,
    pub is_primary: bool,
    /// Measured cross-site replication lag in milliseconds.
    pub replication_lag: u64,
    pub last_sync: DateTime<Utc>,
}

/// One peer's health opinion about another instance (consensus failure detection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthVote {
    pub instance_id: Uuid,
    pub target_id: Uuid,
    pub healthy: bool,
    pub timestamp: DateTime<Utc>,
}
