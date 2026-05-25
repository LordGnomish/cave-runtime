// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in monitoring queries — CloudNativePG `pkg/postgres/monitoring/default_queries.go` analog.
//!
//! Upstream ships a YAML catalog of monitoring queries the operator
//! evaluates on a schedule against every instance. Each entry pairs a
//! `name` with a SQL query plus a column→Prometheus-metric mapping.
//!
//! This module ports the catalog as a compile-time table so consumers
//! (cave-monitoring, cave-metrics) can iterate without hitting a YAML
//! parser. A subset of the upstream catalog covering the high-signal
//! queries is included; full parity with every default-monitoring YAML
//! key lives in cave-monitoring scrape rules.

use std::collections::HashMap;

/// Where a column's emitted value lands in the Prometheus exposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Counter — monotonic, resets only on instance restart.
    Counter,
    /// Gauge — instantaneous value.
    Gauge,
    /// Histogram bucket — column must be paired with `le` label.
    Histogram,
    /// Label-only column — promoted to a Prometheus label, no metric.
    Label,
    /// Discarded — kept for joins / context but not exported.
    Discard,
}

/// A single column inside a monitoring query.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub name: &'static str,
    pub kind: MetricKind,
    pub description: &'static str,
}

/// A monitoring query — name, SQL, runner predicate, and column shape.
#[derive(Debug, Clone, Copy)]
pub struct MonitoringQuery {
    pub name: &'static str,
    pub query: &'static str,
    /// `RunOnPrimary` runs only on the primary. `RunOnReplica` runs only
    /// on replicas. `RunOnAll` runs everywhere. Mirrors the upstream
    /// `runs_on_server` / `master` flags.
    pub run_on: RunOn,
    pub columns: &'static [Column],
    /// Minimum Postgres major version. Queries that require pg_stat_*
    /// views introduced in newer releases are guarded here.
    pub minimum_pg_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOn {
    Primary,
    Replica,
    All,
}

impl RunOn {
    /// Whether this query should be evaluated given the instance role.
    pub fn matches(&self, is_primary: bool) -> bool {
        match self {
            RunOn::All => true,
            RunOn::Primary => is_primary,
            RunOn::Replica => !is_primary,
        }
    }
}

/// `pg_stat_archiver` — WAL archive health on the primary.
pub const PG_STAT_ARCHIVER: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_archiver",
    query: "SELECT archived_count, failed_count, \
            EXTRACT(EPOCH FROM (now() - last_archived_time))::FLOAT AS last_archived_seconds, \
            EXTRACT(EPOCH FROM (now() - last_failed_time))::FLOAT AS last_failed_seconds \
            FROM pg_stat_archiver",
    run_on: RunOn::Primary,
    columns: &[
        Column {
            name: "archived_count",
            kind: MetricKind::Counter,
            description: "Number of WAL files that have been successfully archived",
        },
        Column {
            name: "failed_count",
            kind: MetricKind::Counter,
            description: "Number of failed archive attempts",
        },
        Column {
            name: "last_archived_seconds",
            kind: MetricKind::Gauge,
            description: "Seconds since the last successful archive",
        },
        Column {
            name: "last_failed_seconds",
            kind: MetricKind::Gauge,
            description: "Seconds since the last failed archive attempt",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_stat_bgwriter` — background writer + checkpoint cadence.
pub const PG_STAT_BGWRITER: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_bgwriter",
    query: "SELECT checkpoints_timed, checkpoints_req, buffers_checkpoint, \
            buffers_clean, maxwritten_clean, buffers_backend, buffers_alloc \
            FROM pg_stat_bgwriter",
    run_on: RunOn::All,
    columns: &[
        Column {
            name: "checkpoints_timed",
            kind: MetricKind::Counter,
            description: "Scheduled checkpoint count",
        },
        Column {
            name: "checkpoints_req",
            kind: MetricKind::Counter,
            description: "Requested-by-backend checkpoint count",
        },
        Column {
            name: "buffers_checkpoint",
            kind: MetricKind::Counter,
            description: "Buffers written during checkpoints",
        },
        Column {
            name: "buffers_clean",
            kind: MetricKind::Counter,
            description: "Buffers written by the bgwriter",
        },
        Column {
            name: "maxwritten_clean",
            kind: MetricKind::Counter,
            description: "Bgwriter halt count due to maxwritten",
        },
        Column {
            name: "buffers_backend",
            kind: MetricKind::Counter,
            description: "Buffers written directly by backends",
        },
        Column {
            name: "buffers_alloc",
            kind: MetricKind::Counter,
            description: "Buffer allocation count",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_stat_database` — per-database transaction + cache statistics.
pub const PG_STAT_DATABASE: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_database",
    query: "SELECT datname, xact_commit, xact_rollback, blks_read, blks_hit, \
            tup_returned, tup_fetched, tup_inserted, tup_updated, tup_deleted, \
            conflicts, deadlocks FROM pg_stat_database WHERE datname IS NOT NULL",
    run_on: RunOn::All,
    columns: &[
        Column {
            name: "datname",
            kind: MetricKind::Label,
            description: "Database name",
        },
        Column {
            name: "xact_commit",
            kind: MetricKind::Counter,
            description: "Committed transaction count",
        },
        Column {
            name: "xact_rollback",
            kind: MetricKind::Counter,
            description: "Rolled-back transaction count",
        },
        Column {
            name: "blks_read",
            kind: MetricKind::Counter,
            description: "Disk-block reads",
        },
        Column {
            name: "blks_hit",
            kind: MetricKind::Counter,
            description: "Buffer-cache hits",
        },
        Column {
            name: "tup_returned",
            kind: MetricKind::Counter,
            description: "Rows returned to clients",
        },
        Column {
            name: "tup_fetched",
            kind: MetricKind::Counter,
            description: "Rows fetched through index scans",
        },
        Column {
            name: "tup_inserted",
            kind: MetricKind::Counter,
            description: "Rows inserted",
        },
        Column {
            name: "tup_updated",
            kind: MetricKind::Counter,
            description: "Rows updated",
        },
        Column {
            name: "tup_deleted",
            kind: MetricKind::Counter,
            description: "Rows deleted",
        },
        Column {
            name: "conflicts",
            kind: MetricKind::Counter,
            description: "Recovery conflict count",
        },
        Column {
            name: "deadlocks",
            kind: MetricKind::Counter,
            description: "Deadlock count",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_stat_replication` — primary-side per-replica lag in bytes.
pub const PG_STAT_REPLICATION: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_replication",
    query: "SELECT application_name, client_addr::text AS client_addr, state, sync_state, \
            (pg_current_wal_lsn() - sent_lsn)::FLOAT AS sent_diff_bytes, \
            (pg_current_wal_lsn() - flush_lsn)::FLOAT AS flush_diff_bytes, \
            (pg_current_wal_lsn() - replay_lsn)::FLOAT AS replay_diff_bytes \
            FROM pg_stat_replication",
    run_on: RunOn::Primary,
    columns: &[
        Column {
            name: "application_name",
            kind: MetricKind::Label,
            description: "Replica application_name",
        },
        Column {
            name: "client_addr",
            kind: MetricKind::Label,
            description: "Replica client IP",
        },
        Column {
            name: "state",
            kind: MetricKind::Label,
            description: "Walsender state",
        },
        Column {
            name: "sync_state",
            kind: MetricKind::Label,
            description: "Sync-rep state (async / sync / quorum)",
        },
        Column {
            name: "sent_diff_bytes",
            kind: MetricKind::Gauge,
            description: "Send lag in bytes",
        },
        Column {
            name: "flush_diff_bytes",
            kind: MetricKind::Gauge,
            description: "Flush lag in bytes",
        },
        Column {
            name: "replay_diff_bytes",
            kind: MetricKind::Gauge,
            description: "Replay lag in bytes",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_replication_slots` — slot retention pressure.
pub const PG_REPLICATION_SLOTS: MonitoringQuery = MonitoringQuery {
    name: "pg_replication_slots",
    query: "SELECT slot_name, slot_type, active, \
            (pg_current_wal_lsn() - confirmed_flush_lsn)::FLOAT AS confirmed_flush_diff_bytes \
            FROM pg_replication_slots",
    run_on: RunOn::Primary,
    columns: &[
        Column {
            name: "slot_name",
            kind: MetricKind::Label,
            description: "Replication slot name",
        },
        Column {
            name: "slot_type",
            kind: MetricKind::Label,
            description: "physical or logical",
        },
        Column {
            name: "active",
            kind: MetricKind::Gauge,
            description: "1 if a backend is currently using the slot",
        },
        Column {
            name: "confirmed_flush_diff_bytes",
            kind: MetricKind::Gauge,
            description: "Bytes between current WAL and slot confirmed_flush_lsn",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_locks` — exclusive-lock contention.
pub const PG_LOCKS: MonitoringQuery = MonitoringQuery {
    name: "pg_locks",
    query: "SELECT mode, COUNT(*) AS count FROM pg_locks GROUP BY mode",
    run_on: RunOn::All,
    columns: &[
        Column {
            name: "mode",
            kind: MetricKind::Label,
            description: "Lock mode",
        },
        Column {
            name: "count",
            kind: MetricKind::Gauge,
            description: "Locks held in this mode",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_stat_user_tables` — VACUUM / ANALYZE lag, slow-table contributors.
pub const PG_STAT_USER_TABLES: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_user_tables",
    query: "SELECT schemaname, relname, seq_scan, idx_scan, n_live_tup, n_dead_tup, \
            EXTRACT(EPOCH FROM (now() - last_vacuum))::FLOAT AS last_vacuum_seconds, \
            EXTRACT(EPOCH FROM (now() - last_autovacuum))::FLOAT AS last_autovacuum_seconds \
            FROM pg_stat_user_tables",
    run_on: RunOn::Primary,
    columns: &[
        Column {
            name: "schemaname",
            kind: MetricKind::Label,
            description: "Schema name",
        },
        Column {
            name: "relname",
            kind: MetricKind::Label,
            description: "Relation name",
        },
        Column {
            name: "seq_scan",
            kind: MetricKind::Counter,
            description: "Sequential scan count",
        },
        Column {
            name: "idx_scan",
            kind: MetricKind::Counter,
            description: "Index scan count",
        },
        Column {
            name: "n_live_tup",
            kind: MetricKind::Gauge,
            description: "Estimated live tuples",
        },
        Column {
            name: "n_dead_tup",
            kind: MetricKind::Gauge,
            description: "Estimated dead tuples",
        },
        Column {
            name: "last_vacuum_seconds",
            kind: MetricKind::Gauge,
            description: "Seconds since manual VACUUM",
        },
        Column {
            name: "last_autovacuum_seconds",
            kind: MetricKind::Gauge,
            description: "Seconds since autovacuum",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_stat_wal_receiver` — replica-side streaming health.
pub const PG_STAT_WAL_RECEIVER: MonitoringQuery = MonitoringQuery {
    name: "pg_stat_wal_receiver",
    query: "SELECT status, \
            EXTRACT(EPOCH FROM (now() - last_msg_send_time))::FLOAT AS last_msg_send_age, \
            EXTRACT(EPOCH FROM (now() - last_msg_receipt_time))::FLOAT AS last_msg_receipt_age \
            FROM pg_stat_wal_receiver",
    run_on: RunOn::Replica,
    columns: &[
        Column {
            name: "status",
            kind: MetricKind::Label,
            description: "WAL receiver status",
        },
        Column {
            name: "last_msg_send_age",
            kind: MetricKind::Gauge,
            description: "Seconds since the upstream last sent us a message",
        },
        Column {
            name: "last_msg_receipt_age",
            kind: MetricKind::Gauge,
            description: "Seconds since we last received a message",
        },
    ],
    minimum_pg_version: 130000,
};

/// `pg_database_size` — per-database on-disk size.
pub const PG_DATABASE_SIZE: MonitoringQuery = MonitoringQuery {
    name: "pg_database_size",
    query: "SELECT datname, pg_database_size(datname)::FLOAT AS bytes \
            FROM pg_database WHERE datistemplate = false",
    run_on: RunOn::All,
    columns: &[
        Column {
            name: "datname",
            kind: MetricKind::Label,
            description: "Database name",
        },
        Column {
            name: "bytes",
            kind: MetricKind::Gauge,
            description: "Database size in bytes",
        },
    ],
    minimum_pg_version: 130000,
};

/// The full default-query catalog. Iteration order matches CNPG.
pub const DEFAULT_QUERIES: &[MonitoringQuery] = &[
    PG_STAT_ARCHIVER,
    PG_STAT_BGWRITER,
    PG_STAT_DATABASE,
    PG_STAT_REPLICATION,
    PG_REPLICATION_SLOTS,
    PG_LOCKS,
    PG_STAT_USER_TABLES,
    PG_STAT_WAL_RECEIVER,
    PG_DATABASE_SIZE,
];

/// Look up a query by name. `None` if the name is not in the catalog.
pub fn find(name: &str) -> Option<&'static MonitoringQuery> {
    DEFAULT_QUERIES.iter().find(|q| q.name == name)
}

/// Filter the catalog by instance role. Returned slice is allocation-free
/// when no entries match (returns an empty `Vec` only).
pub fn queries_for_role(is_primary: bool) -> Vec<&'static MonitoringQuery> {
    DEFAULT_QUERIES
        .iter()
        .filter(|q| q.run_on.matches(is_primary))
        .collect()
}

/// Filter the catalog by minimum Postgres version. Used by the
/// cave-monitoring scheduler when deciding which queries to schedule for
/// a given instance.
pub fn queries_for_version(pg_version_num: u32) -> Vec<&'static MonitoringQuery> {
    DEFAULT_QUERIES
        .iter()
        .filter(|q| pg_version_num >= q.minimum_pg_version)
        .collect()
}

/// Build a name → query lookup. Useful for callers that need fast
/// repeated lookups (cave-metrics scraper does this on registration).
pub fn catalog_index() -> HashMap<&'static str, &'static MonitoringQuery> {
    DEFAULT_QUERIES.iter().map(|q| (q.name, q)).collect()
}

/// Count columns of a given kind across the whole catalog. Used by the
/// metric-registry pre-allocation path in cave-metrics.
pub fn count_columns_by_kind(kind: MetricKind) -> usize {
    DEFAULT_QUERIES
        .iter()
        .flat_map(|q| q.columns.iter())
        .filter(|c| c.kind == kind)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_high_signal_queries() {
        for required in &[
            "pg_stat_archiver",
            "pg_stat_bgwriter",
            "pg_stat_database",
            "pg_stat_replication",
            "pg_replication_slots",
            "pg_locks",
            "pg_stat_user_tables",
            "pg_stat_wal_receiver",
            "pg_database_size",
        ] {
            assert!(
                find(required).is_some(),
                "missing required query: {required}"
            );
        }
    }

    #[test]
    fn find_unknown_returns_none() {
        assert!(find("does_not_exist").is_none());
    }

    #[test]
    fn run_on_matches_role() {
        assert!(RunOn::All.matches(true));
        assert!(RunOn::All.matches(false));
        assert!(RunOn::Primary.matches(true));
        assert!(!RunOn::Primary.matches(false));
        assert!(!RunOn::Replica.matches(true));
        assert!(RunOn::Replica.matches(false));
    }

    #[test]
    fn replication_only_runs_on_primary() {
        let q = find("pg_stat_replication").unwrap();
        assert_eq!(q.run_on, RunOn::Primary);
        assert!(q.run_on.matches(true));
        assert!(!q.run_on.matches(false));
    }

    #[test]
    fn wal_receiver_only_runs_on_replica() {
        let q = find("pg_stat_wal_receiver").unwrap();
        assert_eq!(q.run_on, RunOn::Replica);
        assert!(!q.run_on.matches(true));
        assert!(q.run_on.matches(false));
    }

    #[test]
    fn queries_for_primary_excludes_wal_receiver() {
        let qs = queries_for_role(true);
        assert!(qs.iter().any(|q| q.name == "pg_stat_replication"));
        assert!(!qs.iter().any(|q| q.name == "pg_stat_wal_receiver"));
    }

    #[test]
    fn queries_for_replica_excludes_replication() {
        let qs = queries_for_role(false);
        assert!(!qs.iter().any(|q| q.name == "pg_stat_replication"));
        assert!(qs.iter().any(|q| q.name == "pg_stat_wal_receiver"));
    }

    #[test]
    fn queries_for_version_gates_too_old() {
        let qs = queries_for_version(120000);
        assert!(qs.is_empty(), "all queries declared >=13");
    }

    #[test]
    fn queries_for_version_allows_new() {
        let qs = queries_for_version(160000);
        assert_eq!(qs.len(), DEFAULT_QUERIES.len());
    }

    #[test]
    fn catalog_index_is_complete() {
        let idx = catalog_index();
        assert_eq!(idx.len(), DEFAULT_QUERIES.len());
        assert!(idx.contains_key("pg_locks"));
    }

    #[test]
    fn column_kind_counts() {
        let counter_cols = count_columns_by_kind(MetricKind::Counter);
        let gauge_cols = count_columns_by_kind(MetricKind::Gauge);
        let label_cols = count_columns_by_kind(MetricKind::Label);
        assert!(counter_cols > 0);
        assert!(gauge_cols > 0);
        assert!(label_cols > 0);
    }

    #[test]
    fn pg_locks_is_all_role() {
        assert_eq!(find("pg_locks").unwrap().run_on, RunOn::All);
    }

    #[test]
    fn pg_database_size_has_bytes_gauge() {
        let q = find("pg_database_size").unwrap();
        let bytes = q.columns.iter().find(|c| c.name == "bytes").unwrap();
        assert_eq!(bytes.kind, MetricKind::Gauge);
    }
}
