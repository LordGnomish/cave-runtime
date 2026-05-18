// SPDX-License-Identifier: AGPL-3.0-or-later
//! Falco output channels — JSON formatting, HTTP webhook, gRPC-compatible type.

use crate::falco::engine::Alert;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Output format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Json,
    Text,
}

// ---------------------------------------------------------------------------
// gRPC-compatible alert envelope (mirrors Falco gRPC output.proto)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcAlert {
    pub rule: String,
    pub time: String,
    pub priority: String,
    pub source: String,
    pub tags: Vec<String>,
    pub output: String,
    pub output_fields: std::collections::HashMap<String, String>,
}

impl From<&Alert> for GrpcAlert {
    fn from(a: &Alert) -> Self {
        GrpcAlert {
            rule: a.rule_name.clone(),
            time: a.timestamp.to_rfc3339(),
            priority: a.priority.to_string(),
            source: a.source.clone(),
            tags: a.tags.clone(),
            output: a.output.clone(),
            output_fields: a.fields.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP webhook output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub min_priority: crate::falco::rule::Priority,
    #[serde(default)]
    pub custom_headers: std::collections::HashMap<String, String>,
}

pub struct WebhookOutput {
    pub config: WebhookConfig,
    pub client: reqwest::Client,
}

impl WebhookOutput {
    pub fn new(config: WebhookConfig) -> Self {
        WebhookOutput {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Send an alert to the configured webhook URL (fire-and-forget).
    pub async fn send(&self, alert: &Alert) -> anyhow::Result<()> {
        if alert.priority > self.config.min_priority {
            return Ok(()); // below threshold, skip
        }
        let payload = GrpcAlert::from(alert);
        let mut req = self.client.post(&self.config.url).json(&payload);
        for (k, v) in &self.config.custom_headers {
            req = req.header(k, v);
        }
        req.send().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Text formatter
// ---------------------------------------------------------------------------

pub fn format_alert_text(alert: &Alert) -> String {
    format!(
        "{} [{}] {} — {}",
        alert.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        alert.priority,   // uses Display → "WARNING", "CRITICAL", etc.
        alert.rule_name,
        alert.output,
    )
}

pub fn format_alert_json(alert: &Alert) -> String {
    serde_json::to_string(alert).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::falco::{engine::Alert, rule::Priority};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn dummy_alert() -> Alert {
        Alert {
            id: Uuid::new_v4(),
            rule_name: "Test rule".into(),
            priority: Priority::Warning,
            output: "A process did something".into(),
            source: "syscall".into(),
            tags: vec!["test".into()],
            fields: HashMap::new(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn grpc_alert_from_alert() {
        let a = dummy_alert();
        let g = GrpcAlert::from(&a);
        assert_eq!(g.rule, "Test rule");
        assert_eq!(g.priority, "WARNING");
    }

    #[test]
    fn format_text() {
        let a = dummy_alert();
        let txt = format_alert_text(&a);
        assert!(txt.contains("WARNING"));
        assert!(txt.contains("Test rule"));
    }

    #[test]
    fn format_json() {
        let a = dummy_alert();
        let json = format_alert_json(&a);
        assert!(json.contains("rule_name"));
    }
}
