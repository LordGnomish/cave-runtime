// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for DevLake data models — written BEFORE implementation (TDD).

use cave_devlake::models::{
    Commit, DeploymentEnv, DeploymentStatus, DoraRating, Incident, Issue, IssueStatus,
    IssueType, Pipeline, PipelineStatus, PrState, PullRequest, Sprint, SprintState,
};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn pipeline_status_serialises_to_snake_case() {
    let s = serde_json::to_string(&PipelineStatus::InProgress).unwrap();
    assert_eq!(s, r#""in_progress""#);
    let s2 = serde_json::to_string(&PipelineStatus::Success).unwrap();
    assert_eq!(s2, r#""success""#);
}

#[test]
fn deployment_env_round_trips_json() {
    let env = DeploymentEnv::Production;
    let json = serde_json::to_string(&env).unwrap();
    let back: DeploymentEnv = serde_json::from_str(&json).unwrap();
    assert_eq!(env, back);
}

#[test]
fn deployment_status_variants_exist() {
    let _success = DeploymentStatus::Success;
    let _failed = DeploymentStatus::Failed;
    let _rolled = DeploymentStatus::RolledBack;
    let _running = DeploymentStatus::Running;
}

#[test]
fn dora_rating_ordering() {
    // Elite > High > Medium > Low in performance ordering
    assert!(DoraRating::Elite > DoraRating::High);
    assert!(DoraRating::High > DoraRating::Medium);
    assert!(DoraRating::Medium > DoraRating::Low);
}

#[test]
fn pull_request_fields() {
    let pr = PullRequest {
        id: Uuid::new_v4(),
        number: 42,
        title: "Add feature X".to_string(),
        author: "alice".to_string(),
        source_branch: "feature/x".to_string(),
        target_branch: "main".to_string(),
        state: PrState::Merged,
        created_at: Utc::now(),
        merged_at: Some(Utc::now()),
        closed_at: None,
        cycle_time_secs: Some(3600.0),
        review_count: 2,
        comment_count: 5,
        additions: 120,
        deletions: 30,
    };
    assert_eq!(pr.number, 42);
    assert!(matches!(pr.state, PrState::Merged));
}

#[test]
fn commit_fields() {
    let c = Commit {
        sha: "abc123".to_string(),
        author: "bob".to_string(),
        message: "fix: edge case in parser".to_string(),
        committed_at: Utc::now(),
        additions: 10,
        deletions: 5,
        files_changed: 3,
    };
    assert_eq!(c.sha, "abc123");
}

#[test]
fn issue_status_variants() {
    let _open = IssueStatus::Open;
    let _in_progress = IssueStatus::InProgress;
    let _done = IssueStatus::Done;
    let _closed = IssueStatus::Closed;
}

#[test]
fn issue_type_variants() {
    let _bug = IssueType::Bug;
    let _story = IssueType::Story;
    let _task = IssueType::Task;
    let _epic = IssueType::Epic;
}

#[test]
fn issue_fields() {
    let issue = Issue {
        id: Uuid::new_v4(),
        key: "PROJ-42".to_string(),
        title: "Fix login bug".to_string(),
        issue_type: IssueType::Bug,
        status: IssueStatus::InProgress,
        assignee: Some("carol".to_string()),
        priority: "P1".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
        story_points: Some(3),
    };
    assert_eq!(issue.key, "PROJ-42");
}

#[test]
fn sprint_fields() {
    let sprint = Sprint {
        id: Uuid::new_v4(),
        name: "Sprint 42".to_string(),
        state: SprintState::Active,
        start_date: Utc::now(),
        end_date: Utc::now() + chrono::Duration::days(14),
        completed_points: 20,
        planned_points: 25,
        completed_issues: 8,
        planned_issues: 10,
    };
    assert_eq!(sprint.name, "Sprint 42");
    assert!(matches!(sprint.state, SprintState::Active));
}

#[test]
fn incident_fields() {
    let inc = Incident {
        id: Uuid::new_v4(),
        title: "API 5xx spike".to_string(),
        severity: "P1".to_string(),
        started_at: Utc::now(),
        resolved_at: None,
        services: vec!["api-gateway".to_string()],
        linked_deployment_id: None,
    };
    assert!(inc.resolved_at.is_none());
}
