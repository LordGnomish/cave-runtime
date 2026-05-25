// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Observability — Prometheus dashboard panels + alert rules + per-zone
//! request counters.
//!
//! Panels and alert rules are declared as data structures so the runtime
//! can emit them as a Grafana dashboard JSON / Prometheus rule.yaml at
//! startup, and so unit tests can verify the close-out scorecard
//! (8 panels + 5 alerts) without scraping a live Prometheus.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ─── Panel + alert catalogue ────────────────────────────────────────────────

/// A single Grafana-style panel descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub id: u32,
    pub title: String,
    pub query: String,
    pub unit: String,
}

/// A single Prometheus alert rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRule {
    pub name: String,
    pub expression: String,
    pub for_duration: String,
    pub severity: AlertSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

/// The 8 cave-dns dashboard panels — `panels()` is the canonical accessor
/// used by both the runtime startup hook and the self-audit gate.
pub fn panels() -> Vec<Panel> {
    vec![
        Panel {
            id: 1,
            title: "DNS requests per second".into(),
            query: "sum(rate(dns_requests_total[1m]))".into(),
            unit: "ops".into(),
        },
        Panel {
            id: 2,
            title: "Request latency p95".into(),
            query: "histogram_quantile(0.95, sum by (le) (rate(dns_request_duration_seconds_bucket[5m])))".into(),
            unit: "seconds".into(),
        },
        Panel {
            id: 3,
            title: "Response code distribution".into(),
            query: "sum by (rcode) (rate(dns_responses_total[5m]))".into(),
            unit: "ops".into(),
        },
        Panel {
            id: 4,
            title: "Cache hit ratio".into(),
            query: "sum(rate(dns_cache_hits_total[5m])) / sum(rate(dns_cache_lookups_total[5m]))".into(),
            unit: "ratio".into(),
        },
        Panel {
            id: 5,
            title: "Upstream forward failures".into(),
            query: "sum by (upstream) (rate(dns_forward_failures_total[5m]))".into(),
            unit: "ops".into(),
        },
        Panel {
            id: 6,
            title: "Active TCP / DoT / DoH connections".into(),
            query: "sum by (proto) (dns_active_connections)".into(),
            unit: "connections".into(),
        },
        Panel {
            id: 7,
            title: "Zone transfer (AXFR / IXFR) per minute".into(),
            query: "sum by (kind) (rate(dns_zone_transfers_total[1m]))".into(),
            unit: "ops".into(),
        },
        Panel {
            id: 8,
            title: "DNSSEC validation verdicts".into(),
            query: "sum by (verdict) (rate(dns_dnssec_validations_total[5m]))".into(),
            unit: "ops".into(),
        },
    ]
}

/// The 5 cave-dns alert rules — production guardrails.
pub fn alerts() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "DnsHighErrorRate".into(),
            expression: "sum(rate(dns_responses_total{rcode=\"servfail\"}[5m])) / sum(rate(dns_responses_total[5m])) > 0.05".into(),
            for_duration: "10m".into(),
            severity: AlertSeverity::Critical,
        },
        AlertRule {
            name: "DnsLatencyP95High".into(),
            expression: "histogram_quantile(0.95, sum by (le) (rate(dns_request_duration_seconds_bucket[5m]))) > 0.25".into(),
            for_duration: "10m".into(),
            severity: AlertSeverity::Warning,
        },
        AlertRule {
            name: "DnsCacheHitRateLow".into(),
            expression: "sum(rate(dns_cache_hits_total[10m])) / sum(rate(dns_cache_lookups_total[10m])) < 0.5".into(),
            for_duration: "30m".into(),
            severity: AlertSeverity::Warning,
        },
        AlertRule {
            name: "DnsForwardUpstreamDown".into(),
            expression: "sum by (upstream) (rate(dns_forward_failures_total[5m])) > 10".into(),
            for_duration: "5m".into(),
            severity: AlertSeverity::Critical,
        },
        AlertRule {
            name: "DnssecBogusResponsesElevated".into(),
            expression: "sum(rate(dns_dnssec_validations_total{verdict=\"bogus\"}[15m])) > 1".into(),
            for_duration: "15m".into(),
            severity: AlertSeverity::Critical,
        },
    ]
}

// ─── Per-zone counters ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZoneMetrics {
    pub queries: u64,
    pub nxdomain: u64,
    pub servfail: u64,
    pub axfr_count: u64,
    pub ixfr_count: u64,
}

impl ZoneMetrics {
    pub fn record_query(&mut self) {
        self.queries = self.queries.saturating_add(1);
    }

    pub fn record_nxdomain(&mut self) {
        self.nxdomain = self.nxdomain.saturating_add(1);
    }

    pub fn record_servfail(&mut self) {
        self.servfail = self.servfail.saturating_add(1);
    }

    pub fn record_axfr(&mut self) {
        self.axfr_count = self.axfr_count.saturating_add(1);
    }

    pub fn record_ixfr(&mut self) {
        self.ixfr_count = self.ixfr_count.saturating_add(1);
    }

    pub fn error_rate(&self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            (self.nxdomain + self.servfail) as f64 / self.queries as f64
        }
    }
}

#[derive(Default)]
pub struct ObservabilityStore {
    by_zone: Arc<Mutex<HashMap<String, ZoneMetrics>>>,
}

impl ObservabilityStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn touch_zone(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default();
    }

    pub fn record_query(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default().record_query();
    }

    pub fn record_nxdomain(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default().record_nxdomain();
    }

    pub fn record_servfail(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default().record_servfail();
    }

    pub fn record_axfr(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default().record_axfr();
    }

    pub fn record_ixfr(&self, zone: &str) {
        let mut g = self.by_zone.lock().expect("zone metrics lock");
        g.entry(zone.to_string()).or_default().record_ixfr();
    }

    pub fn snapshot(&self, zone: &str) -> Option<ZoneMetrics> {
        let g = self.by_zone.lock().expect("zone metrics lock");
        g.get(zone).cloned()
    }

    pub fn zone_count(&self) -> usize {
        self.by_zone.lock().expect("zone metrics lock").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_dashboard_panels_present_with_unique_ids() {
        let p = panels();
        assert_eq!(p.len(), 8);
        let mut ids: Vec<u32> = p.iter().map(|x| x.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn five_alert_rules_present() {
        let a = alerts();
        assert_eq!(a.len(), 5);
        let names: Vec<&str> = a.iter().map(|x| x.name.as_str()).collect();
        for expected in [
            "DnsHighErrorRate",
            "DnsLatencyP95High",
            "DnsCacheHitRateLow",
            "DnsForwardUpstreamDown",
            "DnssecBogusResponsesElevated",
        ] {
            assert!(names.contains(&expected), "missing alert {expected}");
        }
    }

    #[test]
    fn alerts_include_critical_and_warning() {
        let a = alerts();
        let crit = a.iter().filter(|r| r.severity == AlertSeverity::Critical).count();
        let warn = a.iter().filter(|r| r.severity == AlertSeverity::Warning).count();
        assert!(crit >= 2, "expected >= 2 critical alerts, got {crit}");
        assert!(warn >= 2, "expected >= 2 warning alerts, got {warn}");
    }

    #[test]
    fn panels_all_carry_a_query() {
        for p in panels() {
            assert!(!p.query.is_empty(), "panel {} missing query", p.id);
            assert!(!p.title.is_empty(), "panel {} missing title", p.id);
        }
    }

    #[test]
    fn zone_metrics_default_zero_error_rate() {
        let z = ZoneMetrics::default();
        assert_eq!(z.error_rate(), 0.0);
    }

    #[test]
    fn zone_metrics_error_rate_excludes_success() {
        let mut z = ZoneMetrics::default();
        for _ in 0..8 {
            z.record_query();
        }
        z.record_nxdomain();
        z.record_servfail();
        assert!((z.error_rate() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn observability_store_aggregates_per_zone() {
        let s = ObservabilityStore::new();
        s.record_query("example.com.");
        s.record_query("example.com.");
        s.record_nxdomain("example.com.");
        s.record_axfr("example.com.");
        s.record_ixfr("other.com.");
        assert_eq!(s.zone_count(), 2);
        let snap = s.snapshot("example.com.").unwrap();
        assert_eq!(snap.queries, 2);
        assert_eq!(snap.nxdomain, 1);
        assert_eq!(snap.axfr_count, 1);
        let other = s.snapshot("other.com.").unwrap();
        assert_eq!(other.ixfr_count, 1);
    }

    #[test]
    fn observability_store_touch_creates_entry() {
        let s = ObservabilityStore::new();
        s.touch_zone("only.example.");
        assert_eq!(s.zone_count(), 1);
        let m = s.snapshot("only.example.").unwrap();
        assert_eq!(m.queries, 0);
    }
}
