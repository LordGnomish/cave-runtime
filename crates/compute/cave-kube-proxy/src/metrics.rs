// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Metrics — the Prometheus surface kube-proxy exposes on `:10249`.
//!
//! Cite: `pkg/proxy/metrics/metrics.go:34` (SyncProxyRulesLatency),
//! `:55` (SyncProxyRulesLastTimestampSeconds),
//! `:72` (NetworkProgrammingLatency),
//! `:91` (EndpointChangesPending),
//! `:108` (ServicesEvictedTotal).
//!
//! cave keeps the metric *names* identical so existing Grafana dashboards
//! and alertmanager rules continue to work. We don't pull in a Prometheus
//! client crate at this layer; the harness collects counters from the
//! [`KubeProxyMetrics`] struct and serializes them in cave-metrics.

use std::collections::HashMap;
use std::time::Duration;

/// Cite: `pkg/proxy/metrics/metrics.go:34` (SyncProxyRulesLatency).
#[derive(Debug, Default, Clone)]
pub struct LatencyHistogram {
    pub bucket_counts: Vec<(f64, u64)>,
    pub sum_secs: f64,
    pub count: u64,
}

impl LatencyHistogram {
    /// Standard kube-proxy buckets — `0.001s .. 60s` log-scale.
    pub fn standard_buckets() -> Vec<f64> {
        vec![
            0.001, 0.002, 0.004, 0.008, 0.016, 0.032, 0.064, 0.128, 0.256, 0.512, 1.024, 2.048,
            4.096, 8.192, 16.384, 32.768, 60.0,
        ]
    }

    pub fn new() -> Self {
        let mut h = Self::default();
        h.bucket_counts = Self::standard_buckets().into_iter().map(|b| (b, 0)).collect();
        h
    }

    pub fn observe(&mut self, d: Duration) {
        let secs = d.as_secs_f64();
        self.sum_secs += secs;
        self.count += 1;
        for (bound, count) in self.bucket_counts.iter_mut() {
            if secs <= *bound {
                *count += 1;
            }
        }
    }

    /// Cite: `pkg/proxy/metrics/metrics.go:43` — `_sum / _count` is the
    /// mean latency surfaced to the dashboard.
    pub fn mean_secs(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.sum_secs / self.count as f64)
        }
    }
}

/// Cite: `pkg/proxy/metrics/metrics.go:34..108` — five metric handles
/// the proxier increments during a sync cycle. cave keeps the same
/// names so dashboards transfer untouched.
#[derive(Debug, Default, Clone)]
pub struct KubeProxyMetrics {
    pub sync_proxy_rules_latency: LatencyHistogram,
    pub network_programming_latency: LatencyHistogram,
    /// `kubeproxy_sync_proxy_rules_last_timestamp_seconds` — unix-secs of
    /// the most recent successful sync (gauge).
    pub last_sync_timestamp_secs: u64,
    /// `kubeproxy_endpoint_changes_pending` — gauge of un-applied events.
    pub endpoint_changes_pending: u64,
    /// `kubeproxy_services_evicted_total` — counter of Services that hit
    /// the proxier's gc path (e.g. NodePort range eviction).
    pub services_evicted_total: u64,
    /// Cite: `pkg/proxy/metrics/metrics.go` — per-proxier counters tagged
    /// by `proxy_mode` label.
    pub sync_count_by_mode: HashMap<String, u64>,
}

impl KubeProxyMetrics {
    pub fn new() -> Self {
        Self {
            sync_proxy_rules_latency: LatencyHistogram::new(),
            network_programming_latency: LatencyHistogram::new(),
            ..Self::default()
        }
    }

    pub fn observe_sync(&mut self, mode: &str, duration: Duration, now_secs: u64) {
        self.sync_proxy_rules_latency.observe(duration);
        self.network_programming_latency.observe(duration);
        self.last_sync_timestamp_secs = now_secs;
        *self.sync_count_by_mode.entry(mode.to_string()).or_default() += 1;
    }

    pub fn set_endpoint_changes_pending(&mut self, n: u64) {
        self.endpoint_changes_pending = n;
    }

    pub fn inc_services_evicted(&mut self) {
        self.services_evicted_total += 1;
    }

    /// Cite: Prometheus text exposition format. Used by cave-metrics to
    /// scrape the proxier without an extra dependency at this layer.
    pub fn render_prometheus(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "# HELP kubeproxy_sync_proxy_rules_last_timestamp_seconds Last sync timestamp\n"
        ));
        s.push_str(&format!(
            "# TYPE kubeproxy_sync_proxy_rules_last_timestamp_seconds gauge\n"
        ));
        s.push_str(&format!(
            "kubeproxy_sync_proxy_rules_last_timestamp_seconds {}\n",
            self.last_sync_timestamp_secs
        ));
        s.push_str(&format!(
            "kubeproxy_endpoint_changes_pending {}\n",
            self.endpoint_changes_pending
        ));
        s.push_str(&format!(
            "kubeproxy_services_evicted_total {}\n",
            self.services_evicted_total
        ));
        if let Some(mean) = self.sync_proxy_rules_latency.mean_secs() {
            s.push_str(&format!("kubeproxy_sync_proxy_rules_latency_mean {}\n", mean));
        }
        for (mode, n) in &self.sync_count_by_mode {
            s.push_str(&format!(
                "kubeproxy_sync_total{{mode=\"{}\"}} {}\n",
                mode, n
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_observes_into_buckets() {
        let mut h = LatencyHistogram::new();
        h.observe(Duration::from_millis(50));
        assert_eq!(h.count, 1);
        // 0.05s lands in the 0.064 bucket and all larger ones
        let above = h
            .bucket_counts
            .iter()
            .filter(|(b, _)| *b >= 0.064)
            .all(|(_, c)| *c == 1);
        assert!(above);
    }

    #[test]
    fn histogram_mean_secs_after_observations() {
        let mut h = LatencyHistogram::new();
        h.observe(Duration::from_millis(100));
        h.observe(Duration::from_millis(200));
        let m = h.mean_secs().unwrap();
        assert!((m - 0.15).abs() < 1e-9);
    }

    #[test]
    fn metrics_observe_sync_increments() {
        let mut m = KubeProxyMetrics::new();
        m.observe_sync("nftables", Duration::from_millis(5), 1_700_000_000);
        assert_eq!(m.last_sync_timestamp_secs, 1_700_000_000);
        assert_eq!(m.sync_count_by_mode.get("nftables").copied(), Some(1));
    }

    #[test]
    fn metrics_eviction_counter() {
        let mut m = KubeProxyMetrics::new();
        m.inc_services_evicted();
        m.inc_services_evicted();
        assert_eq!(m.services_evicted_total, 2);
    }

    #[test]
    fn render_prometheus_emits_known_lines() {
        let mut m = KubeProxyMetrics::new();
        m.observe_sync("iptables", Duration::from_millis(7), 1_700_000_100);
        let s = m.render_prometheus();
        assert!(s.contains("kubeproxy_sync_proxy_rules_last_timestamp_seconds 1700000100"));
        assert!(s.contains("kubeproxy_sync_total{mode=\"iptables\"} 1"));
    }
}
