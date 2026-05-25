// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification engine — fires subscribed targets on sync state transitions.
//!
//! MVP scope: Slack webhooks + generic JSON webhook + log-only stubs for
//! email/PagerDuty/MSTeams. Full template engine + retries/dedupe deferred to
//! cave-notify in Phase 2.

use crate::error::DeployError;
use crate::models::{
    Application, NotificationConfig, NotificationDestination, NotificationTrigger,
};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::{error, info};

pub struct NotificationEngine {
    client: Client,
    subscriptions: Vec<NotificationConfig>,
}

impl NotificationEngine {
    pub fn new(subscriptions: Vec<NotificationConfig>) -> Self {
        Self {
            client: Client::new(),
            subscriptions,
        }
    }

    pub fn subscriptions(&self) -> &[NotificationConfig] {
        &self.subscriptions
    }

    /// Fire all matching subscriptions for the trigger + app.
    pub async fn notify(&self, trigger: &NotificationTrigger, app: &Application, message: &str) {
        for sub in &self.subscriptions {
            if !sub.triggers.contains(trigger) {
                continue;
            }
            if let Err(e) = self.send(&sub.destination, app, message, trigger).await {
                error!(app = %app.name, trigger = ?trigger, error = %e, "Notification failed");
            }
        }
    }

    async fn send(
        &self,
        target: &NotificationDestination,
        app: &Application,
        message: &str,
        trigger: &NotificationTrigger,
    ) -> Result<(), DeployError> {
        match target {
            NotificationDestination::Slack { channel } => {
                let payload = build_slack_payload(app, message, slack_color(trigger), channel);
                let resp = self
                    .client
                    .post(slack_webhook_url(channel))
                    .json(&payload)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    return Err(DeployError::Notification(format!(
                        "Slack returned HTTP {}",
                        resp.status()
                    )));
                }
                info!(app = %app.name, channel = %channel, "Slack notification sent");
                Ok(())
            }
            NotificationDestination::Webhook { url } => {
                let payload = build_webhook_payload(app, message, trigger);
                let resp = self.client.post(url).json(&payload).send().await?;
                if !resp.status().is_success() {
                    return Err(DeployError::Notification(format!(
                        "Webhook {} returned HTTP {}",
                        url,
                        resp.status()
                    )));
                }
                info!(app = %app.name, url = %url, "Webhook notification sent");
                Ok(())
            }
            NotificationDestination::Email { addresses } => {
                info!(app = %app.name, ?addresses, "Email notification stubbed: {message}");
                Ok(())
            }
            NotificationDestination::MSTeams { webhook_url } => {
                info!(app = %app.name, url = %webhook_url, "MSTeams notification stubbed: {message}");
                Ok(())
            }
            NotificationDestination::PagerDuty { routing_key_ref } => {
                info!(app = %app.name, ref_ = %routing_key_ref, "PagerDuty notification stubbed: {message}");
                Ok(())
            }
        }
    }
}

/// Map a trigger to a Slack-attachment colour.
pub fn slack_color(trigger: &NotificationTrigger) -> &'static str {
    match trigger {
        NotificationTrigger::OnSyncSucceeded | NotificationTrigger::OnDeployed => "#36a64f",
        NotificationTrigger::OnSyncFailed | NotificationTrigger::OnHealthDegraded => "#ff0000",
        NotificationTrigger::OnSyncRunning => "#ffaa00",
        NotificationTrigger::Custom(_) => "#888888",
    }
}

/// In a real install the webhook URL is looked up from configuration; we keep
/// the channel-derived stub URL for test determinism.
fn slack_webhook_url(channel: &str) -> String {
    format!("https://hooks.slack.example/{}", channel.trim_start_matches('#'))
}

/// Build a Slack Block-Kit attachment payload.
pub fn build_slack_payload(
    app: &Application,
    message: &str,
    color: &str,
    channel: &str,
) -> Value {
    let project = &app.spec.project;
    let repo = &app.spec.source.repo_url;
    let (health, sync, revision) = match &app.status {
        Some(s) => (
            format!("{:?}", s.health.status),
            format!("{:?}", s.sync.status),
            s.sync.revision.clone(),
        ),
        None => ("Unknown".to_string(), "Unknown".to_string(), String::new()),
    };
    json!({
        "channel": channel,
        "attachments": [{
            "color": color,
            "title": format!("Application: {}", app.name),
            "text": message,
            "fields": [
                { "title": "Project",  "value": project,   "short": true },
                { "title": "Health",   "value": health,    "short": true },
                { "title": "Sync",     "value": sync,      "short": true },
                { "title": "Revision", "value": revision,  "short": true },
                { "title": "Repo",     "value": repo,      "short": false },
            ],
            "footer": "cave-deploy",
            "ts": chrono::Utc::now().timestamp(),
        }]
    })
}

fn build_webhook_payload(
    app: &Application,
    message: &str,
    trigger: &NotificationTrigger,
) -> Value {
    let (health, sync) = match &app.status {
        Some(s) => (format!("{:?}", s.health.status), format!("{:?}", s.sync.status)),
        None => ("Unknown".to_string(), "Unknown".to_string()),
    };
    json!({
        "app":       app.name,
        "project":   app.spec.project,
        "health":    health,
        "sync":      sync,
        "trigger":   format!("{trigger:?}"),
        "message":   message,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn dummy_app() -> Application {
        Application {
            id: Uuid::new_v4(),
            name: "myapp".into(),
            namespace: "argocd".into(),
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".into(),
                    target_revision: Some("main".into()),
                    path: Some("manifests/".into()),
                    helm: None,
                    kustomize: None,
                    directory: None,
                },
                sources: vec![],
                destination: Destination {
                    server: "https://kubernetes.default.svc".into(),
                    name: None,
                    namespace: "production".into(),
                },
                project: "myproject".into(),
                sync_policy: None,
                ignored_differences: None,
                info: None,
                revision_history_limit: None,
            },
            status: Some(ApplicationStatus {
                health: HealthCondition {
                    status: HealthStatus::Healthy,
                    message: None,
                },
                sync: SyncCondition {
                    status: SyncStatus::Synced,
                    revision: "abc123".into(),
                    revisions: vec![],
                },
                resources: vec![],
                history: vec![],
                conditions: vec![],
                observed_at: Some(Utc::now()),
                reconciled_at: Some(Utc::now()),
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: Default::default(),
            annotations: Default::default(),
            tracking: ResourceTracking::default(),
        }
    }

    #[test]
    fn test_slack_payload_structure() {
        let app = dummy_app();
        let payload = build_slack_payload(&app, "Sync succeeded", "#36a64f", "#deployments");
        assert!(payload["attachments"].is_array());
        let att = &payload["attachments"][0];
        assert_eq!(att["color"], "#36a64f");
        assert_eq!(att["title"], "Application: myapp");
        assert_eq!(payload["channel"], "#deployments");

        let fields = att["fields"].as_array().unwrap();
        let titles: Vec<&str> = fields.iter().filter_map(|f| f["title"].as_str()).collect();
        assert!(titles.contains(&"Project"));
        assert!(titles.contains(&"Health"));
        assert!(titles.contains(&"Sync"));
        assert!(titles.contains(&"Revision"));
    }

    #[test]
    fn test_engine_no_subscriptions_is_noop() {
        let engine = NotificationEngine::new(vec![]);
        assert_eq!(engine.subscriptions().len(), 0);
    }

    #[test]
    fn slack_color_succeeded_is_green() {
        assert_eq!(slack_color(&NotificationTrigger::OnSyncSucceeded), "#36a64f");
        assert_eq!(slack_color(&NotificationTrigger::OnSyncFailed), "#ff0000");
        assert_eq!(slack_color(&NotificationTrigger::OnSyncRunning), "#ffaa00");
    }

    #[test]
    fn webhook_payload_includes_trigger() {
        let app = dummy_app();
        let p = build_webhook_payload(&app, "ack", &NotificationTrigger::OnDeployed);
        assert_eq!(p["app"], "myapp");
        assert_eq!(p["project"], "myproject");
        assert!(p["trigger"].as_str().unwrap().contains("OnDeployed"));
    }
}
