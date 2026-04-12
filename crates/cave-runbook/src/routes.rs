//! HTTP routes for cave-runbook.

use crate::engine::RunbookEngine;
use crate::models::{
    ApproveStepRequest, CreateRunbookRequest, ExecuteRunbookRequest, ExecutionStatus, Runbook,
    StepExecution,
};
use crate::store::RunbookStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub struct RunbookAppState {
    pub store: RunbookStore,
}

impl Default for RunbookAppState {
    fn default() -> Self {
        Self {
            store: RunbookStore::new(),
        }
    }
}

pub fn create_router(state: Arc<RunbookAppState>) -> Router {
    Router::new()
        .route("/api/runbooks/health", get(health))
        .route("/api/runbooks/executions", get(list_executions))
        .route("/api/runbooks/executions/:id", get(get_execution))
        .route("/api/runbooks/executions/:id/output", get(get_execution_output))
        .route("/api/runbooks/executions/:id/cancel", post(cancel_execution))
        .route("/api/runbooks/executions/:id/approve", post(approve_step))
        .route("/api/runbooks", get(list_runbooks).post(create_runbook))
        .route(
            "/api/runbooks/:id",
            get(get_runbook).put(update_runbook).delete(delete_runbook),
        )
        .route("/api/runbooks/:id/execute", post(execute_runbook))
        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-runbook",
        "status": "ok",
        "upstream": "rundeck, stackstorm"
    }))
}

// ─── Runbooks ─────────────────────────────────────────────────────────────────

async fn list_runbooks(State(state): State<Arc<RunbookAppState>>) -> Json<Vec<Runbook>> {
    Json(state.store.list_runbooks())
}

async fn get_runbook(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Runbook>, StatusCode> {
    state
        .store
        .get_runbook(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_runbook(
    State(state): State<Arc<RunbookAppState>>,
    Json(req): Json<CreateRunbookRequest>,
) -> (StatusCode, Json<Runbook>) {
    let now = Utc::now();
    let runbook = Runbook {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        steps: req.steps,
        parameters: req.parameters.unwrap_or_default(),
        schedule: req.schedule,
        access_control: req.access_control.unwrap_or_default(),
        notifications: req.notifications.unwrap_or_default(),
        timeout_seconds: req.timeout_seconds,
        created_at: now,
        updated_at: now,
        created_by: Uuid::new_v4(), // TODO: extract from auth context
        enabled: true,
    };
    let created = state.store.create_runbook(runbook);
    (StatusCode::CREATED, Json(created))
}

async fn update_runbook(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRunbookRequest>,
) -> Result<Json<Runbook>, StatusCode> {
    let existing = state.store.get_runbook(id).ok_or(StatusCode::NOT_FOUND)?;
    let updated = Runbook {
        id,
        name: req.name,
        description: req.description,
        steps: req.steps,
        parameters: req.parameters.unwrap_or_default(),
        schedule: req.schedule,
        access_control: req.access_control.unwrap_or_default(),
        notifications: req.notifications.unwrap_or(existing.notifications),
        timeout_seconds: req.timeout_seconds,
        updated_at: Utc::now(),
        ..existing
    };
    state
        .store
        .update_runbook(id, updated)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_runbook(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.delete_runbook(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─── Execute ──────────────────────────────────────────────────────────────────

async fn execute_runbook(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ExecuteRunbookRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let runbook = state.store.get_runbook(id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "runbook not found"})),
        )
    })?;

    let params = req.parameters.unwrap_or_default();

    // Validate parameters before execution
    if let Err(errors) = RunbookEngine::validate_parameters(&runbook, &params) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"errors": errors})),
        ));
    }

    let mut execution = RunbookEngine::create_execution(&runbook, req.triggered_by, params);
    RunbookEngine::execute_sequential(&mut execution, &runbook);
    let stored = state.store.add_execution(execution);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"execution_id": stored.id, "status": stored.status})),
    ))
}

// ─── Executions ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ExecutionQuery {
    runbook_id: Option<Uuid>,
}

async fn list_executions(
    State(state): State<Arc<RunbookAppState>>,
    Query(query): Query<ExecutionQuery>,
) -> Json<Vec<crate::models::Execution>> {
    Json(state.store.list_executions(query.runbook_id))
}

async fn get_execution(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::models::Execution>, StatusCode> {
    state
        .store
        .get_execution(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_execution_output(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    state
        .store
        .get_execution_output(id)
        .map(|output| Json(serde_json::json!({"output": output})))
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_execution(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.store.cancel_execution(id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn approve_step(
    State(state): State<Arc<RunbookAppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ApproveStepRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut exec = state.store.get_execution(id).ok_or(StatusCode::NOT_FOUND)?;

    let runbook = state
        .store
        .get_runbook(exec.runbook_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let step_exec = exec
        .step_executions
        .iter_mut()
        .find(|se| se.step_id == req.step_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let new_status = if req.approved {
        ExecutionStatus::Completed
    } else {
        ExecutionStatus::Failed
    };

    let updated_step = StepExecution {
        status: new_status.clone(),
        completed_at: Some(Utc::now()),
        exit_code: if req.approved { Some(0) } else { Some(1) },
        stdout: req
            .comment
            .clone()
            .unwrap_or_else(|| "Approved".to_string()),
        ..step_exec.clone()
    };

    state
        .store
        .update_execution_step(id, req.step_id, updated_step);

    // Re-run remaining steps if approved
    if req.approved {
        let mut refreshed = state.store.get_execution(id).ok_or(StatusCode::NOT_FOUND)?;
        refreshed.status = ExecutionStatus::Running;
        // Execute remaining steps
        let completed_step_ids: Vec<Uuid> = refreshed
            .step_executions
            .iter()
            .filter(|se| se.status == ExecutionStatus::Completed)
            .map(|se| se.step_id)
            .collect();

        for step in &runbook.steps {
            if !completed_step_ids.contains(&step.id) {
                let step_result = RunbookEngine::simulate_step(step);
                state
                    .store
                    .update_execution_step(id, step.id, step_result);
            }
        }
    }

    Ok(Json(serde_json::json!({
        "execution_id": id,
        "step_id": req.step_id,
        "approved": req.approved
    })))
}
