// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus metrics. The numeric counters here are exposed by the
//! production runtime via `prometheus-client`; this module owns the names.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub requests_failed_total: AtomicU64,
    pub upstream_5xx_total: AtomicU64,
    pub upstream_4xx_total: AtomicU64,
    pub upstream_2xx_total: AtomicU64,
    pub rate_limited_total: AtomicU64,
    pub circuit_open_total: AtomicU64,
    pub retries_total: AtomicU64,
    pub cache_hit_total: AtomicU64,
    pub cache_miss_total: AtomicU64,
    pub auth_failed_total: AtomicU64,
    pub latency_sum_ms: AtomicU64,
    pub active_connections: AtomicU64,
}
impl Metrics {
    pub fn new() -> Self { Self::default() }
    pub fn inc_requests(&self) { self.requests_total.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_failed(&self) { self.requests_failed_total.fetch_add(1, Ordering::Relaxed); }
    pub fn observe_status(&self, status: u16) {
        if (500..600).contains(&status) { self.upstream_5xx_total.fetch_add(1, Ordering::Relaxed); }
        else if (400..500).contains(&status) { self.upstream_4xx_total.fetch_add(1, Ordering::Relaxed); }
        else if (200..300).contains(&status) { self.upstream_2xx_total.fetch_add(1, Ordering::Relaxed); }
    }
    pub fn observe_latency_ms(&self, ms: u64) { self.latency_sum_ms.fetch_add(ms, Ordering::Relaxed); }
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            requests_failed_total: self.requests_failed_total.load(Ordering::Relaxed),
            upstream_5xx_total: self.upstream_5xx_total.load(Ordering::Relaxed),
            upstream_4xx_total: self.upstream_4xx_total.load(Ordering::Relaxed),
            upstream_2xx_total: self.upstream_2xx_total.load(Ordering::Relaxed),
            rate_limited_total: self.rate_limited_total.load(Ordering::Relaxed),
            circuit_open_total: self.circuit_open_total.load(Ordering::Relaxed),
            retries_total: self.retries_total.load(Ordering::Relaxed),
            cache_hit_total: self.cache_hit_total.load(Ordering::Relaxed),
            cache_miss_total: self.cache_miss_total.load(Ordering::Relaxed),
            auth_failed_total: self.auth_failed_total.load(Ordering::Relaxed),
            latency_sum_ms: self.latency_sum_ms.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub requests_total: u64, pub requests_failed_total: u64,
    pub upstream_5xx_total: u64, pub upstream_4xx_total: u64, pub upstream_2xx_total: u64,
    pub rate_limited_total: u64, pub circuit_open_total: u64,
    pub retries_total: u64, pub cache_hit_total: u64, pub cache_miss_total: u64,
    pub auth_failed_total: u64, pub latency_sum_ms: u64, pub active_connections: u64,
}

/// The 10 Grafana panels per Charter v2 obs spec.
pub const PROMETHEUS_PANELS: &[&str] = &[
    "apigw_requests_total{route,service}",
    "apigw_requests_failed_total{route}",
    "apigw_upstream_5xx_total{route,upstream}",
    "apigw_upstream_2xx_total{route,upstream}",
    "apigw_latency_sum_ms{route}",
    "apigw_rate_limited_total{consumer}",
    "apigw_circuit_open_total{service}",
    "apigw_retries_total{service}",
    "apigw_cache_hit_total{route}",
    "apigw_auth_failed_total{plugin}",
];

/// 6 alert rules.
pub const ALERT_RULES: &[(&str, &str)] = &[
    ("apigw_5xx_rate_high", "sum(rate(apigw_upstream_5xx_total[5m])) by (route) > 0.05"),
    ("apigw_latency_p99_high", "histogram_quantile(0.99, sum(rate(apigw_latency_sum_ms[5m])) by (le)) > 1000"),
    ("apigw_rate_limit_floods", "rate(apigw_rate_limited_total[1m]) > 100"),
    ("apigw_circuit_open", "apigw_circuit_open_total > 0"),
    ("apigw_auth_failures_spike", "rate(apigw_auth_failed_total[5m]) > 10"),
    ("apigw_cache_miss_ratio_high", "rate(apigw_cache_miss_total[5m]) / (rate(apigw_cache_hit_total[5m]) + rate(apigw_cache_miss_total[5m])) > 0.95"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn counters_increment() {
        let m = Metrics::new();
        m.inc_requests(); m.inc_failed();
        let s = m.snapshot();
        assert_eq!(s.requests_total, 1); assert_eq!(s.requests_failed_total, 1);
    }
    #[test] fn observe_status_buckets() {
        let m = Metrics::new();
        m.observe_status(200); m.observe_status(404); m.observe_status(503);
        let s = m.snapshot();
        assert_eq!(s.upstream_2xx_total, 1);
        assert_eq!(s.upstream_4xx_total, 1);
        assert_eq!(s.upstream_5xx_total, 1);
    }
    #[test] fn ten_panels_present() { assert_eq!(PROMETHEUS_PANELS.len(), 10); }
    #[test] fn six_alerts_present() { assert_eq!(ALERT_RULES.len(), 6); }
    #[test] fn latency_accumulates() {
        let m = Metrics::new();
        m.observe_latency_ms(50); m.observe_latency_ms(100);
        assert_eq!(m.snapshot().latency_sum_ms, 150);
    }
}
