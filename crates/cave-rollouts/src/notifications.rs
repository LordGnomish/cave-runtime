//! Notification payload construction for rollout events.

use crate::types::{NotificationChannel, NotificationEvent, Rollout, RolloutPhase};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct NotificationPayload {
    pub event: String,
    pub rollout_name: String,
    pub namespace: String,
    pub phase: String,
    pub message: Option<String>,
    pub timestamp: String,
}

impl NotificationPayload {
    pub fn from_rollout(event: &NotificationEvent, rollout: &Rollout) -> Self {
        Self {
            event: format!("{event:?}"),
            rollout_name: rollout.name.clone(),
            namespace: rollout.namespace.clone(),
            phase: format!("{:?}", rollout.phase),
            message: rollout.message.clone(),
            timestamp: rollout.updated_at.to_rfc3339(),
        }
    }
}

/// Build a Slack message block for a rollout event.
pub fn slack_message(payload: &NotificationPayload, channel: &str) -> serde_json::Value {
    let emoji = match payload.phase.as_str() {
        "Healthy" => ":white_check_mark:",
        "Degraded" => ":x:",
        "Paused" => ":pause_button:",
        _ => ":arrows_counterclockwise:",
    };
    serde_json::json!({
        "channel": channel,
        "text": format!("{emoji} *{}* — {} (`{}`)",
            payload.event, payload.rollout_name, payload.phase),
        "blocks": [
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!("{emoji} *Rollout Event: {}*\n*Name:* {} | *Phase:* `{}` | *Namespace:* {}",
                        payload.event, payload.rollout_name, payload.phase, payload.namespace)
                }
            }
        ]
    })
}

/// Build a generic webhook body.
pub fn webhook_body(payload: &NotificationPayload) -> serde_json::Value {
    serde_json::to_value(payload).unwrap_or_default()
}

/// Check whether a given event should trigger notification based on configured events.
pub fn should_notify(configured_events: &[NotificationEvent], event: &NotificationEvent) -> bool {
    configured_events
        .iter()
        .any(|e| std::mem::discriminant(e) == std::mem::discriminant(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CanaryStrategy, NotificationEvent, Rollout, RolloutStep, RolloutStrategy};

    fn promoted_rollout() -> Rollout {
        let mut r = Rollout::new(
            "my-app",
            "production",
            RolloutStrategy::Canary(CanaryStrategy {
                steps: vec![RolloutStep::SetWeight { weight: 100 }],
                analysis_template: None,
                max_surge: 1,
            }),
            "v1",
            "v2",
        );
        r.phase = RolloutPhase::Healthy;
        r.message = Some("Promoted successfully".into());
        r
    }

    #[test]
    fn test_notification_payload_from_rollout() {
        let rollout = promoted_rollout();
        let payload = NotificationPayload::from_rollout(&NotificationEvent::RolloutPromoted, &rollout);
        assert_eq!(payload.rollout_name, "my-app");
        assert_eq!(payload.namespace, "production");
        assert!(payload.phase.contains("Healthy"));
        assert!(payload.event.contains("Promoted"));
    }

    #[test]
    fn test_slack_message_contains_channel() {
        let rollout = promoted_rollout();
        let payload = NotificationPayload::from_rollout(&NotificationEvent::RolloutPromoted, &rollout);
        let msg = slack_message(&payload, "#deployments");
        assert_eq!(msg["channel"], "#deployments");
    }

    #[test]
    fn test_slack_message_healthy_emoji() {
        let rollout = promoted_rollout();
        let payload = NotificationPayload::from_rollout(&NotificationEvent::RolloutPromoted, &rollout);
        let msg = slack_message(&payload, "#ops");
        let text = msg["text"].as_str().unwrap_or("");
        assert!(text.contains(":white_check_mark:"));
    }

    #[test]
    fn test_webhook_body_serializes() {
        let rollout = promoted_rollout();
        let payload = NotificationPayload::from_rollout(&NotificationEvent::RolloutStarted, &rollout);
        let body = webhook_body(&payload);
        assert!(body["rollout_name"].as_str().is_some());
    }

    #[test]
    fn test_should_notify_matching_event() {
        let configured = vec![
            NotificationEvent::RolloutPromoted,
            NotificationEvent::RolloutRolledBack,
        ];
        assert!(should_notify(&configured, &NotificationEvent::RolloutPromoted));
        assert!(!should_notify(&configured, &NotificationEvent::RolloutStarted));
    }
}
