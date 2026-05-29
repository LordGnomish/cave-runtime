// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the DevlakeStore in-memory store — written BEFORE implementation (TDD).

use cave_devlake::models::{
    Commit, Deployment, DeploymentEnv, DeploymentStatus, Incident, Issue, IssueStatus, IssueType,
    Pipeline, PipelineStatus, PrState, PullRequest, Sprint, SprintState,
};
use cave_devlake::store::DevlakeStore;
use chrono::{Duration, Utc};
use uuid::Uuid;

fn make_deployment(env: DeploymentEnv, status: DeploymentStatus, lead_time: Option<f64>) -> Deployment {
    Deployment {
        id: Uuid::new_v4(),
        pipeline_id: None,
        service: "svc".to_string(),
        version: "v1.0".to_string(),
        environment: env,
        deployed_at: Utc::now(),
        deployed_by: "alice".to_string(),
        status,
        rollback: false,
        lead_time_secs: lead_time,
    }
}

// ── Pipeline store ────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_get_pipeline() {
    let store = DevlakeStore::new();
    let p = Pipeline {
        id: Uuid::new_v4(),
        name: "build-api".to_string(),
        project: "cave".to_string(),
        repo: "github.com/cave/api".to_string(),
        branch: "main".to_string(),
        status: PipelineStatus::Success,
        triggered_by: "alice".to_string(),
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
        duration_secs: Some(120.0),
        stages: vec![],
        commit_sha: Some("abc123".to_string()),
        environment: DeploymentEnv::Production,
    };
    let id = p.id;
    store.insert_pipeline(p);
    let fetched = store.get_pipeline(id).unwrap();
    assert_eq!(fetched.name, "build-api");
}

#[test]
fn store_pipeline_not_found_returns_none() {
    let store = DevlakeStore::new();
    assert!(store.get_pipeline(Uuid::new_v4()).is_none());
}

#[test]
fn store_list_pipelines_sorted_by_started_at_desc() {
    let store = DevlakeStore::new();
    let now = Utc::now();
    for i in 0..3u32 {
        let p = Pipeline {
            id: Uuid::new_v4(),
            name: format!("build-{i}"),
            project: "cave".to_string(),
            repo: "repo".to_string(),
            branch: "main".to_string(),
            status: PipelineStatus::Success,
            triggered_by: "alice".to_string(),
            started_at: now - Duration::hours(i as i64),
            finished_at: None,
            duration_secs: None,
            stages: vec![],
            commit_sha: None,
            environment: DeploymentEnv::Production,
        };
        store.insert_pipeline(p);
    }
    let list = store.list_pipelines();
    assert_eq!(list.len(), 3);
    // Most recent first
    assert!(list[0].started_at >= list[1].started_at);
    assert!(list[1].started_at >= list[2].started_at);
}

#[test]
fn store_update_pipeline_status() {
    let store = DevlakeStore::new();
    let id = Uuid::new_v4();
    let p = Pipeline {
        id,
        name: "build".to_string(),
        project: "cave".to_string(),
        repo: "repo".to_string(),
        branch: "main".to_string(),
        status: PipelineStatus::Running,
        triggered_by: "alice".to_string(),
        started_at: Utc::now(),
        finished_at: None,
        duration_secs: None,
        stages: vec![],
        commit_sha: None,
        environment: DeploymentEnv::Staging,
    };
    store.insert_pipeline(p);
    let updated = store.update_pipeline_status(id, PipelineStatus::Success, Some(300.0));
    assert!(updated.is_some());
    let updated = updated.unwrap();
    assert_eq!(updated.status, PipelineStatus::Success);
    assert_eq!(updated.duration_secs, Some(300.0));
}

// ── Deployment store ──────────────────────────────────────────────────────────

#[test]
fn store_insert_and_get_deployment() {
    let store = DevlakeStore::new();
    let d = make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, Some(3600.0));
    let id = d.id;
    store.insert_deployment(d);
    assert!(store.get_deployment(id).is_some());
}

#[test]
fn store_list_deployments_sorted_desc() {
    let store = DevlakeStore::new();
    for _ in 0..3 {
        let d = make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, None);
        store.insert_deployment(d);
    }
    let list = store.list_deployments();
    assert_eq!(list.len(), 3);
    assert!(list[0].deployed_at >= list[1].deployed_at);
}

#[test]
fn store_recent_deployments_env_filter() {
    let store = DevlakeStore::new();
    store.insert_deployment(make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, None));
    store.insert_deployment(make_deployment(DeploymentEnv::Staging, DeploymentStatus::Success, None));
    store.insert_deployment(make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, None));

    let prod = store.recent_deployments(Some(&DeploymentEnv::Production), 10);
    assert_eq!(prod.len(), 2);
    for d in &prod {
        assert_eq!(d.environment, DeploymentEnv::Production);
    }
}

#[test]
fn store_deployments_in_period() {
    let store = DevlakeStore::new();
    let now = Utc::now();
    // Insert 3 deployments at different times
    for i in 0..5i64 {
        let mut d = make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, None);
        d.deployed_at = now - Duration::days(i);
        store.insert_deployment(d);
    }
    let from = now - Duration::days(2);
    let to = now;
    let in_period = store.deployments_in_period(from, to);
    // Should include days 0, 1, 2 = 3 deployments
    assert_eq!(in_period.len(), 3);
}

// ── Incident store ────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_resolve_incident() {
    let store = DevlakeStore::new();
    let inc = Incident {
        id: Uuid::new_v4(),
        title: "DB outage".to_string(),
        severity: "P1".to_string(),
        started_at: Utc::now() - Duration::hours(2),
        resolved_at: None,
        services: vec!["db".to_string()],
        linked_deployment_id: None,
    };
    let id = inc.id;
    store.insert_incident(inc);

    let resolved_at = Utc::now();
    let resolved = store.resolve_incident(id, resolved_at).unwrap();
    assert!(resolved.resolved_at.is_some());
}

#[test]
fn store_list_incidents_sorted_desc() {
    let store = DevlakeStore::new();
    let now = Utc::now();
    for i in 0..3i64 {
        let inc = Incident {
            id: Uuid::new_v4(),
            title: format!("Incident {i}"),
            severity: "P2".to_string(),
            started_at: now - Duration::hours(i),
            resolved_at: None,
            services: vec![],
            linked_deployment_id: None,
        };
        store.insert_incident(inc);
    }
    let list = store.list_incidents();
    assert_eq!(list.len(), 3);
    assert!(list[0].started_at >= list[1].started_at);
}

// ── PR store ──────────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_get_pr() {
    let store = DevlakeStore::new();
    let pr = PullRequest {
        id: Uuid::new_v4(),
        number: 99,
        title: "feat: new thing".to_string(),
        author: "alice".to_string(),
        source_branch: "feature/new".to_string(),
        target_branch: "main".to_string(),
        state: PrState::Open,
        created_at: Utc::now(),
        merged_at: None,
        closed_at: None,
        cycle_time_secs: None,
        review_count: 0,
        comment_count: 0,
        additions: 100,
        deletions: 20,
    };
    let id = pr.id;
    store.insert_pr(pr);
    let fetched = store.get_pr(id).unwrap();
    assert_eq!(fetched.number, 99);
}

#[test]
fn store_list_prs_open_only() {
    let store = DevlakeStore::new();
    let make_pr = |state: PrState| PullRequest {
        id: Uuid::new_v4(),
        number: 1,
        title: "PR".to_string(),
        author: "alice".to_string(),
        source_branch: "feat".to_string(),
        target_branch: "main".to_string(),
        state,
        created_at: Utc::now(),
        merged_at: None,
        closed_at: None,
        cycle_time_secs: None,
        review_count: 0,
        comment_count: 0,
        additions: 0,
        deletions: 0,
    };
    store.insert_pr(make_pr(PrState::Open));
    store.insert_pr(make_pr(PrState::Merged));
    store.insert_pr(make_pr(PrState::Open));

    let open = store.prs_by_state(&PrState::Open);
    assert_eq!(open.len(), 2);
}

// ── Commit store ──────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_list_commits() {
    let store = DevlakeStore::new();
    for i in 0..5u32 {
        let c = Commit {
            sha: format!("sha{i:04x}"),
            author: "alice".to_string(),
            message: format!("commit {i}"),
            committed_at: Utc::now() - Duration::hours(i as i64),
            additions: 10,
            deletions: 2,
            files_changed: 1,
        };
        store.insert_commit(c);
    }
    let commits = store.list_commits();
    assert_eq!(commits.len(), 5);
}

// ── Issue store ───────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_get_issue() {
    let store = DevlakeStore::new();
    let issue = Issue {
        id: Uuid::new_v4(),
        key: "CAVE-1".to_string(),
        title: "Some bug".to_string(),
        issue_type: IssueType::Bug,
        status: IssueStatus::Open,
        assignee: None,
        priority: "P2".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
        story_points: Some(3),
    };
    let id = issue.id;
    store.insert_issue(issue);
    assert!(store.get_issue(id).is_some());
}

#[test]
fn store_issues_by_status() {
    let store = DevlakeStore::new();
    let make_issue = |status: IssueStatus| Issue {
        id: Uuid::new_v4(),
        key: "X-1".to_string(),
        title: "issue".to_string(),
        issue_type: IssueType::Task,
        status,
        assignee: None,
        priority: "P3".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
        story_points: None,
    };
    store.insert_issue(make_issue(IssueStatus::Open));
    store.insert_issue(make_issue(IssueStatus::Open));
    store.insert_issue(make_issue(IssueStatus::Done));

    let open = store.issues_by_status(&IssueStatus::Open);
    assert_eq!(open.len(), 2);
}

// ── Sprint store ──────────────────────────────────────────────────────────────

#[test]
fn store_insert_and_get_sprint() {
    let store = DevlakeStore::new();
    let sprint = Sprint {
        id: Uuid::new_v4(),
        name: "Sprint 1".to_string(),
        state: SprintState::Active,
        start_date: Utc::now(),
        end_date: Utc::now() + Duration::days(14),
        completed_points: 0,
        planned_points: 25,
        completed_issues: 0,
        planned_issues: 10,
    };
    let id = sprint.id;
    store.insert_sprint(sprint);
    assert!(store.get_sprint(id).is_some());
}

// ── DORA computation ──────────────────────────────────────────────────────────

#[test]
fn store_compute_dora_report_empty() {
    let store = DevlakeStore::new();
    let report = store.compute_dora_report(30);
    assert_eq!(report.period_days, 30);
    assert_eq!(report.deployment_frequency_per_day, 0.0);
    assert_eq!(report.change_failure_rate_pct, 0.0);
}

#[test]
fn store_compute_dora_report_with_data() {
    let store = DevlakeStore::new();
    let now = Utc::now();
    // Insert 5 successful + 1 failed deployment in the last 5 days
    for i in 0..5i64 {
        let mut d = make_deployment(DeploymentEnv::Production, DeploymentStatus::Success, Some(3600.0));
        d.deployed_at = now - Duration::days(i);
        store.insert_deployment(d);
    }
    let mut failed = make_deployment(DeploymentEnv::Production, DeploymentStatus::Failed, None);
    failed.deployed_at = now - Duration::days(1);
    store.insert_deployment(failed);

    let report = store.compute_dora_report(30);
    // 6 deployments over 30 days = 0.2/day
    assert!((report.deployment_frequency_per_day - 0.2).abs() < 0.001);
    // 1 failure out of 6 = 16.67%
    assert!((report.change_failure_rate_pct - (1.0/6.0 * 100.0)).abs() < 0.1);
}
