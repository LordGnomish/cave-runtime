// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notification routing rules — DefectDojo `dojo/notifications/views.py`
//! exposes a per-user/per-product matrix of "which events trigger which
//! channels" with optional minimum-severity gating. We mirror that here as
//! a pure `Vec<NotificationRule>` engine that filters `NotificationEvent`s
//! to a set of `Channel`s.

use crate::finding::FindingSeverity;
use crate::notifications::{EventKind, NotificationEvent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    InApp,
    Slack,
    Teams,
    Email,
    Webhook,
    Jira,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRule {
    pub name: String,
    /// Empty = match all event kinds.
    #[serde(default)]
    pub event_kinds: Vec<EventKind>,
    /// Minimum severity (inclusive). `None` = no severity gate.
    #[serde(default)]
    pub min_severity: Option<FindingSeverity>,
    /// Product name filter (substring match). Empty = match all.
    #[serde(default)]
    pub product_filter: Option<String>,
    /// Routed channels when the rule fires.
    pub channels: Vec<Channel>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl NotificationRule {
    pub fn matches(&self, event: &NotificationEvent) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.event_kinds.is_empty() && !self.event_kinds.contains(&event.kind) {
            return false;
        }
        if let Some(min) = &self.min_severity {
            if event.severity.weight() < min.weight() {
                return false;
            }
        }
        if let Some(p) = &self.product_filter {
            match &event.product {
                Some(name) if name.contains(p) => {}
                _ => return false,
            }
        }
        true
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationRuleSet {
    pub rules: Vec<NotificationRule>,
}

impl NotificationRuleSet {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, rule: NotificationRule) {
        self.rules.push(rule);
    }
    /// Return every channel that fires for `event` (deduplicated, stable order).
    pub fn route(&self, event: &NotificationEvent) -> Vec<Channel> {
        let mut out = Vec::new();
        for r in &self.rules {
            if r.matches(event) {
                for c in &r.channels {
                    if !out.contains(c) {
                        out.push(c.clone());
                    }
                }
            }
        }
        out
    }
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn evt(kind: EventKind, sev: FindingSeverity, product: Option<&str>) -> NotificationEvent {
        NotificationEvent {
            kind,
            finding_id: Uuid::nil(),
            title: "t".to_string(),
            severity: sev,
            product: product.map(String::from),
            url: None,
            message: String::new(),
        }
    }

    #[test]
    fn match_all_event_kinds_when_empty_list() {
        let r = NotificationRule {
            name: "all".into(),
            event_kinds: vec![],
            min_severity: None,
            product_filter: None,
            channels: vec![Channel::Slack],
            enabled: true,
        };
        let e = evt(EventKind::FindingCreated, FindingSeverity::Low, None);
        assert!(r.matches(&e));
    }

    #[test]
    fn specific_event_kind_filter() {
        let r = NotificationRule {
            name: "sla".into(),
            event_kinds: vec![EventKind::SlaBreached],
            min_severity: None,
            product_filter: None,
            channels: vec![Channel::Teams],
            enabled: true,
        };
        assert!(r.matches(&evt(EventKind::SlaBreached, FindingSeverity::Low, None)));
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None)));
    }

    #[test]
    fn severity_gate_inclusive() {
        let r = NotificationRule {
            name: "high+".into(),
            event_kinds: vec![],
            min_severity: Some(FindingSeverity::High),
            product_filter: None,
            channels: vec![Channel::Email],
            enabled: true,
        };
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Medium, None)));
        assert!(r.matches(&evt(EventKind::FindingCreated, FindingSeverity::High, None)));
        assert!(r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None)));
    }

    #[test]
    fn product_filter_substring_match() {
        let r = NotificationRule {
            name: "p".into(),
            event_kinds: vec![],
            min_severity: None,
            product_filter: Some("acme".into()),
            channels: vec![Channel::Webhook],
            enabled: true,
        };
        assert!(r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Low, Some("acme-portal"))));
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Low, Some("other"))));
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Low, None)));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let r = NotificationRule {
            name: "x".into(),
            event_kinds: vec![],
            min_severity: None,
            product_filter: None,
            channels: vec![Channel::Slack],
            enabled: false,
        };
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None)));
    }

    #[test]
    fn route_deduplicates_channels() {
        let mut set = NotificationRuleSet::new();
        set.push(NotificationRule {
            name: "a".into(),
            event_kinds: vec![],
            min_severity: None,
            product_filter: None,
            channels: vec![Channel::Slack, Channel::Email],
            enabled: true,
        });
        set.push(NotificationRule {
            name: "b".into(),
            event_kinds: vec![],
            min_severity: None,
            product_filter: None,
            channels: vec![Channel::Slack, Channel::Teams],
            enabled: true,
        });
        let chs = set.route(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None));
        assert_eq!(chs.len(), 3);
        assert!(chs.contains(&Channel::Slack));
        assert!(chs.contains(&Channel::Email));
        assert!(chs.contains(&Channel::Teams));
    }

    #[test]
    fn rule_set_json_roundtrip() {
        let mut set = NotificationRuleSet::new();
        set.push(NotificationRule {
            name: "n".into(),
            event_kinds: vec![EventKind::SlaBreached],
            min_severity: Some(FindingSeverity::High),
            product_filter: Some("svc".into()),
            channels: vec![Channel::Email],
            enabled: true,
        });
        let j = set.to_json().unwrap();
        let back = NotificationRuleSet::from_json(&j).unwrap();
        assert_eq!(back.rules.len(), 1);
        assert_eq!(back.rules[0].name, "n");
        assert_eq!(back.rules[0].channels, vec![Channel::Email]);
    }

    #[test]
    fn empty_rule_set_routes_no_channels() {
        let set = NotificationRuleSet::new();
        let chs = set.route(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None));
        assert!(chs.is_empty());
    }

    #[test]
    fn severity_gate_with_event_kind_combined() {
        let r = NotificationRule {
            name: "sla-crit".into(),
            event_kinds: vec![EventKind::SlaBreached],
            min_severity: Some(FindingSeverity::Critical),
            product_filter: None,
            channels: vec![Channel::Webhook],
            enabled: true,
        };
        assert!(r.matches(&evt(EventKind::SlaBreached, FindingSeverity::Critical, None)));
        assert!(!r.matches(&evt(EventKind::SlaBreached, FindingSeverity::High, None)));
        assert!(!r.matches(&evt(EventKind::FindingCreated, FindingSeverity::Critical, None)));
    }
}
