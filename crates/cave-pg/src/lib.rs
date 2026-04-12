<<<<<<< HEAD
//! PostgreSQL management — replaces pgAdmin / CloudNativePG.
//!
//! Replaces: pgAdmin, CloudNativePG
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
=======
//! CAVE PG — PostgreSQL management (Crunchy PGO replacement).

pub mod backup;
pub mod error;
pub mod ha;
pub mod lifecycle;
pub mod monitoring;
pub mod pool;
pub mod routes;
pub mod types;
pub mod user;

pub use error::{PgError, PgResult};
pub use lifecycle::InstanceManager;
pub use pool::ConnectionPool;

pub const MODULE_NAME: &str = "pg";

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::backup::BackupManager;
    use crate::ha::HaController;
    use crate::lifecycle::InstanceManager;
    use crate::monitoring::Monitor;
    use crate::pool::ConnectionPool;
    use crate::types::*;
    use crate::user::{UserManager, UserOptions};

    // ── Test 1: create_and_get_instance ─────────────────────────────────────
    #[test]
    fn create_and_get_instance() {
        let mgr = InstanceManager::new();
        let inst = mgr
            .create_instance("mydb", "15.4", "localhost", 5432, "postgres", "admin")
            .expect("create_instance");

        assert_eq!(inst.name, "mydb");
        assert_eq!(inst.version, "15.4");
        assert_eq!(inst.host, "localhost");
        assert_eq!(inst.port, 5432);
        assert_eq!(inst.database, "postgres");
        assert_eq!(inst.username, "admin");
        assert_eq!(inst.state, InstanceState::Creating);
        assert!(inst.is_primary);

        let fetched = mgr.get_instance(&inst.id).expect("get_instance");
        assert_eq!(fetched.id, inst.id);
        assert_eq!(fetched.name, "mydb");

        // Also test get_by_name
        let by_name = mgr.get_instance_by_name("mydb").expect("by_name");
        assert_eq!(by_name.id, inst.id);
    }

    // ── Test 2: instance_state_transitions ──────────────────────────────────
    #[test]
    fn instance_state_transitions() {
        let mgr = InstanceManager::new();
        let inst = mgr
            .create_instance("transit", "15.4", "localhost", 5432, "db", "user")
            .expect("create");

        assert_eq!(inst.state, InstanceState::Creating);

        mgr.start_instance(&inst.id).expect("start");
        let after_start = mgr.get_instance(&inst.id).unwrap();
        assert_eq!(after_start.state, InstanceState::Running);

        mgr.stop_instance(&inst.id).expect("stop");
        let after_stop = mgr.get_instance(&inst.id).unwrap();
        assert_eq!(after_stop.state, InstanceState::Stopped);

        mgr.restart_instance(&inst.id).expect("restart");
        let after_restart = mgr.get_instance(&inst.id).unwrap();
        // restart_instance transitions through Restarting and ends at Running
        assert_eq!(after_restart.state, InstanceState::Running);
    }

    // ── Test 3: delete_instance ──────────────────────────────────────────────
    #[test]
    fn delete_instance() {
        let mgr = InstanceManager::new();
        let inst = mgr
            .create_instance("todelete", "15.4", "localhost", 5432, "db", "user")
            .expect("create");

        mgr.delete_instance(&inst.id).expect("delete");

        let result = mgr.get_instance(&inst.id);
        assert!(result.is_err(), "instance should not be found after delete");
    }

    // ── Test 4: pool_create_and_stats ────────────────────────────────────────
    #[test]
    fn pool_create_and_stats() {
        let pool = ConnectionPool::new();
        let config = PoolConfig {
            name: "mypool".to_string(),
            instance_id: "inst-1".to_string(),
            mode: PoolMode::Transaction,
            pool_size: 20,
            min_pool_size: 5,
            max_client_connections: 100,
            server_idle_timeout_secs: 600,
            client_idle_timeout_secs: 0,
        };
        pool.create_pool(config).expect("create_pool");

        let retrieved = pool.get_pool_config("mypool").expect("get_pool_config");
        assert_eq!(retrieved.name, "mypool");
        assert_eq!(retrieved.pool_size, 20);

        let stats = pool.get_stats("mypool").expect("get_stats");
        assert_eq!(stats.pool_name, "mypool");
        assert_eq!(stats.total_connections, 0);

        // Acquire a handle and check stats update
        let handle = pool.acquire("mypool").expect("acquire");
        let after_acquire = pool.get_stats("mypool").expect("stats after acquire");
        assert_eq!(after_acquire.active_connections, 1);

        pool.release(handle).expect("release");
        let after_release = pool.get_stats("mypool").expect("stats after release");
        assert_eq!(after_release.active_connections, 0);
    }

    // ── Test 5: pool_mode_descriptions ──────────────────────────────────────
    #[test]
    fn pool_mode_descriptions() {
        assert!(ConnectionPool::mode_description(&PoolMode::Session).contains("session"));
        assert!(ConnectionPool::mode_description(&PoolMode::Transaction).contains("transaction"));
        assert!(ConnectionPool::mode_description(&PoolMode::Statement).contains("statement"));
    }

    // ── Test 6: backup_lifecycle ─────────────────────────────────────────────
    #[test]
    fn backup_lifecycle() {
        let mgr = BackupManager::new("/var/backup");
        let record = mgr
            .start_backup("inst-1", BackupType::Full)
            .expect("start_backup");

        assert_eq!(record.status, BackupStatus::Running);
        assert_eq!(record.instance_id, "inst-1");

        mgr.complete_backup(&record.id, 1024 * 1024, "0/2000000")
            .expect("complete");

        let completed = mgr.get_backup(&record.id).expect("get_backup");
        assert_eq!(completed.status, BackupStatus::Completed);
        assert_eq!(completed.size_bytes, 1024 * 1024);
        assert_eq!(completed.wal_end_lsn.as_deref(), Some("0/2000000"));

        let list = mgr.list_backups("inst-1");
        assert_eq!(list.len(), 1);

        // Retention: 0 days should delete everything
        let deleted = mgr.apply_retention("inst-1", 0);
        assert_eq!(deleted, 1);
        assert!(mgr.list_backups("inst-1").is_empty());
    }

    // ── Test 7: pitr_find_base ───────────────────────────────────────────────
    #[test]
    fn pitr_find_base() {
        let mgr = BackupManager::new("/var/backup");

        // Create 3 backups at different times
        let b1 = mgr.start_backup("inst-pitr", BackupType::Full).unwrap();
        mgr.complete_backup(&b1.id, 100, "0/1000000").unwrap();

        // Small sleep-free trick: use a slightly-in-the-future time to disambiguate ordering
        // Since Utc::now() resolution is fine we create them and they'll have unique times
        let b2 = mgr.start_backup("inst-pitr", BackupType::Incremental).unwrap();
        mgr.complete_backup(&b2.id, 50, "0/1500000").unwrap();

        let b3 = mgr.start_backup("inst-pitr", BackupType::Full).unwrap();
        mgr.complete_backup(&b3.id, 200, "0/2000000").unwrap();

        // Find PITR base for "far future" — should get the latest completed backup
        let target = PitrTarget {
            target_time: Utc::now() + chrono::Duration::hours(1),
            target_lsn: None,
            target_name: None,
        };

        let base = mgr.find_pitr_base("inst-pitr", &target).expect("pitr base");
        // Should be one of the backups (the most recent one before target_time)
        assert_eq!(base.status, BackupStatus::Completed);
        assert!(
            base.id == b1.id || base.id == b2.id || base.id == b3.id,
            "base should be one of our backups"
        );

        // Find PITR for a very old time — no backup should qualify
        let old_target = PitrTarget {
            target_time: chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            target_lsn: None,
            target_name: None,
        };
        let result = mgr.find_pitr_base("inst-pitr", &old_target);
        assert!(result.is_err(), "no backup should be before epoch");
    }

    // ── Test 8: ha_register_replica_and_failover ─────────────────────────────
    #[test]
    fn ha_register_replica_and_failover() {
        let ha = HaController::new();

        // Register two replicas with different lag
        let r1 = ReplicaInfo {
            instance_id: "replica-1".to_string(),
            primary_id: "primary-1".to_string(),
            state: ReplicationState::Streaming,
            lag_bytes: 1000,
            lag_seconds: 0.01,
            sync_state: SyncState::Async,
        };
        let r2 = ReplicaInfo {
            instance_id: "replica-2".to_string(),
            primary_id: "primary-1".to_string(),
            state: ReplicationState::Streaming,
            lag_bytes: 100, // Less lag — should be chosen
            lag_seconds: 0.001,
            sync_state: SyncState::Async,
        };

        ha.register_replica(r1).unwrap();
        ha.register_replica(r2).unwrap();

        let event = ha
            .trigger_failover("primary-1", "test failover")
            .expect("failover");

        // replica-2 has less lag, so it should be promoted
        assert_eq!(event.new_primary_id, "replica-2");
        assert_eq!(event.primary_id, "primary-1");

        let history = ha.failover_history();
        assert_eq!(history.len(), 1);
    }

    // ── Test 9: monitoring_slow_queries ──────────────────────────────────────
    #[test]
    fn monitoring_slow_queries() {
        let activities = vec![
            PgStatActivity {
                pid: 1,
                datname: Some("db".to_string()),
                usename: Some("user".to_string()),
                application_name: None,
                client_addr: None,
                state: Some("active".to_string()),
                query: Some("SELECT 1".to_string()),
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: Some(50),
            },
            PgStatActivity {
                pid: 2,
                datname: Some("db".to_string()),
                usename: Some("user".to_string()),
                application_name: None,
                client_addr: None,
                state: Some("active".to_string()),
                query: Some("SELECT pg_sleep(10)".to_string()),
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: Some(10500),
            },
            PgStatActivity {
                pid: 3,
                datname: Some("db".to_string()),
                usename: Some("user".to_string()),
                application_name: None,
                client_addr: None,
                state: Some("idle".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: Some(0),
            },
        ];

        let slow = Monitor::find_slow_queries(&activities, 1000);
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].pid, 2);
    }

    // ── Test 10: monitoring_connection_counts ────────────────────────────────
    #[test]
    fn monitoring_connection_counts() {
        let activities = vec![
            PgStatActivity {
                pid: 1,
                datname: None,
                usename: None,
                application_name: None,
                client_addr: None,
                state: Some("active".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: None,
            },
            PgStatActivity {
                pid: 2,
                datname: None,
                usename: None,
                application_name: None,
                client_addr: None,
                state: Some("active".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: None,
            },
            PgStatActivity {
                pid: 3,
                datname: None,
                usename: None,
                application_name: None,
                client_addr: None,
                state: Some("idle".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: None,
            },
            PgStatActivity {
                pid: 4,
                datname: None,
                usename: None,
                application_name: None,
                client_addr: None,
                state: Some("idle in transaction".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: None,
            },
            PgStatActivity {
                pid: 5,
                datname: None,
                usename: None,
                application_name: None,
                client_addr: None,
                state: Some("waiting".to_string()),
                query: None,
                wait_event_type: None,
                wait_event: None,
                query_start: None,
                duration_ms: None,
            },
        ];

        let counts = Monitor::count_connections(&activities);
        assert_eq!(counts.active, 2);
        assert_eq!(counts.idle, 1);
        assert_eq!(counts.idle_in_transaction, 1);
        assert_eq!(counts.waiting, 1);
        assert_eq!(counts.total, 5);
    }

    // ── Test 11: user_create_role_sql ────────────────────────────────────────
    #[test]
    fn user_create_role_sql() {
        let mgr = UserManager::new();
        let role = PgRole {
            name: "app_user".to_string(),
            superuser: false,
            create_db: false,
            create_role: false,
            login: true,
            replication: false,
            connection_limit: 10,
            valid_until: None,
        };
        let sql = mgr.create_role(role).expect("create_role");

        assert!(sql.contains("CREATE ROLE"), "SQL should contain CREATE ROLE");
        assert!(sql.contains("app_user"), "SQL should contain role name");
        assert!(sql.contains("LOGIN"), "SQL should contain LOGIN");
        assert!(sql.contains("CONNECTION LIMIT 10"), "SQL should contain connection limit");

        // Test create_user_sql
        let user_sql = UserManager::create_user_sql(
            "myuser",
            "s3cr3t",
            &UserOptions {
                superuser: false,
                create_db: true,
                login: true,
                connection_limit: -1,
            },
        );
        assert!(user_sql.contains("CREATE USER"));
        assert!(user_sql.contains("myuser"));
        assert!(user_sql.contains("s3cr3t"));
        assert!(user_sql.contains("CREATEDB"));
    }

    // ── Test 12: user_register_and_list ─────────────────────────────────────
    #[test]
    fn user_register_and_list() {
        let mgr = UserManager::new();

        let role1 = PgRole {
            name: "reader".to_string(),
            superuser: false,
            create_db: false,
            create_role: false,
            login: true,
            replication: false,
            connection_limit: 5,
            valid_until: None,
        };
        let role2 = PgRole {
            name: "writer".to_string(),
            superuser: false,
            create_db: false,
            create_role: false,
            login: true,
            replication: false,
            connection_limit: 10,
            valid_until: None,
        };

        mgr.register_role(role1).unwrap();
        mgr.register_role(role2).unwrap();

        let roles = mgr.list_roles();
        assert_eq!(roles.len(), 2);

        let reader = mgr.get_role("reader").expect("get reader");
        assert_eq!(reader.name, "reader");
        assert_eq!(reader.connection_limit, 5);

        let writer = mgr.get_role("writer").expect("get writer");
        assert_eq!(writer.name, "writer");

        assert!(mgr.get_role("nonexistent").is_none());

        // Test grant/revoke SQL generation
        let grant_sql = mgr.grant_role("reader", "app_user");
        assert!(grant_sql.contains("GRANT"));
        assert!(grant_sql.contains("reader"));
        assert!(grant_sql.contains("app_user"));

        let revoke_sql = mgr.revoke_role("reader", "app_user");
        assert!(revoke_sql.contains("REVOKE"));
        assert!(revoke_sql.contains("reader"));
        assert!(revoke_sql.contains("app_user"));
    }
}
>>>>>>> claude/dazzling-tesla
