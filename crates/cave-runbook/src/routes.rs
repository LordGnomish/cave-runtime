//! HTTP routes for cave-runbook.

use crate::{
    executor,
    models::{
        ApprovalStatus, ExecutionStatus, IncidentBinding, Runbook, RunbookExecution, StepStatus,
        Trigger,
    },
    templates,
    RunbookState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

// ── Request / response DTOs ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateRunbookRequest {
    name: String,
    description: String,
    trigger: Trigger,
    steps: Vec<crate::models::RunbookStep>,
    owner: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct UpdateRunbookRequest {
    name: Option<String>,
    description: Option<String>,
    trigger: Option<Trigger>,
    steps: Option<Vec<crate::models::RunbookStep>>,
    owner: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ExecuteRequest {
    triggered_by: Option<String>,
    incident_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct CreateBindingRequest {
    name: String,
    incident_pattern: String,
    incident_severity: Option<String>,
    runbook_id: Uuid,
    #[serde(default)]
    auto_execute: bool,
}

#[derive(Deserialize)]
struct ApproveRequest {
    responder: String,
    /// "approve" or "reject"
    decision: String,
}

// ── Router factory ────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<RunbookState>) -> Router {
    Router::new()
        // Health
        .route("/api/v1/runbooks/health", get(health))
        // Templates — static segment, must come before /{id}
        .route("/api/v1/runbooks/templates", get(list_templates))
        // Bindings — static segment, must come before /{id}
        .route(
            "/api/v1/runbooks/bindings",
            get(list_bindings).post(create_binding),
        )
        .route("/api/v1/runbooks/bindings/{id}", delete(delete_binding))
        // Execution detail — "executions" static, must come before /{id}
        .route(
            "/api/v1/runbooks/executions/{id}",
            get(get_execution),
        )
        .route(
            "/api/v1/runbooks/executions/{id}/approve/{step_id}",
            post(approve_step),
        )
        // Runbooks CRUD
        .route(
            "/api/v1/runbooks",
            get(list_runbooks).post(create_runbook),
        )
        .route(
            "/api/v1/runbooks/{id}",
            get(get_runbook).put(update_runbook).delete(delete_runbook),
        )
        // Runbook actions
        .route("/api/v1/runbooks/{id}/execute", post(execute_runbook_handler))
        .route("/api/v1/runbooks/{id}/executions", get(list_executions))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<Value> {
    Json(json!({
        "module": "cave-runbook",
        "status": "ok",
        "upstream": "PagerDuty Runbooks / Rundeck / StackStorm"
    }))
}

// ── Templates ─────────────────────────────────────────────────────────────────

async fn list_templates() -> (StatusCode, Json<Value>) {
    let t = templates::predefined_templates();
    (StatusCode::OK, Json(serde_json::to_value(t).unwrap_or_default()))
}

// ── Runbooks CRUD ─────────────────────────────────────────────────────────────

async fn list_runbooks(State(state): State<Arc<RunbookState>>) -> (StatusCode, Json<Value>) {
    let runbooks = state.runbooks.lock().await;
    let list: Vec<&Runbook> = runbooks.values().collect();
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap_or_default()))
}

async fn create_runbook(
    State(state): State<Arc<RunbookState>>,
    Json(req): Json<CreateRunbookRequest>,
) -> (StatusCode, Json<Value>) {
    let rb = Runbook {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        trigger: req.trigger,
        steps: req.steps,
        owner: req.owner,
        tags: req.tags,
        last_run: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let id = rb.id;
    {
        let mut runbooks = state.runbooks.lock().await;
        runbooks.insert(id, rb.clone());
    }
    (StatusCode::CREATED, Json(serde_json::to_value(rb).unwrap_or_default()))
}

async fn get_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let runbooks = state.runbooks.lock().await;
    match runbooks.get(&id) {
        Some(rb) => (StatusCode::OK, Json(serde_json::to_value(rb).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Runbook not found"}))),
    }
}

async fn update_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRunbookRequest>,
) -> (StatusCode, Json<Value>) {
    let mut runbooks = state.runbooks.lock().await;
    match runbooks.get_mut(&id) {
        Some(rb) => {
            if let Some(v) = req.name {
                rb.name = v;
            }
            if let Some(v) = req.description {
                rb.description = v;
            }
            if let Some(v) = req.trigger {
                rb.trigger = v;
            }
            if let Some(v) = req.steps {
                rb.steps = v;
            }
            if let Some(v) = req.owner {
                rb.owner = v;
            }
            if let Some(v) = req.tags {
                rb.tags = v;
            }
            rb.updated_at = Utc::now();
            (StatusCode::OK, Json(serde_json::to_value(&*rb).unwrap_or_default()))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Runbook not found"}))),
    }
}

async fn delete_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let mut runbooks = state.runbooks.lock().await;
    if runbooks.remove(&id).is_some() {
        (StatusCode::OK, Json(json!({"deleted": id})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Runbook not found"})))
    }
}

// ── Execution ─────────────────────────────────────────────────────────────────

async fn execute_runbook_handler(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ExecuteRequest>,
) -> (StatusCode, Json<Value>) {
    let runbook = {
        let runbooks = state.runbooks.lock().await;
        runbooks.get(&id).cloned()
    };

    let Some(runbook) = runbook else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Runbook not found"})),
        );
    };

    let exec_id = Uuid::new_v4();
    let triggered_by = req
        .triggered_by
        .unwrap_or_else(|| "manual".to_string());

    // Store a stub so callers can poll immediately.
    let stub = RunbookExecution {
        id: exec_id,
        runbook_id: runbook.id,
        runbook_name: runbook.name.clone(),
        status: ExecutionStatus::Running,
        started_at: Utc::now(),
        completed_at: None,
        triggered_by: triggered_by.clone(),
        step_results: Vec::new(),
        incident_id: req.incident_id,
    };
    {
        let mut execs = state.executions.lock().await;
        execs.insert(exec_id, stub);
    }

    // Run in background — caller polls GET /executions/{id} for status.
    let state_bg = Arc::clone(&state);
    let incident_id = req.incident_id;
    tokio::spawn(async move {
        executor::execute_runbook(state_bg, exec_id, runbook, triggered_by, incident_id).await;
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "execution_id": exec_id,
            "runbook_id": id,
            "status": "running",
            "message": "Runbook execution started — poll GET /api/v1/runbooks/executions/{id} for status"
        })),
    )
}

async fn list_executions(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let execs = state.executions.lock().await;
    let mut history: Vec<&RunbookExecution> =
        execs.values().filter(|e| e.runbook_id == id).collect();
    // Most recent first.
    history.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    (StatusCode::OK, Json(serde_json::to_value(history).unwrap_or_default()))
}

async fn get_execution(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let execs = state.executions.lock().await;
    match execs.get(&id) {
        Some(exec) => (StatusCode::OK, Json(serde_json::to_value(exec).unwrap_or_default())),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Execution not found"}))),
    }
}

async fn approve_step(
    State(state): State<Arc<RunbookState>>,
    Path((exec_id, step_id)): Path<(Uuid, String)>,
    Json(req): Json<ApproveRequest>,
) -> (StatusCode, Json<Value>) {
    let approved = req.decision.eq_ignore_ascii_case("approve");

    // Resolve the matching approval request.
    let approval_id = {
        let approvals = state.approvals.lock().await;
        approvals
            .values()
            .find(|a| a.step_id == step_id && a.status == ApprovalStatus::Pending)
            .map(|a| a.id)
    };

    let Some(approval_id) = approval_id else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Pending approval not found for this step"})),
        );
    };

    // Update approval record.
    {
        let mut approvals = state.approvals.lock().await;
        if let Some(a) = approvals.get_mut(&approval_id) {
            a.status = if approved {
                ApprovalStatus::Approved
            } else {
                ApprovalStatus::Rejected
            };
            a.responded_at = Some(Utc::now());
            a.responder = Some(req.responder.clone());
            a.execution_id = exec_id; // patch placeholder set by executor
        }
    }

    // Update step result and execution status.
    {
        let mut execs = state.executions.lock().await;
        if let Some(exec) = execs.get_mut(&exec_id) {
            if let Some(sr) = exec.step_results.iter_mut().find(|r| r.step_id == step_id) {
                sr.status = if approved {
                    StepStatus::Success
                } else {
                    StepStatus::Failed
                };
                sr.completed_at = Some(Utc::now());
            }
            let all_resolved = exec.step_results.iter().all(|r| {
                !matches!(
                    r.status,
                    StepStatus::Pending | StepStatus::Running | StepStatus::PendingApproval
                )
            });
            if all_resolved {
                exec.status = if approved {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Aborted
                };
                exec.completed_at = Some(Utc::now());
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "approval_id": approval_id,
            "execution_id": exec_id,
            "step_id": step_id,
            "decision": req.decision,
            "responder": req.responder
        })),
    )
}

// ── Bindings CRUD ─────────────────────────────────────────────────────────────

async fn list_bindings(State(state): State<Arc<RunbookState>>) -> (StatusCode, Json<Value>) {
    let bindings = state.bindings.lock().await;
    let list: Vec<&IncidentBinding> = bindings.values().collect();
    (StatusCode::OK, Json(serde_json::to_value(list).unwrap_or_default()))
}

async fn create_binding(
    State(state): State<Arc<RunbookState>>,
    Json(req): Json<CreateBindingRequest>,
) -> (StatusCode, Json<Value>) {
    // Verify the referenced runbook exists.
    {
        let runbooks = state.runbooks.lock().await;
        if !runbooks.contains_key(&req.runbook_id) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "Referenced runbook_id does not exist"})),
            );
        }
    }

    let binding = IncidentBinding {
        id: Uuid::new_v4(),
        name: req.name,
        incident_pattern: req.incident_pattern,
        incident_severity: req.incident_severity,
        runbook_id: req.runbook_id,
        auto_execute: req.auto_execute,
        created_at: Utc::now(),
    };
    let id = binding.id;
    {
        let mut bindings = state.bindings.lock().await;
        bindings.insert(id, binding.clone());
    }
    (StatusCode::CREATED, Json(serde_json::to_value(binding).unwrap_or_default()))
}

async fn delete_binding(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let mut bindings = state.bindings.lock().await;
    if bindings.remove(&id).is_some() {
        (StatusCode::OK, Json(json!({"deleted": id})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Binding not found"})))
    }
}
