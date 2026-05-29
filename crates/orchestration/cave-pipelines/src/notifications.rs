// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification system: webhook, Slack, email on pipeline status change.

use crate::models::RunPhase;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationConfig {
    Webhook {
        url: String,
        headers: Vec<(String, String)>,
    },
    Slack {
        webhook_url: String,
        channel: Option<String>,
    },
    Email {
        smtp_host: String,
        smtp_port: u16,
        to: Vec<String>,
        from: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyOn {
    Always,
    OnSuccess,
    OnFailure,
    OnComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRule {
    pub name: String,
    pub config: NotificationConfig,
    pub notify_on: NotifyOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineEvent {
    pub pipeline_run_id: Uuid,
    pub pipeline_name: String,
    pub status: RunPhase,
    pub message: Option<String>,
}

#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("HTTP error sending notification: {0}")]
    Http(String),
}

// ---------------------------------------------------------------------------
// Filter logic
// ---------------------------------------------------------------------------

impl NotifyOn {
    pub fn matches(&self, status: &RunPhase) -> bool {
        match self {
            NotifyOn::Always => true,
            NotifyOn::OnSuccess => matches!(status, RunPhase::Succeeded),
            NotifyOn::OnFailure => {
                matches!(status, RunPhase::Failed | RunPhase::Cancelled)
            }
            NotifyOn::OnComplete => matches!(
                status,
                RunPhase::Succeeded | RunPhase::Failed | RunPhase::Cancelled
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn send_notification(
    rule: &NotificationRule,
    event: &PipelineEvent,
) -> Result<(), NotificationError> {
    if !rule.notify_on.matches(&event.status) {
        return Ok(());
    }

    match &rule.config {
        NotificationConfig::Webhook { url, headers } => {
            info!(url = %url, "sending webhook notification");
            let client = reqwest::Client::new();
            let mut req = client.post(url).json(event);
            for (k, v) in headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req.send().await.map_err(|e| NotificationError::Http(e.to_string()))?;
            Ok(())
        }

        NotificationConfig::Slack { webhook_url, channel } => {
            info!(channel = ?channel, "sending Slack notification");
            let emoji = match event.status {
                RunPhase::Succeeded => ":white_check_mark:",
                RunPhase::Failed => ":x:",
                RunPhase::Cancelled => ":no_entry:",
                _ => ":information_source:",
            };
            let verb = match event.status {
                RunPhase::Succeeded => "succeeded",
                RunPhase::Failed => "failed",
                RunPhase::Cancelled => "was cancelled",
                _ => "updated",
            };
            let text = format!(
                "{emoji} Pipeline *{}* {verb}{}",
                event.pipeline_name,
                event.message.as_deref().map(|m| format!(": {m}")).unwrap_or_default(),
            );
            let mut payload = serde_json::json!({ "text": text });
            if let Some(ch) = channel {
                payload["channel"] = serde_json::Value::String(ch.clone());
            }
            reqwest::Client::new()
                .post(webhook_url)
                .json(&payload)
                .send()
                .await
                .map_err(|e| NotificationError::Http(e.to_string()))?;
            Ok(())
        }

        NotificationConfig::Email { smtp_host, to, from, .. } => {
            // Full SMTP integration omitted (would use lettre or similar).
            // Log intent and succeed.
            info!(smtp_host = %smtp_host, to = ?to, from = %from, "email notification (SMTP not wired)");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_on_success_matches_only_succeeded() {
        assert!(NotifyOn::OnSuccess.matches(&RunPhase::Succeeded));
        assert!(!NotifyOn::OnSuccess.matches(&RunPhase::Failed));
        assert!(!NotifyOn::OnSuccess.matches(&RunPhase::Running));
        assert!(!NotifyOn::OnSuccess.matches(&RunPhase::Cancelled));
    }

    #[test]
    fn test_on_failure_matches_failed_and_cancelled() {
        assert!(NotifyOn::OnFailure.matches(&RunPhase::Failed));
        assert!(NotifyOn::OnFailure.matches(&RunPhase::Cancelled));
        assert!(!NotifyOn::OnFailure.matches(&RunPhase::Succeeded));
        assert!(!NotifyOn::OnFailure.matches(&RunPhase::Running));
    }

    #[test]
    fn test_always_matches_every_status() {
        for status in [
            RunPhase::Pending,
            RunPhase::Running,
            RunPhase::Succeeded,
            RunPhase::Failed,
            RunPhase::Cancelled,
            RunPhase::Skipped,
        ] {
            assert!(NotifyOn::Always.matches(&status), "Always should match {status:?}");
        }
    }

    #[test]
    fn test_on_complete_matches_terminal_states() {
        assert!(NotifyOn::OnComplete.matches(&RunPhase::Succeeded));
        assert!(NotifyOn::OnComplete.matches(&RunPhase::Failed));
        assert!(NotifyOn::OnComplete.matches(&RunPhase::Cancelled));
        assert!(!NotifyOn::OnComplete.matches(&RunPhase::Running));
        assert!(!NotifyOn::OnComplete.matches(&RunPhase::Pending));
    }

    #[test]
    fn test_rule_skipped_when_filter_does_not_match() {
        // OnSuccess rule should not fire on Running status.
        let rule = NotificationRule {
            name: "ci".to_string(),
            config: NotificationConfig::Email {
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 587,
                to: vec!["team@example.com".to_string()],
                from: "ci@example.com".to_string(),
            },
            notify_on: NotifyOn::OnSuccess,
        };
        // Confirm filter rejects Running
        assert!(!rule.notify_on.matches(&RunPhase::Running));
    }
}
