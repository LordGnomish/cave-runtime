// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus metric names + Portal dashboard panel JSON + alert rules.
//!
//! Upstream: `pkg/metrics/metrics.go`, `install/grafana/tetragon.json`.

use serde::{Deserialize, Serialize};

/// Canonical metric names emitted by cave-forensics. Exposed as constants
/// so cave-portal-api can pin them in dashboards without string drift.
pub const METRIC_EVENTS_TOTAL: &str = "cave_forensics_events_total";
pub const METRIC_POLICY_VIOLATIONS_TOTAL: &str = "cave_forensics_policy_violations_total";
pub const METRIC_ENFORCEMENT_ACTIONS_TOTAL: &str = "cave_forensics_enforcement_actions_total";
pub const METRIC_CASES_OPEN: &str = "cave_forensics_cases_open";
pub const METRIC_EVIDENCE_INGESTED_TOTAL: &str = "cave_forensics_evidence_ingested_total";
pub const METRIC_PROCESS_TREE_SIZE: &str = "cave_forensics_process_tree_size";
pub const METRIC_FOLLOWED_FD_COUNT: &str = "cave_forensics_followed_fd_count";
pub const METRIC_POLICY_INSTALL_TOTAL: &str = "cave_forensics_policy_install_total";

/// 8 dashboard panels — one per metric. Mirrors the layout used in
/// `install/grafana/tetragon.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardPanel {
    pub title: String,
    pub metric: String,
    pub query: String,
}

pub fn dashboard_panels() -> Vec<DashboardPanel> {
    vec![
        DashboardPanel {
            title: "Events / second by kind".into(),
            metric: METRIC_EVENTS_TOTAL.into(),
            query: format!("sum by (kind) (rate({}[1m]))", METRIC_EVENTS_TOTAL),
        },
        DashboardPanel {
            title: "Policy violations / minute".into(),
            metric: METRIC_POLICY_VIOLATIONS_TOTAL.into(),
            query: format!(
                "sum by (policy) (rate({}[5m]))",
                METRIC_POLICY_VIOLATIONS_TOTAL
            ),
        },
        DashboardPanel {
            title: "Enforcement actions / second".into(),
            metric: METRIC_ENFORCEMENT_ACTIONS_TOTAL.into(),
            query: format!(
                "sum by (action) (rate({}[1m]))",
                METRIC_ENFORCEMENT_ACTIONS_TOTAL
            ),
        },
        DashboardPanel {
            title: "Open forensic cases".into(),
            metric: METRIC_CASES_OPEN.into(),
            query: METRIC_CASES_OPEN.to_string(),
        },
        DashboardPanel {
            title: "Evidence ingested / minute".into(),
            metric: METRIC_EVIDENCE_INGESTED_TOTAL.into(),
            query: format!("rate({}[5m])", METRIC_EVIDENCE_INGESTED_TOTAL),
        },
        DashboardPanel {
            title: "Process-tree size".into(),
            metric: METRIC_PROCESS_TREE_SIZE.into(),
            query: METRIC_PROCESS_TREE_SIZE.to_string(),
        },
        DashboardPanel {
            title: "Followed FDs".into(),
            metric: METRIC_FOLLOWED_FD_COUNT.into(),
            query: METRIC_FOLLOWED_FD_COUNT.to_string(),
        },
        DashboardPanel {
            title: "Policy install events / hour".into(),
            metric: METRIC_POLICY_INSTALL_TOTAL.into(),
            query: format!("rate({}[1h])", METRIC_POLICY_INSTALL_TOTAL),
        },
    ]
}

/// 5 Prometheus alert rules. Encoded as a Rust struct, serialised to
/// YAML by `to_yaml_string`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertRule {
    pub name: String,
    pub expr: String,
    pub r#for: String,
    pub severity: String,
    pub description: String,
}

pub fn alert_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "ForensicsHighViolationRate".into(),
            expr: format!("rate({}[5m]) > 10", METRIC_POLICY_VIOLATIONS_TOTAL),
            r#for: "10m".into(),
            severity: "critical".into(),
            description: "Policy violation rate > 10/s for 10 minutes".into(),
        },
        AlertRule {
            name: "ForensicsEnforcementSpike".into(),
            expr: format!("rate({}[5m]) > 5", METRIC_ENFORCEMENT_ACTIONS_TOTAL),
            r#for: "5m".into(),
            severity: "warning".into(),
            description: "Enforcement actions > 5/s — likely active attack".into(),
        },
        AlertRule {
            name: "ForensicsCasesOpenTooLong".into(),
            expr: format!("{} > 50", METRIC_CASES_OPEN),
            r#for: "30m".into(),
            severity: "warning".into(),
            description: "More than 50 open cases for 30 minutes".into(),
        },
        AlertRule {
            name: "ForensicsEventsDried".into(),
            expr: format!("rate({}[15m]) == 0", METRIC_EVENTS_TOTAL),
            r#for: "15m".into(),
            severity: "critical".into(),
            description: "No kernel events for 15 minutes — tetragon agent likely down".into(),
        },
        AlertRule {
            name: "ForensicsProcessTreeRunaway".into(),
            expr: format!("{} > 100000", METRIC_PROCESS_TREE_SIZE),
            r#for: "10m".into(),
            severity: "warning".into(),
            description: "Process tree > 100k entries — reaping is broken".into(),
        },
    ]
}

/// Serialise the alert rules as a Prometheus-compatible YAML string
/// (no external yaml crate — handcrafted, but well-formed).
pub fn alert_rules_yaml() -> String {
    let mut out = String::from("groups:\n  - name: cave-forensics\n    rules:\n");
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
    fn test_metric_names_have_cave_forensics_prefix() {
        for m in [
            METRIC_EVENTS_TOTAL,
            METRIC_POLICY_VIOLATIONS_TOTAL,
            METRIC_ENFORCEMENT_ACTIONS_TOTAL,
            METRIC_CASES_OPEN,
            METRIC_EVIDENCE_INGESTED_TOTAL,
            METRIC_PROCESS_TREE_SIZE,
            METRIC_FOLLOWED_FD_COUNT,
            METRIC_POLICY_INSTALL_TOTAL,
        ] {
            assert!(m.starts_with("cave_forensics_"), "{m} must have prefix");
        }
    }

    #[test]
    fn test_panels_reference_known_metrics() {
        let known: std::collections::HashSet<&str> = [
            METRIC_EVENTS_TOTAL,
            METRIC_POLICY_VIOLATIONS_TOTAL,
            METRIC_ENFORCEMENT_ACTIONS_TOTAL,
            METRIC_CASES_OPEN,
            METRIC_EVIDENCE_INGESTED_TOTAL,
            METRIC_PROCESS_TREE_SIZE,
            METRIC_FOLLOWED_FD_COUNT,
            METRIC_POLICY_INSTALL_TOTAL,
        ]
        .into_iter()
        .collect();
        for p in dashboard_panels() {
            assert!(known.contains(p.metric.as_str()), "unknown metric: {}", p.metric);
        }
    }

    #[test]
    fn test_alerts_reference_known_metrics() {
        for r in alert_rules() {
            assert!(
                r.expr.contains("cave_forensics_"),
                "alert {} must reference cave_forensics_ metric: {}",
                r.name,
                r.expr
            );
        }
    }

    #[test]
    fn test_alert_rules_yaml_is_well_formed() {
        let y = alert_rules_yaml();
        assert!(y.starts_with("groups:"));
        assert!(y.contains("name: cave-forensics"));
        assert!(y.contains("alert: ForensicsHighViolationRate"));
    }

    #[test]
    fn test_panels_have_unique_titles() {
        let panels = dashboard_panels();
        let titles: std::collections::HashSet<_> = panels.iter().map(|p| p.title.clone()).collect();
        assert_eq!(titles.len(), panels.len());
    }

    #[test]
    fn test_alerts_have_severity_set() {
        for r in alert_rules() {
            assert!(matches!(r.severity.as_str(), "critical" | "warning"));
        }
    }
}
