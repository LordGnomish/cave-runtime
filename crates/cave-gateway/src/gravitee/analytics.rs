// SPDX-License-Identifier: AGPL-3.0-or-later
//! API analytics store + query layer.
//!
//! Tracks API metrics: latency, error rates, throughput, consumer behavior.
//! In-memory ring buffer with configurable retention (default 100k entries).

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::sync::Arc;

/// Individual API metric record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMetric {
    pub api_id: String,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub latency_ms: u32,
    pub upstream_latency_ms: Option<u32>,
    pub client_ip: Option<String>,
    pub consumer_id: Option<String>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Analytics store with in-memory ring buffer.
pub struct AnalyticsStore {
    metrics: RwLock<Vec<ApiMetric>>,
    max_retention: usize,
}

impl AnalyticsStore {
    /// Create a new analytics store with default retention (100k entries).
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            metrics: RwLock::new(Vec::with_capacity(100_000)),
            max_retention: 100_000,
        })
    }

    /// Create with custom retention.
    pub fn with_retention(max_retention: usize) -> Arc<Self> {
        Arc::new(Self {
            metrics: RwLock::new(Vec::with_capacity(max_retention)),
            max_retention,
        })
    }

    /// Ingest a new metric. Enforces ring buffer size.
    pub fn ingest(&self, metric: ApiMetric) {
        let mut metrics = self.metrics.write().unwrap();
        metrics.push(metric);

        // Enforce ring buffer: drop oldest if we exceed retention
        if metrics.len() > self.max_retention {
            metrics.remove(0);
        }
    }

    /// Get percentile latency for an API.
    pub fn percentile(&self, api_id: &str, percentile: f64) -> Option<u32> {
        let metrics = self.metrics.read().unwrap();
        let mut filtered: Vec<u32> = metrics
            .iter()
            .filter(|m| m.api_id == api_id)
            .map(|m| m.latency_ms)
            .collect();

        if filtered.is_empty() {
            return None;
        }

        filtered.sort_unstable();
        let idx = ((filtered.len() as f64) * (percentile / 100.0)).ceil() as usize;
        let idx = std::cmp::min(idx.saturating_sub(1), filtered.len() - 1);
        Some(filtered[idx])
    }

    /// Get error rate (5xx + 4xx) as a percentage for an API within a time window.
    pub fn error_rate(&self, api_id: &str, window_minutes: u32) -> f64 {
        let metrics = self.metrics.read().unwrap();
        let cutoff = Utc::now() - chrono::Duration::minutes(window_minutes as i64);
        let in_window: Vec<_> = metrics
            .iter()
            .filter(|m| m.api_id == api_id && m.timestamp >= cutoff)
            .collect();

        if in_window.is_empty() {
            return 0.0;
        }

        let errors = in_window.iter().filter(|m| m.status >= 400).count();
        (errors as f64) / (in_window.len() as f64) * 100.0
    }

    /// Get throughput (requests per minute) for an API.
    pub fn throughput(&self, api_id: &str, window_minutes: u32) -> u32 {
        let metrics = self.metrics.read().unwrap();
        let cutoff = Utc::now() - chrono::Duration::minutes(window_minutes as i64);
        let in_window = metrics
            .iter()
            .filter(|m| m.api_id == api_id && m.timestamp >= cutoff)
            .count();

        if window_minutes == 0 {
            return 0;
        }
        (in_window as u32) / window_minutes
    }

    /// Get top N consumers (by request count) for an API.
    pub fn top_consumers(&self, api_id: &str, limit: usize) -> Vec<(String, u32)> {
        let metrics = self.metrics.read().unwrap();
        let mut consumer_counts: DashMap<String, u32> = DashMap::new();

        for m in metrics.iter().filter(|m| m.api_id == api_id) {
            if let Some(cid) = &m.consumer_id {
                consumer_counts
                    .entry(cid.clone())
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }

        let mut result: Vec<_> = consumer_counts
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        result.sort_by(|a, b| b.1.cmp(&a.1));
        result.into_iter().take(limit).collect()
    }

    /// Get top N paths (by request count) for an API.
    pub fn top_paths(&self, api_id: &str, limit: usize) -> Vec<(String, u32)> {
        let metrics = self.metrics.read().unwrap();
        let mut path_counts: DashMap<String, u32> = DashMap::new();

        for m in metrics.iter().filter(|m| m.api_id == api_id) {
            path_counts
                .entry(m.path.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }

        let mut result: Vec<_> = path_counts
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        result.sort_by(|a, b| b.1.cmp(&a.1));
        result.into_iter().take(limit).collect()
    }

    /// Get histogram of status codes for an API.
    pub fn status_code_histogram(&self, api_id: &str) -> Vec<(u16, u32)> {
        let metrics = self.metrics.read().unwrap();
        let mut status_counts: DashMap<u16, u32> = DashMap::new();

        for m in metrics.iter().filter(|m| m.api_id == api_id) {
            status_counts
                .entry(m.status)
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }

        let mut result: Vec<_> = status_counts
            .iter()
            .map(|entry| (*entry.key(), *entry.value()))
            .collect();
        result.sort_by_key(|a| a.0);
        result
    }

    /// Get all metrics (for debugging).
    pub fn all_metrics(&self) -> Vec<ApiMetric> {
        self.metrics.read().unwrap().clone()
    }
}

impl Default for AnalyticsStore {
    fn default() -> Self {
        AnalyticsStore {
            metrics: RwLock::new(Vec::with_capacity(100_000)),
            max_retention: 100_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analytics_ingest_and_percentile() {
        let store = AnalyticsStore::new();
        for i in 0..100 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 200,
                latency_ms: i as u32 + 1,
                upstream_latency_ms: Some(i as u32),
                client_ip: None,
                consumer_id: None,
                bytes_in: 0,
                bytes_out: 0,
            });
        }

        let p50 = store.percentile("api1", 50.0);
        assert!(p50.is_some());
        let p95 = store.percentile("api1", 95.0);
        assert!(p95.is_some());
        assert!(p95 > p50);
    }

    #[test]
    fn test_error_rate() {
        let store = AnalyticsStore::new();
        // 70 successful, 30 errors
        for i in 0..70 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 200,
                latency_ms: 10,
                upstream_latency_ms: None,
                client_ip: None,
                consumer_id: None,
                bytes_in: 0,
                bytes_out: 0,
            });
        }
        for i in 0..30 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 500,
                latency_ms: 10,
                upstream_latency_ms: None,
                client_ip: None,
                consumer_id: None,
                bytes_in: 0,
                bytes_out: 0,
            });
        }

        let rate = store.error_rate("api1", 60);
        assert!(rate > 25.0 && rate < 35.0);
    }

    #[test]
    fn test_throughput() {
        let store = AnalyticsStore::new();
        for i in 0..60 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 200,
                latency_ms: 10,
                upstream_latency_ms: None,
                client_ip: None,
                consumer_id: None,
                bytes_in: 0,
                bytes_out: 0,
            });
        }

        let tps = store.throughput("api1", 1);
        assert_eq!(tps, 60);
    }

    #[test]
    fn test_top_consumers() {
        let store = AnalyticsStore::new();
        for _ in 0..40 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 200,
                latency_ms: 10,
                upstream_latency_ms: None,
                client_ip: None,
                consumer_id: Some("consumer1".to_string()),
                bytes_in: 0,
                bytes_out: 0,
            });
        }
        for _ in 0..20 {
            store.ingest(ApiMetric {
                api_id: "api1".to_string(),
                timestamp: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                status: 200,
                latency_ms: 10,
                upstream_latency_ms: None,
                client_ip: None,
                consumer_id: Some("consumer2".to_string()),
                bytes_in: 0,
                bytes_out: 0,
            });
        }

        let top = store.top_consumers("api1", 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "consumer1");
        assert_eq!(top[0].1, 40);
        assert_eq!(top[1].0, "consumer2");
        assert_eq!(top[1].1, 20);
    }
}
