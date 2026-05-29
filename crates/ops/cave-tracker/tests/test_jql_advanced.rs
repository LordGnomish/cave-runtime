// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing test for enhanced JQL parser (AND/OR/IN/IS EMPTY/ORDER BY).

use cave_tracker::jql_engine::{JqlEngine, JqlValue, JqlCondition, ParsedJql};
use cave_tracker::models::{Issue, Priority, IssueType};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_issue(key: &str, project: &str, status: &str, priority: Priority, issue_type: IssueType, assignee: Option<String>, sprint_id: Option<Uuid>) -> Issue {
    Issue {
        id: Uuid::new_v4(),
        key: key.to_string(),
        project_id: Uuid::new_v4(),
        project_key: project.to_string(),
        issue_type,
        summary: format!("Issue {}", key),
        description: None,
        status: status.to_string(),
        priority,
        assignee,
        reporter: "admin".to_string(),
        labels: vec![],
        components: vec![],
        fix_versions: vec![],
        affects_versions: vec![],
        epic_id: None,
        parent_id: None,
        sprint_id,
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
fn test_jql_parse_project_equals() {
    let parsed = JqlEngine::parse("project = CAVE").unwrap();
    assert!(parsed.condition.is_some());
}

#[test]
fn test_jql_parse_and_condition() {
    let parsed = JqlEngine::parse("project = CAVE AND status = Done").unwrap();
    assert!(matches!(parsed.condition, Some(JqlCondition::And(_, _))));
}

#[test]
fn test_jql_parse_status_in() {
    let parsed = JqlEngine::parse(r#"status IN ("In Progress", "Done")"#).unwrap();
    if let Some(JqlCondition::In { field, values }) = parsed.condition {
        assert_eq!(field, "status");
        assert_eq!(values.len(), 2);
    } else {
        panic!("Expected In condition");
    }
}

#[test]
fn test_jql_parse_sprint_is_empty() {
    let parsed = JqlEngine::parse("sprint IS EMPTY").unwrap();
    if let Some(JqlCondition::IsEmpty { field }) = parsed.condition {
        assert_eq!(field, "sprint");
    } else {
        panic!("Expected IsEmpty condition");
    }
}

#[test]
fn test_jql_parse_order_by_desc() {
    let parsed = JqlEngine::parse("project = CAVE ORDER BY priority DESC").unwrap();
    assert_eq!(parsed.order_by.len(), 1);
    assert_eq!(parsed.order_by[0].field, "priority");
    assert!(!parsed.order_by[0].ascending);
}

#[test]
fn test_jql_evaluate_project_filter() {
    let issues = vec![
        make_issue("CAVE-1", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, None),
        make_issue("OTHER-1", "OTHER", "To Do", Priority::Medium, IssueType::Task, None, None),
    ];
    let parsed = JqlEngine::parse("project = CAVE").unwrap();
    let result = JqlEngine::evaluate(&issues, &parsed);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].project_key, "CAVE");
}

#[test]
fn test_jql_evaluate_status_in() {
    let issues = vec![
        make_issue("CAVE-1", "CAVE", "In Progress", Priority::Medium, IssueType::Task, None, None),
        make_issue("CAVE-2", "CAVE", "Done", Priority::Medium, IssueType::Task, None, None),
        make_issue("CAVE-3", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, None),
    ];
    let parsed = JqlEngine::parse(r#"status IN ("In Progress", "Done")"#).unwrap();
    let result = JqlEngine::evaluate(&issues, &parsed);
    assert_eq!(result.len(), 2);
}

#[test]
fn test_jql_evaluate_sprint_is_empty() {
    let sprint_id = Uuid::new_v4();
    let issues = vec![
        make_issue("CAVE-1", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, None),
        make_issue("CAVE-2", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, Some(sprint_id)),
    ];
    let parsed = JqlEngine::parse("sprint IS EMPTY").unwrap();
    let result = JqlEngine::evaluate(&issues, &parsed);
    assert_eq!(result.len(), 1);
    assert!(result[0].sprint_id.is_none());
}

#[test]
fn test_jql_empty_query_returns_all() {
    let issues = vec![
        make_issue("CAVE-1", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, None),
        make_issue("OTHER-1", "OTHER", "Done", Priority::Low, IssueType::Bug, None, None),
    ];
    let parsed = JqlEngine::parse("").unwrap();
    let result = JqlEngine::evaluate(&issues, &parsed);
    assert_eq!(result.len(), 2);
}

#[test]
fn test_jql_order_by_priority() {
    let issues = vec![
        make_issue("CAVE-1", "CAVE", "To Do", Priority::Low, IssueType::Task, None, None),
        make_issue("CAVE-2", "CAVE", "To Do", Priority::Critical, IssueType::Task, None, None),
        make_issue("CAVE-3", "CAVE", "To Do", Priority::Medium, IssueType::Task, None, None),
    ];
    let parsed = JqlEngine::parse("project = CAVE ORDER BY priority ASC").unwrap();
    let result = JqlEngine::evaluate(&issues, &parsed);
    // Critical < High < Medium < Low < Trivial (ascending = most urgent first)
    assert_eq!(result[0].priority, Priority::Critical);
}
