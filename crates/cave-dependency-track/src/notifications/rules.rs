// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification-rule store + matcher.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum NotificationLevel {
    Informational,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationScope {
    System,
    Portfolio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationTrigger {
    NewVulnerability,
    PolicyViolation,
    BomConsumed,
    BomProcessingFailed,
    ProjectAuditChange,
    UserCreated,
    UserDeleted,
    AnalyzerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PublisherKind {
    Slack,
    Teams,
    Mattermost,
    Email,
    Webhook,
    Jira,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationRule {
    pub uuid: Uuid,
    pub name: String,
    pub scope: NotificationScope,
    pub level: NotificationLevel,
    pub triggers: Vec<NotificationTrigger>,
    pub publisher: PublisherKind,
    pub publisher_config: String,
    pub project_filter: Vec<Uuid>,
    pub enabled: bool,
}

impl NotificationRule {
    pub fn matches(&self, t: NotificationTrigger, level: NotificationLevel, proj: Option<Uuid>) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.triggers.contains(&t) {
            return false;
        }
        if (level as u8) < (self.level as u8) {
            return false;
        }
        if !self.project_filter.is_empty() {
            return match proj {
                Some(p) => self.project_filter.contains(&p),
                None => false,
            };
        }
        true
    }
}

#[derive(Default)]
pub struct NotificationRuleStore {
    rules: RwLock<Vec<NotificationRule>>,
}

impl NotificationRuleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.rules.read().unwrap().len()
    }

    pub fn put(&self, r: NotificationRule) -> NotificationRule {
        let mut guard = self.rules.write().unwrap();
        if let Some(pos) = guard.iter().position(|x| x.uuid == r.uuid) {
            guard[pos] = r.clone();
        } else {
            guard.push(r.clone());
        }
        r
    }

    pub fn list(&self) -> Vec<NotificationRule> {
        self.rules.read().unwrap().clone()
    }

    pub fn delete(&self, uuid: Uuid) -> bool {
        let mut guard = self.rules.write().unwrap();
        let before = guard.len();
        guard.retain(|r| r.uuid != uuid);
        guard.len() != before
    }

    pub fn match_for(
        &self,
        t: NotificationTrigger,
        level: NotificationLevel,
        proj: Option<Uuid>,
    ) -> Vec<NotificationRule> {
        self.rules
            .read()
            .unwrap()
            .iter()
            .filter(|r| r.matches(t, level, proj))
            .cloned()
            .collect()
    }

    pub fn distinct_publishers(&self) -> HashSet<PublisherKind> {
        self.rules.read().unwrap().iter().map(|r| r.publisher).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        triggers: Vec<NotificationTrigger>,
        level: NotificationLevel,
        publisher: PublisherKind,
    ) -> NotificationRule {
        NotificationRule {
            uuid: Uuid::new_v4(),
            name: "r".into(),
            scope: NotificationScope::Portfolio,
            level,
            triggers,
            publisher,
            publisher_config: "{}".into(),
            project_filter: Vec::new(),
            enabled: true,
        }
    }

    #[test]
    fn matches_when_trigger_and_level_align() {
        let r = rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Warning,
            PublisherKind::Slack,
        );
        assert!(r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Warning, None));
        assert!(r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Error, None));
    }

    #[test]
    fn does_not_match_below_level_floor() {
        let r = rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Error,
            PublisherKind::Slack,
        );
        assert!(!r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Informational, None));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let mut r = rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Warning,
            PublisherKind::Slack,
        );
        r.enabled = false;
        assert!(!r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Error, None));
    }

    #[test]
    fn project_filter_restricts_scope() {
        let proj = Uuid::new_v4();
        let mut r = rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Warning,
            PublisherKind::Slack,
        );
        r.project_filter = vec![proj];
        assert!(r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Warning, Some(proj)));
        assert!(!r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Warning, Some(Uuid::new_v4())));
        assert!(!r.matches(NotificationTrigger::NewVulnerability, NotificationLevel::Warning, None));
    }

    #[test]
    fn store_put_replace_and_delete() {
        let s = NotificationRuleStore::new();
        let r = s.put(rule(vec![], NotificationLevel::Warning, PublisherKind::Email));
        let mut r2 = r.clone();
        r2.name = "renamed".into();
        s.put(r2);
        assert_eq!(s.count(), 1);
        assert_eq!(s.list()[0].name, "renamed");
        assert!(s.delete(r.uuid));
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn distinct_publishers_collects_set() {
        let s = NotificationRuleStore::new();
        s.put(rule(vec![], NotificationLevel::Warning, PublisherKind::Slack));
        s.put(rule(vec![], NotificationLevel::Warning, PublisherKind::Teams));
        s.put(rule(vec![], NotificationLevel::Warning, PublisherKind::Slack));
        assert_eq!(s.distinct_publishers().len(), 2);
    }

    #[test]
    fn match_for_aggregates_enabled_only() {
        let s = NotificationRuleStore::new();
        s.put(rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Warning,
            PublisherKind::Slack,
        ));
        let mut off = rule(
            vec![NotificationTrigger::NewVulnerability],
            NotificationLevel::Warning,
            PublisherKind::Email,
        );
        off.enabled = false;
        s.put(off);
        let m = s.match_for(NotificationTrigger::NewVulnerability, NotificationLevel::Error, None);
        assert_eq!(m.len(), 1);
    }
}
