// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Publisher payload renderers.
//!
//! Mirrors `org.dependencytrack.notification.publisher.*`.  The Rust port
//! returns the rendered payload as a JSON `Value` (or `String` for email);
//! the actual `reqwest` post is wired in the route layer.

use async_trait::async_trait;
use serde_json::{Value, json};

/// Notification payload — what every publisher renders against.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationPayload<'a> {
    pub title: &'a str,
    pub level: &'a str,
    pub scope: &'a str,
    pub group: &'a str,
    pub message: &'a str,
    pub project: Option<&'a str>,
}

#[async_trait]
pub trait Publisher: Send + Sync {
    async fn publish(&self, payload: &NotificationPayload<'_>) -> Result<(), String>;
}

pub fn render_slack(p: &NotificationPayload<'_>) -> Value {
    json!({
        "text": format!("*{}*", p.title),
        "attachments": [{
            "color": color_for_level(p.level),
            "fields": [
                {"title":"Level","value":p.level,"short":true},
                {"title":"Scope","value":p.scope,"short":true},
                {"title":"Group","value":p.group,"short":true},
                {"title":"Project","value":p.project.unwrap_or("-"),"short":true},
                {"title":"Message","value":p.message,"short":false},
            ],
        }]
    })
}

pub fn render_teams(p: &NotificationPayload<'_>) -> Value {
    json!({
        "@type":"MessageCard",
        "@context":"https://schema.org/extensions",
        "summary": p.title,
        "themeColor": color_for_level(p.level).trim_start_matches('#'),
        "sections":[{
            "activityTitle": p.title,
            "facts":[
                {"name":"Level","value":p.level},
                {"name":"Scope","value":p.scope},
                {"name":"Group","value":p.group},
                {"name":"Project","value":p.project.unwrap_or("-")},
            ],
            "text": p.message
        }]
    })
}

pub fn render_mattermost(p: &NotificationPayload<'_>) -> Value {
    // Mattermost shares the Slack webhook contract.
    render_slack(p)
}

pub fn render_webhook(p: &NotificationPayload<'_>) -> Value {
    json!({
        "notification": {
            "level": p.level,
            "scope": p.scope,
            "group": p.group,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "title": p.title,
            "content": p.message,
            "subject": {
                "project": p.project.unwrap_or(""),
            }
        }
    })
}

pub fn render_email(p: &NotificationPayload<'_>) -> String {
    let project = p.project.unwrap_or("-");
    format!(
        "Subject: [{level}][{scope}] {title}\r\n\
         From: cave-dependency-track\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n\
         {title}\n\nGroup: {group}\nProject: {project}\n\n{message}\n",
        level = p.level,
        scope = p.scope,
        title = p.title,
        group = p.group,
        project = project,
        message = p.message,
    )
}

pub fn render_jira_issue(p: &NotificationPayload<'_>, project_key: &str) -> Value {
    let issuetype = match p.level {
        "ERROR" => "Bug",
        "WARNING" => "Task",
        _ => "Story",
    };
    json!({
        "fields": {
            "project": {"key": project_key},
            "summary": p.title,
            "description": format!("Level: {}\nScope: {}\nGroup: {}\nProject: {}\n\n{}",
                p.level, p.scope, p.group, p.project.unwrap_or("-"), p.message),
            "issuetype": {"name": issuetype},
        }
    })
}

fn color_for_level(level: &str) -> &'static str {
    match level {
        "ERROR" => "#e01e5a",
        "WARNING" => "#ecb22e",
        _ => "#36a64f",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> NotificationPayload<'static> {
        NotificationPayload {
            title: "New CVE",
            level: "ERROR",
            scope: "PORTFOLIO",
            group: "NEW_VULNERABILITY",
            message: "CVE-2026-1 affecting OpenSSL",
            project: Some("cave"),
        }
    }

    #[test]
    fn slack_includes_attachments_color_red_for_error() {
        let v = render_slack(&payload());
        assert_eq!(v["text"], "*New CVE*");
        assert_eq!(v["attachments"][0]["color"], "#e01e5a");
    }

    #[test]
    fn teams_message_card_schema() {
        let v = render_teams(&payload());
        assert_eq!(v["@type"], "MessageCard");
        assert_eq!(v["summary"], "New CVE");
    }

    #[test]
    fn mattermost_matches_slack_contract() {
        assert_eq!(render_mattermost(&payload()), render_slack(&payload()));
    }

    #[test]
    fn webhook_includes_timestamp_and_subject() {
        let v = render_webhook(&payload());
        assert!(v["notification"]["timestamp"].is_string());
        assert_eq!(v["notification"]["subject"]["project"], "cave");
    }

    #[test]
    fn email_includes_subject_header() {
        let s = render_email(&payload());
        assert!(s.starts_with("Subject: [ERROR][PORTFOLIO]"));
        assert!(s.contains("CVE-2026-1 affecting OpenSSL"));
    }

    #[test]
    fn jira_issuetype_maps_by_level() {
        let v = render_jira_issue(&payload(), "SEC");
        assert_eq!(v["fields"]["issuetype"]["name"], "Bug");
    }

    #[test]
    fn jira_summary_carries_title() {
        let v = render_jira_issue(&payload(), "SEC");
        assert_eq!(v["fields"]["summary"], "New CVE");
        assert_eq!(v["fields"]["project"]["key"], "SEC");
    }
}
