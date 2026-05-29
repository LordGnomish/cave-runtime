// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing test for roadmap / timeline engine.

use cave_tracker::roadmap_engine::{timeline_view, capacity_plan, risk_report, RiskKind};
use cave_tracker::models::{Issue, Sprint, SprintState, IssueType, Priority};
use cave_tracker::TrackerStore;
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn make_issue(project_id: Uuid, issue_type: IssueType, status: &str, sprint_id: Option<Uuid>, story_points: Option<f64>, assignee: Option<String>) -> Issue {
    Issue {
        id: Uuid::new_v4(),
        key: "TEST-1".to_string(),
        project_id,
        project_key: "TEST".to_string(),
        issue_type,
        summary: "test issue".to_string(),
        description: None,
        status: status.to_string(),
        priority: Priority::Medium,
        assignee,
        reporter: "admin".to_string(),
        labels: vec![],
        components: vec![],
        fix_versions: vec![],
        affects_versions: vec![],
        epic_id: None,
        parent_id: None,
        sprint_id,
        story_points,
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

fn make_sprint(project_id: Uuid, state: SprintState, board_id: Uuid) -> Sprint {
    Sprint {
        id: Uuid::new_v4(),
        project_id,
        board_id,
        name: "Sprint 1".to_string(),
        goal: None,
        state,
        start_date: None,
        end_date: None,
        completed_at: None,
        velocity: None,
        created_at: Utc::now(),
    }
}

#[test]
fn test_timeline_view_returns_epics() {
    let project_id = Uuid::new_v4();
    let board_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let epic = make_issue(project_id, IssueType::Epic, "In Progress", None, None, None);
    let task = make_issue(project_id, IssueType::Task, "To Do", None, None, None);
    store.issues.insert(epic.id, epic.clone());
    store.issues.insert(task.id, task);

    let view = timeline_view(&store, project_id);
    assert_eq!(view.project_id, project_id);
    assert_eq!(view.epics.len(), 1);
    assert_eq!(view.epics[0].issue_id, epic.id);
}

#[test]
fn test_capacity_plan_with_completed_sprint() {
    let project_id = Uuid::new_v4();
    let board_id = Uuid::new_v4();
    let mut store = TrackerStore::default();

    let mut sprint = make_sprint(project_id, SprintState::Closed, board_id);
    sprint.velocity = Some(15.0);
    let sprint_id = sprint.id;
    store.sprints.insert(sprint_id, sprint);

    let issue = make_issue(project_id, IssueType::Task, "Done", Some(sprint_id), Some(5.0), None);
    store.issues.insert(issue.id, issue);

    let plan = capacity_plan(&store, project_id);
    assert_eq!(plan.project_id, project_id);
    assert_eq!(plan.sprints.len(), 1);
    assert_eq!(plan.sprints[0].sprint_id, sprint_id);
    assert!(plan.sprints[0].planned_points > 0.0 || plan.sprints[0].issue_count > 0);
}

#[test]
fn test_risk_report_detects_unassigned_in_active_sprint() {
    let project_id = Uuid::new_v4();
    let board_id = Uuid::new_v4();
    let mut store = TrackerStore::default();

    let mut sprint = make_sprint(project_id, SprintState::Active, board_id);
    let sprint_id = sprint.id;
    store.sprints.insert(sprint_id, sprint);

    // Unassigned issue in active sprint
    let issue = make_issue(project_id, IssueType::Task, "In Progress", Some(sprint_id), None, None);
    store.issues.insert(issue.id, issue);

    let report = risk_report(&store, project_id);
    assert_eq!(report.project_id, project_id);
    assert!(report.risks.iter().any(|r| matches!(r.kind, RiskKind::NoAssignee)));
}

#[test]
fn test_risk_report_empty_for_clean_project() {
    let project_id = Uuid::new_v4();
    let store = TrackerStore::default();
    let report = risk_report(&store, project_id);
    assert_eq!(report.risks.len(), 0);
}
