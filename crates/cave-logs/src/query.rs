// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Log query engine: filtering, aggregation (count_over_time, rate, top_k), full-text search.

use crate::models::{LogEntry, LogQuery, LogQueryOp};
use crate::LogsState;
use chrono::DateTime;
use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Result Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub entries: Vec<LogEntry>,
    pub total: usize,
    pub aggregation: Option<AggregationResult>,
}

#[derive(Debug, Serialize)]
pub struct AggregationResult {
    pub operation: String,
    pub values: Vec<AggregationValue>,
}

#[derive(Debug, Serialize)]
pub struct AggregationValue {
    pub label: String,
    pub value: f64,
    pub timestamp: Option<DateTime<Utc>>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Execute a LogQL-style query against the in-memory log store.
pub fn execute_query(state: &Arc<LogsState>, query: &LogQuery) -> QueryResult {
    let snapshot: Vec<LogEntry> = {
        let lock = state.entries.lock().unwrap();
        lock.iter().cloned().collect()
    };

    let filtered = apply_filters(&snapshot, query);
    let limit = query.limit.unwrap_or(1_000).min(10_000);

    match &query.operation {
        LogQueryOp::Filter | LogQueryOp::FullTextSearch => {
            let total = filtered.len();
            // Return newest first
            let entries: Vec<LogEntry> = filtered.into_iter().rev().take(limit).collect();
            QueryResult {
                entries,
                total,
                aggregation: None,
            }
        }
        LogQueryOp::CountOverTime => {
            let total = filtered.len();
            let agg = count_over_time(&filtered, query.step_seconds.unwrap_or(60));
            QueryResult {
                entries: vec![],
                total,
                aggregation: Some(agg),
            }
        }
        LogQueryOp::Rate => {
            let total = filtered.len();
            let agg = rate(&filtered, query.step_seconds.unwrap_or(60));
            QueryResult {
                entries: vec![],
                total,
                aggregation: Some(agg),
            }
        }
        LogQueryOp::TopK => {
            let k = query.top_k.unwrap_or(10);
            let total = filtered.len();
            let agg = top_k_services(&filtered, k);
            QueryResult {
                entries: vec![],
                total,
                aggregation: Some(agg),
            }
        }
    }
}

// ── Filtering ─────────────────────────────────────────────────────────────────

fn apply_filters<'a>(entries: &'a [LogEntry], query: &LogQuery) -> Vec<LogEntry> {
    let regex_filter: Option<Regex> = query
        .regex_filter
        .as_deref()
        .and_then(|p| Regex::new(p).ok());

    entries
        .iter()
        .filter(|e| {
            if let Some(sid) = &query.stream_id {
                if e.stream_id.as_ref() != Some(sid) {
                    return false;
                }
            }
            if let Some(level) = &query.level {
                let entry_level = format!("{:?}", e.level).to_lowercase();
                if entry_level != level.to_lowercase() {
                    return false;
                }
            }
            if let Some(svc) = &query.service {
                if !e.service.to_lowercase().contains(&svc.to_lowercase()) {
                    return false;
                }
            }
            if let Some(start) = &query.start {
                if e.timestamp < *start {
                    return false;
                }
            }
            if let Some(end) = &query.end {
                if e.timestamp > *end {
                    return false;
                }
            }
            if let Some(re) = &regex_filter {
                if !re.is_match(&e.message) {
                    return false;
                }
            }
            if let Some(text) = &query.full_text {
                if !e.message.to_lowercase().contains(&text.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

// ── Aggregations ──────────────────────────────────────────────────────────────

/// Bucket log counts by `step_seconds` intervals.
fn count_over_time(entries: &[LogEntry], step_seconds: u64) -> AggregationResult {
    let step = step_seconds as i64;
    let mut buckets: HashMap<i64, usize> = HashMap::new();
    for e in entries {
        let bucket = e.timestamp.timestamp() / step;
        *buckets.entry(bucket).or_insert(0) += 1;
    }
    let mut values: Vec<AggregationValue> = buckets
        .into_iter()
        .map(|(bucket, count)| AggregationValue {
            label: "count".to_string(),
            value: count as f64,
            timestamp: DateTime::from_timestamp(bucket * step, 0),
        })
        .collect();
    values.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    AggregationResult {
        operation: "count_over_time".to_string(),
        values,
    }
}

/// Lines-per-second rate per bucket.
fn rate(entries: &[LogEntry], step_seconds: u64) -> AggregationResult {
    let mut result = count_over_time(entries, step_seconds);
    for v in &mut result.values {
        v.value /= step_seconds as f64;
    }
    AggregationResult {
        operation: "rate".to_string(),
        values: result.values,
    }
}

/// Return the top-K services by log volume.
fn top_k_services(entries: &[LogEntry], k: usize) -> AggregationResult {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in entries {
        *counts.entry(e.service.clone()).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    let values = sorted
        .into_iter()
        .take(k)
        .map(|(svc, count)| AggregationValue {
            label: svc,
            value: count as f64,
            timestamp: None,
        })
        .collect();
    AggregationResult {
        operation: "top_k".to_string(),
        values,
    }
}
