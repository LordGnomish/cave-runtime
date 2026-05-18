// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notification engine — Slack webhooks, email stubs, and generic webhooks.
//!
//! Fires on sync status changes, health changes, and sync failures.

use crate::error::DeployError;
use crate::models::{Application, OperationPhase};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info};

// ─── Trigger conditions ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationTrigger {
    OnSyncSucceeded,
    OnSyncFailed,
    OnHealthDegraded,
    OnDeployed,
    OnSyncRunning,
}

// ─── Notification targets ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackTarget {
    pub webhook_url: String,
    pub channel: Option<String>,
    pub username: Option<String>,
    pub icon_emoji: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTarget {
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// HTTP method — defaults to POST.
    #[serde(default = "default_post")]
    pub method: String,
}

fn default_post() -> String {
    "POST".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailTarget {
    pub smtp_server: String,
    pub smtp_port: u16,
    pub from: String,
    pub to: Vec<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationTarget {
    Slack(SlackTarget),
    Webhook(WebhookTarget),
    Email(EmailTarget),
}

// ─── Subscription ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub triggers: Vec<NotificationTrigger>,
    pub target: NotificationTarget,
}

// ─── Engine ───────────────────────────────────────────────────────────────────

pub struct NotificationEngine {
    client: Client,
    subscriptions: Vec<Subscription>,
}

impl NotificationEngine {
    pub fn new(subscriptions: Vec<Subscription>) -> Self {
        Self { client: Client::new(), subscriptions }
    }

    /// Fire all matching subscriptions for the given trigger + app.
    pub async fn notify(
        &self,
        trigger: &NotificationTrigger,
        app: &Application,
        message: &str,
    ) {
        for sub in &self.subscriptions {
            if !sub.triggers.contains(trigger) {
                continue;
            }
            let result = self.send(&sub.target, app, message, trigger).await;
            if let Err(e) = result {
                error!(app = %app.name, trigger = ?trigger, error = %e, "Notification failed");
            }
        }
    }

    async fn send(
        &self,
        target: &NotificationTarget,
        app: &Application,
        message: &str,
        trigger: &NotificationTrigger,
    ) -> Result<(), DeployError> {
        match target {
            NotificationTarget::Slack(t) => self.send_slack(t, app, message, trigger).await,
            NotificationTarget::Webhook(t) => self.send_webhook(t, app, message).await,
            NotificationTarget::Email(_t) => {
                // Email notifications require SMTP — log and stub for now.
                info!(app = %app.name, "Email notification (stub): {message}");
                Ok(())
            }
        }
    }

    async fn send_slack(
        &self,
        target: &SlackTarget,
        app: &Application,
        message: &str,
        trigger: &NotificationTrigger,
    ) -> Result<(), DeployError> {
        let color = match trigger {
            NotificationTrigger::OnSyncSucceeded | NotificationTrigger::OnDeployed => "#36a64f",
            NotificationTrigger::OnSyncFailed | NotificationTrigger::OnHealthDegraded => "#ff0000",
            NotificationTrigger::OnSyncRunning => "#ffaa00",
        };

        let payload = build_slack_payload(app, message, color, target);
        let resp = self
            .client
            .post(&target.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(DeployError::Notification(format!(
                "Slack returned HTTP {}",
                resp.status()
            )));
        }
        info!(app = %app.name, channel = ?target.channel, "Slack notification sent");
        Ok(())
    }

    async fn send_webhook(
        &self,
        target: &WebhookTarget,
        app: &Application,
        message: &str,
    ) -> Result<(), DeployError> {
        let payload = json!({
            "app":     app.name,
            "project": app.spec.project,
            "health":  app.status.health.status,
            "sync":    app.status.sync.status,
            "message": message,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let mut req = self
            .client
            .request(
                target.method.parse().unwrap_or(reqwest::Method::POST),
                &target.url,
            )
            .json(&payload);

        for (k, v) in &target.headers {
            req = req.header(k, v);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DeployError::Notification(format!(
                "Webhook {} returned HTTP {}",
                target.url,
                resp.status()
            )));
        }
        info!(app = %app.name, url = %target.url, "Webhook notification sent");
        Ok(())
    }
}

// ─── Slack payload builder ────────────────────────────────────────────────────

/// Build a Slack Block Kit attachment payload.
pub fn build_slack_payload(
    app: &Application,
    message: &str,
    color: &str,
    target: &SlackTarget,
) -> Value {
    let health = &app.status.health.status;
    let sync = &app.status.sync.status;
    let project = &app.spec.project;
    let repo = &app.spec.source.repo_url;
    let revision = app.status.sync.revision.as_deref().unwrap_or("unknown");

    let mut body = json!({
        "attachments": [{
            "color": color,
            "title": format!("Application: {}", app.name),
            "text":  message,
            "fields": [
                { "title": "Project",  "value": project,  "short": true },
                { "title": "Health",   "value": health,   "short": true },
                { "title": "Sync",     "value": sync,     "short": true },
                { "title": "Revision", "value": revision, "short": true },
                { "title": "Repo",     "value": repo,     "short": false },
            ],
            "footer": "cave-deploy",
            "ts": chrono::Utc::now().timestamp(),
        }]
    });

    if let Some(channel) = &target.channel {
        body["channel"] = Value::String(channel.clone());
    }
    if let Some(username) = &target.username {
        body["username"] = Value::String(username.clone());
    }
    if let Some(icon) = &target.icon_emoji {
        body["icon_emoji"] = Value::String(icon.clone());
    }

    body
}

// ─── Okta SSO integration hooks ───────────────────────────────────────────────

/// Metadata used to attach an Okta identity to a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoContext {
    pub provider: String,
    pub user_email: String,
    pub groups: Vec<String>,
    pub access_token_hint: Option<String>,
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Application, ApplicationSpec, ApplicationSource, ApplicationDestination,
        ApplicationStatus, HealthStatusDetail, SyncStatusDetail};
    use chrono::Utc;
    use uuid::Uuid;

    fn dummy_app() -> Application {
        Application {
            id: Uuid::new_v4(),
            name: "myapp".to_string(),
            namespace: "argocd".to_string(),
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".to_string(),
                    path: Some("manifests/".to_string()),
                    target_revision: Some("main".to_string()),
                    ..Default::default()
                },
                destination: ApplicationDestination {
                    server: Some("https://kubernetes.default.svc".to_string()),
                    namespace: "production".to_string(),
                    ..Default::default()
                },
                project: "myproject".to_string(),
                ..Default::default()
            },
            status: ApplicationStatus {
                health: HealthStatusDetail {
                    status: "Healthy".to_string(),
                    message: None,
                },
                sync: SyncStatusDetail {
                    status: "Synced".to_string(),
                    revision: Some("abc123".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: None,
            finalizers: vec![],
        }
    }

    #[test]
    fn test_slack_payload_structure() {
        let app = dummy_app();
        let target = SlackTarget {
            webhook_url: "https://hooks.slack.com/services/XXX/YYY/ZZZ".to_string(),
            channel: Some("#deployments".to_string()),
            username: Some("cave-deploy".to_string()),
            icon_emoji: Some(":rocket:".to_string()),
        };
        let payload = build_slack_payload(&app, "Sync succeeded", "#36a64f", &target);

        // Verify structure
        assert!(payload["attachments"].is_array());
        let att = &payload["attachments"][0];
        assert_eq!(att["color"], "#36a64f");
        assert_eq!(att["title"], "Application: myapp");
        assert_eq!(payload["channel"], "#deployments");
        assert_eq!(payload["username"], "cave-deploy");

        // Fields present
        let fields = att["fields"].as_array().unwrap();
        let titles: Vec<&str> = fields.iter()
            .filter_map(|f| f["title"].as_str())
            .collect();
        assert!(titles.contains(&"Project"));
        assert!(titles.contains(&"Health"));
        assert!(titles.contains(&"Sync"));
        assert!(titles.contains(&"Revision"));
    }

    #[test]
    fn test_engine_no_subscriptions_is_noop() {
        let engine = NotificationEngine::new(vec![]);
        // Just verifies construction doesn't panic
        assert_eq!(engine.subscriptions.len(), 0);
    }
}
