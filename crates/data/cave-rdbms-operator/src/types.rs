// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InstanceState {
    Creating,
    Running,
    Stopped,
    Failed,
    Deleting,
    Restarting,
    Promoting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgInstance {
    pub id: String,
    pub name: String,
    pub version: String,
    pub state: InstanceState,
    pub connection_string: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub max_connections: u32,
    pub shared_buffers_mb: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub labels: HashMap<String, String>,
    pub is_primary: bool,
    pub replication_lag_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolMode {
    Session,
    Transaction,
    Statement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    pub name: String,
    pub instance_id: String,
    pub mode: PoolMode,
    pub pool_size: u32,
    pub min_pool_size: u32,
    pub max_client_connections: u32,
    pub server_idle_timeout_secs: u32,
    pub client_idle_timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupType {
    Full,
    Incremental,
    WAL,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BackupStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub instance_id: String,
    pub backup_type: BackupType,
    pub status: BackupStatus,
    pub size_bytes: u64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub storage_path: String,
    pub wal_start_lsn: Option<String>,
    pub wal_end_lsn: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaInfo {
    pub instance_id: String,
    pub primary_id: String,
    pub state: ReplicationState,
    pub lag_bytes: i64,
    pub lag_seconds: f64,
    pub sync_state: SyncState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReplicationState {
    Streaming,
    CatchingUp,
    Paused,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncState {
    Sync,
    Async,
    Quorum,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgStatActivity {
    pub pid: i32,
    pub datname: Option<String>,
    pub usename: Option<String>,
    pub application_name: Option<String>,
    pub client_addr: Option<String>,
    pub state: Option<String>,
    pub query: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub query_start: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgStatTable {
    pub schemaname: String,
    pub tablename: String,
    pub seq_scan: i64,
    pub idx_scan: i64,
    pub n_live_tup: i64,
    pub n_dead_tup: i64,
    pub last_vacuum: Option<DateTime<Utc>>,
    pub last_analyze: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgRole {
    pub name: String,
    pub superuser: bool,
    pub create_db: bool,
    pub create_role: bool,
    pub login: bool,
    pub replication: bool,
    pub connection_limit: i32,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitrTarget {
    pub target_time: DateTime<Utc>,
    pub target_lsn: Option<String>,
    pub target_name: Option<String>,
}
