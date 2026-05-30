// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! RED→GREEN strict-TDD port of grafana/oncall `engine/config_integrations/`
//! per-source webhook receivers. Each test pins the exact normalization
//! contract a Grafana OnCall integration applies to an incoming payload:
//! `grouping_id`, `resolve_condition`, `source_link`, and `web_title`.
//!
//! Payloads are copied verbatim from the upstream `example_payload` blocks
//! (v1.10.0) so the parser is checked against the real wire format, not a
//! convenient simplification.

use cave_oncall::integrations::{parse_for, IntegrationType};
use cave_oncall::models::Severity;
use serde_json::json;

// ── Alertmanager (Prometheus) — config_integrations/alertmanager.py ──────────

#[test]
fn test_alertmanager_parses_grouping_and_firing() {
    // Verbatim from alertmanager.py example_payload.
    let payload = json!({
        "alerts": [
            {
                "endsAt": "0001-01-01T00:00:00Z",
                "labels": {
                    "job": "node", "group": "production", "instance": "localhost:8081",
                    "severity": "critical", "alertname": "InstanceDown"
                },
                "status": "firing",
                "startsAt": "2023-06-12T08:24:38.326Z",
                "annotations": {
                    "title": "Instance localhost:8081 down",
                    "description": "localhost:8081 of job node has been down for more than 1 minute."
                },
                "fingerprint": "f404ecabc8dd5cd7",
                "generatorURL": "http://prom/graph"
            }
        ],
        "status": "firing",
        "version": "4",
        "groupKey": "{}:{alertname=\"InstanceDown\"}",
        "receiver": "combo",
        "numFiring": 2,
        "externalURL": "",
        "groupLabels": {"alertname": "InstanceDown"},
        "numResolved": 0,
        "commonLabels": {"job": "node", "severity": "critical", "alertname": "InstanceDown"},
        "truncatedAlerts": 0,
        "commonAnnotations": {}
    });

    let a = parse_for("alertmanager", &payload).expect("alertmanager parse");
    assert_eq!(a.integration, IntegrationType::Alertmanager);
    // grouping_id = payload.groupKey
    assert_eq!(a.grouping_id, "{}:{alertname=\"InstanceDown\"}");
    // resolve_condition = (payload.status == "resolved")
    assert!(!a.is_resolved);
    // web_title: alertname popped from groupLabels (no remaining labels here)
    assert_eq!(a.title, "InstanceDown");
    // source_link = payload.alerts[0].generatorURL
    assert_eq!(a.source_link.as_deref(), Some("http://prom/graph"));
    // severity from groupLabels/commonLabels severity
    assert_eq!(a.severity, Severity::Critical);
}

#[test]
fn test_alertmanager_resolved_status_and_extra_grouplabels() {
    let payload = json!({
        "alerts": [{"generatorURL": "", "status": "resolved"}],
        "status": "resolved",
        "groupKey": "g2",
        "groupLabels": {"alertname": "DiskFull", "region": "eu-1"},
        "commonLabels": {"severity": "warning"},
        "commonAnnotations": {}
    });
    let a = parse_for("alertmanager", &payload).expect("alertmanager parse");
    assert!(a.is_resolved);
    // web_title joins remaining groupLabels in parens after the popped alertname
    assert_eq!(a.title, "DiskFull (eu-1)");
    assert_eq!(a.severity, Severity::Medium); // "warning" maps to Medium
    assert_eq!(a.grouping_id, "g2");
}

// ── Grafana Alerting (unified) — config_integrations/grafana_alerting.py ─────

#[test]
fn test_grafana_alerting_is_alertmanager_shaped() {
    let payload = json!({
        "alerts": [{"generatorURL": "http://g/alert"}],
        "status": "firing",
        "groupKey": "ga-1",
        "groupLabels": {"alertname": "HighMem"},
        "commonLabels": {"severity": "critical"},
        "commonAnnotations": {}
    });
    let a = parse_for("grafana_alerting", &payload).expect("grafana_alerting parse");
    assert_eq!(a.integration, IntegrationType::GrafanaAlerting);
    assert_eq!(a.grouping_id, "ga-1");
    assert!(!a.is_resolved);
    assert_eq!(a.source_link.as_deref(), Some("http://g/alert"));
    assert_eq!(a.title, "HighMem");
}

// ── Grafana Legacy Alerting — config_integrations/grafana.py ─────────────────

#[test]
fn test_grafana_legacy_rulename_grouping_and_ok_resolves() {
    // grafana.py: grouping_id = ruleName or labels.alertname;
    //             resolve = state=="ok" or status=="resolved";
    //             source_link = ruleUrl or generatorURL
    let payload = json!({
        "state": "ok",
        "ruleName": "CPU load",
        "ruleUrl": "http://grafana/d/abc",
        "evalMatches": [],
        "title": "[OK] CPU load"
    });
    let a = parse_for("grafana", &payload).expect("grafana legacy parse");
    assert_eq!(a.integration, IntegrationType::GrafanaLegacy);
    assert_eq!(a.grouping_id, "CPU load");
    assert!(a.is_resolved); // state == "ok"
    assert_eq!(a.source_link.as_deref(), Some("http://grafana/d/abc"));
    assert_eq!(a.title, "CPU load");
}

#[test]
fn test_grafana_legacy_alerting_state_falls_back_to_labels() {
    let payload = json!({
        "state": "alerting",
        "labels": {"alertname": "DBdown"},
        "generatorURL": "http://gen/url"
    });
    let a = parse_for("grafana", &payload).expect("grafana legacy parse");
    assert!(!a.is_resolved);
    assert_eq!(a.grouping_id, "DBdown"); // ruleName missing -> labels.alertname
    assert_eq!(a.source_link.as_deref(), Some("http://gen/url")); // ruleUrl missing -> generatorURL
}

// ── Formatted webhook — config_integrations/formatted_webhook.py ─────────────

#[test]
fn test_formatted_webhook_fields() {
    // Verbatim from formatted_webhook.py example_payload.
    let payload = json!({
        "alert_uid": "08d6891a-835c-e661-39fa-96b6a9e26552",
        "title": "TestAlert: The whole system is down",
        "image_url": "https://example/cat.jpg",
        "state": "alerting",
        "link_to_upstream_details": "https://en.wikipedia.org/wiki/Downtime",
        "message": "This alert was sent by user for demonstration purposes"
    });
    let a = parse_for("formatted_webhook", &payload).expect("formatted parse");
    assert_eq!(a.integration, IntegrationType::FormattedWebhook);
    assert_eq!(a.grouping_id, "08d6891a-835c-e661-39fa-96b6a9e26552");
    assert_eq!(a.title, "TestAlert: The whole system is down");
    assert!(!a.is_resolved); // state.upper() != "OK"
    assert_eq!(
        a.source_link.as_deref(),
        Some("https://en.wikipedia.org/wiki/Downtime")
    );
    assert_eq!(a.image_url.as_deref(), Some("https://example/cat.jpg"));
    assert_eq!(a.message, "This alert was sent by user for demonstration purposes");
}

#[test]
fn test_formatted_webhook_state_ok_resolves_case_insensitive() {
    let payload = json!({"title": "t", "alert_uid": "u1", "state": "Ok"});
    let a = parse_for("formatted_webhook", &payload).expect("formatted parse");
    assert!(a.is_resolved); // "Ok".upper() == "OK"
}

// ── Generic webhook — config_integrations/webhook.py ─────────────────────────

#[test]
fn test_generic_webhook_default_title_and_heartbeat_grouping() {
    // webhook.py: web_title = "Incident"; resolve = state.upper()=="OK";
    //   grouping_id = alert_uid when is_oncall_heartbeat present, else whole payload
    let payload = json!({
        "is_oncall_heartbeat": true,
        "alert_uid": "hb-42",
        "url": "http://src",
        "state": "ok"
    });
    let a = parse_for("webhook", &payload).expect("webhook parse");
    assert_eq!(a.integration, IntegrationType::Webhook);
    assert_eq!(a.title, "Incident");
    assert_eq!(a.grouping_id, "hb-42");
    assert!(a.is_resolved);
    assert_eq!(a.source_link.as_deref(), Some("http://src"));
}

#[test]
fn test_generic_webhook_non_heartbeat_groups_on_payload() {
    let payload = json!({"message": "boom"});
    let a = parse_for("webhook", &payload).expect("webhook parse");
    // No is_oncall_heartbeat -> grouping_id is the canonical JSON of the payload
    assert_eq!(a.grouping_id, serde_json::to_string(&payload).unwrap());
    assert!(!a.is_resolved);
}

// ── Unknown slug is an error, not a panic ────────────────────────────────────

#[test]
fn test_unknown_integration_slug_errors() {
    let payload = json!({"x": 1});
    assert!(parse_for("does_not_exist", &payload).is_err());
}
