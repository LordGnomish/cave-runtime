// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scheduled scan DAG: master+node+etcd → report → notify.
//!
//! Upstream: kube-bench `cmd/run.go` job model + kubescape `core/cautils/scanInfo.go`.
//!
//! Cron strings handled by an in-house minimal matcher; full cron is the
//! responsibility of cave-streams / cave-ha.

use serde::{Deserialize, Serialize};

/// One scheduled scan definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledScan {
    pub id: String,
    pub profile_id: String,
    /// 5-field cron expression: `m h dom mon dow`.
    pub cron: String,
    pub enabled: bool,
    pub notify: NotifyAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotifyAction {
    None,
    Slack(String),
    PagerDuty(String),
    Webhook(String),
}

impl ScheduledScan {
    pub fn new(id: impl Into<String>, profile_id: impl Into<String>, cron: impl Into<String>) -> Self {
        ScheduledScan {
            id: id.into(),
            profile_id: profile_id.into(),
            cron: cron.into(),
            enabled: true,
            notify: NotifyAction::None,
        }
    }
}

/// DAG node — one phase of the scan workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DagNode {
    /// Run CIS master checks.
    CisMaster,
    /// Run CIS node checks.
    CisNode,
    /// Run CIS etcd checks.
    CisEtcd,
    /// Run NSA controls (manifest scan).
    Nsa,
    /// Compile findings into a report.
    Report,
    /// Emit notification.
    Notify,
}

/// Default DAG for a benchmark run.
pub fn default_dag() -> Vec<DagNode> {
    vec![
        DagNode::CisMaster,
        DagNode::CisNode,
        DagNode::CisEtcd,
        DagNode::Nsa,
        DagNode::Report,
        DagNode::Notify,
    ]
}

/// In-memory schedule registry.
#[derive(Debug, Default)]
pub struct ScheduleRegistry {
    inner: dashmap::DashMap<String, ScheduledScan>,
}

impl ScheduleRegistry {
    pub fn add(&self, s: ScheduledScan) {
        self.inner.insert(s.id.clone(), s);
    }
    pub fn remove(&self, id: &str) -> Option<ScheduledScan> {
        self.inner.remove(id).map(|(_, v)| v)
    }
    pub fn get(&self, id: &str) -> Option<ScheduledScan> {
        self.inner.get(id).map(|v| v.clone())
    }
    pub fn list(&self) -> Vec<ScheduledScan> {
        self.inner.iter().map(|e| e.clone()).collect()
    }
    pub fn count(&self) -> usize {
        self.inner.len()
    }
    /// Return enabled schedules whose cron *would* fire at `(minute, hour)`.
    /// Minimal matcher: only handles `*`, `*/N`, and literal integers in
    /// the minute + hour positions.
    pub fn due_at(&self, minute: u32, hour: u32) -> Vec<ScheduledScan> {
        self.inner
            .iter()
            .filter(|e| e.enabled)
            .filter(|e| cron_matches(&e.cron, minute, hour))
            .map(|e| e.clone())
            .collect()
    }
}

fn cron_matches(cron: &str, minute: u32, hour: u32) -> bool {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() < 2 {
        return false;
    }
    field_matches(parts[0], minute) && field_matches(parts[1], hour)
}

fn field_matches(field: &str, val: u32) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(rest) = field.strip_prefix("*/") {
        if let Ok(step) = rest.parse::<u32>() {
            if step == 0 {
                return false;
            }
            return val % step == 0;
        }
    }
    field.split(',').any(|p| p.trim().parse::<u32>().map(|n| n == val).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_dag_has_six_nodes() {
        assert_eq!(default_dag().len(), 6);
    }

    #[test]
    fn test_default_dag_ends_with_notify() {
        assert_eq!(default_dag().last(), Some(&DagNode::Notify));
    }

    #[test]
    fn test_schedule_add_get_remove() {
        let r = ScheduleRegistry::default();
        r.add(ScheduledScan::new("s1", "cis-1.10", "0 2 * * *"));
        assert_eq!(r.count(), 1);
        assert!(r.get("s1").is_some());
        assert!(r.remove("s1").is_some());
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn test_due_at_literal_match() {
        let r = ScheduleRegistry::default();
        r.add(ScheduledScan::new("s1", "cis-1.10", "0 2 * * *"));
        let due = r.due_at(0, 2);
        assert_eq!(due.len(), 1);
        let none = r.due_at(0, 3);
        assert!(none.is_empty());
    }

    #[test]
    fn test_due_at_star_minutes() {
        let r = ScheduleRegistry::default();
        r.add(ScheduledScan::new("s1", "cis-1.10", "* 2 * * *"));
        assert_eq!(r.due_at(0, 2).len(), 1);
        assert_eq!(r.due_at(45, 2).len(), 1);
    }

    #[test]
    fn test_due_at_step_minutes() {
        let r = ScheduleRegistry::default();
        r.add(ScheduledScan::new("s1", "cis-1.10", "*/15 * * * *"));
        assert_eq!(r.due_at(0, 0).len(), 1);
        assert_eq!(r.due_at(15, 7).len(), 1);
        assert_eq!(r.due_at(7, 0).len(), 0);
    }

    #[test]
    fn test_disabled_not_due() {
        let r = ScheduleRegistry::default();
        let mut s = ScheduledScan::new("s1", "cis-1.10", "0 2 * * *");
        s.enabled = false;
        r.add(s);
        assert_eq!(r.due_at(0, 2).len(), 0);
    }

    #[test]
    fn test_notify_action_variants() {
        let a = NotifyAction::Slack("#ops".into());
        match a {
            NotifyAction::Slack(c) => assert_eq!(c, "#ops"),
            _ => panic!(),
        }
    }
}
