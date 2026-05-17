// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/notification/NotificationRouter.java
//   src/main/java/org/dependencytrack/notification/publisher/{SlackPublisher,MsTeamsPublisher,SendMailPublisher,JiraPublisher,WebhookPublisher}.java
//
//! Pluggable notification sinks — Webhook / Slack / Teams / Email / Jira.

pub mod jira;
pub mod mail;
pub mod webhook;

use crate::policy::PolicyViolation;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

/// Mirror of `org.dependencytrack.notification.NotificationGroup`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationGroup {
    BomConsumed,
    BomProcessed,
    NewVulnerability,
    PolicyViolation,
    ProjectAuditChange,
    UserCreated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationLevel {
    Informational,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notification {
    pub group: NotificationGroup,
    pub level: NotificationLevel,
    pub title: String,
    pub content: String,
    /// Optional serialized payload (the offending PolicyViolation, the CVE, etc.).
    pub payload: Option<serde_json::Value>,
}

impl Notification {
    pub fn from_policy_violation(v: &PolicyViolation) -> Self {
        Self {
            group: NotificationGroup::PolicyViolation,
            level: match v.violation_state {
                crate::policy::ViolationState::Fail => NotificationLevel::Error,
                crate::policy::ViolationState::Warn => NotificationLevel::Warning,
                crate::policy::ViolationState::Info => NotificationLevel::Informational,
            },
            title: format!("Policy violation: {}", v.policy_name),
            content: v.message.clone(),
            payload: Some(serde_json::to_value(v).unwrap_or(serde_json::Value::Null)),
        }
    }
}

/// Mirror of `org.dependencytrack.model.NotificationRule`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotificationRule {
    pub uuid: Uuid,
    pub name: String,
    pub enabled: bool,
    pub notify_on: Vec<NotificationGroup>,
    pub min_level: NotificationLevel,
    pub publisher: PublisherKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum PublisherKind {
    Webhook { url: String },
    Slack { webhook_url: String },
    Teams { webhook_url: String },
    Email { to: String },
    Jira { base_url: String, project_key: String },
    Console,
}

#[async_trait]
pub trait Publisher: Send + Sync {
    async fn publish(&self, n: &Notification) -> Result<(), PublishError>;
    fn kind_name(&self) -> &'static str;
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("rejected by remote: {0}")]
    Rejected(String),
}

/// In-memory publisher for tests and the `Console` sink — records every call.
pub struct ConsolePublisher {
    pub seen: Mutex<Vec<Notification>>,
}

impl Default for ConsolePublisher {
    fn default() -> Self {
        Self { seen: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl Publisher for ConsolePublisher {
    async fn publish(&self, n: &Notification) -> Result<(), PublishError> {
        self.seen.lock().unwrap().push(n.clone());
        Ok(())
    }
    fn kind_name(&self) -> &'static str {
        "Console"
    }
}

/// Should a rule fire for this notification? Mirrors
/// `NotificationRouter.resolveRules(Notification)` filter logic.
pub fn rule_matches(rule: &NotificationRule, n: &Notification) -> bool {
    if !rule.enabled {
        return false;
    }
    if !rule.notify_on.contains(&n.group) {
        return false;
    }
    level_rank(n.level) >= level_rank(rule.min_level)
}

fn level_rank(l: NotificationLevel) -> u8 {
    match l {
        NotificationLevel::Informational => 0,
        NotificationLevel::Warning => 1,
        NotificationLevel::Error => 2,
    }
}

/// Dispatch a notification through every matching rule. Returns `(matches, errors)`.
pub async fn dispatch(
    rules: &[NotificationRule],
    n: &Notification,
    pub_for: impl Fn(&PublisherKind) -> Box<dyn Publisher>,
) -> (usize, Vec<PublishError>) {
    let mut matches = 0;
    let mut errs = Vec::new();
    for r in rules.iter().filter(|r| rule_matches(r, n)) {
        matches += 1;
        let publisher = pub_for(&r.publisher);
        if let Err(e) = publisher.publish(n).await {
            errs.push(e);
        }
    }
    (matches, errs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ViolationState;

    fn console_rule() -> NotificationRule {
        NotificationRule {
            uuid: Uuid::new_v4(),
            name: "console".into(),
            enabled: true,
            notify_on: vec![NotificationGroup::PolicyViolation],
            min_level: NotificationLevel::Warning,
            publisher: PublisherKind::Console,
        }
    }

    fn n_warn() -> Notification {
        Notification {
            group: NotificationGroup::PolicyViolation,
            level: NotificationLevel::Warning,
            title: "t".into(),
            content: "c".into(),
            payload: None,
        }
    }

    #[test]
    fn rule_matches_on_group_and_level() {
        assert!(rule_matches(&console_rule(), &n_warn()));
    }

    #[test]
    fn rule_disabled_never_matches() {
        let mut r = console_rule();
        r.enabled = false;
        assert!(!rule_matches(&r, &n_warn()));
    }

    #[test]
    fn rule_below_min_level_does_not_match() {
        let r = console_rule();
        let mut n = n_warn();
        n.level = NotificationLevel::Informational;
        assert!(!rule_matches(&r, &n));
    }

    #[test]
    fn rule_other_group_does_not_match() {
        let r = console_rule();
        let mut n = n_warn();
        n.group = NotificationGroup::BomConsumed;
        assert!(!rule_matches(&r, &n));
    }

    #[tokio::test]
    async fn dispatch_invokes_matching_publishers() {
        use std::sync::Arc;
        let console = Arc::new(ConsolePublisher::default());
        let rules = vec![console_rule(), console_rule()];
        let console_for_dispatch = console.clone();
        let (m, _e) = dispatch(&rules, &n_warn(), |_k| {
            Box::new(SharedConsole(console_for_dispatch.clone())) as Box<dyn Publisher>
        })
        .await;
        assert_eq!(m, 2);
        assert_eq!(console.seen.lock().unwrap().len(), 2);
    }

    struct SharedConsole(std::sync::Arc<ConsolePublisher>);

    #[async_trait]
    impl Publisher for SharedConsole {
        async fn publish(&self, n: &Notification) -> Result<(), PublishError> {
            self.0.publish(n).await
        }
        fn kind_name(&self) -> &'static str {
            "Console"
        }
    }

    #[test]
    fn from_policy_violation_maps_level() {
        let v = PolicyViolation {
            policy_uuid: Uuid::new_v4(),
            policy_name: "P".into(),
            component_uuid: Uuid::new_v4(),
            condition_index: 0,
            violation_state: ViolationState::Fail,
            message: "boom".into(),
        };
        let n = Notification::from_policy_violation(&v);
        assert_eq!(n.level, NotificationLevel::Error);
        assert_eq!(n.group, NotificationGroup::PolicyViolation);
    }
}
