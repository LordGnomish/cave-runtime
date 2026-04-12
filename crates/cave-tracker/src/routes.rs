//! HTTP routes for cave-tracker.

use crate::automation::{self, Event};
use crate::models::*;
use crate::tracker::{self, CreateIssueRequest, UpdateIssueRequest};
use crate::{board, roadmap, TrackerState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ── Convenience alias ─────────────────────────────────────────────────────────

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": msg})))
}

fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": msg})))
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<TrackerState>) -> Router {
    Router::new()
        // Health
        .route("/api/v1/tracker/health", get(health))
        // Full-text search
        .route("/api/v1/tracker/search", get(search_issues))
        // Projects
        .route("/api/v1/tracker/projects", get(list_projects).post(create_project))
        .route(
            "/api/v1/tracker/projects/{project_id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        // Issues under project
        .route(
            "/api/v1/tracker/projects/{project_id}/issues",
            get(list_project_issues).post(create_project_issue),
        )
        // Sprints
        .route(
            "/api/v1/tracker/projects/{project_id}/sprints",
            get(list_sprints).post(create_sprint),
        )
        .route(
            "/api/v1/tracker/projects/{project_id}/sprints/{sprint_id}/start",
            post(start_sprint),
        )
        .route(
            "/api/v1/tracker/projects/{project_id}/sprints/{sprint_id}/complete",
            post(complete_sprint),
        )
        // Board / backlog / roadmap / metrics
        .route("/api/v1/tracker/projects/{project_id}/board", get(kanban_board))
        .route("/api/v1/tracker/projects/{project_id}/backlog", get(project_backlog))
        .route("/api/v1/tracker/projects/{project_id}/roadmap", get(project_roadmap))
        .route("/api/v1/tracker/projects/{project_id}/metrics", get(project_metrics))
        // Labels
        .route(
            "/api/v1/tracker/projects/{project_id}/labels",
            get(list_labels).post(create_label),
        )
        // Automations
        .route(
            "/api/v1/tracker/projects/{project_id}/automations",
            get(list_automations).post(create_automation),
        )
        // Individual issues
        .route(
            "/api/v1/tracker/issues/{issue_id}",
            get(get_issue).put(update_issue_handler).delete(delete_issue),
        )
        .route("/api/v1/tracker/issues/{issue_id}/activity", get(get_issue_activity))
        .route("/api/v1/tracker/issues/{issue_id}/transition", post(transition_issue_handler))
        .route(
            "/api/v1/tracker/issues/{issue_id}/comments",
            get(list_comments).post(add_comment),
        )
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-tracker",
        "status": "ok",
        "upstream": "Jira / Linear / Plane"
    }))
}

// ── Search ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn search_issues(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<SearchParams>,
) -> Json<Vec<Issue>> {
    let q = params.q.to_lowercase();
    let issues = state.issues.lock().await;
    let results: Vec<Issue> = issues.values()
        .filter(|i| {
            i.title.to_lowercase().contains(&q)
                || i.key.to_lowercase().contains(&q)
                || i.description.as_deref().unwrap_or("").to_lowercase().contains(&q)
        })
        .take(params.limit)
        .cloned()
        .collect();
    Json(results)
}

// ── Projects ──────────────────────────────────────────────────────────────────

async fn list_projects(State(state): State<Arc<TrackerState>>) -> Json<Vec<Project>> {
    let projects = state.projects.lock().await;
    Json(projects.values().cloned().collect())
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    name: String,
    /// Uppercase short key, e.g. "CAVE".
    key: String,
    description: Option<String>,
    lead: Option<String>,
}

async fn create_project(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Project> {
    let key = req.key.to_uppercase();
    {
        let projects = state.projects.lock().await;
        if projects.values().any(|p| p.key == key) {
            return Err(bad_request(&format!("Project key '{}' already exists", key)));
        }
    }
    let project = Project {
        id: Uuid::new_v4(),
        key,
        name: req.name,
        description: req.description,
        lead: req.lead,
        default_workflow: None,
        created_at: Utc::now(),
    };
    state.projects.lock().await.insert(project.id, project.clone());
    Ok(Json(project))
}

async fn get_project(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Project> {
    let projects = state.projects.lock().await;
    projects.get(&project_id).cloned().map(Json).ok_or_else(|| not_found("Project not found"))
}

#[derive(Deserialize)]
struct UpdateProjectRequest {
    name: Option<String>,
    description: Option<String>,
    lead: Option<String>,
}

async fn update_project(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> ApiResult<Project> {
    let mut projects = state.projects.lock().await;
    let project = projects.get_mut(&project_id).ok_or_else(|| not_found("Project not found"))?;
    if let Some(name) = req.name { project.name = name; }
    if let Some(desc) = req.description { project.description = Some(desc); }
    if let Some(lead) = req.lead { project.lead = Some(lead); }
    Ok(Json(project.clone()))
}

async fn delete_project(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let removed = state.projects.lock().await.remove(&project_id).is_some();
    if removed {
        Ok(Json(serde_json::json!({"deleted": project_id})))
    } else {
        Err(not_found("Project not found"))
    }
}

// ── Issues ────────────────────────────────────────────────────────────────────

async fn list_project_issues(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<Vec<Issue>> {
    let issues = state.issues.lock().await;
    Json(issues.values().filter(|i| i.project_id == project_id).cloned().collect())
}

async fn create_project_issue(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateIssueRequest>,
) -> ApiResult<Issue> {
    tracker::create_issue(&state, project_id, req)
        .await
        .map(Json)
        .map_err(|e| bad_request(&e))
}

async fn get_issue(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<Issue> {
    let issues = state.issues.lock().await;
    issues.get(&issue_id).cloned().map(Json).ok_or_else(|| not_found("Issue not found"))
}

#[derive(Deserialize)]
struct UpdateIssueBody {
    #[serde(flatten)]
    fields: UpdateIssueRequest,
    #[serde(default = "system_actor")]
    actor: String,
}

fn system_actor() -> String { "system".into() }

async fn update_issue_handler(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(body): Json<UpdateIssueBody>,
) -> ApiResult<Issue> {
    tracker::update_issue(&state, issue_id, body.fields, body.actor)
        .await
        .map(Json)
        .map_err(|e| bad_request(&e))
}

async fn delete_issue(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    let removed = state.issues.lock().await.remove(&issue_id).is_some();
    if removed {
        Ok(Json(serde_json::json!({"deleted": issue_id})))
    } else {
        Err(not_found("Issue not found"))
    }
}

// ── Activity feed ─────────────────────────────────────────────────────────────

async fn get_issue_activity(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> Json<Vec<Activity>> {
    let activities = state.activities.lock().await;
    Json(activities.get(&issue_id).cloned().unwrap_or_default())
}

// ── Transition ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TransitionRequest {
    to_status: String,
    #[serde(default = "system_actor")]
    actor: String,
}

async fn transition_issue_handler(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<TransitionRequest>,
) -> ApiResult<Issue> {
    let issue = tracker::transition_issue(&state, issue_id, req.to_status.clone(), req.actor.clone())
        .await
        .map_err(|e| bad_request(&e))?;

    // Fire automation rules for status change.
    let event = Event::StatusChanged {
        from: issue.status.clone(), // already updated, so we pass new as both for now
        to: req.to_status,
    };
    let _ = automation::evaluate_rules(&state, issue_id, &event).await;

    Ok(Json(issue))
}

// ── Comments ──────────────────────────────────────────────────────────────────

async fn list_comments(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
) -> Json<Vec<Comment>> {
    let comments = state.comments.lock().await;
    Json(comments.get(&issue_id).cloned().unwrap_or_default())
}

#[derive(Deserialize)]
struct AddCommentRequest {
    author: String,
    body: String,
}

async fn add_comment(
    State(state): State<Arc<TrackerState>>,
    Path(issue_id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> ApiResult<Comment> {
    // Verify issue exists.
    {
        let issues = state.issues.lock().await;
        if !issues.contains_key(&issue_id) {
            return Err(not_found("Issue not found"));
        }
    }
    let comment = Comment {
        id: Uuid::new_v4(),
        issue_id,
        author: req.author,
        body: req.body,
        created_at: Utc::now(),
    };
    state.comments.lock().await
        .entry(issue_id)
        .or_insert_with(Vec::new)
        .push(comment.clone());
    Ok(Json(comment))
}

// ── Sprints ───────────────────────────────────────────────────────────────────

async fn list_sprints(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<Vec<Sprint>> {
    let sprints = state.sprints.lock().await;
    Json(sprints.values().filter(|s| s.project_id == project_id).cloned().collect())
}

#[derive(Deserialize)]
struct CreateSprintRequest {
    name: String,
    goal: Option<String>,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
}

async fn create_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateSprintRequest>,
) -> ApiResult<Sprint> {
    // Verify project exists.
    {
        let projects = state.projects.lock().await;
        if !projects.contains_key(&project_id) {
            return Err(not_found("Project not found"));
        }
    }
    let sprint = Sprint {
        id: Uuid::new_v4(),
        project_id,
        name: req.name,
        goal: req.goal,
        start_date: req.start_date,
        end_date: req.end_date,
        status: SprintStatus::Planning,
    };
    state.sprints.lock().await.insert(sprint.id, sprint.clone());
    Ok(Json(sprint))
}

async fn start_sprint(
    State(state): State<Arc<TrackerState>>,
    Path((project_id, sprint_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Sprint> {
    let mut sprints = state.sprints.lock().await;
    let sprint = sprints.get_mut(&sprint_id).ok_or_else(|| not_found("Sprint not found"))?;
    if sprint.project_id != project_id {
        return Err(not_found("Sprint not found in project"));
    }
    if sprint.status != SprintStatus::Planning {
        return Err(bad_request("Sprint is not in Planning status"));
    }
    sprint.status = SprintStatus::Active;
    if sprint.start_date.is_none() {
        sprint.start_date = Some(Utc::now());
    }
    Ok(Json(sprint.clone()))
}

async fn complete_sprint(
    State(state): State<Arc<TrackerState>>,
    Path((project_id, sprint_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Sprint> {
    let mut sprints = state.sprints.lock().await;
    let sprint = sprints.get_mut(&sprint_id).ok_or_else(|| not_found("Sprint not found"))?;
    if sprint.project_id != project_id {
        return Err(not_found("Sprint not found in project"));
    }
    if sprint.status != SprintStatus::Active {
        return Err(bad_request("Sprint is not Active"));
    }
    sprint.status = SprintStatus::Completed;
    if sprint.end_date.is_none() {
        sprint.end_date = Some(Utc::now());
    }
    Ok(Json(sprint.clone()))
}

// ── Board ─────────────────────────────────────────────────────────────────────

async fn kanban_board(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<board::KanbanView> {
    Json(board::kanban_view(&state, project_id).await)
}

// ── Backlog ───────────────────────────────────────────────────────────────────

async fn project_backlog(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<board::BacklogView> {
    Json(board::backlog_view(&state, project_id).await)
}

// ── Roadmap ───────────────────────────────────────────────────────────────────

async fn project_roadmap(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<roadmap::TimelineView> {
    Json(roadmap::timeline_view(&state, project_id).await)
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ProjectMetrics {
    project_id: Uuid,
    avg_cycle_time_hours: f64,
    avg_lead_time_hours: f64,
    blockers: Vec<tracker::BlockerInfo>,
    sprint_suggestion: tracker::SprintSuggestion,
    capacity: roadmap::CapacityPlan,
    risks: roadmap::RiskReport,
    wip_violations: Vec<board::WipViolation>,
}

async fn project_metrics(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<ProjectMetrics> {
    let (cycle_time, lead_time, blockers, sprint_suggestion, capacity, risks, wip) = tokio::join!(
        tracker::calculate_cycle_time(&state, project_id),
        tracker::calculate_lead_time(&state, project_id),
        tracker::detect_blockers(&state, project_id),
        tracker::suggest_sprint_scope(&state, project_id),
        roadmap::capacity_planning(&state, project_id),
        roadmap::risk_detection(&state, project_id),
        board::wip_violations(&state, project_id),
    );

    Json(ProjectMetrics {
        project_id,
        avg_cycle_time_hours: cycle_time,
        avg_lead_time_hours: lead_time,
        blockers,
        sprint_suggestion,
        capacity,
        risks,
        wip_violations: wip,
    })
}

// ── Labels ────────────────────────────────────────────────────────────────────

async fn list_labels(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<Vec<Label>> {
    let labels = state.labels.lock().await;
    Json(labels.values().filter(|l| l.project_id == project_id).cloned().collect())
}

#[derive(Deserialize)]
struct CreateLabelRequest {
    name: String,
    #[serde(default = "default_color")]
    color: String,
}

fn default_color() -> String { "#6b7280".into() }

async fn create_label(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateLabelRequest>,
) -> ApiResult<Label> {
    let label = Label {
        id: Uuid::new_v4(),
        project_id,
        name: req.name,
        color: req.color,
    };
    state.labels.lock().await.insert(label.id, label.clone());
    Ok(Json(label))
}

// ── Automations ───────────────────────────────────────────────────────────────

async fn list_automations(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
) -> Json<Vec<Automation>> {
    let automations = state.automations.lock().await;
    Json(automations.iter().filter(|a| a.project_id == project_id).cloned().collect())
}

#[derive(Deserialize)]
struct CreateAutomationRequest {
    name: String,
    trigger: AutomationTrigger,
    condition: AutomationCondition,
    action: AutomationAction,
    #[serde(default = "bool_true")]
    enabled: bool,
}

fn bool_true() -> bool { true }

async fn create_automation(
    State(state): State<Arc<TrackerState>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateAutomationRequest>,
) -> ApiResult<Automation> {
    let rule = Automation {
        id: Uuid::new_v4(),
        project_id,
        name: req.name,
        trigger: req.trigger,
        condition: req.condition,
        action: req.action,
        enabled: req.enabled,
        created_at: Utc::now(),
    };
    state.automations.lock().await.push(rule.clone());
    Ok(Json(rule))
}
