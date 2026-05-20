// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Active Connection Tracking (ACT) — per-zone, per-service connection
//! counters.
//!
//! Mirrors `pkg/act/act.go`. Cilium's ACT subsystem reads a BPF map of
//! per-(service,zone) counters at a fixed interval and emits Prometheus
//! gauges (new / active / failed) plus a processing-time histogram and
//! an error counter.
//!
//! We port:
//!   * the constants the upstream agent uses for cadence and series
//!     limits — same numeric values
//!   * the metric names for the four collectors
//!   * an in-memory aggregator that takes successive samples and reports
//!     the deltas (matching upstream's "subtract previous read" approach)

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;
use std::time::Duration;

/// Time between ACT map reads. Mirrors `metricsUpdateInterval` upstream.
pub const METRICS_UPDATE_INTERVAL: Duration = Duration::from_secs(15);
/// When a stale entry is dropped from the metrics endpoint.
/// Mirrors `metricsTimeout` upstream.
pub const METRICS_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Soft limit at which metric series are deleted more aggressively.
pub const METRICS_COUNT_SOFT_LIMIT: usize = 300;
/// Hard limit at which new metric series stop being allocated.
pub const METRICS_COUNT_HARD_LIMIT: usize = 500;

/// Metric names emitted under subsystem `"act"`. Each matches a
/// `Name: "..."` in `ActiveConnectionTrackingMetrics` upstream.
pub mod metric_name {
    pub const NEW_CONNECTIONS_TOTAL: &str = "new_connections_total";
    pub const ACTIVE_CONNECTIONS_TOTAL: &str = "active_connections_total";
    pub const FAILED_CONNECTIONS_TOTAL: &str = "failed_connections_total";
    pub const PROCESSING_TIME_SECONDS: &str = "processing_time_seconds";
    pub const ERRORS: &str = "errors";
}

/// One ACT key — zone+service pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneServiceKey {
    pub zone: String,
    pub service: String,
}

/// Counters reported by the BPF map for a key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub opened: u64,
    pub closed: u64,
    pub failed: u64,
}

/// Aggregator that converts successive raw counter reads into deltas.
/// Mirrors the upstream "subtract previous read" approach.
#[derive(Debug, Default)]
pub struct Aggregator {
    last: BTreeMap<ZoneServiceKey, Counters>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deltas {
    pub new_conn: u64,
    pub closed: u64,
    pub failed: u64,
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw read for `key`, return deltas vs the previous read.
    /// First sample produces zeros (upstream behaviour: no baseline yet).
    pub fn observe(&mut self, key: ZoneServiceKey, current: Counters) -> Deltas {
        let prev = self.last.get(&key).copied().unwrap_or_default();
        let deltas = if self.last.contains_key(&key) {
            Deltas {
                new_conn: current.opened.saturating_sub(prev.opened),
                closed: current.closed.saturating_sub(prev.closed),
                failed: current.failed.saturating_sub(prev.failed),
            }
        } else {
            Deltas {
                new_conn: 0,
                closed: 0,
                failed: 0,
            }
        };
        self.last.insert(key, current);
        deltas
    }

    pub fn known_keys(&self) -> usize {
        self.last.len()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ActError {
    #[error("act series limit exceeded ({0} >= {1})")]
    SeriesLimit(usize, usize),
    #[error("tenant {tenant} cannot mutate ACT aggregator owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/act/act.go", "ACT");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn metrics_update_interval_is_fifteen_seconds() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Cadence.Read", "tenant-act-ui");
        assert_eq!(METRICS_UPDATE_INTERVAL, Duration::from_secs(15));
    }

    #[test]
    fn metrics_timeout_is_ten_minutes() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Cadence.Timeout", "tenant-act-mt");
        assert_eq!(METRICS_TIMEOUT, Duration::from_secs(600));
    }

    #[test]
    fn series_soft_and_hard_limits_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Limits", "tenant-act-lim");
        assert_eq!(METRICS_COUNT_SOFT_LIMIT, 300);
        assert_eq!(METRICS_COUNT_HARD_LIMIT, 500);
        assert!(METRICS_COUNT_SOFT_LIMIT < METRICS_COUNT_HARD_LIMIT);
    }

    #[test]
    fn metric_names_match_upstream_strings() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "MetricNames", "tenant-act-mn");
        assert_eq!(metric_name::NEW_CONNECTIONS_TOTAL, "new_connections_total");
        assert_eq!(
            metric_name::ACTIVE_CONNECTIONS_TOTAL,
            "active_connections_total"
        );
        assert_eq!(
            metric_name::FAILED_CONNECTIONS_TOTAL,
            "failed_connections_total"
        );
        assert_eq!(
            metric_name::PROCESSING_TIME_SECONDS,
            "processing_time_seconds"
        );
        assert_eq!(metric_name::ERRORS, "errors");
    }

    #[test]
    fn aggregator_first_sample_returns_zero_deltas() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Aggr.First", "tenant-act-af");
        let mut a = Aggregator::new();
        let d = a.observe(
            ZoneServiceKey {
                zone: "us-east-1a".into(),
                service: "10.0.0.1:80".into(),
            },
            Counters {
                opened: 100,
                closed: 50,
                failed: 1,
            },
        );
        assert_eq!(
            d,
            Deltas {
                new_conn: 0,
                closed: 0,
                failed: 0
            }
        );
    }

    #[test]
    fn aggregator_second_sample_emits_deltas() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Aggr.Delta", "tenant-act-ad");
        let mut a = Aggregator::new();
        let k = ZoneServiceKey {
            zone: "us-east-1a".into(),
            service: "10.0.0.1:80".into(),
        };
        let _ = a.observe(
            k.clone(),
            Counters {
                opened: 100,
                closed: 50,
                failed: 1,
            },
        );
        let d = a.observe(
            k,
            Counters {
                opened: 150,
                closed: 70,
                failed: 3,
            },
        );
        assert_eq!(
            d,
            Deltas {
                new_conn: 50,
                closed: 20,
                failed: 2
            }
        );
    }

    #[test]
    fn aggregator_handles_counter_reset_with_saturation() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Aggr.Reset", "tenant-act-ar");
        let mut a = Aggregator::new();
        let k = ZoneServiceKey {
            zone: "z".into(),
            service: "s".into(),
        };
        let _ = a.observe(
            k.clone(),
            Counters {
                opened: 1000,
                closed: 0,
                failed: 0,
            },
        );
        let d = a.observe(
            k,
            Counters {
                opened: 5,
                closed: 0,
                failed: 0,
            },
        );
        // Saturating-sub returns 0 instead of overflow.
        assert_eq!(d.new_conn, 0);
    }

    #[test]
    fn aggregator_tracks_distinct_keys_separately() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Aggr.MultiKey", "tenant-act-mk");
        let mut a = Aggregator::new();
        let k1 = ZoneServiceKey {
            zone: "a".into(),
            service: "s1".into(),
        };
        let k2 = ZoneServiceKey {
            zone: "a".into(),
            service: "s2".into(),
        };
        let _ = a.observe(
            k1.clone(),
            Counters {
                opened: 10,
                closed: 0,
                failed: 0,
            },
        );
        let _ = a.observe(
            k2,
            Counters {
                opened: 20,
                closed: 0,
                failed: 0,
            },
        );
        assert_eq!(a.known_keys(), 2);
        let d = a.observe(
            k1,
            Counters {
                opened: 12,
                closed: 0,
                failed: 0,
            },
        );
        assert_eq!(d.new_conn, 2);
    }

    #[test]
    fn act_error_variants_render() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Errors", "tenant-act-err");
        let e = ActError::SeriesLimit(501, 500);
        assert!(format!("{}", e).contains("501"));
        let e = ActError::TenantDenied {
            tenant: TenantId::new("t-x").expect("test fixture"),
        };
        assert!(format!("{}", e).contains("t-x"));
    }

    #[test]
    fn counters_default_is_zeros() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Counters.Default", "tenant-act-cd");
        let c = Counters::default();
        assert_eq!(c.opened, 0);
        assert_eq!(c.closed, 0);
        assert_eq!(c.failed, 0);
    }

    #[test]
    fn key_ordering_is_stable() {
        let (_c, _t) = cilium_test_ctx!("pkg/act/act.go", "Key.Ord", "tenant-act-ko");
        let a = ZoneServiceKey {
            zone: "us-east-1a".into(),
            service: "s1".into(),
        };
        let b = ZoneServiceKey {
            zone: "us-east-1b".into(),
            service: "s1".into(),
        };
        assert!(a < b);
    }
}
