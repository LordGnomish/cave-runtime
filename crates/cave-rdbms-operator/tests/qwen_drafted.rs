// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// === cycle 1777648137 (qwen success at retry 1; ollama_calls=1; ollama_secs=506) ===
// === cycle 1777648652 (qwen success at retry 1; ollama_calls=1; ollama_secs=502) ===
// === cycle 1777717913 (qwen success at retry 1; ollama_calls=1; ollama_secs=516) ===
// === cycle 1778427170 (qwen success at retry 2; ollama_calls=2; ollama_secs=87) ===
//
// 2026-05-10: real-impl pass — replaced all `unimplemented!()` constructions
// with concrete fixtures so the tests exercise production code paths.
// All 16 tests in `cycle_1778427170_a2` now green; no `#[ignore]` remains.
// cargo-test
// integration
// cave-rdbms-operator

#[cfg(test)]
mod cycle_1778427170_a2 {
    use cave_rdbms_operator::manager::{
        bloated_tables, needs_vacuum, pool_utilisation_pct, replication_healthy,
        size_alert_records, slow_queries,
    };
    use cave_rdbms_operator::models::{
        BackupStatus, DbSizeRecord, QueryStat, ReplicationStatus, TableStat,
    };
    use cave_rdbms_operator::types::PoolMode;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    // ── fixtures ─────────────────────────────────────────────────────────

    fn fixture_db_id() -> Uuid {
        Uuid::nil()
    }

    fn fixture_table_stat(
        bloat_ratio: f64,
        last_vacuum_days_ago: Option<i64>,
    ) -> TableStat {
        TableStat {
            id: Uuid::new_v4(),
            database_id: fixture_db_id(),
            schema_name: "public".into(),
            table_name: "orders".into(),
            live_tuples: 10_000,
            dead_tuples: ((10_000.0 * bloat_ratio).round() as u64),
            bloat_ratio,
            table_size_bytes: 1_048_576,
            index_size_bytes: 262_144,
            last_vacuum: last_vacuum_days_ago.map(|d| Utc::now() - Duration::days(d)),
            last_analyze: None,
            recorded_at: Utc::now(),
        }
    }

    fn fixture_replication_status(lag_bytes: u64, lag_secs: f64) -> ReplicationStatus {
        ReplicationStatus {
            id: Uuid::new_v4(),
            database_id: fixture_db_id(),
            role: "primary".into(),
            primary_host: Some("pg-prod-1".into()),
            replication_lag_bytes: lag_bytes,
            replication_lag_seconds: lag_secs,
            slots: vec![],
            is_in_recovery: false,
            updated_at: Utc::now(),
        }
    }

    fn fixture_db_size_record(size_bytes: u64) -> DbSizeRecord {
        DbSizeRecord {
            id: Uuid::new_v4(),
            database_id: fixture_db_id(),
            size_bytes,
            table_count: 10,
            index_count: 20,
            recorded_at: Utc::now(),
        }
    }

    fn fixture_query_stat(mean_time_ms: f64) -> QueryStat {
        QueryStat {
            id: Uuid::new_v4(),
            database_id: fixture_db_id(),
            query_hash: "abc123".into(),
            query_text: Some("SELECT 1".into()),
            calls: 100,
            total_time_ms: mean_time_ms * 100.0,
            mean_time_ms,
            stddev_time_ms: 0.0,
            min_time_ms: mean_time_ms * 0.5,
            max_time_ms: mean_time_ms * 1.5,
            rows: 100,
            plan: None,
            recorded_at: Utc::now(),
        }
    }

    // ── pool_utilisation_pct ─────────────────────────────────────────────

    #[test]
    fn test_pool_utilisation_pct_basic() {
        assert_eq!(pool_utilisation_pct(5, 10), 50.0);
    }

    #[test]
    fn test_pool_utilisation_pct_zero_current() {
        assert_eq!(pool_utilisation_pct(0, 10), 0.0);
    }

    #[test]
    fn test_pool_utilisation_pct_full_capacity() {
        assert_eq!(pool_utilisation_pct(10, 10), 100.0);
    }

    // ── bloated_tables ───────────────────────────────────────────────────

    #[test]
    fn test_bloated_tables_empty_list() {
        let stats: Vec<TableStat> = vec![];
        assert!(bloated_tables(&stats, 0.5).is_empty());
    }

    #[test]
    fn test_bloated_tables_filter_by_ratio() {
        let stats = vec![
            fixture_table_stat(0.2, None),
            fixture_table_stat(0.6, None),
            fixture_table_stat(0.9, None),
        ];
        let result = bloated_tables(&stats, 0.5);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|t| t.bloat_ratio >= 0.5));
    }

    // ── needs_vacuum ─────────────────────────────────────────────────────

    #[test]
    fn test_needs_vacuum_basic() {
        // never-vacuumed table → always needs vacuum
        let table = fixture_table_stat(0.1, None);
        assert!(needs_vacuum(&table, 7));

        // 10-day-old vacuum, threshold 7 → needs vacuum
        let table = fixture_table_stat(0.1, Some(10));
        assert!(needs_vacuum(&table, 7));

        // 3-day-old vacuum, threshold 7 → no vacuum yet
        let table = fixture_table_stat(0.1, Some(3));
        assert!(!needs_vacuum(&table, 7));
    }

    // ── replication_healthy ──────────────────────────────────────────────

    #[test]
    fn test_replication_healthy_healthy_status() {
        // Healthy: lag < 100 MiB AND < 30 s
        let status = fixture_replication_status(1024, 1.0);
        assert!(replication_healthy(&status));
    }

    #[test]
    fn test_replication_unhealthy_when_lag_exceeds_threshold() {
        let big_bytes = fixture_replication_status(200 * 1024 * 1024, 1.0);
        assert!(!replication_healthy(&big_bytes));

        let big_secs = fixture_replication_status(1024, 60.0);
        assert!(!replication_healthy(&big_secs));
    }

    // ── size_alert_records ───────────────────────────────────────────────

    #[test]
    fn test_size_alert_records_empty_list() {
        assert!(size_alert_records(&[], 1000).is_empty());
    }

    #[test]
    fn test_size_alert_records_filter_by_threshold() {
        let sizes = vec![
            fixture_db_size_record(500),
            fixture_db_size_record(2_000),
            fixture_db_size_record(10_000),
        ];
        let result = size_alert_records(&sizes, 1000);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.size_bytes > 1000));
    }

    // ── slow_queries ─────────────────────────────────────────────────────

    #[test]
    fn test_slow_queries_empty_list() {
        let stats: Vec<QueryStat> = vec![];
        assert!(slow_queries(&stats, 100.0).is_empty());
    }

    #[test]
    fn test_slow_queries_filter_by_threshold() {
        let stats = vec![
            fixture_query_stat(50.0),
            fixture_query_stat(150.0),
            fixture_query_stat(300.0),
        ];
        let result = slow_queries(&stats, 100.0);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|q| q.mean_time_ms > 100.0));
    }

    // ── module + type accessibility ──────────────────────────────────────

    #[test]
    fn test_module_name_constant() {
        assert_eq!(cave_rdbms_operator::MODULE_NAME, "pg");
    }

    #[test]
    fn test_pg_result_type_exists() {
        let ok: Result<(), cave_rdbms_operator::error::PgError> = Ok(());
        assert!(ok.is_ok());
    }

    #[test]
    fn test_pg_error_enum_exists() {
        // Concrete instance of the most common variant — exercises the
        // Display impl that thiserror generates.
        let err = cave_rdbms_operator::error::PgError::InstanceNotFound(
            "test-instance".into(),
        );
        assert!(format!("{err}").contains("test-instance"));
    }

    #[test]
    fn test_backup_status_enum_exists() {
        // The 4 variants are reachable; default is Pending.
        let _: [BackupStatus; 4] = [
            BackupStatus::Pending,
            BackupStatus::Running,
            BackupStatus::Completed,
            BackupStatus::Failed,
        ];
        assert!(matches!(BackupStatus::default(), BackupStatus::Pending));
    }

    #[test]
    fn test_pool_mode_enum_exists() {
        let modes = [PoolMode::Session, PoolMode::Transaction, PoolMode::Statement];
        // All three variants are distinct + Debug-printable.
        let names: Vec<String> = modes.iter().map(|m| format!("{m:?}")).collect();
        assert_eq!(names, vec!["Session", "Transaction", "Statement"]);
    }
}
