//! cave-pg integration tests.
//!
//! Upstream parity reference: PostgreSQL `src/test/regress` (pg_regress smoke
//! suite for instance/role/backup/replication operations) and CloudNativePG
//! e2e tests for HA failover & PITR base selection. These tests exercise the
//! pure in-memory domain managers — no PostgreSQL daemon required.

use cave_rdbms_operator::backup::BackupManager;
use cave_rdbms_operator::ha::HaController;
use cave_rdbms_operator::lifecycle::InstanceManager;
use cave_rdbms_operator::manager::{
    bloated_tables, needs_vacuum, pool_utilisation_pct, replication_healthy,
    size_alert_records, slow_queries,
};
use cave_rdbms_operator::models::{
    DbSizeRecord, QueryStat, ReplicationSlot, ReplicationStatus, TableStat,
};
use cave_rdbms_operator::monitoring::Monitor;
use cave_rdbms_operator::pool::ConnectionPool;
use cave_rdbms_operator::types::{
    BackupStatus, BackupType, InstanceState, PgRole, PgStatActivity, PgStatTable, PitrTarget,
    PoolConfig, PoolMode, ReplicaInfo, ReplicationState, SyncState,
};
use cave_rdbms_operator::user::{RoleUpdate, UserManager, UserOptions};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn sample_replica(instance_id: &str, primary: &str, lag: i64) -> ReplicaInfo {
    ReplicaInfo {
        instance_id: instance_id.to_string(),
        primary_id: primary.to_string(),
        state: ReplicationState::Streaming,
        lag_bytes: lag,
        lag_seconds: 0.5,
        sync_state: SyncState::Async,
    }
}

fn sample_role(name: &str) -> PgRole {
    PgRole {
        name: name.to_string(),
        superuser: false,
        create_db: false,
        create_role: false,
        login: true,
        replication: false,
        connection_limit: -1,
        valid_until: None,
    }
}

fn sample_query_stat(mean_ms: f64) -> QueryStat {
    QueryStat {
        id: Uuid::new_v4(),
        database_id: Uuid::new_v4(),
        query_hash: "abc".into(),
        query_text: Some("SELECT 1".into()),
        calls: 1,
        total_time_ms: mean_ms,
        mean_time_ms: mean_ms,
        stddev_time_ms: 0.0,
        min_time_ms: mean_ms,
        max_time_ms: mean_ms,
        rows: 0,
        plan: None,
        recorded_at: Utc::now(),
    }
}

fn sample_table_stat(bloat: f64) -> TableStat {
    TableStat {
        id: Uuid::new_v4(),
        database_id: Uuid::new_v4(),
        schema_name: "public".into(),
        table_name: "t".into(),
        live_tuples: 100,
        dead_tuples: (100.0 * bloat) as u64,
        bloat_ratio: bloat,
        table_size_bytes: 1_000,
        index_size_bytes: 100,
        last_vacuum: None,
        last_analyze: None,
        recorded_at: Utc::now(),
    }
}

fn sample_size(bytes: u64) -> DbSizeRecord {
    DbSizeRecord {
        id: Uuid::new_v4(),
        database_id: Uuid::new_v4(),
        size_bytes: bytes,
        table_count: 1,
        index_count: 1,
        recorded_at: Utc::now(),
    }
}

// ── InstanceManager ──────────────────────────────────────────────────────────

#[test]
fn instance_manager_creates_unique_instance() {
    let mgr = InstanceManager::new();
    let inst = mgr
        .create_instance("primary", "16.2", "10.0.0.1", 5432, "app", "appuser")
        .unwrap();
    assert_eq!(inst.name, "primary");
    assert_eq!(inst.port, 5432);
    assert_eq!(inst.state, InstanceState::Creating);
    assert!(inst.connection_string.contains("postgres://"));
}

#[test]
fn instance_manager_rejects_duplicate_name() {
    let mgr = InstanceManager::new();
    mgr.create_instance("dup", "16", "h", 5432, "d", "u").unwrap();
    let err = mgr.create_instance("dup", "16", "h", 5432, "d", "u");
    assert!(err.is_err(), "duplicate must fail");
}

#[test]
fn instance_manager_get_by_id_and_name() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("a", "16", "h", 5432, "d", "u").unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().name, "a");
    assert_eq!(mgr.get_instance_by_name("a").unwrap().id, inst.id);
}

#[test]
fn instance_manager_lists_all() {
    let mgr = InstanceManager::new();
    mgr.create_instance("a", "16", "h", 5432, "d", "u").unwrap();
    mgr.create_instance("b", "16", "h", 5433, "d", "u").unwrap();
    assert_eq!(mgr.list_instances().len(), 2);
}

#[test]
fn instance_manager_lifecycle_transitions() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("life", "16", "h", 5432, "d", "u").unwrap();
    mgr.start_instance(&inst.id).unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().state, InstanceState::Running);
    mgr.stop_instance(&inst.id).unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().state, InstanceState::Stopped);
    mgr.restart_instance(&inst.id).unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().state, InstanceState::Running);
    mgr.mark_failed(&inst.id, "oom").unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().state, InstanceState::Failed);
}

#[test]
fn instance_manager_promote_clears_lag() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("p", "16", "h", 5432, "d", "u").unwrap();
    mgr.promote(&inst.id).unwrap();
    let after = mgr.get_instance(&inst.id).unwrap();
    assert!(after.is_primary);
    assert!(after.replication_lag_bytes.is_none());
    assert_eq!(after.state, InstanceState::Running);
}

#[test]
fn instance_manager_labels_replace() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("l", "16", "h", 5432, "d", "u").unwrap();
    let mut labels = HashMap::new();
    labels.insert("env".into(), "prod".into());
    mgr.update_labels(&inst.id, labels).unwrap();
    assert_eq!(mgr.get_instance(&inst.id).unwrap().labels.get("env"), Some(&"prod".into()));
}

#[test]
fn instance_manager_delete_removes() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("d", "16", "h", 5432, "d", "u").unwrap();
    mgr.delete_instance(&inst.id).unwrap();
    assert!(mgr.get_instance(&inst.id).is_err());
}

#[test]
fn instance_manager_connection_string_format() {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance("cs", "16", "db.local", 5433, "shop", "shopper").unwrap();
    let url = mgr.connection_string(&inst.id).unwrap();
    assert_eq!(url, "postgres://shopper@db.local:5433/shop");
}

// ── BackupManager ────────────────────────────────────────────────────────────

#[test]
fn backup_manager_starts_and_completes() {
    let bm = BackupManager::new("/wal");
    let rec = bm.start_backup("inst-1", BackupType::Full).unwrap();
    assert_eq!(rec.status, BackupStatus::Running);
    bm.complete_backup(&rec.id, 1024, "0/3000000").unwrap();
    let after = bm.get_backup(&rec.id).unwrap();
    assert_eq!(after.status, BackupStatus::Completed);
    assert_eq!(after.size_bytes, 1024);
    assert_eq!(after.wal_end_lsn.as_deref(), Some("0/3000000"));
}

#[test]
fn backup_manager_fails_a_backup() {
    let bm = BackupManager::new("/wal");
    let rec = bm.start_backup("i", BackupType::Incremental).unwrap();
    bm.fail_backup(&rec.id).unwrap();
    assert_eq!(bm.get_backup(&rec.id).unwrap().status, BackupStatus::Failed);
}

#[test]
fn backup_manager_lists_per_instance() {
    let bm = BackupManager::new("/wal");
    bm.start_backup("a", BackupType::Full).unwrap();
    bm.start_backup("a", BackupType::WAL).unwrap();
    bm.start_backup("b", BackupType::Full).unwrap();
    assert_eq!(bm.list_backups("a").len(), 2);
    assert_eq!(bm.list_backups("b").len(), 1);
    assert_eq!(bm.list_backups("c").len(), 0);
}

#[test]
fn backup_manager_delete() {
    let bm = BackupManager::new("/wal");
    let r = bm.start_backup("i", BackupType::Full).unwrap();
    bm.delete_backup(&r.id).unwrap();
    assert!(bm.delete_backup(&r.id).is_err());
}

#[test]
fn backup_manager_finds_pitr_base() {
    let bm = BackupManager::new("/wal");
    let early = bm.start_backup("i", BackupType::Full).unwrap();
    bm.complete_backup(&early.id, 1, "0/1").unwrap();
    let target = PitrTarget {
        target_time: Utc::now() + Duration::seconds(60),
        target_lsn: None,
        target_name: None,
    };
    let base = bm.find_pitr_base("i", &target).unwrap();
    assert_eq!(base.id, early.id);
}

#[test]
fn backup_manager_pitr_no_base_errors() {
    let bm = BackupManager::new("/wal");
    let target = PitrTarget {
        target_time: Utc::now(),
        target_lsn: None,
        target_name: None,
    };
    assert!(bm.find_pitr_base("nobody", &target).is_err());
}

#[test]
fn backup_manager_retention_drops_old() {
    let bm = BackupManager::new("/wal");
    let r = bm.start_backup("i", BackupType::Full).unwrap();
    bm.complete_backup(&r.id, 1, "0/1").unwrap();
    // Retention 0 days = drop everything
    let dropped = bm.apply_retention("i", 0);
    assert_eq!(dropped, 1);
}

#[test]
fn backup_manager_wal_segment_path_escapes_slash() {
    let bm = BackupManager::new("/wal");
    let p = bm.wal_segment_path("0/3000000");
    assert!(p.contains("0_3000000"));
    assert!(!p.contains("0/3000000"));
}

#[test]
fn backup_manager_lists_wal_segments() {
    let bm = BackupManager::new("/wal");
    let r = bm.start_backup("i", BackupType::Full).unwrap();
    bm.complete_backup(&r.id, 1, "0/2").unwrap();
    let segs = bm.list_wal_segments("i");
    assert_eq!(segs.len(), 1);
}

// ── HaController ─────────────────────────────────────────────────────────────

#[test]
fn ha_controller_registers_and_lists_replicas() {
    let ha = HaController::new();
    ha.register_replica(sample_replica("r1", "p", 0)).unwrap();
    ha.register_replica(sample_replica("r2", "p", 100)).unwrap();
    ha.register_replica(sample_replica("r3", "other", 0)).unwrap();
    assert_eq!(ha.list_replicas("p").len(), 2);
    assert_eq!(ha.list_replicas("other").len(), 1);
}

#[test]
fn ha_controller_updates_lag() {
    let ha = HaController::new();
    ha.register_replica(sample_replica("r1", "p", 0)).unwrap();
    ha.update_replica_lag("r1", 999, 1.5).unwrap();
    let r = ha.get_replica("r1").unwrap();
    assert_eq!(r.lag_bytes, 999);
    assert_eq!(r.lag_seconds, 1.5);
}

#[test]
fn ha_controller_failover_picks_lowest_lag() {
    let ha = HaController::new();
    ha.register_replica(sample_replica("r-far", "p", 5_000_000)).unwrap();
    ha.register_replica(sample_replica("r-near", "p", 100)).unwrap();
    let event = ha.trigger_failover("p", "primary down").unwrap();
    assert_eq!(event.new_primary_id, "r-near");
    assert_eq!(event.reason, "primary down");
}

#[test]
fn ha_controller_failover_records_history() {
    let ha = HaController::new();
    ha.register_replica(sample_replica("r1", "p", 0)).unwrap();
    ha.trigger_failover("p", "test").unwrap();
    assert_eq!(ha.failover_history().len(), 1);
}

#[test]
fn ha_controller_failover_without_replicas_errors() {
    let ha = HaController::new();
    assert!(ha.trigger_failover("nobody", "x").is_err());
}

#[test]
fn ha_controller_health_summary() {
    let ha = HaController::new();
    ha.register_replica(sample_replica("r1", "p", 50)).unwrap();
    ha.register_replica(sample_replica("r2", "p", 150)).unwrap();
    let h = ha.check_replication_health("p");
    assert_eq!(h.replica_count, 2);
    assert_eq!(h.healthy_replicas, 2);
    assert_eq!(h.max_lag_bytes, 150);
    assert!(h.is_healthy);
}

#[test]
fn ha_controller_health_unhealthy_when_lag_huge() {
    let ha = HaController::new();
    let mut r = sample_replica("r1", "p", 200 * 1024 * 1024);
    r.lag_seconds = 60.0;
    ha.register_replica(r).unwrap();
    let h = ha.check_replication_health("p");
    assert!(!h.is_healthy);
}

#[test]
fn ha_controller_pg_rewind_conservative() {
    let ha = HaController::new();
    assert!(ha.pg_rewind_needed("old_primary", "new_primary"));
}

// ── ConnectionPool ───────────────────────────────────────────────────────────

fn sample_pool_config(name: &str) -> PoolConfig {
    PoolConfig {
        name: name.into(),
        instance_id: "i".into(),
        mode: PoolMode::Transaction,
        pool_size: 20,
        min_pool_size: 2,
        max_client_connections: 100,
        server_idle_timeout_secs: 600,
        client_idle_timeout_secs: 60,
    }
}

#[test]
fn pool_creates_and_lists() {
    let pool = ConnectionPool::new();
    pool.create_pool(sample_pool_config("a")).unwrap();
    pool.create_pool(sample_pool_config("b")).unwrap();
    assert_eq!(pool.list_pools().len(), 2);
}

#[test]
fn pool_rejects_duplicate_name() {
    let pool = ConnectionPool::new();
    pool.create_pool(sample_pool_config("dup")).unwrap();
    assert!(pool.create_pool(sample_pool_config("dup")).is_err());
}

#[test]
fn pool_remove_unknown_errors() {
    let pool = ConnectionPool::new();
    assert!(pool.remove_pool("nope").is_err());
}

#[test]
fn pool_acquire_increments_stats_release_decrements() {
    let pool = ConnectionPool::new();
    pool.create_pool(sample_pool_config("p")).unwrap();
    let h1 = pool.acquire("p").unwrap();
    let h2 = pool.acquire("p").unwrap();
    let s = pool.get_stats("p").unwrap();
    assert_eq!(s.active_connections, 2);
    pool.release(h1).unwrap();
    pool.release(h2).unwrap();
    let s = pool.get_stats("p").unwrap();
    assert_eq!(s.active_connections, 0);
}

#[test]
fn pool_acquire_unknown_pool_errors() {
    let pool = ConnectionPool::new();
    assert!(pool.acquire("missing").is_err());
}

#[test]
fn pool_get_all_stats_returns_all() {
    let pool = ConnectionPool::new();
    pool.create_pool(sample_pool_config("p1")).unwrap();
    pool.create_pool(sample_pool_config("p2")).unwrap();
    assert_eq!(pool.get_all_stats().len(), 2);
}

#[test]
fn pool_mode_descriptions() {
    assert!(ConnectionPool::mode_description(&PoolMode::Session).contains("session"));
    assert!(ConnectionPool::mode_description(&PoolMode::Transaction).contains("transaction"));
    assert!(ConnectionPool::mode_description(&PoolMode::Statement).contains("statement"));
}

#[test]
fn pool_remove_drops_stats() {
    let pool = ConnectionPool::new();
    pool.create_pool(sample_pool_config("p")).unwrap();
    pool.remove_pool("p").unwrap();
    assert!(pool.get_stats("p").is_err());
}

// ── UserManager ──────────────────────────────────────────────────────────────

#[test]
fn user_manager_create_role_emits_sql() {
    let um = UserManager::new();
    let mut role = sample_role("alice");
    role.create_db = true;
    role.connection_limit = 5;
    let sql = um.create_role(role).unwrap();
    assert!(sql.starts_with("CREATE ROLE \"alice\""));
    assert!(sql.contains("LOGIN"));
    assert!(sql.contains("CREATEDB"));
    assert!(sql.contains("CONNECTION LIMIT 5"));
}

#[test]
fn user_manager_create_role_rejects_duplicate() {
    let um = UserManager::new();
    um.create_role(sample_role("dup")).unwrap();
    assert!(um.create_role(sample_role("dup")).is_err());
}

#[test]
fn user_manager_drop_role() {
    let um = UserManager::new();
    um.create_role(sample_role("doomed")).unwrap();
    let sql = um.drop_role("doomed").unwrap();
    assert_eq!(sql, "DROP ROLE \"doomed\";");
    assert!(um.drop_role("doomed").is_err());
}

#[test]
fn user_manager_alter_role_combines_options() {
    let um = UserManager::new();
    um.create_role(sample_role("rotate")).unwrap();
    let sql = um
        .alter_role(
            "rotate",
            RoleUpdate {
                superuser: Some(true),
                create_db: Some(true),
                create_role: None,
                login: Some(false),
                connection_limit: Some(10),
                password: Some("hunter2".into()),
            },
        )
        .unwrap();
    assert!(sql.contains("SUPERUSER"));
    assert!(sql.contains("CREATEDB"));
    assert!(sql.contains("NOLOGIN"));
    assert!(sql.contains("CONNECTION LIMIT 10"));
    assert!(sql.contains("PASSWORD 'hunter2'"));
}

#[test]
fn user_manager_grant_revoke() {
    let um = UserManager::new();
    assert_eq!(um.grant_role("admin", "alice"), "GRANT \"admin\" TO \"alice\";");
    assert_eq!(um.revoke_role("admin", "alice"), "REVOKE \"admin\" FROM \"alice\";");
}

#[test]
fn user_manager_create_user_sql_full_options() {
    let sql = UserManager::create_user_sql(
        "bob",
        "secret",
        &UserOptions {
            superuser: true,
            create_db: true,
            login: true,
            connection_limit: 3,
        },
    );
    assert!(sql.contains("CREATE USER \"bob\""));
    assert!(sql.contains("PASSWORD 'secret'"));
    assert!(sql.contains("SUPERUSER"));
    assert!(sql.contains("CONNECTION LIMIT 3"));
}

#[test]
fn user_manager_register_and_list() {
    let um = UserManager::new();
    um.register_role(sample_role("alice")).unwrap();
    um.register_role(sample_role("bob")).unwrap();
    assert_eq!(um.list_roles().len(), 2);
    assert_eq!(um.get_role("alice").unwrap().name, "alice");
    assert!(um.get_role("missing").is_none());
}

// ── Monitor ──────────────────────────────────────────────────────────────────

#[test]
fn monitor_parses_stat_activity() {
    let mut row = HashMap::new();
    row.insert("pid".into(), serde_json::json!(42));
    row.insert("datname".into(), serde_json::json!("app"));
    row.insert("state".into(), serde_json::json!("active"));
    row.insert("duration_ms".into(), serde_json::json!(1234));
    let parsed = Monitor::parse_stat_activity(&[row]);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].pid, 42);
    assert_eq!(parsed[0].datname.as_deref(), Some("app"));
    assert_eq!(parsed[0].duration_ms, Some(1234));
}

#[test]
fn monitor_parses_stat_tables() {
    let mut row = HashMap::new();
    row.insert("schemaname".into(), serde_json::json!("public"));
    row.insert("tablename".into(), serde_json::json!("users"));
    row.insert("n_dead_tup".into(), serde_json::json!(500));
    let parsed = Monitor::parse_stat_tables(&[row]);
    assert_eq!(parsed[0].tablename, "users");
    assert_eq!(parsed[0].n_dead_tup, 500);
}

#[test]
fn monitor_lag_summary_aggregates() {
    let r1 = sample_replica("a", "p", 100);
    let r2 = ReplicaInfo { lag_seconds: 2.0, ..sample_replica("b", "p", 200) };
    let s = Monitor::compute_lag_summary(&[r1, r2]);
    assert_eq!(s.replica_count, 2);
    assert_eq!(s.max_lag_bytes, 200);
    assert!((s.avg_lag_bytes - 150.0).abs() < f64::EPSILON);
    assert_eq!(s.max_lag_seconds, 2.0);
}

#[test]
fn monitor_lag_summary_empty() {
    let s = Monitor::compute_lag_summary(&[]);
    assert_eq!(s.replica_count, 0);
    assert_eq!(s.max_lag_bytes, 0);
    assert_eq!(s.max_lag_seconds, 0.0);
}

#[test]
fn monitor_count_connections_buckets() {
    let acts: Vec<PgStatActivity> = ["active", "idle", "idle in transaction", "waiting", "active"]
        .iter()
        .map(|state| PgStatActivity {
            pid: 0,
            datname: None,
            usename: None,
            application_name: None,
            client_addr: None,
            state: Some((*state).into()),
            query: None,
            wait_event_type: None,
            wait_event: None,
            query_start: None,
            duration_ms: None,
        })
        .collect();
    let c = Monitor::count_connections(&acts);
    assert_eq!(c.active, 2);
    assert_eq!(c.idle, 1);
    assert_eq!(c.idle_in_transaction, 1);
    assert_eq!(c.waiting, 1);
    assert_eq!(c.total, 5);
}

#[test]
fn monitor_finds_slow_queries() {
    let acts = vec![
        PgStatActivity {
            pid: 1,
            datname: None,
            usename: None,
            application_name: None,
            client_addr: None,
            state: None,
            query: None,
            wait_event_type: None,
            wait_event: None,
            query_start: None,
            duration_ms: Some(50),
        },
        PgStatActivity {
            pid: 2,
            datname: None,
            usename: None,
            application_name: None,
            client_addr: None,
            state: None,
            query: None,
            wait_event_type: None,
            wait_event: None,
            query_start: None,
            duration_ms: Some(2000),
        },
    ];
    assert_eq!(Monitor::find_slow_queries(&acts, 1000).len(), 1);
}

#[test]
fn monitor_metrics_text_emits_prometheus_format() {
    let acts = vec![PgStatActivity {
        pid: 1,
        datname: None,
        usename: None,
        application_name: None,
        client_addr: None,
        state: Some("active".into()),
        query: None,
        wait_event_type: None,
        wait_event: None,
        query_start: None,
        duration_ms: None,
    }];
    let tables = vec![PgStatTable {
        schemaname: "public".into(),
        tablename: "t".into(),
        seq_scan: 0,
        idx_scan: 0,
        n_live_tup: 1,
        n_dead_tup: 2,
        last_vacuum: None,
        last_analyze: None,
    }];
    let txt = Monitor::metrics_text("inst", &acts, &tables);
    assert!(txt.contains("pg_connections_active{instance=\"inst\"} 1"));
    assert!(txt.contains("pg_table_dead_tuples"));
}

// ── Pure manager functions ───────────────────────────────────────────────────

#[test]
fn manager_slow_queries_filters_by_threshold() {
    let stats = vec![sample_query_stat(50.0), sample_query_stat(500.0), sample_query_stat(1500.0)];
    assert_eq!(slow_queries(&stats, 100.0).len(), 2);
    assert_eq!(slow_queries(&stats, 1000.0).len(), 1);
    assert_eq!(slow_queries(&stats, 2000.0).len(), 0);
}

#[test]
fn manager_bloated_tables_filters() {
    let stats = vec![sample_table_stat(0.1), sample_table_stat(0.3), sample_table_stat(0.9)];
    assert_eq!(bloated_tables(&stats, 0.2).len(), 2);
    assert_eq!(bloated_tables(&stats, 0.5).len(), 1);
}

#[test]
fn manager_size_alert_records_filter_by_threshold() {
    let s = vec![sample_size(100), sample_size(2_000), sample_size(20_000)];
    assert_eq!(size_alert_records(&s, 1_000).len(), 2);
}

#[test]
fn manager_replication_healthy_thresholds() {
    let healthy = ReplicationStatus {
        id: Uuid::new_v4(),
        database_id: Uuid::new_v4(),
        role: "replica".into(),
        primary_host: Some("p".into()),
        replication_lag_bytes: 1024,
        replication_lag_seconds: 1.0,
        slots: vec![],
        is_in_recovery: true,
        updated_at: Utc::now(),
    };
    assert!(replication_healthy(&healthy));

    let unhealthy = ReplicationStatus {
        replication_lag_bytes: 200 * 1024 * 1024,
        ..healthy.clone()
    };
    assert!(!replication_healthy(&unhealthy));

    let slow = ReplicationStatus { replication_lag_seconds: 60.0, ..healthy };
    assert!(!replication_healthy(&slow));
}

#[test]
fn manager_pool_utilisation() {
    assert_eq!(pool_utilisation_pct(0, 0), 0.0);
    assert_eq!(pool_utilisation_pct(0, 10), 0.0);
    assert_eq!(pool_utilisation_pct(5, 10), 50.0);
    assert_eq!(pool_utilisation_pct(10, 10), 100.0);
}

#[test]
fn manager_needs_vacuum_logic() {
    let mut t = sample_table_stat(0.0);
    t.last_vacuum = None;
    assert!(needs_vacuum(&t, 7));
    t.last_vacuum = Some(Utc::now() - Duration::days(10));
    assert!(needs_vacuum(&t, 7));
    t.last_vacuum = Some(Utc::now() - Duration::days(1));
    assert!(!needs_vacuum(&t, 7));
}

// ── Replication slot model ───────────────────────────────────────────────────

#[test]
fn replication_slot_round_trips_via_serde() {
    let slot = ReplicationSlot {
        slot_name: "logical_repl".into(),
        plugin: "pgoutput".into(),
        active: true,
        lag_bytes: 1024,
    };
    let json = serde_json::to_string(&slot).unwrap();
    let back: ReplicationSlot = serde_json::from_str(&json).unwrap();
    assert_eq!(back.slot_name, "logical_repl");
    assert_eq!(back.lag_bytes, 1024);
    assert!(back.active);
}
