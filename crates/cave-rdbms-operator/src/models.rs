// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Domain models for cave-pg.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Domain enums ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DbStatus {
    Online,
    Offline,
    Degraded,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Pending,
    Applied,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupType {
    Full,
    Incremental,
    SchemaOnly,
    DataOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
}

// ── Domain structs ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInstance {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub status: DbStatus,
    pub version: Option<String>,
    pub registered_at: DateTime<Utc>,
    pub last_checked: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub id: Uuid,
    pub database_id: Uuid,
    pub version: String,
    pub name: String,
    pub checksum: String,
    pub status: MigrationStatus,
    pub applied_at: Option<DateTime<Utc>>,
    pub execution_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPool {
    pub id: Uuid,
    pub database_id: Uuid,
    pub name: String,
    pub min_size: u32,
    pub max_size: u32,
    pub current_size: u32,
    pub idle_connections: u32,
    pub active_connections: u32,
    pub waiting_clients: u32,
    pub total_checkout_count: u64,
    pub avg_checkout_ms: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStat {
    pub id: Uuid,
    pub database_id: Uuid,
    pub query_hash: String,
    pub query_text: Option<String>,
    pub calls: u64,
    pub total_time_ms: f64,
    pub mean_time_ms: f64,
    pub stddev_time_ms: f64,
    pub min_time_ms: f64,
    pub max_time_ms: f64,
    pub rows: u64,
    pub plan: Option<serde_json::Value>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupJob {
    pub id: Uuid,
    pub database_id: Uuid,
    pub backup_type: BackupType,
    pub status: BackupStatus,
    pub destination: String,
    pub size_bytes: Option<u64>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationSlot {
    pub slot_name: String,
    pub plugin: String,
    pub active: bool,
    pub lag_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationStatus {
    pub id: Uuid,
    pub database_id: Uuid,
    /// "primary" or "replica"
    pub role: String,
    pub primary_host: Option<String>,
    pub replication_lag_bytes: u64,
    pub replication_lag_seconds: f64,
    pub slots: Vec<ReplicationSlot>,
    pub is_in_recovery: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableStat {
    pub id: Uuid,
    pub database_id: Uuid,
    pub schema_name: String,
    pub table_name: String,
    pub live_tuples: u64,
    pub dead_tuples: u64,
    /// Ratio of dead/live tuples — higher means more bloat.
    pub bloat_ratio: f64,
    pub table_size_bytes: u64,
    pub index_size_bytes: u64,
    pub last_vacuum: Option<DateTime<Utc>>,
    pub last_analyze: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbUser {
    pub id: Uuid,
    pub database_id: Uuid,
    pub username: String,
    pub roles: Vec<String>,
    pub can_login: bool,
    pub is_superuser: bool,
    pub connection_limit: Option<i32>,
    pub valid_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSizeRecord {
    pub id: Uuid,
    pub database_id: Uuid,
    pub size_bytes: u64,
    pub table_count: u32,
    pub index_count: u32,
    pub recorded_at: DateTime<Utc>,
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterDatabaseRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RecordMigrationRequest {
    pub database_id: Uuid,
    pub version: String,
    pub name: String,
    pub checksum: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePoolRequest {
    pub database_id: Uuid,
    pub name: String,
    pub min_size: u32,
    pub max_size: u32,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePoolStatsRequest {
    pub current_size: u32,
    pub idle_connections: u32,
    pub active_connections: u32,
    pub waiting_clients: u32,
    pub total_checkout_count: u64,
    pub avg_checkout_ms: f64,
}

#[derive(Debug, Deserialize)]
pub struct RecordQueryStatRequest {
    pub database_id: Uuid,
    pub query_hash: String,
    pub query_text: Option<String>,
    pub calls: u64,
    pub total_time_ms: f64,
    pub mean_time_ms: f64,
    pub stddev_time_ms: f64,
    pub min_time_ms: f64,
    pub max_time_ms: f64,
    pub rows: u64,
    pub plan: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBackupRequest {
    pub database_id: Uuid,
    pub backup_type: BackupType,
    pub destination: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterReplicationRequest {
    pub database_id: Uuid,
    pub role: String,
    pub primary_host: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReplicationRequest {
    pub replication_lag_bytes: u64,
    pub replication_lag_seconds: f64,
    pub slots: Vec<ReplicationSlot>,
    pub is_in_recovery: bool,
}

#[derive(Debug, Deserialize)]
pub struct RecordTableStatRequest {
    pub database_id: Uuid,
    pub schema_name: String,
    pub table_name: String,
    pub live_tuples: u64,
    pub dead_tuples: u64,
    pub bloat_ratio: f64,
    pub table_size_bytes: u64,
    pub index_size_bytes: u64,
    pub last_vacuum: Option<DateTime<Utc>>,
    pub last_analyze: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub database_id: Uuid,
    pub username: String,
    pub roles: Vec<String>,
    pub can_login: bool,
    pub is_superuser: bool,
    pub connection_limit: Option<i32>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RecordSizeRequest {
    pub database_id: Uuid,
    pub size_bytes: u64,
    pub table_count: u32,
    pub index_count: u32,
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SlowQueryParams {
    /// Mean execution time threshold in milliseconds (default 100).
    pub threshold_ms: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct BloatParams {
    /// Minimum bloat ratio to include (default 0.2 = 20%).
    pub min_ratio: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct SizeAlertParams {
    /// Alert threshold in bytes (default 10 GiB).
    pub threshold_bytes: Option<u64>,
}
