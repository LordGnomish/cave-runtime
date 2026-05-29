// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing test for sprint velocity / analytics engine.

use cave_tracker::analytics::{sprint_velocity, cycle_time_stats, triage_issue};
use cave_tracker::models::{Issue, Sprint, SprintState, IssueType, Priority};
use cave_tracker::TrackerStore;
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn make_issue_with_sp(project_id: Uuid, sprint_id: Option<Uuid>, story_points: Option<f64>, status: &str) -> Issue {
    Issue {
        id: Uuid::new_v4(),
        key: "TEST-1".to_string(),
        project_id,
        project_key: "TEST".to_string(),
        issue_type: IssueType::Task,
        summary: "test".to_string(),
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

fn make_completed_sprint(project_id: Uuid, velocity: f64) -> Sprint {
    Sprint {
        id: Uuid::new_v4(),
        project_id,
        board_id: Uuid::new_v4(),
        name: "Sprint 1".to_string(),
        goal: None,
        state: SprintState::Closed,
        start_date: Some(Utc::now() - Duration::days(14)),
        end_date: Some(Utc::now() - Duration::days(1)),
        completed_at: Some(Utc::now() - Duration::days(1)),
        velocity: Some(velocity),
        created_at: Utc::now() - Duration::days(14),
    }
}

#[test]
fn test_sprint_velocity_with_completed_sprints() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();

    let sprint = make_completed_sprint(project_id, 20.0);
    let sprint_id = sprint.id;
    store.sprints.insert(sprint_id, sprint);

    // Done issue in that sprint
    let issue = make_issue_with_sp(project_id, Some(sprint_id), Some(5.0), "Done");
    store.issues.insert(issue.id, issue);

    let velocities = sprint_velocity(&store, project_id);
    assert_eq!(velocities.len(), 1);
    assert_eq!(velocities[0].sprint_id, sprint_id);
    assert!(velocities[0].completed_points >= 0.0);
}

#[test]
fn test_sprint_velocity_empty_project() {
    let project_id = Uuid::new_v4();
    let store = TrackerStore::default();
    let velocities = sprint_velocity(&store, project_id);
    assert!(velocities.is_empty());
}

#[test]
fn test_cycle_time_stats_no_done_issues() {
    let project_id = Uuid::new_v4();
    let store = TrackerStore::default();
    let stats = cycle_time_stats(&store, project_id);
    assert_eq!(stats.count, 0);
    assert_eq!(stats.avg_hours, 0.0);
}

#[test]
fn test_cycle_time_stats_with_resolved_issue() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();

    let mut issue = make_issue_with_sp(project_id, None, None, "Done");
    issue.resolved_at = Some(Utc::now());
    // created_at is set to now - we won't get a meaningful cycle time but it should not crash
    store.issues.insert(issue.id, issue);

    let stats = cycle_time_stats(&store, project_id);
    assert_eq!(stats.count, 1);
    assert!(stats.avg_hours >= 0.0);
}

#[test]
fn test_triage_critical_keywords() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let mut issue = make_issue_with_sp(project_id, None, None, "To Do");
    issue.summary = "production outage: service down".to_string();
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let suggestion = triage_issue(&store, issue_id).unwrap();
    assert_eq!(suggestion.issue_id, issue_id);
    assert_eq!(suggestion.suggested_priority, Priority::Critical);
}

#[test]
fn test_triage_security_label() {
    let project_id = Uuid::new_v4();
    let mut store = TrackerStore::default();
    let mut issue = make_issue_with_sp(project_id, None, None, "To Do");
    issue.summary = "security vulnerability in auth module".to_string();
    let issue_id = issue.id;
    store.issues.insert(issue_id, issue);

    let suggestion = triage_issue(&store, issue_id).unwrap();
    assert!(suggestion.suggested_labels.iter().any(|l| l == "security"));
}
