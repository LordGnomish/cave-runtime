// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing test for automation rules engine.

use cave_tracker::automation_engine::{
    AutomationRule, AutomationTrigger, AutomationCondition, AutomationAction,
    evaluate_rules, RuleEvent,
};
use cave_tracker::models::{Issue, IssueType, Priority};
use cave_tracker::TrackerStore;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_issue(project_id: Uuid, status: &str, priority: Priority) -> Issue {
    Issue {
        id: Uuid::new_v4(),
        key: "TEST-1".to_string(),
        project_id,
        project_key: "TEST".to_string(),
        issue_type: IssueType::Task,
        summary: "test issue".to_string(),
        description: None,
        status: status.to_string(),
        priority,
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
fn test_automation_assign_on_creation() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let issue = make_issue(project_id, "To Do", Priority::Medium);
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let rule = AutomationRule {
        id: Uuid::new_v4(),
        project_id,
        name: "Auto-assign on create".to_string(),
        trigger: AutomationTrigger::IssueCreated,
        condition: AutomationCondition::Always,
        action: AutomationAction::Assign { to: "alice".to_string() },
        enabled: true,
    };

    let results = evaluate_rules(&mut store, issue_id, &RuleEvent::IssueCreated, &[rule]);
    assert_eq!(results.len(), 1);
    assert!(results[0].applied);
    assert_eq!(store.issues[&issue_id].assignee.as_deref(), Some("alice"));
}

#[test]
fn test_automation_trigger_not_matching() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let issue = make_issue(project_id, "To Do", Priority::Medium);
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let rule = AutomationRule {
        id: Uuid::new_v4(),
        project_id,
        name: "Only on sprint start".to_string(),
        trigger: AutomationTrigger::SprintStarted,
        condition: AutomationCondition::Always,
        action: AutomationAction::Assign { to: "bob".to_string() },
        enabled: true,
    };

    let results = evaluate_rules(&mut store, issue_id, &RuleEvent::IssueCreated, &[rule]);
    // Should not fire because trigger doesn't match
    assert!(results.is_empty() || !results[0].applied);
    // Assignee should remain None
    assert!(store.issues[&issue_id].assignee.is_none());
}

#[test]
fn test_automation_status_transition_action() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let issue = make_issue(project_id, "In Progress", Priority::Medium);
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let rule = AutomationRule {
        id: Uuid::new_v4(),
        project_id,
        name: "Close on Done status change".to_string(),
        trigger: AutomationTrigger::StatusChanged {
            from: Some("In Progress".to_string()),
            to: Some("Done".to_string()),
        },
        condition: AutomationCondition::Always,
        action: AutomationAction::AddLabel { label: "completed".to_string() },
        enabled: true,
    };

    let results = evaluate_rules(
        &mut store,
        issue_id,
        &RuleEvent::StatusChanged { from: "In Progress".to_string(), to: "Done".to_string() },
        &[rule],
    );
    assert_eq!(results.len(), 1);
    assert!(results[0].applied);
    assert!(store.issues[&issue_id].labels.contains(&"completed".to_string()));
}

#[test]
fn test_automation_disabled_rule_skipped() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let issue = make_issue(project_id, "To Do", Priority::Medium);
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let rule = AutomationRule {
        id: Uuid::new_v4(),
        project_id,
        name: "Disabled rule".to_string(),
        trigger: AutomationTrigger::IssueCreated,
        condition: AutomationCondition::Always,
        action: AutomationAction::Assign { to: "charlie".to_string() },
        enabled: false,
    };

    let results = evaluate_rules(&mut store, issue_id, &RuleEvent::IssueCreated, &[rule]);
    assert!(results.is_empty());
    assert!(store.issues[&issue_id].assignee.is_none());
}
