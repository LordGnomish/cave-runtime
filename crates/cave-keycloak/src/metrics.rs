// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Observability — Prometheus-style metric names and Grafana dashboard
//! + alert manifests that cave-metrics + cave-dashboard + cave-oncall
//! pick up. The strings are the contract.
//!
//! Upstream parity: Keycloak ships `keycloak.user.*` / `keycloak.client.*`
//! Micrometer meters via the `keycloak-quarkus-server` runtime. The cave
//! port mirrors the names so any existing Keycloak dashboard works
//! unchanged when scraping the cave-keycloak `/metrics` endpoint.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricPanel {
    pub metric: String,
    pub title: String,
    pub kind: PanelKind,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRule {
    pub name: String,
    pub expr: String,
    pub for_seconds: u32,
    pub severity: String,
    pub runbook: String,
}

/// Cave-keycloak emits at minimum these metrics. cave-runtime wires the
/// observation points; this module owns the names + dashboard contract.
pub fn standard_panels() -> Vec<MetricPanel> {
    vec![
        MetricPanel {
            metric: "keycloak_logins_total".into(),
            title: "Successful logins (by realm)".into(),
            kind: PanelKind::Counter,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_login_errors_total".into(),
            title: "Failed logins (by realm + error)".into(),
            kind: PanelKind::Counter,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_token_endpoint_seconds".into(),
            title: "Token endpoint p99 latency".into(),
            kind: PanelKind::Histogram,
            unit: "s".into(),
        },
        MetricPanel {
            metric: "keycloak_active_sessions".into(),
            title: "Active SSO sessions".into(),
            kind: PanelKind::Gauge,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_active_offline_sessions".into(),
            title: "Active offline sessions".into(),
            kind: PanelKind::Gauge,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_refresh_token_replays_total".into(),
            title: "Refresh-token replay events (chain revoked)".into(),
            kind: PanelKind::Counter,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_brute_force_lockouts_total".into(),
            title: "Brute-force lockouts".into(),
            kind: PanelKind::Counter,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_credentials_locked_users".into(),
            title: "Users currently locked (gauge)".into(),
            kind: PanelKind::Gauge,
            unit: "1".into(),
        },
        MetricPanel {
            metric: "keycloak_jwks_rotation_age_seconds".into(),
            title: "Active signing-key age".into(),
            kind: PanelKind::Gauge,
            unit: "s".into(),
        },
        MetricPanel {
            metric: "keycloak_idp_brokered_logins_total".into(),
            title: "External-IDP brokered logins (by alias)".into(),
            kind: PanelKind::Counter,
            unit: "1".into(),
        },
    ]
}

pub fn standard_alerts() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "KeycloakLoginErrorSpike".into(),
            expr: "rate(keycloak_login_errors_total[5m]) > 5 * rate(keycloak_login_errors_total[1h] offset 1h)".into(),
            for_seconds: 600,
            severity: "warning".into(),
            runbook: "docs/runbooks/keycloak-login-errors.md".into(),
        },
        AlertRule {
            name: "KeycloakTokenEndpointLatency".into(),
            expr: "histogram_quantile(0.99, sum(rate(keycloak_token_endpoint_seconds_bucket[5m])) by (le)) > 1.5".into(),
            for_seconds: 600,
            severity: "warning".into(),
            runbook: "docs/runbooks/keycloak-token-latency.md".into(),
        },
        AlertRule {
            name: "KeycloakRefreshTokenReplay".into(),
            expr: "increase(keycloak_refresh_token_replays_total[10m]) > 0".into(),
            for_seconds: 0,
            severity: "critical".into(),
            runbook: "docs/runbooks/keycloak-refresh-token-replay.md".into(),
        },
        AlertRule {
            name: "KeycloakBruteForceFlood".into(),
            expr: "rate(keycloak_brute_force_lockouts_total[5m]) > 10".into(),
            for_seconds: 300,
            severity: "critical".into(),
            runbook: "docs/runbooks/keycloak-brute-force.md".into(),
        },
        AlertRule {
            name: "KeycloakStaleSigningKey".into(),
            expr: "keycloak_jwks_rotation_age_seconds > 60 * 60 * 24 * 90".into(),
            for_seconds: 3600,
            severity: "warning".into(),
            runbook: "docs/runbooks/keycloak-key-rotation.md".into(),
        },
        AlertRule {
            name: "KeycloakHealthEndpointDown".into(),
            expr: "up{job=\"cave-keycloak\"} == 0".into(),
            for_seconds: 120,
            severity: "critical".into(),
            runbook: "docs/runbooks/keycloak-down.md".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_standard_panels() {
        let p = standard_panels();
        assert_eq!(p.len(), 10);
        let names: Vec<_> = p.iter().map(|m| m.metric.clone()).collect();
        for n in &names {
            assert!(n.starts_with("keycloak_"));
        }
    }

    #[test]
    fn six_standard_alerts() {
        let a = standard_alerts();
        assert_eq!(a.len(), 6);
        let names: Vec<_> = a.iter().map(|r| r.name.clone()).collect();
        assert!(names.contains(&"KeycloakRefreshTokenReplay".to_string()));
        assert!(names.contains(&"KeycloakHealthEndpointDown".to_string()));
    }

    #[test]
    fn every_alert_carries_a_runbook() {
        for a in standard_alerts() {
            assert!(a.runbook.starts_with("docs/runbooks/"));
        }
    }

    #[test]
    fn refresh_token_replay_is_critical_with_zero_window() {
        let a = standard_alerts();
        let r = a.iter().find(|x| x.name == "KeycloakRefreshTokenReplay").unwrap();
        assert_eq!(r.severity, "critical");
        assert_eq!(r.for_seconds, 0);
    }

    #[test]
    fn token_endpoint_has_histogram_panel() {
        let p = standard_panels();
        let h = p.iter().find(|m| m.metric == "keycloak_token_endpoint_seconds").unwrap();
        assert_eq!(h.kind, PanelKind::Histogram);
        assert_eq!(h.unit, "s");
    }
}
