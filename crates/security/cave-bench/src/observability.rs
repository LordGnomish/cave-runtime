// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus metric names + Portal dashboard panels + alert rules.
//!
//! Upstream: kube-bench `metrics/` + kubescape `pkg/prom/` patterns.

use serde::{Deserialize, Serialize};

pub const METRIC_SCAN_TOTAL: &str = "cave_bench_scan_total";
pub const METRIC_CONTROL_FAIL_TOTAL: &str = "cave_bench_control_fail_total";
pub const METRIC_CIS_SCORE: &str = "cave_bench_cis_score";
pub const METRIC_NSA_CONTROL_GAPS: &str = "cave_bench_nsa_control_gaps";
pub const METRIC_MITRE_TACTIC_COVERAGE: &str = "cave_bench_mitre_tactic_coverage";
pub const METRIC_TOP_FAILING_NODES: &str = "cave_bench_top_failing_nodes";
pub const METRIC_SCAN_DURATION_SECONDS: &str = "cave_bench_scan_duration_seconds";
pub const METRIC_SCAN_QUEUE_DEPTH: &str = "cave_bench_scan_queue_depth";
pub const METRIC_CHECK_SKIP_ERROR_TOTAL: &str = "cave_bench_check_skip_error_total";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardPanel {
    pub title: String,
    pub metric: String,
    pub query: String,
}

/// 8 dashboard panels — covers control fail rate, score trend, MITRE coverage etc.
pub fn dashboard_panels() -> Vec<DashboardPanel> {
    vec![
        DashboardPanel {
            title: "Control failure rate (1m)".into(),
            metric: METRIC_CONTROL_FAIL_TOTAL.into(),
            query: format!("sum by (framework) (rate({}[1m]))", METRIC_CONTROL_FAIL_TOTAL),
        },
        DashboardPanel {
            title: "CIS score trend".into(),
            metric: METRIC_CIS_SCORE.into(),
            query: format!("avg by (profile) ({})", METRIC_CIS_SCORE),
        },
        DashboardPanel {
            title: "NSA control gaps".into(),
            metric: METRIC_NSA_CONTROL_GAPS.into(),
            query: METRIC_NSA_CONTROL_GAPS.to_string(),
        },
        DashboardPanel {
            title: "MITRE ATT&CK tactic coverage".into(),
            metric: METRIC_MITRE_TACTIC_COVERAGE.into(),
            query: format!("avg by (tactic) ({})", METRIC_MITRE_TACTIC_COVERAGE),
        },
        DashboardPanel {
            title: "Top failing nodes".into(),
            metric: METRIC_TOP_FAILING_NODES.into(),
            query: format!("topk(10, {})", METRIC_TOP_FAILING_NODES),
        },
        DashboardPanel {
            title: "Profile execution duration".into(),
            metric: METRIC_SCAN_DURATION_SECONDS.into(),
            query: format!("histogram_quantile(0.95, rate({}_bucket[5m]))", METRIC_SCAN_DURATION_SECONDS),
        },
        DashboardPanel {
            title: "Scan queue depth".into(),
            metric: METRIC_SCAN_QUEUE_DEPTH.into(),
            query: METRIC_SCAN_QUEUE_DEPTH.to_string(),
        },
        DashboardPanel {
            title: "Checks skipped due to error".into(),
            metric: METRIC_CHECK_SKIP_ERROR_TOTAL.into(),
            query: format!("rate({}[5m])", METRIC_CHECK_SKIP_ERROR_TOTAL),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertRule {
    pub name: String,
    pub expr: String,
    pub r#for: String,
    pub severity: String,
    pub description: String,
}

/// 5 alert rules — high-fail rate, score regression, queue blocked, etc.
pub fn alert_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "BenchControlFailureSpike".into(),
            expr: format!("rate({}[5m]) > 10", METRIC_CONTROL_FAIL_TOTAL),
            r#for: "10m".into(),
            severity: "critical".into(),
            description: "Bench control-fail rate > 10/s for 10 minutes".into(),
        },
        AlertRule {
            name: "BenchCisScoreRegression".into(),
            expr: format!("avg_over_time({}[1h]) < 0.85", METRIC_CIS_SCORE),
            r#for: "1h".into(),
            severity: "warning".into(),
            description: "Average CIS score over 1 hour dropped below 85%".into(),
        },
        AlertRule {
            name: "BenchNsaGapsHigh".into(),
            expr: format!("{} > 5", METRIC_NSA_CONTROL_GAPS),
            r#for: "30m".into(),
            severity: "warning".into(),
            description: "More than 5 NSA control gaps for 30 minutes".into(),
        },
        AlertRule {
            name: "BenchScanQueueStalled".into(),
            expr: format!("{} > 100", METRIC_SCAN_QUEUE_DEPTH),
            r#for: "15m".into(),
            severity: "critical".into(),
            description: "Scan queue depth > 100 — runner stalled".into(),
        },
        AlertRule {
            name: "BenchCheckErrorsHigh".into(),
            expr: format!("rate({}[10m]) > 1", METRIC_CHECK_SKIP_ERROR_TOTAL),
            r#for: "20m".into(),
            severity: "warning".into(),
            description: "Checks erroring (>1/s) for 20 minutes — env mis-mounted".into(),
        },
    ]
}

/// Serialise alerts as Prometheus-compatible YAML.
pub fn alert_rules_yaml() -> String {
    let mut out = String::from("groups:\n  - name: cave-bench\n    rules:\n");
    for r in alert_rules() {
        out.push_str(&format!("      - alert: {}\n", r.name));
        out.push_str(&format!("        expr: {}\n", r.expr));
        out.push_str(&format!("        for: {}\n", r.r#for));
        out.push_str(&format!("        labels:\n          severity: {}\n", r.severity));
        out.push_str(&format!(
            "        annotations:\n          description: \"{}\"\n",
            r.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_has_eight_panels() {
        assert_eq!(dashboard_panels().len(), 8);
    }

    #[test]
    fn test_alert_rules_has_five_alerts() {
        assert_eq!(alert_rules().len(), 5);
    }

    #[test]
    fn test_metric_names_prefixed() {
        for m in [
            METRIC_SCAN_TOTAL,
            METRIC_CONTROL_FAIL_TOTAL,
            METRIC_CIS_SCORE,
            METRIC_NSA_CONTROL_GAPS,
            METRIC_MITRE_TACTIC_COVERAGE,
            METRIC_TOP_FAILING_NODES,
            METRIC_SCAN_DURATION_SECONDS,
            METRIC_SCAN_QUEUE_DEPTH,
            METRIC_CHECK_SKIP_ERROR_TOTAL,
        ] {
            assert!(m.starts_with("cave_bench_"), "{m} missing prefix");
        }
    }

    #[test]
    fn test_panels_reference_known_metrics() {
        let known: std::collections::HashSet<&str> = [
            METRIC_SCAN_TOTAL,
            METRIC_CONTROL_FAIL_TOTAL,
            METRIC_CIS_SCORE,
            METRIC_NSA_CONTROL_GAPS,
            METRIC_MITRE_TACTIC_COVERAGE,
            METRIC_TOP_FAILING_NODES,
            METRIC_SCAN_DURATION_SECONDS,
            METRIC_SCAN_QUEUE_DEPTH,
            METRIC_CHECK_SKIP_ERROR_TOTAL,
        ]
        .into_iter()
        .collect();
        for p in dashboard_panels() {
            assert!(known.contains(p.metric.as_str()), "{}", p.metric);
        }
    }

    #[test]
    fn test_alerts_yaml_contains_all_alerts() {
        let y = alert_rules_yaml();
        for r in alert_rules() {
            assert!(y.contains(&format!("alert: {}", r.name)));
        }
    }

    #[test]
    fn test_alerts_reference_known_metrics() {
        let known = [
            METRIC_CONTROL_FAIL_TOTAL,
            METRIC_CIS_SCORE,
            METRIC_NSA_CONTROL_GAPS,
            METRIC_SCAN_QUEUE_DEPTH,
            METRIC_CHECK_SKIP_ERROR_TOTAL,
        ];
        for r in alert_rules() {
            assert!(known.iter().any(|k| r.expr.contains(k)), "{} missing metric", r.name);
        }
    }
}
