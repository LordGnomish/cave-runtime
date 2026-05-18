// SPDX-License-Identifier: AGPL-3.0-or-later
//! Domain logic for PostgreSQL management.
//!
//! Pure functions operating on model slices — no I/O, easily testable.

use crate::models::{DbSizeRecord, QueryStat, ReplicationStatus, TableStat};

/// Return queries whose mean execution time exceeds `threshold_ms`.
pub fn slow_queries(stats: &[QueryStat], threshold_ms: f64) -> Vec<&QueryStat> {
    stats
        .iter()
        .filter(|q| q.mean_time_ms > threshold_ms)
        .collect()
}

/// Return tables whose bloat ratio exceeds `min_ratio` (0.0–1.0).
pub fn bloated_tables(stats: &[TableStat], min_ratio: f64) -> Vec<&TableStat> {
    stats
        .iter()
        .filter(|t| t.bloat_ratio > min_ratio)
        .collect()
}

/// Return the most recent size record per database that exceeds `threshold_bytes`.
pub fn size_alert_records(sizes: &[DbSizeRecord], threshold_bytes: u64) -> Vec<&DbSizeRecord> {
    sizes
        .iter()
        .filter(|s| s.size_bytes > threshold_bytes)
        .collect()
}

/// Return `true` when replication lag is within acceptable bounds.
///
/// Thresholds: < 100 MiB and < 30 s.
pub fn replication_healthy(status: &ReplicationStatus) -> bool {
    const MAX_LAG_BYTES: u64 = 100 * 1024 * 1024; // 100 MiB
    const MAX_LAG_SECS: f64 = 30.0;
    status.replication_lag_bytes < MAX_LAG_BYTES
        && status.replication_lag_seconds < MAX_LAG_SECS
}

/// Summarise pool utilisation as a percentage of max capacity.
pub fn pool_utilisation_pct(current: u32, max: u32) -> f64 {
    if max == 0 {
        return 0.0;
    }
    (current as f64 / max as f64) * 100.0
}

/// True when a table has not been vacuumed since `stale_days` ago.
pub fn needs_vacuum(table: &TableStat, stale_days: i64) -> bool {
    match table.last_vacuum {
        None => true,
        Some(last) => {
            let age = chrono::Utc::now().signed_duration_since(last);
            age.num_days() >= stale_days
        }
    }
}
