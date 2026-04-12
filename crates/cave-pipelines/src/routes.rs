//! Admin API routes for cave-pipelines.
//!
//! Endpoints:
//!   GET/POST   /api/v1/pipelines
//!   GET/PUT/DELETE /api/v1/pipelines/:id
//!   POST       /api/v1/pipelines/:id/run
//!   GET        /api/v1/pipelineruns
//!   GET        /api/v1/pipelineruns/:id
//!   POST       /api/v1/pipelineruns/:id/cancel
//!   GET/POST   /api/v1/tasks
//!   GET/PUT/DELETE /api/v1/tasks/:id
//!   GET        /api/v1/taskruns
//!   GET        /api/v1/taskruns/:id
//!   GET/POST   /api/v1/triggers
//!   GET/DELETE /api/v1/triggers/:id
//!   GET        /api/v1/catalog
//!   POST       /api/v1/approvals/:id/approve
//!   POST       /api/v1/approvals/:id/reject

use crate::models::{ApprovalStatus, Pipeline, PipelineRun, RunStatus, Task, TaskRun};
use crate::triggers::Trigger;
use crate::State;
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // Health
        .route("/api/pipelines/health", get(health))
        // Pipelines
        .route("/api/v1/pipelines", get(list_pipelines).post(create_pipeline))
        .route(
            "/api/v1/pipelines/:id",
            get(get_pipeline).put(update_pipeline).delete(delete_pipeline),
        )
        .route("/api/v1/pipelines/:id/run", post(run_pipeline))
        // Pipeline runs
        .route("/api/v1/pipelineruns", get(list_pipeline_runs))
        .route("/api/v1/pipelineruns/:id", get(get_pipeline_run))
        .route("/api/v1/pipelineruns/:id/cancel", post(cancel_pipeline_run))
        // Tasks
        .route("/api/v1/tasks", get(list_tasks).post(create_task))
        .route(
            "/api/v1/tasks/:id",
            get(get_task).put(update_task).delete(delete_task),
        )
        // Task runs
        .route("/api/v1/taskruns", get(list_task_runs))
        .route("/api/v1/taskruns/:id", get(get_task_run))
        // Triggers
        .route("/api/v1/triggers", get(list_triggers).post(create_trigger))
        .route("/api/v1/triggers/:id", get(get_trigger).delete(delete_trigger))
        // Catalog
        .route("/api/v1/catalog", get(list_catalog))
        // Approvals
        .route("/api/v1/approvals/:id/approve", post(approve_gate))
        .route("/api/v1/approvals/:id/reject", post(reject_gate))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-pipelines",
        "status": "ok",
        "upstream": "Tekton / Jenkins",
    }))
}

// ---------------------------------------------------------------------------
// Pipelines
// ---------------------------------------------------------------------------

async fn list_pipelines(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let items: Vec<&Pipeline> = store.pipelines.values().collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

async fn create_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Json(pipeline): Json<Pipeline>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    store.pipelines.insert(pipeline.id, pipeline.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&pipeline).unwrap_or_default()))
}

async fn get_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = state.store.lock().await;
    match store.pipelines.get(&id) {
        Some(p) => (StatusCode::OK, Json(serde_json::to_value(p).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "pipeline not found" }))),
    }
}

async fn update_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(mut pipeline): Json<Pipeline>,
) -> (StatusCode, Json<serde_json::Value>) {
    pipeline.id = id;
    let mut store = state.store.lock().await;
    if store.pipelines.contains_key(&id) {
        store.pipelines.insert(id, pipeline.clone());
        (StatusCode::OK, Json(serde_json::to_value(&pipeline).unwrap_or_default()))
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "pipeline not found" })))
    }
}

async fn delete_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().await;
    if store.pipelines.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn run_pipeline(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Read pipeline details, release lock, then write the new run.
    let run = {
        let store = state.store.lock().await;
        match store.pipelines.get(&id) {
            Some(p) => PipelineRun::new(p.id, &p.name),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "pipeline not found" })),
                )
            }
        }
    };
    let mut store = state.store.lock().await;
    store.pipeline_runs.insert(run.id, run.clone());
    (StatusCode::ACCEPTED, Json(serde_json::to_value(&run).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Pipeline runs
// ---------------------------------------------------------------------------

async fn list_pipeline_runs(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let items: Vec<&PipelineRun> = store.pipeline_runs.values().collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

async fn get_pipeline_run(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = state.store.lock().await;
    match store.pipeline_runs.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(r).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "run not found" }))),
    }
}

async fn cancel_pipeline_run(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    match store.pipeline_runs.get_mut(&id) {
        Some(run) => {
            run.status = RunStatus::Cancelled;
            run.completed_at = Some(Utc::now());
            (StatusCode::OK, Json(serde_json::json!({ "status": "cancelled" })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "run not found" }))),
    }
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

async fn list_tasks(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let items: Vec<&Task> = store.tasks.values().collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

async fn create_task(
    AxumState(state): AxumState<Arc<State>>,
    Json(task): Json<Task>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    store.tasks.insert(task.id, task.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&task).unwrap_or_default()))
}

async fn get_task(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = state.store.lock().await;
    match store.tasks.get(&id) {
        Some(t) => (StatusCode::OK, Json(serde_json::to_value(t).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "task not found" }))),
    }
}

async fn update_task(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
    Json(mut task): Json<Task>,
) -> (StatusCode, Json<serde_json::Value>) {
    task.id = id;
    let mut store = state.store.lock().await;
    if store.tasks.contains_key(&id) {
        store.tasks.insert(id, task.clone());
        (StatusCode::OK, Json(serde_json::to_value(&task).unwrap_or_default()))
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "task not found" })))
    }
}

async fn delete_task(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().await;
    if store.tasks.remove(&id).is_some() { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

// ---------------------------------------------------------------------------
// Task runs
// ---------------------------------------------------------------------------

async fn list_task_runs(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let items: Vec<&TaskRun> = store.task_runs.values().collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

async fn get_task_run(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = state.store.lock().await;
    match store.task_runs.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(r).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "task run not found" }))),
    }
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

async fn list_triggers(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let items: Vec<&Trigger> = store.triggers.values().collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

async fn create_trigger(
    AxumState(state): AxumState<Arc<State>>,
    Json(trigger): Json<Trigger>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    store.triggers.insert(trigger.id, trigger.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&trigger).unwrap_or_default()))
}

async fn get_trigger(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = state.store.lock().await;
    match store.triggers.get(&id) {
        Some(t) => (StatusCode::OK, Json(serde_json::to_value(t).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "trigger not found" }))),
    }
}

async fn delete_trigger(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().await;
    if store.triggers.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

async fn list_catalog(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let entries = state.catalog.list();
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "version": e.version,
                "description": e.description,
            })
        })
        .collect();
    Json(serde_json::json!({ "items": items, "total": items.len() }))
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

async fn approve_gate(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    match store.approvals.get_mut(&id) {
        Some(gate) => {
            gate.status = ApprovalStatus::Approved;
            gate.decided_at = Some(Utc::now());
            (StatusCode::OK, Json(serde_json::json!({ "status": "approved" })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval gate not found" }))),
    }
}

async fn reject_gate(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    match store.approvals.get_mut(&id) {
        Some(gate) => {
            gate.status = ApprovalStatus::Rejected;
            gate.decided_at = Some(Utc::now());
            (StatusCode::OK, Json(serde_json::json!({ "status": "rejected" })))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval gate not found" }))),
    }
}
