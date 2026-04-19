//! PostgreSQL management — compatible with pgAdmin / CloudNativePG.
//!
//! Compatible with: pgAdmin, CloudNativePG
//! Upstream tracking: see cave-upstream for monitored features.
//!
//! # Features
//! - Database instance registry (register, list, health-check)
//! - Schema migration tracking
//! - Connection pool monitoring (PgBouncer-style)
//! - Query analytics: slow-query log, plan analysis, pg_stat_statements
//! - Backup / restore orchestration (pg_dump / pg_restore wrappers)
//! - Replication monitoring (primary / replica status, lag tracking)
//! - Table & index statistics, bloat detection
//! - User / role management
//! - Database size monitoring and threshold alerts

pub mod manager;
pub mod models;
pub mod routes;

use axum::Router;
use models::{BackupJob, ConnectionPool, DatabaseInstance, DbSizeRecord, DbUser, MigrationRecord, QueryStat, ReplicationStatus, TableStat};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Shared in-memory state for the pg module.
#[derive(Default)]
pub struct PgState {
    /// Registered database instances.
    pub databases: Arc<Mutex<HashMap<Uuid, DatabaseInstance>>>,
    /// Migration history records.
    pub migrations: Arc<Mutex<HashMap<Uuid, MigrationRecord>>>,
    /// Connection pool configurations and live stats.
    pub pools: Arc<Mutex<HashMap<Uuid, ConnectionPool>>>,
    /// Per-query execution statistics (pg_stat_statements-style).
    pub query_stats: Arc<Mutex<Vec<QueryStat>>>,
    /// Backup jobs (pg_dump orchestration).
    pub backups: Arc<Mutex<HashMap<Uuid, BackupJob>>>,
    /// Replication topology and lag snapshots.
    pub replication: Arc<Mutex<HashMap<Uuid, ReplicationStatus>>>,
    /// Table/index bloat snapshots.
    pub table_stats: Arc<Mutex<Vec<TableStat>>>,
    /// Database users and roles.
    pub users: Arc<Mutex<HashMap<Uuid, DbUser>>>,
    /// Database size samples (triggers alerts when thresholds exceeded).
    pub sizes: Arc<Mutex<Vec<DbSizeRecord>>>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<PgState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pg";
