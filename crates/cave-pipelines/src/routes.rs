//! HTTP API routes for cave-pipelines.

use crate::{
    engine::{validate_params, Dag},
    models::*,
    triggers::{CronTrigger, EventListener, WebhookEvent, passes_interceptors},
    PipelinesState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<PipelinesState>) -> Router {
    Router::new()
        // Pipeline CRUD
        .route("/api/pipelines", get(list_pipelines).post(create_pipeline))
        .route("/api/pipelines/{id}", get(get_pipeline).put(update_pipeline).delete(delete_pipeline))
        // PipelineRun
        .route("/api/pipelines/{id}/runs", post(start_pipeline_run))
        .route("/api/pipeline-runs", get(list_pipeline_runs))
        .route("/api/pipeline-runs/{run_id}", get(get_pipeline_run))
        .route("/api/pipeline-runs/{run_id}/cancel", post(cancel_pipeline_run))
        // TaskRun
        .route("/api/task-runs/{run_id}", get(get_task_run))
        .route("/api/task-runs/{run_id}/logs", get(get_task_run_logs))
        // Task CRUD
        .route("/api/tasks", get(list_tasks).post(create_task))
        .route("/api/tasks/{id}", get(get_task))
        // Catalog
        .route("/api/catalog/tasks", get(list_catalog))
        .route("/api/catalog/tasks/{name}", get(get_catalog_task))
        // Triggers
        .route("/api/triggers/webhook", post(handle_webhook))
        .route("/api/triggers/cron", get(list_cron_triggers).post(create_cron_trigger))
        .route("/api/event-listeners", get(list_event_listeners).post(create_event_listener))
        // Jenkins compat
        .route("/api/jenkins/import", post(import_jenkinsfile))
        // Build status
        .route("/api/build-status", post(post_build_status))
        // Health
        .route("/api/pipelines/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-pipelines",
        "status": "ok",
        "upstream": ["Tekton Pipelines", "Jenkins"]
    }))
}

// ─── Pipeline CRUD ────────────────────────────────────────────────────────────

async fn list_pipelines(
    State(_state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "pipelines": [], "total": 0 }))
}

async fn create_pipeline(
    State(_state): State<Arc<PipelinesState>>,
    Json(spec): Json<PipelineSpec>,
) -> Result<(StatusCode, Json<Pipeline>), (StatusCode, Json<serde_json::Value>)> {
    // Validate DAG
    let dag = Dag::from_spec(&spec.tasks);
    if let Err(e) = dag.execution_waves() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        ));
    }

    let pipeline = Pipeline {
        id: Uuid::new_v4(),
        name: format!("pipeline-{}", Uuid::new_v4()),
        namespace: None,
        spec,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: Default::default(),
    };

    Ok((StatusCode::CREATED, Json(pipeline)))
}

async fn get_pipeline(
    State(_state): State<Arc<PipelinesState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Pipeline>, (StatusCode, Json<serde_json::Value>)> {
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("Pipeline {} not found", id) })),
    ))
}

async fn update_pipeline(
    State(_state): State<Arc<PipelinesState>>,
    Path(_id): Path<Uuid>,
    Json(_spec): Json<PipelineSpec>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "updated" }))
}

async fn delete_pipeline(
    State(_state): State<Arc<PipelinesState>>,
    Path(_id): Path<Uuid>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ─── PipelineRun ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct StartRunRequest {
    params: Option<Vec<Param>>,
    workspaces: Option<Vec<WorkspaceAssignment>>,
}

async fn start_pipeline_run(
    State(state): State<Arc<PipelinesState>>,
    Path(pipeline_id): Path<Uuid>,
    Json(req): Json<StartRunRequest>,
) -> (StatusCode, Json<PipelineRun>) {
    let run = PipelineRun {
        id: Uuid::new_v4(),
        name: format!("run-{}", Uuid::new_v4()),
        namespace: None,
        spec: PipelineRunSpec {
            pipeline_ref: Some(PipelineRef { name: pipeline_id.to_string() }),
            pipeline_spec: None,
            params: req.params.unwrap_or_default(),
            workspaces: req.workspaces.unwrap_or_default(),
            service_account: None,
            timeout: None,
        },
        phase: RunPhase::Pending,
        task_runs: vec![],
        results: vec![],
        start_time: Some(Utc::now()),
        completion_time: None,
        created_at: Utc::now(),
        trigger_source: Some(TriggerSource::Manual { user: "api".to_string() }),
        labels: Default::default(),
    };
    (StatusCode::CREATED, Json(run))
}

async fn list_pipeline_runs(
    State(_state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "runs": [], "total": 0 }))
}

async fn get_pipeline_run(
    State(_state): State<Arc<PipelinesState>>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<PipelineRun>, (StatusCode, Json<serde_json::Value>)> {
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("PipelineRun {} not found", run_id) })),
    ))
}

async fn cancel_pipeline_run(
    State(state): State<Arc<PipelinesState>>,
    Path(run_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let runs = state.active_runs.read().await;
    if let Some(handle) = runs.get(&run_id) {
        let _ = handle.cancel_tx.try_send(());
        Json(serde_json::json!({ "status": "cancellation requested" }))
    } else {
        Json(serde_json::json!({ "status": "run not found or already complete" }))
    }
}

// ─── TaskRun ─────────────────────────────────────────────────────────────────

async fn get_task_run(
    State(_state): State<Arc<PipelinesState>>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<TaskRun>, (StatusCode, Json<serde_json::Value>)> {
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("TaskRun {} not found", run_id) })),
    ))
}

async fn get_task_run_logs(
    State(_state): State<Arc<PipelinesState>>,
    Path(run_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "run_id": run_id,
        "logs": [],
        "streaming": false
    }))
}

// ─── Task CRUD ────────────────────────────────────────────────────────────────

async fn list_tasks(
    State(_state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "tasks": [], "total": 0 }))
}

async fn create_task(
    State(_state): State<Arc<PipelinesState>>,
    Json(spec): Json<TaskSpec>,
) -> (StatusCode, Json<Task>) {
    let task = Task {
        id: Uuid::new_v4(),
        name: format!("task-{}", Uuid::new_v4()),
        namespace: None,
        spec,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: Default::default(),
        annotations: Default::default(),
    };
    (StatusCode::CREATED, Json(task))
}

async fn get_task(
    State(_state): State<Arc<PipelinesState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, (StatusCode, Json<serde_json::Value>)> {
    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("Task {} not found", id) })),
    ))
}

// ─── Catalog ─────────────────────────────────────────────────────────────────

async fn list_catalog(
    State(state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    let tasks: Vec<serde_json::Value> = state.catalog.list().iter().map(|e| {
        serde_json::json!({
            "name": e.name,
            "version": e.version,
            "description": e.description,
            "tags": e.tags,
        })
    }).collect();
    Json(serde_json::json!({ "tasks": tasks, "total": tasks.len() }))
}

async fn get_catalog_task(
    State(state): State<Arc<PipelinesState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match state.catalog.get(&name) {
        Some(entry) => Ok(Json(serde_json::json!({
            "name": entry.name,
            "version": entry.version,
            "description": entry.description,
            "tags": entry.tags,
            "spec": entry.spec,
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Catalog task '{}' not found", name) })),
        )),
    }
}

// ─── Triggers ────────────────────────────────────────────────────────────────

async fn handle_webhook(
    State(_state): State<Arc<PipelinesState>>,
    Json(event): Json<WebhookEvent>,
) -> Json<serde_json::Value> {
    tracing::info!(event_type = %event.event_type, "Received webhook event");
    Json(serde_json::json!({
        "status": "received",
        "event_type": event.event_type,
        "runs_triggered": 0
    }))
}

async fn list_cron_triggers(
    State(_state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "triggers": [] }))
}

async fn create_cron_trigger(
    State(_state): State<Arc<PipelinesState>>,
    Json(trigger): Json<CronTrigger>,
) -> (StatusCode, Json<CronTrigger>) {
    (StatusCode::CREATED, Json(trigger))
}

async fn list_event_listeners(
    State(_state): State<Arc<PipelinesState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "listeners": [] }))
}

async fn create_event_listener(
    State(_state): State<Arc<PipelinesState>>,
    Json(listener): Json<EventListener>,
) -> (StatusCode, Json<EventListener>) {
    (StatusCode::CREATED, Json(listener))
}

// ─── Jenkins import ──────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct JenkinsfileImport {
    content: String,
    pipeline_name: Option<String>,
}

async fn import_jenkinsfile(
    State(_state): State<Arc<PipelinesState>>,
    Json(req): Json<JenkinsfileImport>,
) -> Result<(StatusCode, Json<Pipeline>), (StatusCode, Json<serde_json::Value>)> {
    let jf = crate::jenkins::parse_jenkinsfile(&req.content)
        .map_err(|e| (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        ))?;

    let spec = crate::jenkins::to_pipeline_spec(&jf);
    let pipeline = Pipeline {
        id: Uuid::new_v4(),
        name: req.pipeline_name.unwrap_or_else(|| format!("jenkins-import-{}", Uuid::new_v4())),
        namespace: None,
        spec,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: Default::default(),
    };

    Ok((StatusCode::CREATED, Json(pipeline)))
}

// ─── Build status ────────────────────────────────────────────────────────────

async fn post_build_status(
    State(_state): State<Arc<PipelinesState>>,
    Json(status): Json<BuildStatus>,
) -> Json<serde_json::Value> {
    tracing::info!(
        provider = ?status.provider,
        repo = %status.repo,
        sha = %status.commit_sha,
        state = ?status.state,
        "Build status update"
    );
    Json(serde_json::json!({ "status": "accepted" }))
}
