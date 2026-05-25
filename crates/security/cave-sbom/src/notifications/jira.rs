// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/notification/publisher/JiraPublisher.java
//
//! Jira publisher — trait + payload builder.

use super::{Notification, NotificationLevel, PublishError, Publisher};
use async_trait::async_trait;
use std::sync::Mutex;

/// Build the Jira REST API v2 `/issue` payload. Mirrors
/// `JiraPublisher.prepareTemplate`.
pub fn jira_issue_payload(project_key: &str, n: &Notification) -> serde_json::Value {
    let priority = match n.level {
        NotificationLevel::Error => "Highest",
        NotificationLevel::Warning => "Medium",
        NotificationLevel::Informational => "Low",
    };
    serde_json::json!({
        "fields": {
            "project": { "key": project_key },
            "summary": n.title,
            "description": n.content,
            "issuetype": { "name": "Task" },
            "priority": { "name": priority },
            "labels": [ format!("{:?}", n.group).to_lowercase() ]
        }
    })
}

/// In-memory Jira sink — records every issue-create-equivalent call.
pub struct InMemoryJira {
    pub project_key: String,
    pub created: Mutex<Vec<serde_json::Value>>,
}

impl InMemoryJira {
    pub fn new(project_key: impl Into<String>) -> Self {
        Self {
            project_key: project_key.into(),
            created: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Publisher for InMemoryJira {
    async fn publish(&self, n: &Notification) -> Result<(), PublishError> {
        let payload = jira_issue_payload(&self.project_key, n);
        self.created.lock().unwrap().push(payload);
        Ok(())
    }
    fn kind_name(&self) -> &'static str {
        "InMemoryJira"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{Notification, NotificationGroup, NotificationLevel};

    fn n(level: NotificationLevel) -> Notification {
        Notification {
            group: NotificationGroup::PolicyViolation,
            level,
            title: "Violation".into(),
            content: "deny-list hit".into(),
            payload: None,
        }
    }

    #[test]
    fn payload_includes_project_key_and_summary() {
        let p = jira_issue_payload("CAVE", &n(NotificationLevel::Warning));
        assert_eq!(p["fields"]["project"]["key"], "CAVE");
        assert_eq!(p["fields"]["summary"], "Violation");
        assert_eq!(p["fields"]["priority"]["name"], "Medium");
    }

    #[test]
    fn priority_maps_from_level() {
        assert_eq!(
            jira_issue_payload("X", &n(NotificationLevel::Error))["fields"]["priority"]["name"],
            "Highest"
        );
        assert_eq!(
            jira_issue_payload("X", &n(NotificationLevel::Informational))["fields"]["priority"]["name"],
            "Low"
        );
    }

    #[tokio::test]
    async fn in_memory_records_issue_create() {
        let j = InMemoryJira::new("CAVE");
        j.publish(&n(NotificationLevel::Error)).await.unwrap();
        assert_eq!(j.created.lock().unwrap().len(), 1);
    }
}
