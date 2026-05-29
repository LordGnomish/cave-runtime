// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workflow automation engine: trigger → condition → action rule evaluation.
//!
//! Adapted from the orphan `automation.rs` to work with the current models
//! and TrackerStore (non-async, HashMap-based).

use crate::models::{IssueType, Priority};
use crate::TrackerStore;
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

// ── Rule model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AutomationRule {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub trigger: AutomationTrigger,
    pub condition: AutomationCondition,
    pub action: AutomationAction,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub enum AutomationTrigger {
    IssueCreated,
    StatusChanged { from: Option<String>, to: Option<String> },
    SprintStarted,
    DueDateApproaching { days_before: u32 },
}

#[derive(Debug, Clone)]
pub enum AutomationCondition {
    Always,
    IssueType { issue_type: IssueType },
    Priority { priority: Priority },
    HasLabel { label: String },
}

#[derive(Debug, Clone)]
pub enum AutomationAction {
    Assign { to: String },
    Transition { to_status: String },
    AddLabel { label: String },
    Notify { message: String },
}

// ── Event ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RuleEvent {
    IssueCreated,
    StatusChanged { from: String, to: String },
    SprintStarted,
    DueDateApproaching { days_until: u32 },
}

// ── Result ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AutomationResult {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub applied: bool,
    pub message: String,
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Evaluate all enabled, applicable rules for `issue_id` + `event`.
/// Modifies `store` in-place when rules apply.
pub fn evaluate_rules(
    store: &mut TrackerStore,
    issue_id: Uuid,
    event: &RuleEvent,
    rules: &[AutomationRule],
) -> Vec<AutomationResult> {
    // Snapshot issue data (avoid borrow issues).
    let issue_snapshot = match store.issues.get(&issue_id).cloned() {
        Some(i) => i,
        None => return Vec::new(),
    };

    let mut results = Vec::new();

    for rule in rules {
        if !rule.enabled { continue; }
        if rule.project_id != issue_snapshot.project_id { continue; }
        if !matches_trigger(&rule.trigger, event) { continue; }
        if !matches_condition(&rule.condition, &issue_snapshot) { continue; }

        let outcome = apply_action(store, issue_id, &rule.action);
        let applied = outcome.is_ok();
        let message = outcome.unwrap_or_else(|e| e);
        results.push(AutomationResult {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            applied,
            message,
        });
    }
    results
}

// ── Matching ──────────────────────────────────────────────────────────────────

fn matches_trigger(trigger: &AutomationTrigger, event: &RuleEvent) -> bool {
    match (trigger, event) {
        (AutomationTrigger::IssueCreated, RuleEvent::IssueCreated) => true,
        (
            AutomationTrigger::StatusChanged { from: tf, to: tt },
            RuleEvent::StatusChanged { from: ef, to: et },
        ) => {
            let from_ok = tf.as_deref().map(|f| f == ef).unwrap_or(true);
            let to_ok = tt.as_deref().map(|t| t == et).unwrap_or(true);
            from_ok && to_ok
        }
        (AutomationTrigger::SprintStarted, RuleEvent::SprintStarted) => true,
        (
            AutomationTrigger::DueDateApproaching { days_before: tb },
            RuleEvent::DueDateApproaching { days_until: eu },
        ) => eu <= tb,
        _ => false,
    }
}

fn matches_condition(condition: &AutomationCondition, issue: &crate::models::Issue) -> bool {
    match condition {
        AutomationCondition::Always => true,
        AutomationCondition::IssueType { issue_type } => &issue.issue_type == issue_type,
        AutomationCondition::Priority { priority } => &issue.priority == priority,
        AutomationCondition::HasLabel { label } => issue.labels.iter().any(|l| l == label),
    }
}

// ── Action execution (synchronous) ────────────────────────────────────────────

fn apply_action(
    store: &mut TrackerStore,
    issue_id: Uuid,
    action: &AutomationAction,
) -> Result<String, String> {
    match action {
        AutomationAction::Assign { to } => {
            let issue = store.issues.get_mut(&issue_id).ok_or("Issue not found")?;
            issue.assignee = Some(to.clone());
            issue.updated_at = Utc::now();
            Ok(format!("Assigned to {}", to))
        }
        AutomationAction::Transition { to_status } => {
            let issue = store.issues.get_mut(&issue_id).ok_or("Issue not found")?;
            issue.status = to_status.clone();
            issue.updated_at = Utc::now();
            Ok(format!("Transitioned to '{}'", to_status))
        }
        AutomationAction::AddLabel { label } => {
            let issue = store.issues.get_mut(&issue_id).ok_or("Issue not found")?;
            if !issue.labels.contains(label) {
                issue.labels.push(label.clone());
                issue.updated_at = Utc::now();
            }
            Ok(format!("Added label '{}'", label))
        }
        AutomationAction::Notify { message } => {
            // Notifications are dispatched via cave-alerts / cave-incidents (parallel-track).
            // Here we log at tracing level only.
            tracing::info!(issue_id = %issue_id, "Automation notify: {}", message);
            Ok(format!("Notification logged: {}", message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Issue, IssueType, Priority};
    use crate::TrackerStore;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_issue(project_id: Uuid) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            key: "TEST-1".to_string(),
            project_id,
            project_key: "TEST".to_string(),
            issue_type: IssueType::Task,
            summary: "test".to_string(),
            description: None,
            status: "To Do".to_string(),
            priority: Priority::Medium,
            assignee: None,
            reporter: "admin".to_string(),
            labels: vec![],
            components: vec![],
            fix_versions: vec![],
            affects_versions: vec![],
            epic_id: None,
            parent_id: None,
            sprint_id: None,
            story_points: None,
            time_estimate_seconds: None,
            time_spent_seconds: 0,
            custom_fields: HashMap::new(),
            watchers: vec![],
            votes: 0,
            rank: 0,
            resolution: None,
            due_date: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn auto_assign_on_create() {
        let project_id = Uuid::new_v4();
        let mut store = TrackerStore::default();
        let issue = make_issue(project_id);
        let id = issue.id;
        store.issues.insert(id, issue);

        let rule = AutomationRule {
            id: Uuid::new_v4(),
            project_id,
            name: "Auto-assign".to_string(),
            trigger: AutomationTrigger::IssueCreated,
            condition: AutomationCondition::Always,
            action: AutomationAction::Assign { to: "alice".to_string() },
            enabled: true,
        };
        let results = evaluate_rules(&mut store, id, &RuleEvent::IssueCreated, &[rule]);
        assert_eq!(results.len(), 1);
        assert!(results[0].applied);
        assert_eq!(store.issues[&id].assignee.as_deref(), Some("alice"));
    }

    #[test]
    fn disabled_rule_not_applied() {
        let project_id = Uuid::new_v4();
        let mut store = TrackerStore::default();
        let issue = make_issue(project_id);
        let id = issue.id;
        store.issues.insert(id, issue);

        let rule = AutomationRule {
            id: Uuid::new_v4(),
            project_id,
            name: "Disabled".to_string(),
            trigger: AutomationTrigger::IssueCreated,
            condition: AutomationCondition::Always,
            action: AutomationAction::Assign { to: "bob".to_string() },
            enabled: false,
        };
        let results = evaluate_rules(&mut store, id, &RuleEvent::IssueCreated, &[rule]);
        assert!(results.is_empty());
    }

    #[test]
    fn add_label_action() {
        let project_id = Uuid::new_v4();
        let mut store = TrackerStore::default();
        let issue = make_issue(project_id);
        let id = issue.id;
        store.issues.insert(id, issue);

        let rule = AutomationRule {
            id: Uuid::new_v4(),
            project_id,
            name: "Label on done".to_string(),
            trigger: AutomationTrigger::StatusChanged {
                from: Some("In Progress".to_string()),
                to: Some("Done".to_string()),
            },
            condition: AutomationCondition::Always,
            action: AutomationAction::AddLabel { label: "reviewed".to_string() },
            enabled: true,
        };
        let results = evaluate_rules(
            &mut store,
            id,
            &RuleEvent::StatusChanged {
                from: "In Progress".to_string(),
                to: "Done".to_string(),
            },
            &[rule],
        );
        assert_eq!(results.len(), 1);
        assert!(store.issues[&id].labels.contains(&"reviewed".to_string()));
    }
}
