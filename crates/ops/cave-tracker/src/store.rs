// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for cave-tracker (stub for production cave-db backend).

use crate::jql::{JqlEvaluator, JqlParser};
use crate::models::{
    BulkOperation, BulkOperationRequest, Comment, CreateIssueRequest, CreateSprintRequest, Issue,
    IssueStatus, JqlResult, Sprint, SprintState, UpdateIssueRequest,
};
use crate::workflow::WorkflowEngine;
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS cave_tracker.issues (
    id UUID PRIMARY KEY,
    project_key TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    issue_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    description TEXT,
    assignee UUID,
    reporter UUID NOT NULL,
    priority TEXT NOT NULL DEFAULT 'P3',
    status TEXT NOT NULL DEFAULT 'todo',
    labels TEXT[] NOT NULL DEFAULT '{}',
    components TEXT[] NOT NULL DEFAULT '{}',
    sprint_id UUID,
    story_points REAL,
    due_date TIMESTAMPTZ,
    parent_id UUID,
    epic_id UUID,
    original_estimate_minutes INTEGER,
    time_spent_minutes INTEGER,
    remaining_estimate_minutes INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by UUID NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tracker_issues_key ON cave_tracker.issues(project_key, issue_number);
CREATE INDEX IF NOT EXISTS idx_tracker_issues_status ON cave_tracker.issues(status);
CREATE INDEX IF NOT EXISTS idx_tracker_issues_assignee ON cave_tracker.issues(assignee);
CREATE INDEX IF NOT EXISTS idx_tracker_issues_sprint ON cave_tracker.issues(sprint_id);
CREATE INDEX IF NOT EXISTS idx_tracker_issues_labels ON cave_tracker.issues USING GIN(labels);

CREATE TABLE IF NOT EXISTS cave_tracker.sprints (
    id UUID PRIMARY KEY,
    project_key TEXT NOT NULL,
    name TEXT NOT NULL,
    goal TEXT,
    state TEXT NOT NULL DEFAULT 'planning',
    start_date TIMESTAMPTZ,
    end_date TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS cave_tracker.comments (
    id UUID PRIMARY KEY,
    issue_id UUID NOT NULL REFERENCES cave_tracker.issues(id) ON DELETE CASCADE,
    author UUID NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS cave_tracker.issue_watchers (
    issue_id UUID NOT NULL REFERENCES cave_tracker.issues(id) ON DELETE CASCADE,
    user_id UUID NOT NULL,
    PRIMARY KEY (issue_id, user_id)
);

CREATE TABLE IF NOT EXISTS cave_tracker.issue_events (
    id BIGSERIAL PRIMARY KEY,
    issue_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    actor UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SprintVelocity {
    pub sprint_id: Uuid,
    pub sprint_name: String,
    pub completed_points: f32,
    pub committed_points: f32,
}

pub struct TrackerStore {
    issues: Arc<Mutex<Vec<Issue>>>,
    sprints: Arc<Mutex<Vec<Sprint>>>,
    comments: Arc<Mutex<Vec<Comment>>>,
    workflow: WorkflowEngine,
}

impl TrackerStore {
    pub fn new() -> Self {
        Self {
            issues: Arc::new(Mutex::new(Vec::new())),
            sprints: Arc::new(Mutex::new(Vec::new())),
            comments: Arc::new(Mutex::new(Vec::new())),
            workflow: WorkflowEngine::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Issues
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub fn list_issues(&self, project_key: &str) -> Vec<Issue> {
        let issues = self.issues.lock().unwrap();
        issues
            .iter()
            .filter(|i| i.project_key == project_key)
            .cloned()
            .collect()
    }

    #[allow(dead_code)]
    pub fn get_issue(&self, id: Uuid) -> Option<Issue> {
        let issues = self.issues.lock().unwrap();
        issues.iter().find(|i| i.id == id).cloned()
    }

    #[allow(dead_code)]
    pub fn create_issue(
        &self,
        req: CreateIssueRequest,
        reporter: Uuid,
        project_key: &str,
    ) -> Issue {
        let mut issues = self.issues.lock().unwrap();
        let next_number = issues
            .iter()
            .filter(|i| i.project_key == project_key)
            .map(|i| i.issue_number)
            .max()
            .unwrap_or(0)
            + 1;

        let now = Utc::now();
        let issue = Issue {
            id: Uuid::new_v4(),
            project_key: project_key.to_string(),
            issue_number: next_number,
            issue_type: req.issue_type,
            summary: req.summary,
            description: req.description,
            assignee: req.assignee,
            reporter,
            priority: req.priority.unwrap_or(crate::models::Priority::P3),
            status: IssueStatus::ToDo,
            labels: req.labels.unwrap_or_default(),
            components: req.components.unwrap_or_default(),
            sprint_id: req.sprint_id,
            story_points: req.story_points,
            due_date: req.due_date,
            parent_id: req.parent_id,
            epic_id: req.epic_id,
            created_at: now,
            updated_at: now,
            created_by: reporter,
            original_estimate_minutes: req.original_estimate_minutes,
            time_spent_minutes: None,
            remaining_estimate_minutes: req.original_estimate_minutes,
        };
        issues.push(issue.clone());
        issue
    }

    #[allow(dead_code)]
    pub fn update_issue(&self, id: Uuid, req: UpdateIssueRequest) -> Option<Issue> {
        let mut issues = self.issues.lock().unwrap();
        if let Some(issue) = issues.iter_mut().find(|i| i.id == id) {
            if let Some(s) = req.summary {
                issue.summary = s;
            }
            if let Some(d) = req.description {
                issue.description = Some(d);
            }
            if let Some(a) = req.assignee {
                issue.assignee = Some(a);
            }
            if let Some(p) = req.priority {
                issue.priority = p;
            }
            if let Some(l) = req.labels {
                issue.labels = l;
            }
            if let Some(c) = req.components {
                issue.components = c;
            }
            if let Some(s) = req.sprint_id {
                issue.sprint_id = Some(s);
            }
            if let Some(sp) = req.story_points {
                issue.story_points = Some(sp);
            }
            if let Some(dd) = req.due_date {
                issue.due_date = Some(dd);
            }
            if let Some(e) = req.epic_id {
                issue.epic_id = Some(e);
            }
            if let Some(ts) = req.time_spent_minutes {
                issue.time_spent_minutes = Some(ts);
            }
            if let Some(re) = req.remaining_estimate_minutes {
                issue.remaining_estimate_minutes = Some(re);
            }
            issue.updated_at = Utc::now();
            Some(issue.clone())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn transition_issue(
        &self,
        id: Uuid,
        to_status: IssueStatus,
        _actor: Uuid,
    ) -> Result<Issue, String> {
        let mut issues = self.issues.lock().unwrap();
        if let Some(issue) = issues.iter_mut().find(|i| i.id == id) {
            if !self.workflow.can_transition(&issue.status, &to_status) {
                return Err(format!(
                    "Transition from {:?} to {:?} is not allowed",
                    issue.status, to_status
                ));
            }
            issue.status = to_status;
            issue.updated_at = Utc::now();
            Ok(issue.clone())
        } else {
            Err(format!("Issue {id} not found"))
        }
    }

    // -----------------------------------------------------------------------
    // Sprints
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub fn list_sprints(&self, project_key: &str) -> Vec<Sprint> {
        let sprints = self.sprints.lock().unwrap();
        sprints
            .iter()
            .filter(|s| s.project_key == project_key)
            .cloned()
            .collect()
    }

    #[allow(dead_code)]
    pub fn create_sprint(&self, req: CreateSprintRequest, project_key: &str) -> Sprint {
        let mut sprints = self.sprints.lock().unwrap();
        let sprint = Sprint {
            id: Uuid::new_v4(),
            project_key: project_key.to_string(),
            name: req.name,
            goal: req.goal,
            state: SprintState::Planning,
            start_date: req.start_date,
            end_date: req.end_date,
            created_at: Utc::now(),
        };
        sprints.push(sprint.clone());
        sprint
    }

    #[allow(dead_code)]
    pub fn start_sprint(&self, id: Uuid) -> Option<Sprint> {
        let mut sprints = self.sprints.lock().unwrap();
        if let Some(sprint) = sprints.iter_mut().find(|s| s.id == id) {
            sprint.state = SprintState::Active;
            if sprint.start_date.is_none() {
                sprint.start_date = Some(Utc::now());
            }
            Some(sprint.clone())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn complete_sprint(&self, id: Uuid) -> Option<Sprint> {
        let mut sprints = self.sprints.lock().unwrap();
        if let Some(sprint) = sprints.iter_mut().find(|s| s.id == id) {
            sprint.state = SprintState::Completed;
            if sprint.end_date.is_none() {
                sprint.end_date = Some(Utc::now());
            }
            Some(sprint.clone())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn get_sprint_backlog(&self, sprint_id: Uuid) -> Vec<Issue> {
        let issues = self.issues.lock().unwrap();
        issues
            .iter()
            .filter(|i| i.sprint_id == Some(sprint_id))
            .cloned()
            .collect()
    }

    #[allow(dead_code)]
    pub fn get_sprint_velocity(&self, project_key: &str) -> Vec<SprintVelocity> {
        let sprints = self.sprints.lock().unwrap();
        let issues = self.issues.lock().unwrap();

        sprints
            .iter()
            .filter(|s| s.project_key == project_key && s.state == SprintState::Completed)
            .map(|sprint| {
                let sprint_issues: Vec<&Issue> = issues
                    .iter()
                    .filter(|i| i.sprint_id == Some(sprint.id))
                    .collect();

                let committed_points: f32 =
                    sprint_issues.iter().map(|i| i.story_points.unwrap_or(0.0)).sum();

                let completed_points: f32 = sprint_issues
                    .iter()
                    .filter(|i| i.status == IssueStatus::Done)
                    .map(|i| i.story_points.unwrap_or(0.0))
                    .sum();

                SprintVelocity {
                    sprint_id: sprint.id,
                    sprint_name: sprint.name.clone(),
                    completed_points,
                    committed_points,
                }
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Bulk operations
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub fn bulk_operate(&self, req: BulkOperationRequest) -> Vec<Issue> {
        let mut issues = self.issues.lock().unwrap();
        let now = Utc::now();
        let mut updated = Vec::new();

        for issue in issues.iter_mut() {
            if !req.issue_ids.contains(&issue.id) {
                continue;
            }
            match &req.operation {
                BulkOperation::Assign(user_id) => {
                    issue.assignee = Some(*user_id);
                    issue.updated_at = now;
                }
                BulkOperation::Transition(status) => {
                    if self.workflow.can_transition(&issue.status, status) {
                        issue.status = status.clone();
                        issue.updated_at = now;
                    }
                }
                BulkOperation::AddLabel(label) => {
                    if !issue.labels.contains(label) {
                        issue.labels.push(label.clone());
                    }
                    issue.updated_at = now;
                }
                BulkOperation::RemoveLabel(label) => {
                    issue.labels.retain(|l| l != label);
                    issue.updated_at = now;
                }
            }
            updated.push(issue.clone());
        }

        updated
    }

    // -----------------------------------------------------------------------
    // JQL
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub fn query_jql(&self, query: &str, current_user: Option<Uuid>) -> JqlResult {
        let issues = self.issues.lock().unwrap();
        match JqlParser::parse(query) {
            Ok(parsed) => {
                let matched = JqlEvaluator::evaluate(&issues, &parsed, current_user);
                let total = matched.len();
                JqlResult {
                    issues: matched,
                    total,
                }
            }
            Err(_) => JqlResult {
                issues: vec![],
                total: 0,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Comments
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    pub fn add_comment(&self, issue_id: Uuid, author: Uuid, body: String) -> Comment {
        let mut comments = self.comments.lock().unwrap();
        let now = Utc::now();
        let comment = Comment {
            id: Uuid::new_v4(),
            issue_id,
            author,
            body,
            created_at: now,
            updated_at: now,
        };
        comments.push(comment.clone());
        comment
    }

    #[allow(dead_code)]
    pub fn list_comments(&self, issue_id: Uuid) -> Vec<Comment> {
        let comments = self.comments.lock().unwrap();
        comments
            .iter()
            .filter(|c| c.issue_id == issue_id)
            .cloned()
            .collect()
    }
}

impl Default for TrackerStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BulkOperation, BulkOperationRequest, IssueType, Priority};

    fn make_create_req(summary: &str) -> CreateIssueRequest {
        CreateIssueRequest {
            issue_type: IssueType::Task,
            summary: summary.to_string(),
            description: None,
            assignee: None,
            priority: None,
            labels: None,
            components: None,
            sprint_id: None,
            story_points: None,
            due_date: None,
            parent_id: None,
            epic_id: None,
            original_estimate_minutes: None,
        }
    }

    #[test]
    fn test_create_and_get_issue() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("Fix bug"), reporter, "CAVE");
        assert_eq!(issue.summary, "Fix bug");
        assert_eq!(issue.status, IssueStatus::ToDo);
        assert_eq!(issue.priority, Priority::P3);

        let fetched = store.get_issue(issue.id).unwrap();
        assert_eq!(fetched.id, issue.id);
    }

    #[test]
    fn test_issue_number_increments() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let a = store.create_issue(make_create_req("A"), reporter, "CAVE");
        let b = store.create_issue(make_create_req("B"), reporter, "CAVE");
        assert_eq!(b.issue_number, a.issue_number + 1);
    }

    #[test]
    fn test_list_issues_by_project() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        store.create_issue(make_create_req("A"), reporter, "CAVE");
        store.create_issue(make_create_req("B"), reporter, "OTHER");
        let cave_issues = store.list_issues("CAVE");
        assert_eq!(cave_issues.len(), 1);
    }

    #[test]
    fn test_update_issue() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("Old summary"), reporter, "CAVE");
        let updated = store.update_issue(
            issue.id,
            UpdateIssueRequest {
                summary: Some("New summary".to_string()),
                description: None,
                assignee: None,
                priority: Some(Priority::P1),
                labels: None,
                components: None,
                sprint_id: None,
                story_points: None,
                due_date: None,
                epic_id: None,
                time_spent_minutes: None,
                remaining_estimate_minutes: None,
            },
        );
        let updated = updated.unwrap();
        assert_eq!(updated.summary, "New summary");
        assert_eq!(updated.priority, Priority::P1);
    }

    #[test]
    fn test_transition_issue_valid() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("X"), reporter, "CAVE");
        let actor = Uuid::new_v4();
        let result = store.transition_issue(issue.id, IssueStatus::InProgress, actor);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().status, IssueStatus::InProgress);
    }

    #[test]
    fn test_transition_issue_invalid() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("X"), reporter, "CAVE");
        let actor = Uuid::new_v4();
        // ToDo → Done is not a valid transition
        let result = store.transition_issue(issue.id, IssueStatus::Done, actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_sprint_lifecycle() {
        let store = TrackerStore::new();
        let sprint = store.create_sprint(
            CreateSprintRequest {
                name: "Sprint 1".to_string(),
                goal: Some("Deliver MVP".to_string()),
                start_date: None,
                end_date: None,
            },
            "CAVE",
        );
        assert_eq!(sprint.state, SprintState::Planning);

        let started = store.start_sprint(sprint.id).unwrap();
        assert_eq!(started.state, SprintState::Active);

        let completed = store.complete_sprint(sprint.id).unwrap();
        assert_eq!(completed.state, SprintState::Completed);
    }

    #[test]
    fn test_sprint_backlog() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let sprint = store.create_sprint(
            CreateSprintRequest {
                name: "S1".to_string(),
                goal: None,
                start_date: None,
                end_date: None,
            },
            "CAVE",
        );
        let mut req = make_create_req("Task in sprint");
        req.sprint_id = Some(sprint.id);
        store.create_issue(req, reporter, "CAVE");
        store.create_issue(make_create_req("Backlog task"), reporter, "CAVE");

        let backlog = store.get_sprint_backlog(sprint.id);
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].sprint_id, Some(sprint.id));
    }

    #[test]
    fn test_bulk_assign() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("A"), reporter, "CAVE");
        let user = Uuid::new_v4();
        let updated = store.bulk_operate(BulkOperationRequest {
            issue_ids: vec![issue.id],
            operation: BulkOperation::Assign(user),
        });
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].assignee, Some(user));
    }

    #[test]
    fn test_bulk_add_remove_label() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("A"), reporter, "CAVE");
        store.bulk_operate(BulkOperationRequest {
            issue_ids: vec![issue.id],
            operation: BulkOperation::AddLabel("frontend".to_string()),
        });
        let updated = store.bulk_operate(BulkOperationRequest {
            issue_ids: vec![issue.id],
            operation: BulkOperation::RemoveLabel("frontend".to_string()),
        });
        assert!(updated[0].labels.is_empty());
    }

    #[test]
    fn test_comments_crud() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        let issue = store.create_issue(make_create_req("Issue"), reporter, "CAVE");
        let author = Uuid::new_v4();
        let comment = store.add_comment(issue.id, author, "Nice work!".to_string());
        assert_eq!(comment.body, "Nice work!");

        let comments = store.list_comments(issue.id);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, comment.id);
    }

    #[test]
    fn test_jql_query_via_store() {
        let store = TrackerStore::new();
        let reporter = Uuid::new_v4();
        store.create_issue(make_create_req("Task A"), reporter, "CAVE");
        store.create_issue(make_create_req("Task B"), reporter, "OTHER");

        let result = store.query_jql("project = CAVE", None);
        assert_eq!(result.total, 1);
        assert_eq!(result.issues[0].project_key, "CAVE");
    }
}
