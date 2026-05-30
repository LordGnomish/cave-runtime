// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-source webhook receivers — a faithful port of grafana/oncall
//! `engine/config_integrations/` (v1.10.0).
//!
//! Upstream encodes each integration as a set of Jinja templates over the raw
//! request `payload`. The four fields that actually drive alert-group routing
//! are pure data transforms, independent of the presentation templates:
//!
//!   * `grouping_id`      — how repeated pages collapse into one alert group
//!   * `resolve_condition`— when an incoming page auto-resolves the group
//!   * `source_link`      — deep link back to the upstream alert
//!   * `web_title`        — human title for the alert group
//!
//! This module ports those transforms for the AlertManager (Prometheus),
//! Grafana Alerting (unified), Grafana Legacy Alerting, Formatted webhook, and
//! generic Webhook receivers, normalizing every source into an
//! [`IncomingAlert`].

use crate::models::{Alert, AlertState, Severity};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The set of upstream integration slugs cave-oncall can normalize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationType {
    /// `config_integrations/alertmanager.py` — external Prometheus Alertmanager.
    Alertmanager,
    /// `config_integrations/grafana_alerting.py` — Grafana unified alerting
    /// (Alertmanager-shaped payload).
    GrafanaAlerting,
    /// `config_integrations/grafana.py` — Grafana legacy alerting.
    GrafanaLegacy,
    /// `config_integrations/formatted_webhook.py`.
    FormattedWebhook,
    /// `config_integrations/webhook.py` — generic catch-all.
    Webhook,
}

impl IntegrationType {
    /// The upstream `slug` constant for this integration.
    pub fn slug(self) -> &'static str {
        match self {
            IntegrationType::Alertmanager => "alertmanager",
            IntegrationType::GrafanaAlerting => "grafana_alerting",
            IntegrationType::GrafanaLegacy => "grafana",
            IntegrationType::FormattedWebhook => "formatted_webhook",
            IntegrationType::Webhook => "webhook",
        }
    }

    /// Every integration slug cave-oncall can dispatch to.
    pub fn all() -> [IntegrationType; 5] {
        [
            IntegrationType::Alertmanager,
            IntegrationType::GrafanaAlerting,
            IntegrationType::GrafanaLegacy,
            IntegrationType::FormattedWebhook,
            IntegrationType::Webhook,
        ]
    }
}

/// Errors raised while normalizing an incoming webhook payload.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IntegrationError {
    /// The slug does not map to any supported integration.
    #[error("unknown integration slug: {0}")]
    UnknownSlug(String),
}

/// A monitoring payload normalized into cave-oncall's canonical shape,
/// independent of which upstream integration produced it.
#[derive(Debug, Clone)]
pub struct IncomingAlert {
    pub integration: IntegrationType,
    pub title: String,
    pub message: String,
    pub image_url: Option<String>,
    pub source_link: Option<String>,
    pub grouping_id: String,
    pub is_resolved: bool,
    pub severity: Severity,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

impl IncomingAlert {
    /// Project the normalized alert into a storable [`Alert`]. The
    /// `grouping_id` becomes the dedupe fingerprint (matching upstream, where
    /// the grouping template feeds AlertGroup deduplication).
    pub fn to_alert(&self, team_id: Uuid) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            team_id,
            title: self.title.clone(),
            severity: self.severity.clone(),
            source: self.integration.slug().to_string(),
            fingerprint: self.grouping_id.clone(),
            state: if self.is_resolved {
                AlertState::Resolved
            } else {
                AlertState::Firing
            },
            labels: self.labels.clone(),
            annotations: self.annotations.clone(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: if self.is_resolved { Some(Utc::now()) } else { None },
            escalation_policy_id: None,
            current_escalation_step: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Field helpers (mirror the Jinja `payload.get(...)` accessors)
// ---------------------------------------------------------------------------

/// `payload.get(key)` when it is a non-empty string — upstream templates treat
/// an empty string as falsy in their `a or b` fallbacks.
fn truthy_str(v: &Value, key: &str) -> Option<String> {
    match v.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

/// `payload.get(key, "")` — present string or empty.
fn str_or_empty(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Flatten a JSON object of string values into a map (non-string values skipped).
fn string_map(v: &Value, key: &str) -> HashMap<String, String> {
    v.get(key)
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Map a monitoring `severity` label onto cave-oncall's [`Severity`] ladder.
fn severity_from_label(s: &str) -> Severity {
    match s.to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" | "error" => Severity::High,
        "warning" | "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Info,
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Normalize `payload` using the integration identified by `slug`.
pub fn parse_for(slug: &str, payload: &Value) -> Result<IncomingAlert, IntegrationError> {
    match slug {
        "alertmanager" => Ok(parse_alertmanager(payload, IntegrationType::Alertmanager)),
        "grafana_alerting" => Ok(parse_alertmanager(payload, IntegrationType::GrafanaAlerting)),
        "grafana" => Ok(parse_grafana_legacy(payload)),
        "formatted_webhook" => Ok(parse_formatted_webhook(payload)),
        "webhook" => Ok(parse_generic_webhook(payload)),
        other => Err(IntegrationError::UnknownSlug(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Alertmanager / Grafana Alerting (shared, `based_on_alertmanager = True`)
// ---------------------------------------------------------------------------

/// Port of `alertmanager.py` / `grafana_alerting.py`:
///   * `grouping_id = payload.groupKey`
///   * `resolve_condition = payload.status == "resolved"`
///   * `source_link = payload.alerts[0].generatorURL`
///   * `web_title`: pop `alertname` from `groupLabels` (or read legacy
///     `labels.alertname`), then append remaining groupLabel values in parens.
pub fn parse_alertmanager(payload: &Value, kind: IntegrationType) -> IncomingAlert {
    let grouping_id = str_or_empty(payload, "groupKey");
    let is_resolved = payload.get("status").and_then(Value::as_str) == Some("resolved");

    let source_link = payload
        .get("alerts")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|first| truthy_str(first, "generatorURL"));

    let mut group_labels = string_map(payload, "groupLabels");
    let common_labels = string_map(payload, "commonLabels");

    // web_title — legacy single-alert payloads carry top-level `labels`.
    let title = if payload.get("labels").is_some() {
        string_map(payload, "labels")
            .get("alertname")
            .cloned()
            .unwrap_or_default()
    } else {
        let alertname = group_labels.remove("alertname").unwrap_or_default();
        if group_labels.is_empty() {
            alertname
        } else {
            let mut rest: Vec<&String> = group_labels.values().collect();
            rest.sort();
            let joined = rest
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            format!("{alertname} ({joined})")
        }
    };

    let severity = group_labels
        .get("severity")
        .or_else(|| common_labels.get("severity"))
        .map(|s| severity_from_label(s))
        .unwrap_or(Severity::Info);

    IncomingAlert {
        integration: kind,
        title,
        message: String::new(),
        image_url: None,
        source_link,
        grouping_id,
        is_resolved,
        severity,
        labels: if common_labels.is_empty() {
            string_map(payload, "labels")
        } else {
            common_labels
        },
        annotations: string_map(payload, "commonAnnotations"),
    }
}

/// Convenience wrapper preserving the upstream slug split.
pub fn parse_grafana_alerting(payload: &Value) -> IncomingAlert {
    parse_alertmanager(payload, IntegrationType::GrafanaAlerting)
}

// ---------------------------------------------------------------------------
// Grafana Legacy Alerting (`grafana.py`)
// ---------------------------------------------------------------------------

/// Port of `grafana.py`:
///   * `source_link = payload.ruleUrl or payload.generatorURL`
///   * `grouping_id = payload.ruleName or payload.labels.alertname or <default>`
///   * `resolve_condition = state == "ok" or status == "resolved"`
pub fn parse_grafana_legacy(payload: &Value) -> IncomingAlert {
    const NO_TITLE: &str = "No title (check Web Title Template)";

    let source_link = truthy_str(payload, "ruleUrl").or_else(|| truthy_str(payload, "generatorURL"));

    let labels = string_map(payload, "labels");
    let alertname = labels.get("alertname").cloned();
    let grouping_id = truthy_str(payload, "ruleName")
        .or_else(|| alertname.clone())
        .unwrap_or_else(|| NO_TITLE.to_string());

    let is_resolved = payload.get("state").and_then(Value::as_str) == Some("ok")
        || payload.get("status").and_then(Value::as_str) == Some("resolved");

    let title = truthy_str(payload, "ruleName")
        .or(alertname)
        .or_else(|| truthy_str(payload, "title"))
        .unwrap_or_else(|| NO_TITLE.to_string());

    let severity = labels
        .get("severity")
        .map(|s| severity_from_label(s))
        .unwrap_or(Severity::Info);

    IncomingAlert {
        integration: IntegrationType::GrafanaLegacy,
        title,
        message: str_or_empty(payload, "message"),
        image_url: truthy_str(payload, "imageUrl"),
        source_link,
        grouping_id,
        is_resolved,
        severity,
        labels,
        annotations: string_map(payload, "annotations"),
    }
}

// ---------------------------------------------------------------------------
// Formatted webhook (`formatted_webhook.py`)
// ---------------------------------------------------------------------------

/// Port of `formatted_webhook.py`:
///   * `web_title = payload.title`
///   * `grouping_id = payload.alert_uid`
///   * `resolve_condition = payload.state.upper() == "OK"`
///   * `source_link = payload.link_to_upstream_details`
pub fn parse_formatted_webhook(payload: &Value) -> IncomingAlert {
    let title = truthy_str(payload, "title")
        .unwrap_or_else(|| "Title undefined (Check Web Title Template)".to_string());
    let grouping_id = str_or_empty(payload, "alert_uid");
    let is_resolved = str_or_empty(payload, "state").to_ascii_uppercase() == "OK";

    IncomingAlert {
        integration: IntegrationType::FormattedWebhook,
        title,
        message: str_or_empty(payload, "message"),
        image_url: truthy_str(payload, "image_url"),
        source_link: truthy_str(payload, "link_to_upstream_details"),
        grouping_id,
        is_resolved,
        severity: Severity::Info,
        labels: HashMap::new(),
        annotations: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Generic webhook (`webhook.py`)
// ---------------------------------------------------------------------------

/// Port of `webhook.py`:
///   * `web_title = "Incident"`
///   * `resolve_condition = payload.state.upper() == "OK"`
///   * `source_link = payload.url`
///   * `grouping_id`: `payload.alert_uid` for OnCall heartbeats, otherwise the
///     entire payload (so distinct payloads never collapse together).
pub fn parse_generic_webhook(payload: &Value) -> IncomingAlert {
    let is_resolved = str_or_empty(payload, "state").to_ascii_uppercase() == "OK";

    let grouping_id = if payload.get("is_oncall_heartbeat").is_some() {
        str_or_empty(payload, "alert_uid")
    } else {
        serde_json::to_string(payload).unwrap_or_default()
    };

    IncomingAlert {
        integration: IntegrationType::Webhook,
        title: "Incident".to_string(),
        message: str_or_empty(payload, "message"),
        image_url: None,
        source_link: truthy_str(payload, "url"),
        grouping_id,
        is_resolved,
        severity: Severity::Info,
        labels: HashMap::new(),
        annotations: HashMap::new(),
    }
}
