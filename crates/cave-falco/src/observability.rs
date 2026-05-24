// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Dashboards + alerts surface — read by cave-portal-api / cave-metrics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub title: String,
    pub query: String,
    pub unit: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRule {
    pub alert: String,
    pub expr: String,
    pub for_: &'static str,
    pub severity: &'static str,
    pub summary: String,
}

pub fn dashboard_panels() -> Vec<DashboardPanel> {
    vec![
        DashboardPanel {
            title: "Falco alerts/min — Critical+".into(),
            query: "sum(rate(falco_alerts_total{priority=~\"CRITICAL|ALERT|EMERGENCY\"}[1m]))".into(),
            unit: "alerts/min",
        },
        DashboardPanel {
            title: "Falco alerts/min — Warning".into(),
            query: "sum(rate(falco_alerts_total{priority=\"WARNING\"}[1m]))".into(),
            unit: "alerts/min",
        },
        DashboardPanel {
            title: "Engine rule load duration".into(),
            query: "histogram_quantile(0.95, falco_engine_compile_seconds_bucket)".into(),
            unit: "seconds",
        },
        DashboardPanel {
            title: "Events evaluated/sec by source".into(),
            query: "sum by (source) (rate(falco_events_total[1m]))".into(),
            unit: "events/sec",
        },
        DashboardPanel {
            title: "Top-10 firing rules (5m)".into(),
            query: "topk(10, sum by (rule) (rate(falco_alerts_total[5m])))".into(),
            unit: "alerts",
        },
        DashboardPanel {
            title: "k8s_audit projection latency p95".into(),
            query: "histogram_quantile(0.95, falco_k8saudit_project_seconds_bucket)".into(),
            unit: "seconds",
        },
        DashboardPanel {
            title: "Plugin extraction calls/sec".into(),
            query: "sum(rate(falco_plugin_extract_total[1m]))".into(),
            unit: "calls/sec",
        },
        DashboardPanel {
            title: "Output sidekick send failures".into(),
            query: "sum(rate(falco_output_send_failures_total[5m]))".into(),
            unit: "failures/min",
        },
    ]
}

pub fn alert_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            alert: "FalcoCriticalAlertSurge".into(),
            expr: "sum(rate(falco_alerts_total{priority=~\"CRITICAL|ALERT|EMERGENCY\"}[5m])) > 10".into(),
            for_: "5m",
            severity: "page",
            summary: "Falco critical-tier alerts > 10/min for 5m".into(),
        },
        AlertRule {
            alert: "FalcoEngineCompileStalled".into(),
            expr: "histogram_quantile(0.95, falco_engine_compile_seconds_bucket) > 5".into(),
            for_: "10m",
            severity: "ticket",
            summary: "Falco rule compile p95 > 5s — likely runaway macro expansion".into(),
        },
        AlertRule {
            alert: "FalcoK8sAuditBacklog".into(),
            expr: "falco_k8saudit_queue_depth > 1000".into(),
            for_: "5m",
            severity: "ticket",
            summary: "k8s_audit ingestion queue > 1000 events".into(),
        },
        AlertRule {
            alert: "FalcoOutputSendFailures".into(),
            expr: "sum(rate(falco_output_send_failures_total[5m])) > 0.5".into(),
            for_: "10m",
            severity: "ticket",
            summary: "Falco output sink failures sustained > 0.5/sec".into(),
        },
        AlertRule {
            alert: "FalcoRulesStaleness".into(),
            expr: "time() - falco_engine_rules_loaded_timestamp_seconds > 86400 * 7".into(),
            for_: "1h",
            severity: "info",
            summary: "Falco rules not refreshed in > 7 days".into(),
        },
    ]
}

pub fn alert_rules_yaml() -> String {
    let mut s = String::new();
    s.push_str("groups:\n- name: falco\n  rules:\n");
    for r in alert_rules() {
        s.push_str(&format!(
            "  - alert: {}\n    expr: {}\n    for: {}\n    labels:\n      severity: {}\n    annotations:\n      summary: \"{}\"\n",
            r.alert, r.expr, r.for_, r.severity, r.summary,
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_count_is_eight() {
        assert_eq!(dashboard_panels().len(), 8);
    }

    #[test]
    fn alert_count_is_five() {
        assert_eq!(alert_rules().len(), 5);
    }

    #[test]
    fn panels_all_have_non_empty_queries() {
        for p in dashboard_panels() {
            assert!(!p.query.is_empty(), "panel '{}' missing query", p.title);
        }
    }

    #[test]
    fn alert_yaml_starts_with_groups_block() {
        let y = alert_rules_yaml();
        assert!(y.starts_with("groups:\n"));
        assert!(y.contains("FalcoCriticalAlertSurge"));
    }

    #[test]
    fn alert_severities_use_known_tiers() {
        let known = ["page", "ticket", "info"];
        for a in alert_rules() {
            assert!(known.contains(&a.severity), "unknown severity '{}'", a.severity);
        }
    }
}
