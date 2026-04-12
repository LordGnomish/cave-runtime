//! HTTP routes for cave-tracker.

use crate::models::{
    BulkOperationRequest, Comment, CreateIssueRequest, CreateSprintRequest, Issue, JqlResult,
    Sprint, TransitionRequest, UpdateIssueRequest,
};
use crate::store::SprintVelocity;
use crate::TrackerState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<TrackerState>) -> Router {
    Router::new()
        .route("/api/tracker/health", get(health))
        // Issues
        .route("/api/tracker/issues", get(list_issues).post(create_issue))
        .route(
            "/api/tracker/issues/:id",
            get(get_issue).put(update_issue),
        )
        .route("/api/tracker/issues/:id/transition", post(transition_issue))
        .route("/api/tracker/issues/bulk", post(bulk_operate))
        .route("/api/tracker/issues/query", post(query_jql))
        .route(
            "/api/tracker/issues/:id/comments",
            get(list_comments).post(add_comment),
        )
        // Sprints
        .route(
            "/api/tracker/sprints",
            get(list_sprints).post(create_sprint),
        )
        .route("/api/tracker/sprints/:id/start", post(start_sprint))
        .route("/api/tracker/sprints/:id/complete", post(complete_sprint))
        .route("/api/tracker/sprints/:id/backlog", get(get_sprint_backlog))
        // Velocity
        .route("/api/tracker/velocity", get(get_velocity))
        .with_state(state)
}

// ─── Health ─────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-tracker",
        "status": "ok",
        "upstream": "jira"
    }))
}

// ─── Issues ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProjectQuery {
    project: Option<String>,
}

async fn list_issues(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<ProjectQuery>,
) -> Json<Vec<Issue>> {
    let project = params.project.unwrap_or_default();
    Json(state.store.list_issues(&project))
}

async fn create_issue(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<ProjectQuery>,
    Json(req): Json<CreateIssueRequest>,
) -> (StatusCode, Json<Issue>) {
    // In production, reporter would come from auth middleware.
    let reporter = Uuid::nil();
    let project = params.project.unwrap_or_else(|| "DEFAULT".to_string());
    let issue = state.store.create_issue(req, reporter, &project);
    (StatusCode::CREATED, Json(issue))
}

async fn get_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Issue>, StatusCode> {
    state
        .store
        .get_issue(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateIssueRequest>,
) -> Result<Json<Issue>, StatusCode> {
    state
        .store
        .update_issue(id, req)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn transition_issue(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionRequest>,
) -> Result<Json<Issue>, (StatusCode, Json<serde_json::Value>)> {
    let actor = Uuid::nil(); // would come from auth
    let fields = req.fields.unwrap_or(serde_json::Value::Null);

    // Validate via workflow engine before persisting
    if let Some(issue) = state.store.get_issue(id) {
        if let Err(e) = state
            .workflow
            .validate_transition(&issue, &req.to_status, &fields)
        {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": e })),
            ));
        }
    }

    state
        .store
        .transition_issue(id, req.to_status, actor)
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": e })),
            )
        })
}

async fn bulk_operate(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<BulkOperationRequest>,
) -> Json<Vec<Issue>> {
    Json(state.store.bulk_operate(req))
}

#[derive(Deserialize)]
struct JqlRequest {
    jql: String,
}

async fn query_jql(
    State(state): State<Arc<TrackerState>>,
    Json(req): Json<JqlRequest>,
) -> Json<JqlResult> {
    // current_user would come from auth middleware in production
    Json(state.store.query_jql(&req.jql, None))
}

async fn list_comments(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<Comment>> {
    Json(state.store.list_comments(id))
}

#[derive(Deserialize)]
struct AddCommentRequest {
    body: String,
}

async fn add_comment(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> (StatusCode, Json<Comment>) {
    let author = Uuid::nil(); // would come from auth
    let comment = state.store.add_comment(id, author, req.body);
    (StatusCode::CREATED, Json(comment))
}

// ─── Sprints ─────────────────────────────────────────────────────────────────

async fn list_sprints(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<ProjectQuery>,
) -> Json<Vec<Sprint>> {
    let project = params.project.unwrap_or_default();
    Json(state.store.list_sprints(&project))
}

async fn create_sprint(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<ProjectQuery>,
    Json(req): Json<CreateSprintRequest>,
) -> (StatusCode, Json<Sprint>) {
    let project = params.project.unwrap_or_else(|| "DEFAULT".to_string());
    let sprint = state.store.create_sprint(req, &project);
    (StatusCode::CREATED, Json(sprint))
}

async fn start_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Sprint>, StatusCode> {
    state
        .store
        .start_sprint(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn complete_sprint(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Sprint>, StatusCode> {
    state
        .store
        .complete_sprint(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_sprint_backlog(
    State(state): State<Arc<TrackerState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<Issue>> {
    Json(state.store.get_sprint_backlog(id))
}

// ─── Velocity ────────────────────────────────────────────────────────────────

async fn get_velocity(
    State(state): State<Arc<TrackerState>>,
    Query(params): Query<ProjectQuery>,
) -> Json<Vec<SprintVelocity>> {
    let project = params.project.unwrap_or_default();
    Json(state.store.get_sprint_velocity(&project))
}
