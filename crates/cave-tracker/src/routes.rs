// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::*;
use crate::{TrackerState, TrackerStore};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

// ===== Request Types =====

#[derive(serde::Deserialize)]
pub struct CreateProjectRequest {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub project_type: ProjectType,
    pub lead: String,
    pub workflow_id: Option<Uuid>,
}

#[derive(serde::Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub lead: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CreateIssueRequest {
    pub project_id: Uuid,
    pub issue_type: IssueType,
    pub summary: String,
    pub description: Option<String>,
    pub priority: Option<Priority>,
    pub assignee: Option<String>,
    pub reporter: String,
    pub labels: Option<Vec<String>>,
    pub story_points: Option<f64>,
    pub time_estimate_seconds: Option<u64>,
    pub epic_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub sprint_id: Option<Uuid>,
    pub custom_fields: Option<HashMap<String, serde_json::Value>>,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(serde::Deserialize)]
pub struct UpdateIssueRequest {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub priority: Option<Priority>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    pub story_points: Option<f64>,
    pub sprint_id: Option<Uuid>,
    pub custom_fields: Option<HashMap<String, serde_json::Value>>,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(serde::Deserialize)]
pub struct TransitionRequest {
    pub transition_id: String,
    pub comment: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CreateCommentRequest {
    pub author: String,
    pub body: String,
}

#[derive(serde::Deserialize)]
pub struct UpdateCommentRequest {
    pub body: String,
}

#[derive(serde::Deserialize)]
pub struct CreateSprintRequest {
    pub project_id: Uuid,
    pub board_id: Uuid,
    pub name: String,
    pub goal: Option<String>,
    pub end_date: Option<DateTime<Utc>>,
}

#[derive(serde::Deserialize)]
pub struct CreateIssueLinkRequest {
    pub to_issue_id: Uuid,
    pub link_type: LinkType,
}

#[derive(serde::Deserialize)]
pub struct LogTimeRequest {
    pub author: String,
    pub time_spent_seconds: u64,
    pub comment: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct BulkUpdate {
    pub assignee: Option<String>,
    pub priority: Option<Priority>,
    pub sprint_id: Option<Uuid>,
    pub labels: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
pub struct BulkUpdateRequest {
    pub issue_ids: Vec<Uuid>,
    pub updates: BulkUpdate,
}

#[derive(serde::Deserialize)]
pub struct BulkTransitionRequest {
    pub issue_ids: Vec<Uuid>,
    pub transition_id: String,
}

#[derive(serde::Deserialize)]
pub struct JqlQueryRequest {
    pub jql: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
pub struct IssueListQuery {
    pub project: Option<String>,
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub issue_type: Option<String>,
    pub priority: Option<String>,
    pub label: Option<String>,
    pub sprint_id: Option<Uuid>,
    pub jql: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub struct ActivityQuery {
    pub project_id: Option<Uuid>,
    pub issue_id: Option<Uuid>,
    pub limit: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
pub struct NotificationQuery {
    pub recipient: Option<String>,
    pub unread_only: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub struct SprintQuery {
    pub project_id: Option<Uuid>,
    pub state: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct AssignRequest {
    pub assignee: String,
}

#[derive(serde::Deserialize)]
pub struct WatchRequest {
    pub user: String,
}

#[derive(serde::Deserialize)]
pub struct RankRequest {
    pub rank: i64,
}

#[derive(serde::Deserialize)]
pub struct AddAttachmentRequest {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub url: String,
    pub uploaded_by: String,
}

#[derive(serde::Deserialize)]
pub struct CreateBoardRequest {
    pub project_id: Uuid,
    pub name: String,
    pub board_type: BoardType,
}

#[derive(serde::Deserialize)]
pub struct CreateFieldRequest {
    pub name: String,
    pub field_type: CustomFieldType,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub options: Option<Vec<String>>,
}

// ===== Helper =====

fn next_issue_key(project_key: &str, issues: &HashMap<Uuid, Issue>) -> String {
    let max_num = issues.values()
        .filter(|i| i.project_key == project_key)
        .filter_map(|i| i.key.split('-').nth(1).and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    format!("{}-{}", project_key, max_num + 1)
}

fn record_activity(store: &mut TrackerStore, event: ActivityEvent) {
    store.activity_events.push(event);
}

// ===== Router =====

pub fn create_router(state: Arc<TrackerState>) -> Router {
    Router::new()
        // Health
        .route("/api/tracker/health", get(health))
        // Projects
        .route("/api/tracker/projects", post(create_project).get(list_projects))
        .route("/api/tracker/projects/{id}", get(get_project).put(update_project).delete(delete_project))
        .route("/api/tracker/projects/{id}/board", get(get_project_board))
        .route("/api/tracker/projects/{id}/backlog", get(get_project_backlog))
        .route("/api/tracker/projects/{id}/stats", get(get_project_stats))
        // Issues
        .route("/api/tracker/issues", post(create_issue).get(list_issues))
        .route("/api/tracker/issues/bulk-update", post(bulk_update_issues))
        .route("/api/tracker/issues/bulk-transition", post(bulk_transition_issues))
        .route("/api/tracker/issues/{id}", get(get_issue).put(update_issue).delete(delete_issue))
        .route("/api/tracker/issues/{id}/transition", post(transition_issue))
        .route("/api/tracker/issues/{id}/transitions", get(list_transitions))
        .route("/api/tracker/issues/{id}/assign", post(assign_issue))
        .route("/api/tracker/issues/{id}/watch", post(watch_issue))
        .route("/api/tracker/issues/{id}/vote", post(vote_issue))
        .route("/api/tracker/issues/{id}/rank", post(rank_issue))
        // Comments
        .route("/api/tracker/issues/{id}/comments", post(add_comment).get(list_comments))
        .route("/api/tracker/comments/{id}", put(update_comment).delete(delete_comment))
        // Attachments
        .route("/api/tracker/issues/{id}/attachments", post(add_attachment).get(list_attachments))
        .route("/api/tracker/attachments/{id}", delete(delete_attachment))
        // Issue Links
        .route("/api/tracker/issues/{id}/links", post(create_link).get(list_links))
        .route("/api/tracker/links/{id}", delete(delete_link))
        // Time Tracking
        .route("/api/tracker/issues/{id}/timelog", post(log_time).get(get_timelogs))
        // Sprints
        .route("/api/tracker/sprints", post(create_sprint).get(list_sprints))
        .route("/api/tracker/sprints/{id}", get(get_sprint))
        .route("/api/tracker/sprints/{id}/start", post(start_sprint))
        .route("/api/tracker/sprints/{id}/complete", post(complete_sprint))
        .route("/api/tracker/sprints/{id}/issues", get(get_sprint_issues))
        .route("/api/tracker/sprints/{id}/stats", get(get_sprint_stats))
        // Boards
        .route("/api/tracker/boards", post(create_board))
        .route("/api/tracker/boards/{id}", get(get_board))
        .route("/api/tracker/boards/{id}/view", get(get_board_view))
        // Custom Fields
        .route("/api/tracker/fields", post(create_field_def).get(list_fields))
        .route("/api/tracker/fields/{id}", get(get_field).delete(delete_field))
        // Workflows
        .route("/api/tracker/workflows", get(list_workflows))
        .route("/api/tracker/workflows/{id}", get(get_workflow))
        // Query
        .route("/api/tracker/query", post(execute_query))
        // Activity & Notifications
        .route("/api/tracker/activity", get(get_activity))
        .route("/api/tracker/notifications", get(get_notifications))
        .route("/api/tracker/notifications/{id}/read", post(mark_notification_read))
        .with_state(state)
}

// ===== Health =====

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "module": "tracker" }))
}

// ===== Projects =====

async fn create_project(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;

    // Find workflow
    let workflow_id = req.workflow_id.unwrap_or_else(|| {
        store.workflows.values()
            .find(|w| w.is_default)
            .map(|w| w.id)
            .unwrap_or(Uuid::new_v4())
    });

    let project_id = Uuid::new_v4();
    let now = Utc::now();
    let project = Project {
        id: project_id,
        key: req.key.to_uppercase(),
        name: req.name.clone(),
        description: req.description.unwrap_or_default(),
        project_type: req.project_type.clone(),
        workflow_id,
        lead: req.lead,
        members: vec![],
        issue_types: vec![IssueType::Epic, IssueType::Story, IssueType::Task, IssueType::Bug, IssueType::Subtask],
        custom_field_ids: vec![],
        created_at: now,
        updated_at: now,
    };

    // Create default board
    let board = match req.project_type {
        ProjectType::Kanban => crate::board::default_kanban_board(project_id, &req.name),
        _ => crate::board::default_scrum_board(project_id, &req.name),
    };

    store.boards.insert(board.id, board);
    store.projects.insert(project_id, project.clone());

    (StatusCode::CREATED, Json(project))
}

async fn list_projects(State(state): State<Arc<TrackerState>>) -> impl IntoResponse {
    let store = state.store.read().await;
    let projects: Vec<Project> = store.projects.values().cloned().collect();
    Json(projects)
}

async fn get_project(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.projects.get(&id) {
        Some(p) => (StatusCode::OK, Json(serde_json::to_value(p).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Project not found" }))),
    }
}

async fn update_project(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.projects.get_mut(&id) {
        Some(p) => {
            if let Some(name) = req.name { p.name = name; }
            if let Some(desc) = req.description { p.description = desc; }
            if let Some(lead) = req.lead { p.lead = lead; }
            p.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::to_value(p.clone()).unwrap()))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Project not found" }))),
    }
}

async fn delete_project(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.projects.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

async fn get_project_board(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let board = store.boards.values().find(|b| b.project_id == id);
    match board {
        Some(board) => {
            let issue_refs: Vec<&Issue> = store.issues.values()
                .filter(|i| i.project_id == id)
                .collect();
            let view = crate::board::board_view(board, &issue_refs);
            let columns: Vec<serde_json::Value> = view.into_iter().map(|(col, issues)| {
                serde_json::json!({ "column": col, "issues": issues })
            }).collect();
            (StatusCode::OK, Json(serde_json::json!({ "board": board, "columns": columns })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Board not found" }))),
    }
}

async fn get_project_backlog(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let backlog: Vec<Issue> = crate::sprint::backlog_issues(store.issues.values(), id)
        .into_iter().cloned().collect();
    Json(backlog)
}

async fn get_project_stats(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let issues: Vec<&Issue> = store.issues.values().filter(|i| i.project_id == id).collect();
    let total = issues.len();

    let mut by_status: HashMap<&str, usize> = HashMap::new();
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for issue in &issues {
        *by_status.entry(issue.status.as_str()).or_insert(0) += 1;
        *by_type.entry(issue.issue_type.to_string()).or_insert(0) += 1;
    }

    Json(serde_json::json!({
        "project_id": id,
        "total_issues": total,
        "by_status": by_status,
        "by_type": by_type,
    }))
}

// ===== Issues =====

async fn create_issue(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateIssueRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;

    let project = match store.projects.get(&req.project_id) {
        Some(p) => p.clone(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Project not found" }))),
    };

    // Get default status from workflow
    let default_status = store.workflows.get(&project.workflow_id)
        .and_then(|wf| wf.statuses.first())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "To Do".to_string());

    let issue_key = next_issue_key(&project.key, &store.issues);
    let issue_id = Uuid::new_v4();
    let now = Utc::now();

    let issue = Issue {
        id: issue_id,
        key: issue_key,
        project_id: req.project_id,
        project_key: project.key.clone(),
        issue_type: req.issue_type,
        summary: req.summary,
        description: req.description,
        status: default_status,
        priority: req.priority.unwrap_or(Priority::Medium),
        assignee: req.assignee.clone(),
        reporter: req.reporter.clone(),
        labels: req.labels.unwrap_or_default(),
        components: vec![],
        fix_versions: vec![],
        affects_versions: vec![],
        epic_id: req.epic_id,
        parent_id: req.parent_id,
        sprint_id: req.sprint_id,
        story_points: req.story_points,
        time_estimate_seconds: req.time_estimate_seconds,
        time_spent_seconds: 0,
        custom_fields: req.custom_fields.unwrap_or_default(),
        watchers: vec![],
        votes: 0,
        rank: store.issues.len() as i64,
        resolution: None,
        due_date: req.due_date,
        created_at: now,
        updated_at: now,
        resolved_at: None,
    };

    let event = ActivityEvent {
        id: Uuid::new_v4(),
        issue_id: Some(issue_id),
        project_id: Some(req.project_id),
        actor: issue.reporter.clone(),
        event_type: ActivityEventType::IssueCreated,
        details: serde_json::json!({ "key": &issue.key, "summary": &issue.summary }),
        occurred_at: now,
    };

    store.issues.insert(issue_id, issue.clone());
    record_activity(&mut store, event);

    (StatusCode::CREATED, Json(serde_json::to_value(issue).unwrap()))
}

async fn list_issues(
    State(state): State<Arc<TrackerState>>,
    Query(query): Query<IssueListQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;

    if let Some(jql) = query.jql {
        let filter = crate::query::parse_jql(&jql);
        let results = crate::query::apply_filter(store.issues.values(), &filter);
        return Json(serde_json::to_value(results).unwrap());
    }

    let mut filter = crate::query::IssueFilter::default();
    filter.project_key = query.project;
    filter.status = query.status;
    filter.assignee = query.assignee;
    filter.sprint_id = query.sprint_id;
    filter.label = query.label;

    if let Some(pt) = query.priority {
        filter.priority = match pt.to_lowercase().as_str() {
            "critical" => Some(Priority::Critical),
            "high" => Some(Priority::High),
            "medium" => Some(Priority::Medium),
            "low" => Some(Priority::Low),
            "trivial" => Some(Priority::Trivial),
            _ => None,
        };
    }
    if let Some(it) = query.issue_type {
        filter.issue_type = match it.to_lowercase().as_str() {
            "epic" => Some(IssueType::Epic),
            "story" => Some(IssueType::Story),
            "task" => Some(IssueType::Task),
            "bug" => Some(IssueType::Bug),
            "subtask" => Some(IssueType::Subtask),
            _ => None,
        };
    }

    let results = crate::query::apply_filter(store.issues.values(), &filter);
    Json(serde_json::to_value(results).unwrap())
}

async fn get_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.issues.get(&id) {
        Some(i) => (StatusCode::OK, Json(serde_json::to_value(i).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

async fn update_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateIssueRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let old_assignee = store.issues.get(&id).and_then(|i| i.assignee.clone());

    match store.issues.get_mut(&id) {
        Some(issue) => {
            if let Some(summary) = req.summary { issue.summary = summary; }
            if let Some(description) = req.description { issue.description = Some(description); }
            if let Some(priority) = req.priority { issue.priority = priority; }
            if let Some(assignee) = req.assignee.clone() { issue.assignee = Some(assignee); }
            if let Some(labels) = req.labels { issue.labels = labels; }
            if let Some(sp) = req.story_points { issue.story_points = Some(sp); }
            if let Some(sid) = req.sprint_id { issue.sprint_id = Some(sid); }
            if let Some(cf) = req.custom_fields { issue.custom_fields.extend(cf); }
            if let Some(dd) = req.due_date { issue.due_date = Some(dd); }
            issue.updated_at = Utc::now();

            let issue_id = issue.id;
            let project_id = issue.project_id;
            let actor = issue.reporter.clone();
            let issue_val = serde_json::to_value(issue.clone()).unwrap();

            if req.assignee.is_some() && req.assignee != old_assignee {
                let event = ActivityEvent {
                    id: Uuid::new_v4(),
                    issue_id: Some(issue_id),
                    project_id: Some(project_id),
                    actor,
                    event_type: ActivityEventType::AssigneeChanged,
                    details: serde_json::json!({ "assignee": req.assignee }),
                    occurred_at: Utc::now(),
                };
                record_activity(&mut store, event);
            }

            (StatusCode::OK, Json(issue_val))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

async fn delete_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issues.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

async fn transition_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;

    let (project_id, workflow_id, current_status) = match store.issues.get(&id) {
        Some(issue) => {
            let project = store.projects.get(&issue.project_id).cloned();
            let wf_id = project.map(|p| p.workflow_id).unwrap_or(Uuid::nil());
            (issue.project_id, wf_id, issue.status.clone())
        }
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    };

    let workflow = match store.workflows.get(&workflow_id) {
        Some(wf) => wf.clone(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Workflow not found" }))),
    };

    if !crate::workflow::can_transition(&workflow, &current_status, &req.transition_id) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": format!("Cannot transition from '{}' with transition '{}'", current_status, req.transition_id)
        })));
    }

    let transition = match workflow.transitions.iter().find(|t| t.id == req.transition_id) {
        Some(t) => t.clone(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Transition not found" }))),
    };

    let new_status = transition.to_status.clone();
    let issue = store.issues.get_mut(&id).unwrap();
    crate::workflow::apply_transition(issue, &transition);

    let actor = issue.reporter.clone();
    let issue_val = serde_json::to_value(issue.clone()).unwrap();

    let event = ActivityEvent {
        id: Uuid::new_v4(),
        issue_id: Some(id),
        project_id: Some(project_id),
        actor,
        event_type: ActivityEventType::StatusChanged,
        details: serde_json::json!({
            "from": current_status,
            "to": new_status,
            "transition": req.transition_id,
            "comment": req.comment,
        }),
        occurred_at: Utc::now(),
    };
    record_activity(&mut store, event);

    (StatusCode::OK, Json(issue_val))
}

async fn list_transitions(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let issue = match store.issues.get(&id) {
        Some(i) => i,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    };
    let project = match store.projects.get(&issue.project_id) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Project not found" }))),
    };
    let workflow = match store.workflows.get(&project.workflow_id) {
        Some(wf) => wf,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Workflow not found" }))),
    };
    let transitions = crate::workflow::available_transitions(workflow, &issue.status);
    (StatusCode::OK, Json(serde_json::to_value(transitions).unwrap()))
}

async fn assign_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issues.get_mut(&id) {
        Some(issue) => {
            issue.assignee = Some(req.assignee.clone());
            issue.updated_at = Utc::now();
            let issue_id = issue.id;
            let project_id = issue.project_id;
            let actor = issue.reporter.clone();
            let issue_val = serde_json::to_value(issue.clone()).unwrap();
            let event = ActivityEvent {
                id: Uuid::new_v4(),
                issue_id: Some(issue_id),
                project_id: Some(project_id),
                actor,
                event_type: ActivityEventType::AssigneeChanged,
                details: serde_json::json!({ "assignee": req.assignee }),
                occurred_at: Utc::now(),
            };
            record_activity(&mut store, event);
            (StatusCode::OK, Json(issue_val))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

async fn watch_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<WatchRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issues.get_mut(&id) {
        Some(issue) => {
            if !issue.watchers.contains(&req.user) {
                issue.watchers.push(req.user.clone());
            }
            issue.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::to_value(issue.clone()).unwrap()))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

async fn vote_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issues.get_mut(&id) {
        Some(issue) => {
            issue.votes += 1;
            issue.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::json!({ "votes": issue.votes })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

async fn rank_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RankRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issues.get_mut(&id) {
        Some(issue) => {
            issue.rank = req.rank;
            issue.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::json!({ "rank": issue.rank })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" }))),
    }
}

// ===== Comments =====

async fn add_comment(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<CreateCommentRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    if !store.issues.contains_key(&issue_id) {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" })));
    }
    let now = Utc::now();
    let comment = Comment {
        id: Uuid::new_v4(),
        issue_id,
        author: req.author.clone(),
        body: req.body.clone(),
        mentions: vec![],
        created_at: now,
        updated_at: now,
    };
    let event = ActivityEvent {
        id: Uuid::new_v4(),
        issue_id: Some(issue_id),
        project_id: store.issues.get(&issue_id).map(|i| i.project_id),
        actor: req.author,
        event_type: ActivityEventType::CommentAdded,
        details: serde_json::json!({ "comment_id": comment.id }),
        occurred_at: now,
    };
    store.comments.insert(comment.id, comment.clone());
    record_activity(&mut store, event);
    (StatusCode::CREATED, Json(serde_json::to_value(comment).unwrap()))
}

async fn list_comments(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let comments: Vec<Comment> = store.comments.values()
        .filter(|c| c.issue_id == issue_id)
        .cloned()
        .collect();
    Json(comments)
}

async fn update_comment(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCommentRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.comments.get_mut(&id) {
        Some(c) => {
            c.body = req.body;
            c.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::to_value(c.clone()).unwrap()))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Comment not found" }))),
    }
}

async fn delete_comment(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.comments.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

// ===== Attachments =====

async fn add_attachment(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<AddAttachmentRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    if !store.issues.contains_key(&issue_id) {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" })));
    }
    let now = Utc::now();
    let attachment = Attachment {
        id: Uuid::new_v4(),
        issue_id,
        filename: req.filename,
        content_type: req.content_type,
        size_bytes: req.size_bytes,
        url: req.url,
        uploaded_by: req.uploaded_by.clone(),
        uploaded_at: now,
    };
    let event = ActivityEvent {
        id: Uuid::new_v4(),
        issue_id: Some(issue_id),
        project_id: store.issues.get(&issue_id).map(|i| i.project_id),
        actor: req.uploaded_by,
        event_type: ActivityEventType::AttachmentAdded,
        details: serde_json::json!({ "attachment_id": attachment.id, "filename": &attachment.filename }),
        occurred_at: now,
    };
    store.attachments.insert(attachment.id, attachment.clone());
    record_activity(&mut store, event);
    (StatusCode::CREATED, Json(serde_json::to_value(attachment).unwrap()))
}

async fn list_attachments(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let attachments: Vec<Attachment> = store.attachments.values()
        .filter(|a| a.issue_id == issue_id)
        .cloned()
        .collect();
    Json(attachments)
}

async fn delete_attachment(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.attachments.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

// ===== Issue Links =====

async fn create_link(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<CreateIssueLinkRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    if !store.issues.contains_key(&issue_id) {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" })));
    }
    if !store.issues.contains_key(&req.to_issue_id) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Target issue not found" })));
    }
    let now = Utc::now();
    let link = IssueLink {
        id: Uuid::new_v4(),
        from_issue_id: issue_id,
        to_issue_id: req.to_issue_id,
        link_type: req.link_type,
        created_at: now,
    };
    let event = ActivityEvent {
        id: Uuid::new_v4(),
        issue_id: Some(issue_id),
        project_id: store.issues.get(&issue_id).map(|i| i.project_id),
        actor: "system".to_string(),
        event_type: ActivityEventType::IssueLinked,
        details: serde_json::json!({ "link_id": link.id, "to_issue_id": req.to_issue_id }),
        occurred_at: now,
    };
    store.issue_links.insert(link.id, link.clone());
    record_activity(&mut store, event);
    (StatusCode::CREATED, Json(serde_json::to_value(link).unwrap()))
}

async fn list_links(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let links: Vec<IssueLink> = store.issue_links.values()
        .filter(|l| l.from_issue_id == issue_id || l.to_issue_id == issue_id)
        .cloned()
        .collect();
    Json(links)
}

async fn delete_link(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.issue_links.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

// ===== Time Tracking =====

async fn log_time(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<LogTimeRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    if !store.issues.contains_key(&issue_id) {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Issue not found" })));
    }
    let now = Utc::now();
    let log = TimeLog {
        id: Uuid::new_v4(),
        issue_id,
        author: req.author,
        time_spent_seconds: req.time_spent_seconds,
        work_date: now,
        comment: req.comment,
        created_at: now,
    };
    if let Some(issue) = store.issues.get_mut(&issue_id) {
        issue.time_spent_seconds += req.time_spent_seconds;
        issue.updated_at = now;
    }
    store.time_logs.insert(log.id, log.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(log).unwrap()))
}

async fn get_timelogs(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let logs: Vec<TimeLog> = store.time_logs.values()
        .filter(|l| l.issue_id == issue_id)
        .cloned()
        .collect();
    Json(logs)
}

// ===== Sprints =====

async fn create_sprint(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateSprintRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let sprint = Sprint {
        id: Uuid::new_v4(),
        project_id: req.project_id,
        board_id: req.board_id,
        name: req.name,
        goal: req.goal,
        state: SprintState::Future,
        start_date: None,
        end_date: req.end_date,
        completed_at: None,
        velocity: None,
        created_at: Utc::now(),
    };
    store.sprints.insert(sprint.id, sprint.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(sprint).unwrap()))
}

async fn list_sprints(
    State(state): State<Arc<TrackerState>>,
    Query(query): Query<SprintQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let sprints: Vec<Sprint> = store.sprints.values()
        .filter(|s| {
            if let Some(pid) = query.project_id { if s.project_id != pid { return false; } }
            if let Some(ref st) = query.state {
                let matches = match st.as_str() {
                    "future" => s.state == SprintState::Future,
                    "active" => s.state == SprintState::Active,
                    "closed" => s.state == SprintState::Closed,
                    _ => true,
                };
                if !matches { return false; }
            }
            true
        })
        .cloned()
        .collect();
    Json(sprints)
}

async fn get_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.sprints.get(&id) {
        Some(s) => (StatusCode::OK, Json(serde_json::to_value(s).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Sprint not found" }))),
    }
}

async fn start_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let project_id = store.sprints.get(&id).map(|s| s.project_id);
    match store.sprints.get_mut(&id) {
        Some(sprint) => {
            match crate::sprint::start_sprint(sprint) {
                Ok(()) => {
                    let sprint_val = serde_json::to_value(sprint.clone()).unwrap();
                    let event = ActivityEvent {
                        id: Uuid::new_v4(),
                        issue_id: None,
                        project_id,
                        actor: "system".to_string(),
                        event_type: ActivityEventType::SprintStarted,
                        details: serde_json::json!({ "sprint_id": id }),
                        occurred_at: Utc::now(),
                    };
                    record_activity(&mut store, event);
                    (StatusCode::OK, Json(sprint_val))
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
            }
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Sprint not found" }))),
    }
}

async fn complete_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let sprint_issues: Vec<Issue> = store.issues.values()
        .filter(|i| i.sprint_id == Some(id))
        .cloned()
        .collect();
    let issue_refs: Vec<&Issue> = sprint_issues.iter().collect();
    let project_id = store.sprints.get(&id).map(|s| s.project_id);

    match store.sprints.get_mut(&id) {
        Some(sprint) => {
            match crate::sprint::complete_sprint(sprint, &issue_refs) {
                Ok(()) => {
                    let sprint_val = serde_json::to_value(sprint.clone()).unwrap();
                    let event = ActivityEvent {
                        id: Uuid::new_v4(),
                        issue_id: None,
                        project_id,
                        actor: "system".to_string(),
                        event_type: ActivityEventType::SprintCompleted,
                        details: serde_json::json!({ "sprint_id": id }),
                        occurred_at: Utc::now(),
                    };
                    record_activity(&mut store, event);
                    (StatusCode::OK, Json(sprint_val))
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))),
            }
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Sprint not found" }))),
    }
}

async fn get_sprint_issues(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let issues: Vec<Issue> = store.issues.values()
        .filter(|i| i.sprint_id == Some(id))
        .cloned()
        .collect();
    Json(issues)
}

async fn get_sprint_stats(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let sprint = match store.sprints.get(&id) {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Sprint not found" }))),
    };
    let issues: Vec<&Issue> = store.issues.values()
        .filter(|i| i.sprint_id == Some(id))
        .collect();
    let stats = crate::sprint::sprint_stats(sprint, &issues);
    (StatusCode::OK, Json(stats))
}

// ===== Boards =====

async fn create_board(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateBoardRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let board = Board {
        id: Uuid::new_v4(),
        project_id: req.project_id,
        name: req.name,
        board_type: req.board_type,
        columns: vec![
            BoardColumn { name: "To Do".to_string(), statuses: vec!["To Do".to_string()], wip_limit: None },
            BoardColumn { name: "In Progress".to_string(), statuses: vec!["In Progress".to_string()], wip_limit: None },
            BoardColumn { name: "Done".to_string(), statuses: vec!["Done".to_string()], wip_limit: None },
        ],
        backlog_enabled: true,
        current_sprint_id: None,
        created_at: Utc::now(),
    };
    store.boards.insert(board.id, board.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(board).unwrap()))
}

async fn get_board(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.boards.get(&id) {
        Some(b) => (StatusCode::OK, Json(serde_json::to_value(b).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Board not found" }))),
    }
}

async fn get_board_view(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let board = match store.boards.get(&id) {
        Some(b) => b,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Board not found" }))),
    };
    let issue_refs: Vec<&Issue> = store.issues.values()
        .filter(|i| i.project_id == board.project_id)
        .collect();
    let view = crate::board::board_view(board, &issue_refs);
    let wip_violations = crate::board::check_wip_violations(board, &issue_refs);
    let columns: Vec<serde_json::Value> = view.into_iter().map(|(col, issues)| {
        serde_json::json!({ "column": col, "issues": issues })
    }).collect();
    (StatusCode::OK, Json(serde_json::json!({
        "board": board,
        "columns": columns,
        "wip_violations": wip_violations,
    })))
}

// ===== Custom Fields =====

async fn create_field_def(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateFieldRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let field = crate::fields::create_field(
        &req.name,
        req.field_type,
        req.description.as_deref().unwrap_or(""),
        req.required.unwrap_or(false),
    );
    let mut field = field;
    if let Some(opts) = req.options { field.options = opts; }
    store.custom_field_defs.insert(field.id, field.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(field).unwrap()))
}

async fn list_fields(State(state): State<Arc<TrackerState>>) -> impl IntoResponse {
    let store = state.store.read().await;
    let fields: Vec<CustomFieldDef> = store.custom_field_defs.values().cloned().collect();
    Json(fields)
}

async fn get_field(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.custom_field_defs.get(&id) {
        Some(f) => (StatusCode::OK, Json(serde_json::to_value(f).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Field not found" }))),
    }
}

async fn delete_field(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.custom_field_defs.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

// ===== Workflows =====

async fn list_workflows(State(state): State<Arc<TrackerState>>) -> impl IntoResponse {
    let store = state.store.read().await;
    let workflows: Vec<Workflow> = store.workflows.values().cloned().collect();
    Json(workflows)
}

async fn get_workflow(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.workflows.get(&id) {
        Some(w) => (StatusCode::OK, Json(serde_json::to_value(w).unwrap())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Workflow not found" }))),
    }
}

// ===== Query =====

async fn execute_query(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<JqlQueryRequest>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let mut filter = crate::query::parse_jql(&req.jql);
    filter.limit = req.limit;
    filter.offset = req.offset;
    let results = crate::query::apply_filter(store.issues.values(), &filter);
    Json(serde_json::json!({
        "total": results.len(),
        "issues": results,
    }))
}

// ===== Bulk Operations =====

async fn bulk_update_issues(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<BulkUpdateRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let mut updated = 0usize;
    for id in &req.issue_ids {
        if let Some(issue) = store.issues.get_mut(id) {
            if let Some(ref a) = req.updates.assignee { issue.assignee = Some(a.clone()); }
            if let Some(ref p) = req.updates.priority { issue.priority = p.clone(); }
            if let Some(sid) = req.updates.sprint_id { issue.sprint_id = Some(sid); }
            if let Some(ref labels) = req.updates.labels { issue.labels = labels.clone(); }
            issue.updated_at = Utc::now();
            updated += 1;
        }
    }
    Json(serde_json::json!({ "updated": updated }))
}

async fn bulk_transition_issues(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<BulkTransitionRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;

    // Collect project->workflow mapping
    let project_workflows: HashMap<Uuid, Uuid> = store.projects.values()
        .map(|p| (p.id, p.workflow_id))
        .collect();
    let workflows: HashMap<Uuid, Workflow> = store.workflows.clone();

    let mut transitioned = 0usize;
    let mut errors: Vec<String> = vec![];

    for id in &req.issue_ids {
        if let Some(issue) = store.issues.get_mut(id) {
            let wf_id = project_workflows.get(&issue.project_id).copied().unwrap_or(Uuid::nil());
            if let Some(workflow) = workflows.get(&wf_id) {
                if crate::workflow::can_transition(workflow, &issue.status, &req.transition_id) {
                    if let Some(transition) = workflow.transitions.iter().find(|t| t.id == req.transition_id) {
                        let t = transition.clone();
                        crate::workflow::apply_transition(issue, &t);
                        transitioned += 1;
                    }
                } else {
                    errors.push(format!("Issue {} cannot transition from '{}'", issue.key, issue.status));
                }
            }
        }
    }

    Json(serde_json::json!({ "transitioned": transitioned, "errors": errors }))
}

// ===== Activity & Notifications =====

async fn get_activity(
    State(state): State<Arc<TrackerState>>,
    Query(query): Query<ActivityQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let limit = query.limit.unwrap_or(50);
    let events: Vec<ActivityEvent> = store.activity_events.iter()
        .filter(|e| {
            if let Some(pid) = query.project_id {
                if e.project_id != Some(pid) { return false; }
            }
            if let Some(iid) = query.issue_id {
                if e.issue_id != Some(iid) { return false; }
            }
            true
        })
        .rev()
        .take(limit)
        .cloned()
        .collect();
    Json(events)
}

async fn get_notifications(
    State(state): State<Arc<TrackerState>>,
    Query(query): Query<NotificationQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let notifications: Vec<Notification> = store.notifications.iter()
        .filter(|n| {
            if let Some(ref r) = query.recipient {
                if &n.recipient != r { return false; }
            }
            if let Some(unread_only) = query.unread_only {
                if unread_only && n.read { return false; }
            }
            true
        })
        .cloned()
        .collect();
    Json(notifications)
}

async fn mark_notification_read(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let notification = store.notifications.iter_mut().find(|n| n.id == id);
    match notification {
        Some(n) => {
            n.read = true;
            (StatusCode::OK, Json(serde_json::json!({ "read": true })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Notification not found" }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::Response,
    };
    use tower::util::ServiceExt;

    fn test_state() -> Arc<TrackerState> {
        Arc::new(TrackerState::default())
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = create_router(test_state());
        let response: Response = app
            .oneshot(Request::builder().uri("/api/tracker/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_projects_empty() {
        let app = create_router(test_state());
        let response: Response = app
            .oneshot(Request::builder().uri("/api/tracker/projects").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_and_get_project() {
        let state = test_state();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "key": "TEST",
            "name": "Test Project",
            "project_type": "scrum",
            "lead": "admin"
        });

        let response: Response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/tracker/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_list_workflows() {
        let app = create_router(test_state());
        let response: Response = app
            .oneshot(Request::builder().uri("/api/tracker/workflows").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
