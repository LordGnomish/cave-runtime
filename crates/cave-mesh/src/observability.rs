//! Per-service latency histograms and golden signals.
//!
//! Uses a standalone `ObservabilityStore` (not MeshMetrics) so it can track
//! per-service-UUID metrics independently of the Prometheus export pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

// ─── Core storage types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServiceMetrics {
    pub service_id: Uuid,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub latency_sum_ms: u64,
    pub latency_buckets: LatencyBuckets,
    pub last_updated: Option<DateTime<Utc>>,
}

impl ServiceMetrics {
    pub fn new(service_id: Uuid) -> Self {
        Self { service_id, ..Default::default() }
    }

    pub fn record(&mut self, latency_ms: u64, success: bool) {
        self.total_requests += 1;
        self.latency_sum_ms = self.latency_sum_ms.saturating_add(latency_ms);
        self.last_updated = Some(Utc::now());
        if success {
            self.successful_requests += 1;
        } else {
            self.failed_requests += 1;
        }
        self.latency_buckets.record(latency_ms);
    }

    pub fn avg_latency_ms(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.latency_sum_ms as f64 / self.total_requests as f64
        }
    }
}

/// Prometheus-style cumulative histogram with fixed buckets (ms).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LatencyBuckets {
    pub le_1ms: u64,
    pub le_5ms: u64,
    pub le_10ms: u64,
    pub le_25ms: u64,
    pub le_50ms: u64,
    pub le_100ms: u64,
    pub le_250ms: u64,
    pub le_500ms: u64,
    pub le_1000ms: u64,
    /// +Inf bucket = total count.
    pub le_inf: u64,
}

impl LatencyBuckets {
    pub fn record(&mut self, latency_ms: u64) {
        self.le_inf += 1;
        if latency_ms <= 1 {
            self.le_1ms += 1;
        }
        if latency_ms <= 5 {
            self.le_5ms += 1;
        }
        if latency_ms <= 10 {
            self.le_10ms += 1;
        }
        if latency_ms <= 25 {
            self.le_25ms += 1;
        }
        if latency_ms <= 50 {
            self.le_50ms += 1;
        }
        if latency_ms <= 100 {
            self.le_100ms += 1;
        }
        if latency_ms <= 250 {
            self.le_250ms += 1;
        }
        if latency_ms <= 500 {
            self.le_500ms += 1;
        }
        if latency_ms <= 1000 {
            self.le_1000ms += 1;
        }
    }
}

// ─── Response types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetricsResponse {
    pub service_id: Uuid,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub avg_latency_ms: f64,
    pub last_updated: Option<DateTime<Utc>>,
}

/// The four golden signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenSignals {
    pub service_id: Uuid,
    pub traffic_total: u64,
    pub latency_avg_ms: f64,
    pub latency_p99_ms: f64,
    pub error_rate: f64,
    pub saturation: f64,
    pub snapshot_at: DateTime<Utc>,
}

// ─── ObservabilityStore ───────────────────────────────────────────────────────

/// Thread-safe store for per-service latency / golden-signal metrics.
#[derive(Clone)]
pub struct ObservabilityStore {
    inner: Arc<Mutex<HashMap<Uuid, ServiceMetrics>>>,
}

impl Default for ObservabilityStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservabilityStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn record_request(&self, service_id: Uuid, latency_ms: u64, success: bool) {
        let mut map = self.inner.lock().unwrap();
        map.entry(service_id)
            .or_insert_with(|| ServiceMetrics::new(service_id))
            .record(latency_ms, success);
    }

    pub fn request_metrics(&self, service_id: Uuid) -> Option<RequestMetricsResponse> {
        let map = self.inner.lock().unwrap();
        map.get(&service_id).map(|m| RequestMetricsResponse {
            service_id,
            total_requests: m.total_requests,
            successful_requests: m.successful_requests,
            failed_requests: m.failed_requests,
            avg_latency_ms: m.avg_latency_ms(),
            last_updated: m.last_updated,
        })
    }

    pub fn latency_histogram(&self, service_id: Uuid) -> Option<LatencyBuckets> {
        let map = self.inner.lock().unwrap();
        map.get(&service_id).map(|m| m.latency_buckets.clone())
    }

    pub fn error_rate(&self, service_id: Uuid) -> f64 {
        let map = self.inner.lock().unwrap();
        match map.get(&service_id) {
            None => 0.0,
            Some(m) if m.total_requests == 0 => 0.0,
            Some(m) => m.failed_requests as f64 / m.total_requests as f64,
        }
    }

    pub fn golden_signals(&self, service_id: Uuid) -> GoldenSignals {
        let (total, failed, avg_lat, p99) = {
            let map = self.inner.lock().unwrap();
            match map.get(&service_id) {
                None => (0u64, 0u64, 0.0f64, 0.0f64),
                Some(m) => {
                    let p99 = estimate_p99(&m.latency_buckets, m.total_requests);
                    (m.total_requests, m.failed_requests, m.avg_latency_ms(), p99)
                }
            }
        };

        let err_rate = if total == 0 { 0.0 } else { failed as f64 / total as f64 };
        GoldenSignals {
            service_id,
            traffic_total: total,
            latency_avg_ms: avg_lat,
            latency_p99_ms: p99,
            error_rate: err_rate,
            saturation: err_rate,
            snapshot_at: Utc::now(),
        }
    }

    /// List all service IDs that have metrics.
    pub fn all_service_ids(&self) -> Vec<Uuid> {
        self.inner.lock().unwrap().keys().cloned().collect()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn estimate_p99(buckets: &LatencyBuckets, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let p99_count = ((total as f64) * 0.99) as u64;
    if buckets.le_1ms >= p99_count {
        return 1.0;
    }
    if buckets.le_5ms >= p99_count {
        return 5.0;
    }
    if buckets.le_10ms >= p99_count {
        return 10.0;
    }
    if buckets.le_25ms >= p99_count {
        return 25.0;
    }
    if buckets.le_50ms >= p99_count {
        return 50.0;
    }
    if buckets.le_100ms >= p99_count {
        return 100.0;
    }
    if buckets.le_250ms >= p99_count {
        return 250.0;
    }
    if buckets.le_500ms >= p99_count {
        return 500.0;
    }
    if buckets.le_1000ms >= p99_count {
        return 1000.0;
    }
    5000.0
}
