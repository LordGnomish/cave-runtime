// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for pre-existing cave-tracker modules.
//!
//! These assert real observed behaviors — they pass immediately because
//! the code already exists on origin/main and compiles correctly.
//! No red-first TDD pair needed for pre-existing code.

use cave_tracker::board::{board_view, check_wip_violations, default_kanban_board, default_scrum_board};
use cave_tracker::fields::{create_field, validate_field_value};
use cave_tracker::models::{CustomFieldType, IssueType, LinkType, Priority};
use cave_tracker::query::{apply_filter, parse_jql, IssueFilter};
use cave_tracker::sprint::{backlog_issues, complete_sprint, sprint_stats, start_sprint};
use cave_tracker::workflow::{
    apply_transition, available_transitions, can_transition, default_kanban_workflow,
    default_scrum_workflow,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_test_issue(key: &str, project_key: &str, status: &str) -> cave_tracker::models::Issue {
    cave_tracker::models::Issue {
        id: Uuid::new_v4(),
        key: key.to_string(),
        project_id: Uuid::new_v4(),
        project_key: project_key.to_string(),
        issue_type: IssueType::Task,
        summary: format!("Issue {}", key),
        description: None,
        status: status.to_string(),
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

// ── workflow.rs ───────────────────────────────────────────────────────────────

#[test]
fn characterize_scrum_workflow_statuses() {
    let wf = default_scrum_workflow();
    assert_eq!(wf.statuses.len(), 5);
    assert!(wf.is_default);
}

#[test]
fn characterize_kanban_workflow_not_default() {
    let wf = default_kanban_workflow();
    assert!(!wf.is_default);
    assert_eq!(wf.statuses.len(), 3);
}

#[test]
fn characterize_can_transition_todo_to_in_progress() {
    let wf = default_scrum_workflow();
    assert!(can_transition(&wf, "To Do", "start"));
    assert!(!can_transition(&wf, "Done", "start"));
}

#[test]
fn characterize_apply_transition_sets_status() {
    let wf = default_scrum_workflow();
    let mut issue = make_test_issue("TEST-1", "TEST", "To Do");
    let t = wf.transitions.iter().find(|t| t.id == "start").unwrap();
    apply_transition(&mut issue, t);
    assert_eq!(issue.status, "In Progress");
}

#[test]
fn characterize_available_transitions_from_backlog() {
    let wf = default_scrum_workflow();
    let ts = available_transitions(&wf, "Backlog");
    assert!(ts.iter().any(|t| t.to_status == "In Progress"));
}

// ── sprint.rs ─────────────────────────────────────────────────────────────────

#[test]
fn characterize_start_sprint_sets_active() {
    let mut sprint = cave_tracker::models::Sprint {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        board_id: Uuid::new_v4(),
        name: "S1".to_string(),
        goal: None,
        state: cave_tracker::models::SprintState::Future,
        start_date: None,
        end_date: None,
        completed_at: None,
        velocity: None,
        created_at: Utc::now(),
    };
    assert!(start_sprint(&mut sprint).is_ok());
    assert_eq!(sprint.state, cave_tracker::models::SprintState::Active);
}

#[test]
fn characterize_backlog_issues_excludes_sprint_issues() {
    let project_id = Uuid::new_v4();
    let sprint_id = Uuid::new_v4();
    let mut i1 = make_test_issue("T-1", "T", "To Do");
    i1.project_id = project_id;
    i1.sprint_id = Some(sprint_id);
    let mut i2 = make_test_issue("T-2", "T", "To Do");
    i2.project_id = project_id;
    // i2 has no sprint_id → backlog
    let all = vec![i1, i2];
    let backlog = backlog_issues(all.iter(), project_id);
    assert_eq!(backlog.len(), 1);
    assert!(backlog[0].sprint_id.is_none());
}

// ── board.rs ──────────────────────────────────────────────────────────────────

#[test]
fn characterize_scrum_board_has_4_columns() {
    let board = default_scrum_board(Uuid::new_v4(), "Test");
    assert_eq!(board.columns.len(), 4);
    assert!(board.backlog_enabled);
}

#[test]
fn characterize_kanban_board_no_backlog() {
    let board = default_kanban_board(Uuid::new_v4(), "Test");
    assert!(!board.backlog_enabled);
    assert_eq!(board.columns.len(), 3);
}

#[test]
fn characterize_board_view_places_issues_in_columns() {
    let project_id = Uuid::new_v4();
    let board = default_scrum_board(project_id, "Test");
    let mut i1 = make_test_issue("T-1", "T", "To Do");
    i1.project_id = project_id;
    let mut i2 = make_test_issue("T-2", "T", "Done");
    i2.project_id = project_id;
    let view = board_view(&board, &[&i1, &i2]);
    let todo_col = view.iter().find(|(name, _)| name == "To Do").unwrap();
    assert_eq!(todo_col.1.len(), 1);
    let done_col = view.iter().find(|(name, _)| name == "Done").unwrap();
    assert_eq!(done_col.1.len(), 1);
}

#[test]
fn characterize_wip_no_violations_empty() {
    let board = default_scrum_board(Uuid::new_v4(), "T");
    assert!(check_wip_violations(&board, &[]).is_empty());
}

// ── fields.rs ─────────────────────────────────────────────────────────────────

#[test]
fn characterize_create_field_number() {
    let f = create_field("Points", CustomFieldType::Number, "SP", false);
    assert_eq!(f.name, "Points");
    assert!(validate_field_value(&f, &serde_json::json!(5)).is_empty());
    assert!(!validate_field_value(&f, &serde_json::json!("oops")).is_empty());
}

// ── query.rs ──────────────────────────────────────────────────────────────────

#[test]
fn characterize_jql_parse_project_and_status() {
    let f = parse_jql("project=CAVE AND status=Done");
    assert_eq!(f.project_key, Some("CAVE".to_string()));
    assert_eq!(f.status, Some("Done".to_string()));
}

#[test]
fn characterize_apply_filter_by_status() {
    let issues = vec![
        make_test_issue("CAVE-1", "CAVE", "In Progress"),
        make_test_issue("CAVE-2", "CAVE", "Done"),
    ];
    let filter = IssueFilter {
        status: Some("Done".to_string()),
        ..Default::default()
    };
    let results = apply_filter(issues.iter(), &filter);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "CAVE-2");
}

#[test]
fn characterize_filter_text_search() {
    let issues = vec![
        make_test_issue("CAVE-1", "CAVE", "To Do"),
        make_test_issue("CAVE-2", "CAVE", "To Do"),
    ];
    let filter = IssueFilter {
        text_search: Some("CAVE-1".to_string()),
        ..Default::default()
    };
    let results = apply_filter(issues.iter(), &filter);
    assert_eq!(results.len(), 1);
}
