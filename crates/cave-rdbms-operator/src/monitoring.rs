// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::types::{PgStatActivity, PgStatTable, ReplicaInfo};
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LagSummary {
    pub replica_count: usize,
    pub max_lag_bytes: i64,
    pub avg_lag_bytes: f64,
    pub max_lag_seconds: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionCounts {
    pub active: usize,
    pub idle: usize,
    pub idle_in_transaction: usize,
    pub waiting: usize,
    pub total: usize,
}

pub struct Monitor;

impl Monitor {
    /// Parse pg_stat_activity rows from a generic map representation.
    pub fn parse_stat_activity(
        rows: &[HashMap<String, serde_json::Value>],
    ) -> Vec<PgStatActivity> {
        rows.iter()
            .map(|row| PgStatActivity {
                pid: row
                    .get("pid")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32,
                datname: row
                    .get("datname")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                usename: row
                    .get("usename")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                application_name: row
                    .get("application_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                client_addr: row
                    .get("client_addr")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                state: row
                    .get("state")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                query: row
                    .get("query")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                wait_event_type: row
                    .get("wait_event_type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                wait_event: row
                    .get("wait_event")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                query_start: None,
                duration_ms: row
                    .get("duration_ms")
                    .and_then(|v| v.as_i64()),
            })
            .collect()
    }

    /// Parse pg_stat_user_tables rows.
    pub fn parse_stat_tables(
        rows: &[HashMap<String, serde_json::Value>],
    ) -> Vec<PgStatTable> {
        rows.iter()
            .map(|row| PgStatTable {
                schemaname: row
                    .get("schemaname")
                    .and_then(|v| v.as_str())
                    .unwrap_or("public")
                    .to_string(),
                tablename: row
                    .get("tablename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                seq_scan: row.get("seq_scan").and_then(|v| v.as_i64()).unwrap_or(0),
                idx_scan: row.get("idx_scan").and_then(|v| v.as_i64()).unwrap_or(0),
                n_live_tup: row.get("n_live_tup").and_then(|v| v.as_i64()).unwrap_or(0),
                n_dead_tup: row.get("n_dead_tup").and_then(|v| v.as_i64()).unwrap_or(0),
                last_vacuum: None,
                last_analyze: None,
            })
            .collect()
    }

    pub fn compute_lag_summary(replicas: &[ReplicaInfo]) -> LagSummary {
        let replica_count = replicas.len();
        if replica_count == 0 {
            return LagSummary {
                replica_count: 0,
                max_lag_bytes: 0,
                avg_lag_bytes: 0.0,
                max_lag_seconds: 0.0,
            };
        }

        let max_lag_bytes = replicas.iter().map(|r| r.lag_bytes).max().unwrap_or(0);
        let avg_lag_bytes =
            replicas.iter().map(|r| r.lag_bytes as f64).sum::<f64>() / replica_count as f64;
        let max_lag_seconds = replicas
            .iter()
            .map(|r| r.lag_seconds)
            .fold(0.0f64, f64::max);

        LagSummary {
            replica_count,
            max_lag_bytes,
            avg_lag_bytes,
            max_lag_seconds,
        }
    }

    pub fn count_connections(activities: &[PgStatActivity]) -> ConnectionCounts {
        let mut active = 0usize;
        let mut idle = 0usize;
        let mut idle_in_transaction = 0usize;
        let mut waiting = 0usize;

        for a in activities {
            match a.state.as_deref() {
                Some("active") => active += 1,
                Some("idle") => idle += 1,
                Some("idle in transaction") => idle_in_transaction += 1,
                Some("waiting") => waiting += 1,
                _ => {}
            }
        }

        ConnectionCounts {
            active,
            idle,
            idle_in_transaction,
            waiting,
            total: activities.len(),
        }
    }

    pub fn find_slow_queries<'a>(
        activities: &'a [PgStatActivity],
        threshold_ms: i64,
    ) -> Vec<&'a PgStatActivity> {
        activities
            .iter()
            .filter(|a| a.duration_ms.map_or(false, |d| d > threshold_ms))
            .collect()
    }

    pub fn find_bloated_tables<'a>(
        tables: &'a [PgStatTable],
        dead_tup_threshold: i64,
    ) -> Vec<&'a PgStatTable> {
        tables
            .iter()
            .filter(|t| t.n_dead_tup > dead_tup_threshold)
            .collect()
    }

    pub fn metrics_text(
        instance_id: &str,
        activities: &[PgStatActivity],
        tables: &[PgStatTable],
    ) -> String {
        let counts = Self::count_connections(activities);
        let mut lines = vec![
            format!(
                "# HELP pg_connections Total connections for instance {}",
                instance_id
            ),
            format!("# TYPE pg_connections gauge"),
            format!("pg_connections_active{{instance=\"{}\"}} {}", instance_id, counts.active),
            format!("pg_connections_idle{{instance=\"{}\"}} {}", instance_id, counts.idle),
            format!(
                "pg_connections_idle_in_transaction{{instance=\"{}\"}} {}",
                instance_id, counts.idle_in_transaction
            ),
            format!("pg_connections_total{{instance=\"{}\"}} {}", instance_id, counts.total),
        ];

        for table in tables {
            lines.push(format!(
                "pg_table_dead_tuples{{instance=\"{}\",schema=\"{}\",table=\"{}\"}} {}",
                instance_id, table.schemaname, table.tablename, table.n_dead_tup
            ));
            lines.push(format!(
                "pg_table_live_tuples{{instance=\"{}\",schema=\"{}\",table=\"{}\"}} {}",
                instance_id, table.schemaname, table.tablename, table.n_live_tup
            ));
        }

        lines.join("\n")
    }
}
