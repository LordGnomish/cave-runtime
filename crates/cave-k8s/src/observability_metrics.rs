// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus-shaped metrics emitter for the umbrella control plane.
//!
//! The cave-runtime observability ring (`cave-metrics` + `cave-logs` +
//! `cave-dashboard` + `cave-trace`) consumes these series.  cave-k8s
//! does NOT depend on `cave-metrics` directly — it exposes the
//! `MetricRegistry::scrape_text()` function returning Prometheus
//! line-protocol text, and `cave-metrics`' scraper reads it through
//! the standard `/metrics` route.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub kind: MetricKind,
    pub help: String,
    /// Sorted label sequence -> value.
    pub samples: BTreeMap<String, f64>,
}

impl Metric {
    pub fn new(name: impl Into<String>, kind: MetricKind, help: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            help: help.into(),
            samples: BTreeMap::new(),
        }
    }
    pub fn set(&mut self, labels: &[(&str, &str)], v: f64) {
        let key = encode_labels(labels);
        self.samples.insert(key, v);
    }
    pub fn add(&mut self, labels: &[(&str, &str)], v: f64) {
        let key = encode_labels(labels);
        let entry = self.samples.entry(key).or_insert(0.0);
        *entry += v;
    }
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# HELP {} {}\n", self.name, self.help));
        let kind = match self.kind {
            MetricKind::Counter => "counter",
            MetricKind::Gauge => "gauge",
            MetricKind::Histogram => "histogram",
        };
        out.push_str(&format!("# TYPE {} {}\n", self.name, kind));
        for (lbls, v) in &self.samples {
            if lbls.is_empty() {
                out.push_str(&format!("{} {}\n", self.name, v));
            } else {
                out.push_str(&format!("{}{{{}}} {}\n", self.name, lbls, v));
            }
        }
        out
    }
}

fn encode_labels(labels: &[(&str, &str)]) -> String {
    let mut v: Vec<(&str, &str)> = labels.to_vec();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v.into_iter()
        .map(|(k, val)| format!("{}=\"{}\"", k, val.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(",")
}

pub struct MetricRegistry {
    inner: RwLock<BTreeMap<String, Metric>>,
}

impl Default for MetricRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricRegistry {
    pub fn new() -> Self {
        let mut m = BTreeMap::new();
        m.insert(
            "cave_k8s_pod_count".into(),
            Metric::new("cave_k8s_pod_count", MetricKind::Gauge, "Pods by namespace + phase"),
        );
        m.insert(
            "cave_k8s_apiserver_request_total".into(),
            Metric::new(
                "cave_k8s_apiserver_request_total",
                MetricKind::Counter,
                "Apiserver request count by verb + resource",
            ),
        );
        m.insert(
            "cave_k8s_scheduler_pending_pods".into(),
            Metric::new(
                "cave_k8s_scheduler_pending_pods",
                MetricKind::Gauge,
                "Pods awaiting placement",
            ),
        );
        m.insert(
            "cave_k8s_etcd_lag_seconds".into(),
            Metric::new(
                "cave_k8s_etcd_lag_seconds",
                MetricKind::Gauge,
                "etcd Raft commit-to-apply lag",
            ),
        );
        m.insert(
            "cave_k8s_node_ready".into(),
            Metric::new(
                "cave_k8s_node_ready",
                MetricKind::Gauge,
                "1 if node is Ready, 0 otherwise",
            ),
        );
        m.insert(
            "cave_k8s_admission_denied_total".into(),
            Metric::new(
                "cave_k8s_admission_denied_total",
                MetricKind::Counter,
                "Admission rejections by plugin",
            ),
        );
        m.insert(
            "cave_k8s_quota_exceeded_total".into(),
            Metric::new(
                "cave_k8s_quota_exceeded_total",
                MetricKind::Counter,
                "ResourceQuota rejection count by quota",
            ),
        );
        Self {
            inner: RwLock::new(m),
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().expect("metrics").keys().cloned().collect()
    }

    pub fn set_gauge(&self, name: &str, labels: &[(&str, &str)], v: f64) -> bool {
        let mut g = self.inner.write().expect("metrics");
        let Some(m) = g.get_mut(name) else {
            return false;
        };
        if m.kind != MetricKind::Gauge {
            return false;
        }
        m.set(labels, v);
        true
    }

    pub fn inc_counter(&self, name: &str, labels: &[(&str, &str)], v: f64) -> bool {
        let mut g = self.inner.write().expect("metrics");
        let Some(m) = g.get_mut(name) else {
            return false;
        };
        if m.kind != MetricKind::Counter {
            return false;
        }
        m.add(labels, v);
        true
    }

    pub fn scrape_text(&self) -> String {
        let g = self.inner.read().expect("metrics");
        let mut out = String::new();
        for m in g.values() {
            out.push_str(&m.render());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_seeds_seven_metrics() {
        let r = MetricRegistry::new();
        assert_eq!(r.names().len(), 7);
        assert!(r.names().iter().any(|n| n == "cave_k8s_pod_count"));
    }

    #[test]
    fn gauge_set_and_render() {
        let r = MetricRegistry::new();
        assert!(r.set_gauge("cave_k8s_pod_count", &[("namespace", "default"), ("phase", "Running")], 3.0));
        let s = r.scrape_text();
        assert!(s.contains("cave_k8s_pod_count"));
        assert!(s.contains("namespace=\"default\""));
        assert!(s.contains("phase=\"Running\""));
        assert!(s.contains(" 3\n"));
    }

    #[test]
    fn counter_inc_accumulates() {
        let r = MetricRegistry::new();
        r.inc_counter("cave_k8s_admission_denied_total", &[("plugin", "PodSecurity")], 1.0);
        r.inc_counter("cave_k8s_admission_denied_total", &[("plugin", "PodSecurity")], 2.0);
        let s = r.scrape_text();
        assert!(s.contains("cave_k8s_admission_denied_total{plugin=\"PodSecurity\"} 3"));
    }

    #[test]
    fn wrong_kind_rejected() {
        let r = MetricRegistry::new();
        assert!(!r.inc_counter("cave_k8s_pod_count", &[], 1.0));
        assert!(!r.set_gauge("cave_k8s_apiserver_request_total", &[], 1.0));
    }

    #[test]
    fn unknown_metric_rejected() {
        let r = MetricRegistry::new();
        assert!(!r.set_gauge("not_a_metric", &[], 1.0));
        assert!(!r.inc_counter("not_a_metric", &[], 1.0));
    }

    #[test]
    fn render_includes_help_and_type() {
        let mut m = Metric::new("x", MetricKind::Gauge, "test help");
        m.set(&[("foo", "bar")], 1.0);
        let s = m.render();
        assert!(s.contains("# HELP x test help"));
        assert!(s.contains("# TYPE x gauge"));
    }

    #[test]
    fn label_encoding_escapes_quotes() {
        let mut m = Metric::new("x", MetricKind::Gauge, "h");
        m.set(&[("k", "v\"w")], 1.0);
        let s = m.render();
        assert!(s.contains("v\\\"w"));
    }
}
