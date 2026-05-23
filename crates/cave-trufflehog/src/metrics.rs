// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Observability — port of the Prometheus surface in upstream
//! `pkg/engine/metrics.go` + `pkg/verificationcache/metrics_reporter.go`.
//!
//! Six panels exposed: chunks_processed, findings_emitted, verified_findings,
//! verification_latency_ms, verification_cache_hit_rate, scan_duration_seconds.
//! Four alert rules: ZeroFindingsScan, HighVerificationFailure,
//! VerificationCacheStarved, LongRunningScan.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub unit: &'static str,
    pub promql: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertSpec {
    pub id: &'static str,
    pub for_window: &'static str,
    pub severity: &'static str,
    pub promql: &'static str,
    pub annotation: &'static str,
}

pub fn dashboard_panels() -> Vec<PanelSpec> {
    vec![
        PanelSpec {
            id: "chunks_processed",
            title: "Chunks Processed (rate)",
            unit: "ops",
            promql: "rate(trufflehog_chunks_processed_total[5m])",
        },
        PanelSpec {
            id: "findings_emitted",
            title: "Findings Emitted (rate)",
            unit: "ops",
            promql: "rate(trufflehog_findings_total[5m])",
        },
        PanelSpec {
            id: "verified_findings",
            title: "Verified Findings (rate)",
            unit: "ops",
            promql: "rate(trufflehog_verified_findings_total[5m])",
        },
        PanelSpec {
            id: "verification_latency_ms",
            title: "Verification HTTP Latency p99",
            unit: "ms",
            promql:
                "histogram_quantile(0.99, sum(rate(trufflehog_verification_latency_ms_bucket[5m])) by (le))",
        },
        PanelSpec {
            id: "verification_cache_hit_rate",
            title: "Verification Cache Hit Rate",
            unit: "%",
            promql: "rate(trufflehog_verification_cache_hits_total[5m]) / clamp_min(rate(trufflehog_verification_cache_total[5m]),1)",
        },
        PanelSpec {
            id: "scan_duration_seconds",
            title: "Scan Duration p95",
            unit: "s",
            promql:
                "histogram_quantile(0.95, sum(rate(trufflehog_scan_duration_seconds_bucket[10m])) by (le))",
        },
    ]
}

pub fn alert_rules() -> Vec<AlertSpec> {
    vec![
        AlertSpec {
            id: "ZeroFindingsScan",
            for_window: "30m",
            severity: "warning",
            promql:
                "sum(increase(trufflehog_findings_total[1h])) == 0 and sum(increase(trufflehog_chunks_processed_total[1h])) > 0",
            annotation: "Engine processed chunks for over 1h but produced no findings — detector regression?",
        },
        AlertSpec {
            id: "HighVerificationFailure",
            for_window: "10m",
            severity: "warning",
            promql: "rate(trufflehog_verification_errors_total[10m]) > 0.10",
            annotation: "Verification error rate >10% — upstream provider rate-limit or outage.",
        },
        AlertSpec {
            id: "VerificationCacheStarved",
            for_window: "15m",
            severity: "info",
            promql:
                "rate(trufflehog_verification_cache_hits_total[15m]) / clamp_min(rate(trufflehog_verification_cache_total[15m]),1) < 0.10",
            annotation: "Cache hit rate <10% — bump capacity or TTL.",
        },
        AlertSpec {
            id: "LongRunningScan",
            for_window: "1h",
            severity: "warning",
            promql: "trufflehog_scan_duration_seconds > 7200",
            annotation: "Scan exceeded 2h — partition source or enable resume checkpoints.",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_has_six_panels() {
        assert_eq!(dashboard_panels().len(), 6);
    }

    #[test]
    fn alerts_have_four_rules() {
        assert_eq!(alert_rules().len(), 4);
    }

    #[test]
    fn panel_ids_unique() {
        let mut ids: Vec<_> = dashboard_panels().iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 6);
    }

    #[test]
    fn alert_ids_unique() {
        let mut ids: Vec<_> = alert_rules().iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn promql_non_empty() {
        for p in dashboard_panels() {
            assert!(!p.promql.is_empty());
        }
        for a in alert_rules() {
            assert!(!a.promql.is_empty());
        }
    }

    #[test]
    fn alerts_have_valid_severity() {
        let valid = ["info", "warning", "critical"];
        for a in alert_rules() {
            assert!(valid.contains(&a.severity));
        }
    }
}
