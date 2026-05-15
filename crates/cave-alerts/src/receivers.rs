// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Receiver dispatch.
//!
//! Each receiver type renders the firing alerts into a payload appropriate
//! for the upstream system (Slack JSON, PagerDuty Events v2, OpsGenie API,
//! generic webhook, RFC 5322 mail, etc). The actual outbound HTTP/SMTP call
//! is intentionally **not** performed inside this crate — sending is left to
//! the runtime's `cave-runtime` integration layer (so we keep the unit-test
//! surface deterministic).
//!
//! The crate exposes `render_payload()` which returns a `RenderedNotification`
//! containing the destination URL/address, HTTP method, headers, and body
//! that the runtime can submit, and `dispatch()` which keeps a journal of
//! prepared notifications without performing network I/O.

use crate::models::{
    Alert, AlertState, EmailConfig, GrafanaOnCallConfig, OpsGenieConfig, PagerDutyConfig,
    Receiver, ReceiverConfig, SlackConfig, WebhookConfig,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Http { url: String, method: String, headers: Vec<(String, String)> },
    Smtp { host: String, port: u16, from: String, to: Vec<String>, require_tls: bool },
    Noop,
}

#[derive(Debug, Clone)]
pub struct RenderedNotification {
    pub receiver: String,
    pub kind: &'static str,
    pub transport: Transport,
    pub body: Value,
}

/// Render a single receiver-config to a notification payload.
pub fn render_payload(
    receiver_name: &str,
    config: &ReceiverConfig,
    firing: &[Alert],
    resolved: &[Alert],
) -> RenderedNotification {
    match config {
        ReceiverConfig::Webhook(c) => render_webhook(receiver_name, c, firing, resolved),
        ReceiverConfig::Slack(c) => render_slack(receiver_name, c, firing, resolved),
        ReceiverConfig::Email(c) => render_email(receiver_name, c, firing, resolved),
        ReceiverConfig::PagerDuty(c) => render_pagerduty(receiver_name, c, firing, resolved),
        ReceiverConfig::OpsGenie(c) => render_opsgenie(receiver_name, c, firing, resolved),
        ReceiverConfig::GrafanaOnCall(c) => render_grafana_oncall(receiver_name, c, firing, resolved),
    }
}

/// Render every config attached to a Receiver definition.
pub fn render_all(
    receiver: &Receiver,
    firing: &[Alert],
    resolved: &[Alert],
) -> Vec<RenderedNotification> {
    if firing.is_empty() && resolved.is_empty() {
        return vec![];
    }
    receiver
        .configs
        .iter()
        .filter(|c| !drop_resolved_only(c, firing, resolved))
        .map(|c| render_payload(&receiver.name, c, firing, resolved))
        .collect()
}

fn drop_resolved_only(c: &ReceiverConfig, firing: &[Alert], resolved: &[Alert]) -> bool {
    if !firing.is_empty() {
        return false;
    }
    let send_resolved = match c {
        ReceiverConfig::Webhook(c) => c.send_resolved,
        ReceiverConfig::Slack(c) => c.send_resolved,
        ReceiverConfig::Email(_) => true,
        ReceiverConfig::PagerDuty(c) => c.send_resolved,
        ReceiverConfig::OpsGenie(c) => c.send_resolved,
        ReceiverConfig::GrafanaOnCall(c) => c.send_resolved,
    };
    !send_resolved && !resolved.is_empty()
}

// ─── Per-receiver renderers ────────────────────────────────────────────────

fn render_webhook(name: &str, c: &WebhookConfig, firing: &[Alert], resolved: &[Alert]) -> RenderedNotification {
    let body = json!({
        "version": "4",
        "receiver": name,
        "status": if firing.is_empty() { "resolved" } else { "firing" },
        "alerts": alerts_to_array(firing, resolved),
    });
    RenderedNotification {
        receiver: name.to_string(),
        kind: "webhook",
        transport: Transport::Http {
            url: c.url.clone(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        },
        body,
    }
}

fn render_slack(name: &str, c: &SlackConfig, firing: &[Alert], resolved: &[Alert]) -> RenderedNotification {
    let title = if firing.is_empty() {
        format!("✅ {} alerts resolved", resolved.len())
    } else {
        format!("🔥 {} alerts firing", firing.len())
    };

    let mut blocks: Vec<Value> = vec![json!({
        "type": "header",
        "text": { "type": "plain_text", "text": title }
    })];

    for a in firing.iter().chain(resolved.iter()).take(20) {
        blocks.push(json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!(
                    "*{}* `{}`\n{}",
                    a.name,
                    severity_text(a),
                    a.annotations.get("summary").cloned().unwrap_or_default()
                )
            }
        }));
    }

    let body = json!({
        "channel": c.channel,
        "username": c.username.clone().unwrap_or_else(|| "CAVE Alerts".into()),
        "blocks": blocks,
    });
    RenderedNotification {
        receiver: name.to_string(),
        kind: "slack",
        transport: Transport::Http {
            url: c.webhook_url.clone(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        },
        body,
    }
}

fn render_email(name: &str, c: &EmailConfig, firing: &[Alert], resolved: &[Alert]) -> RenderedNotification {
    let subject = if firing.is_empty() {
        format!("[CAVE] {} resolved", resolved.len())
    } else {
        format!("[CAVE] {} firing", firing.len())
    };
    let body = json!({
        "from": c.from,
        "to": c.to,
        "subject": subject,
        "alerts": alerts_to_array(firing, resolved),
    });
    RenderedNotification {
        receiver: name.to_string(),
        kind: "email",
        transport: Transport::Smtp {
            host: c.smtp_host.clone(),
            port: c.smtp_port,
            from: c.from.clone(),
            to: c.to.clone(),
            require_tls: c.require_tls,
        },
        body,
    }
}

fn render_pagerduty(name: &str, c: &PagerDutyConfig, firing: &[Alert], resolved: &[Alert]) -> RenderedNotification {
    let event_action = if firing.is_empty() { "resolve" } else { "trigger" };
    let primary = firing.first().or_else(|| resolved.first());
    let summary = primary.map(|a| a.name.clone()).unwrap_or_else(|| name.to_string());
    let dedup_key = primary.map(|a| a.fingerprint.clone()).unwrap_or_else(|| name.to_string());
    let severity = primary.map(severity_text).unwrap_or_else(|| "warning".into());

    let body = json!({
        "routing_key": c.routing_key,
        "event_action": event_action,
        "dedup_key": dedup_key,
        "payload": {
            "summary": summary,
            "severity": severity,
            "source": "cave-alerts",
            "custom_details": alerts_to_array(firing, resolved),
        }
    });
    RenderedNotification {
        receiver: name.to_string(),
        kind: "pagerduty",
        transport: Transport::Http {
            url: "https://events.pagerduty.com/v2/enqueue".into(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        },
        body,
    }
}

fn render_opsgenie(name: &str, c: &OpsGenieConfig, firing: &[Alert], resolved: &[Alert]) -> RenderedNotification {
    let primary = firing.first().or_else(|| resolved.first());
    let alias = primary.map(|a| a.fingerprint.clone()).unwrap_or_else(|| name.to_string());
    let url = if firing.is_empty() {
        format!("{}/v2/alerts/{}/close?identifierType=alias", c.api_url.trim_end_matches('/'), alias)
    } else {
        format!("{}/v2/alerts", c.api_url.trim_end_matches('/'))
    };

    let body = if firing.is_empty() {
        json!({"note": "resolved by cave-alerts"})
    } else {
        json!({
            "message": primary.map(|a| a.name.clone()).unwrap_or_default(),
            "alias": alias,
            "details": alerts_to_array(firing, resolved),
            "priority": opsgenie_priority(primary),
        })
    };

    RenderedNotification {
        receiver: name.to_string(),
        kind: "opsgenie",
        transport: Transport::Http {
            url,
            method: "POST".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Authorization".into(), format!("GenieKey {}", c.api_key)),
            ],
        },
        body,
    }
}

fn render_grafana_oncall(
    name: &str,
    c: &GrafanaOnCallConfig,
    firing: &[Alert],
    resolved: &[Alert],
) -> RenderedNotification {
    let body = json!({
        "alert_uid": firing.first().or_else(|| resolved.first()).map(|a| a.fingerprint.clone()).unwrap_or_default(),
        "title": firing.first().or_else(|| resolved.first()).map(|a| a.name.clone()).unwrap_or_default(),
        "state": if firing.is_empty() { "ok" } else { "alerting" },
        "alerts": alerts_to_array(firing, resolved),
    });
    RenderedNotification {
        receiver: name.to_string(),
        kind: "grafana_oncall",
        transport: Transport::Http {
            url: c.webhook_url.clone(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        },
        body,
    }
}

fn alerts_to_array(firing: &[Alert], resolved: &[Alert]) -> Value {
    let mut v: Vec<Value> = Vec::with_capacity(firing.len() + resolved.len());
    for a in firing.iter().chain(resolved.iter()) {
        v.push(json!({
            "status": match a.state {
                AlertState::Firing => "firing",
                AlertState::Resolved => "resolved",
                AlertState::Silenced => "silenced",
            },
            "labels": a.labels,
            "annotations": a.annotations,
            "startsAt": a.starts_at.to_rfc3339(),
            "endsAt": a.ends_at.map(|e| e.to_rfc3339()),
            "fingerprint": a.fingerprint,
            "generatorURL": a.generator_url,
        }));
    }
    Value::Array(v)
}

fn severity_text(a: &Alert) -> String {
    match a.severity {
        crate::models::AlertSeverity::Critical => "critical".into(),
        crate::models::AlertSeverity::Warning => "warning".into(),
        crate::models::AlertSeverity::Info => "info".into(),
    }
}

fn opsgenie_priority(primary: Option<&Alert>) -> &'static str {
    match primary.map(|a| a.severity) {
        Some(crate::models::AlertSeverity::Critical) => "P1",
        Some(crate::models::AlertSeverity::Warning) => "P3",
        Some(crate::models::AlertSeverity::Info) => "P5",
        None => "P3",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertSeverity, AlertState};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert_named(n: &str, sev: AlertSeverity) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: n.into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            severity: sev,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: format!("fp-{n}"),
            tenant_id: "t".into(),
            generator_url: None,
        }
    }

    #[test]
    fn test_webhook_payload_shape() {
        let a = alert_named("X", AlertSeverity::Warning);
        let n = render_webhook("rcv", &WebhookConfig { url: "http://h".into(), send_resolved: true }, &[a], &[]);
        match &n.transport {
            Transport::Http { url, method, .. } => {
                assert_eq!(url, "http://h");
                assert_eq!(method, "POST");
            }
            _ => panic!("expected http"),
        }
        assert_eq!(n.body["status"], "firing");
        assert_eq!(n.body["receiver"], "rcv");
    }

    #[test]
    fn test_webhook_resolved_status() {
        let a = alert_named("X", AlertSeverity::Warning);
        let mut a = a;
        a.state = AlertState::Resolved;
        let n = render_webhook("rcv", &WebhookConfig { url: "http://h".into(), send_resolved: true }, &[], &[a]);
        assert_eq!(n.body["status"], "resolved");
    }

    #[test]
    fn test_slack_payload_has_blocks_and_channel() {
        let a = alert_named("X", AlertSeverity::Warning);
        let n = render_slack(
            "rcv",
            &SlackConfig {
                webhook_url: "http://x".into(),
                channel: "#alerts".into(),
                username: None,
                send_resolved: true,
            },
            &[a],
            &[],
        );
        assert_eq!(n.body["channel"], "#alerts");
        assert!(n.body["blocks"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_email_uses_smtp_transport() {
        let a = alert_named("X", AlertSeverity::Critical);
        let n = render_email(
            "rcv",
            &EmailConfig {
                to: vec!["ops@example.com".into()],
                from: "alerts@example.com".into(),
                smtp_host: "smtp.example.com".into(),
                smtp_port: 587,
                auth_username: None,
                auth_password: None,
                require_tls: true,
            },
            &[a],
            &[],
        );
        match &n.transport {
            Transport::Smtp { host, port, require_tls, .. } => {
                assert_eq!(host, "smtp.example.com");
                assert_eq!(*port, 587);
                assert!(*require_tls);
            }
            _ => panic!("expected smtp"),
        }
    }

    #[test]
    fn test_pagerduty_trigger_then_resolve() {
        let a = alert_named("X", AlertSeverity::Critical);
        let n_trigger = render_pagerduty("pd", &PagerDutyConfig { routing_key: "k".into(), send_resolved: true, severity_label: None }, &[a.clone()], &[]);
        assert_eq!(n_trigger.body["event_action"], "trigger");
        assert_eq!(n_trigger.body["payload"]["severity"], "critical");
        let mut resolved = a;
        resolved.state = AlertState::Resolved;
        let n_resolve = render_pagerduty("pd", &PagerDutyConfig { routing_key: "k".into(), send_resolved: true, severity_label: None }, &[], &[resolved]);
        assert_eq!(n_resolve.body["event_action"], "resolve");
    }

    #[test]
    fn test_opsgenie_close_url_for_resolve() {
        let a = alert_named("X", AlertSeverity::Critical);
        let n = render_opsgenie(
            "og",
            &OpsGenieConfig { api_key: "secret".into(), api_url: "https://api.opsgenie.com".into(), send_resolved: true },
            &[],
            &[a.clone()],
        );
        match &n.transport {
            Transport::Http { url, headers, .. } => {
                assert!(url.ends_with("/v2/alerts/fp-X/close?identifierType=alias"));
                let auth = headers.iter().find(|(k, _)| k == "Authorization").unwrap();
                assert_eq!(auth.1, "GenieKey secret");
            }
            _ => panic!("http expected"),
        }
    }

    #[test]
    fn test_opsgenie_priority_critical() {
        let a = alert_named("X", AlertSeverity::Critical);
        let n = render_opsgenie(
            "og",
            &OpsGenieConfig { api_key: "k".into(), api_url: "https://api.opsgenie.com".into(), send_resolved: true },
            &[a],
            &[],
        );
        assert_eq!(n.body["priority"], "P1");
    }

    #[test]
    fn test_grafana_oncall_alert_uid_set() {
        let a = alert_named("X", AlertSeverity::Warning);
        let n = render_grafana_oncall(
            "go",
            &GrafanaOnCallConfig { webhook_url: "http://x".into(), send_resolved: true },
            &[a],
            &[],
        );
        assert_eq!(n.body["state"], "alerting");
        assert_eq!(n.body["alert_uid"], "fp-X");
    }

    #[test]
    fn test_render_all_skips_resolved_when_send_resolved_false() {
        let a = alert_named("X", AlertSeverity::Warning);
        let mut resolved = a.clone();
        resolved.state = AlertState::Resolved;
        let receiver = Receiver::new("rcv").with_config(ReceiverConfig::Webhook(WebhookConfig {
            url: "http://x".into(),
            send_resolved: false,
        }));
        let out = render_all(&receiver, &[], &[resolved]);
        assert!(out.is_empty(), "should drop resolved-only when send_resolved=false");
    }

    #[test]
    fn test_render_all_includes_when_firing_present() {
        let a = alert_named("X", AlertSeverity::Warning);
        let receiver = Receiver::new("rcv").with_config(ReceiverConfig::Webhook(WebhookConfig {
            url: "http://x".into(),
            send_resolved: false,
        }));
        let out = render_all(&receiver, &[a], &[]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn test_alerts_to_array_includes_status_strings() {
        let mut a = alert_named("X", AlertSeverity::Warning);
        a.state = AlertState::Silenced;
        let v = alerts_to_array(&[], &[a]);
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["status"], "silenced");
    }
}
