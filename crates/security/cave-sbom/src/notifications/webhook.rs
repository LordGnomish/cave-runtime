// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/notification/publisher/WebhookPublisher.java
//   src/main/java/org/dependencytrack/notification/publisher/SlackPublisher.java
//   src/main/java/org/dependencytrack/notification/publisher/MsTeamsPublisher.java
//
//! Webhook publisher trait + Slack / Teams payload formatters.

use super::{Notification, NotificationLevel};

/// Format a Notification as a Slack Block Kit payload. Mirrors
/// `SlackPublisher.prepareTemplate`.
pub fn slack_payload(n: &Notification) -> serde_json::Value {
    let color = match n.level {
        NotificationLevel::Error => "#d63333",
        NotificationLevel::Warning => "#f0b400",
        NotificationLevel::Informational => "#36a64f",
    };
    serde_json::json!({
        "attachments": [{
            "color": color,
            "title": n.title,
            "text": n.content,
            "fields": [
                { "title": "Group", "value": format!("{:?}", n.group), "short": true },
                { "title": "Level", "value": format!("{:?}", n.level), "short": true }
            ]
        }]
    })
}

/// Format a Notification as a Microsoft Teams MessageCard. Mirrors
/// `MsTeamsPublisher.prepareTemplate`.
pub fn teams_payload(n: &Notification) -> serde_json::Value {
    let theme_color = match n.level {
        NotificationLevel::Error => "D63333",
        NotificationLevel::Warning => "F0B400",
        NotificationLevel::Informational => "36A64F",
    };
    serde_json::json!({
        "@type": "MessageCard",
        "@context": "http://schema.org/extensions",
        "themeColor": theme_color,
        "summary": n.title,
        "title": n.title,
        "text": n.content
    })
}

/// Generic JSON webhook payload — Dependency-Track's default.
pub fn webhook_payload(n: &Notification) -> serde_json::Value {
    serde_json::json!({
        "notification": {
            "level": format!("{:?}", n.level).to_uppercase(),
            "scope": "PORTFOLIO",
            "group": format!("{:?}", n.group),
            "title": n.title,
            "content": n.content,
            "subject": n.payload.clone().unwrap_or(serde_json::Value::Null),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{Notification, NotificationGroup, NotificationLevel};

    fn n() -> Notification {
        Notification {
            group: NotificationGroup::NewVulnerability,
            level: NotificationLevel::Error,
            title: "Critical CVE".into(),
            content: "CVE-2024-1 in openssl".into(),
            payload: None,
        }
    }

    #[test]
    fn slack_payload_includes_color_and_fields() {
        let p = slack_payload(&n());
        assert_eq!(p["attachments"][0]["color"], "#d63333");
        assert!(p["attachments"][0]["fields"].is_array());
        assert_eq!(p["attachments"][0]["title"], "Critical CVE");
    }

    #[test]
    fn teams_payload_includes_message_card_envelope() {
        let p = teams_payload(&n());
        assert_eq!(p["@type"], "MessageCard");
        assert_eq!(p["themeColor"], "D63333");
        assert_eq!(p["title"], "Critical CVE");
    }

    #[test]
    fn webhook_payload_uses_dt_envelope() {
        let p = webhook_payload(&n());
        assert_eq!(p["notification"]["level"], "ERROR");
        assert_eq!(p["notification"]["group"], "NewVulnerability");
    }

    #[test]
    fn slack_color_changes_by_level() {
        let mut x = n();
        x.level = NotificationLevel::Warning;
        assert_eq!(slack_payload(&x)["attachments"][0]["color"], "#f0b400");
        x.level = NotificationLevel::Informational;
        assert_eq!(slack_payload(&x)["attachments"][0]["color"], "#36a64f");
    }
}
