// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-runbook.
use crate::models::*;
use crate::RunbookState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateRunbookRequest {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub tags: Option<Vec<String>>,
    pub parameters: Option<Vec<ParameterDef>>,
    pub steps: Vec<Step>,
    pub timeout_seconds: Option<u64>,
    pub on_failure: Option<FailureAction>,
    pub created_by: Option<String>,
}

#[derive(Deserialize)]
pub struct ExecuteRequest {
    pub runbook_id: Uuid,
    pub triggered_by: Option<String>,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Deserialize)]
pub struct ApprovalDecisionRequest {
    pub decided_by: String,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateTriggerRequest {
    pub runbook_id: Uuid,
    pub trigger_type: TriggerType,
    pub cron_expression: Option<String>,
    pub alert_source: Option<String>,
    pub alert_condition: Option<String>,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Deserialize, Default)]
pub struct RunbookQuery {
    pub template: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ApprovalQuery {
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn create_router(state: Arc<RunbookState>) -> Router {
    Router::new()
        // Runbooks
        .route("/api/runbook/runbooks", get(list_runbooks).post(create_runbook))
        .route(
            "/api/runbook/runbooks/{id}",
            get(get_runbook)
                .put(update_runbook)
                .delete(delete_runbook),
        )
        .route("/api/runbook/runbooks/{id}/steps", get(list_steps))
        // Executions
        .route(
            "/api/runbook/executions",
            get(list_executions).post(execute_runbook),
        )
        .route(
            "/api/runbook/executions/{id}",
            get(get_execution).delete(cancel_execution),
        )
        .route("/api/runbook/executions/{id}/logs", get(get_execution_logs))
        // Approvals
        .route("/api/runbook/approvals", get(list_approvals))
        .route("/api/runbook/approvals/{id}", get(get_approval))
        .route("/api/runbook/approvals/{id}/approve", post(approve_request))
        .route("/api/runbook/approvals/{id}/reject", post(reject_request))
        // Triggers
        .route(
            "/api/runbook/triggers",
            get(list_triggers).post(create_trigger),
        )
        .route("/api/runbook/triggers/{id}", delete(delete_trigger))
        // Library & health
        .route("/api/runbook/library", get(list_library))
        .route("/api/runbook/health", get(health))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Runbook handlers
// ---------------------------------------------------------------------------

async fn list_runbooks(
    State(state): State<Arc<RunbookState>>,
    Query(query): Query<RunbookQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let runbooks: Vec<Runbook> = match query.template.as_deref() {
        Some("true") => store
            .runbooks
            .values()
            .filter(|r| r.is_template)
            .cloned()
            .collect(),
        Some("false") => store
            .runbooks
            .values()
            .filter(|r| !r.is_template)
            .cloned()
            .collect(),
        _ => store.runbooks.values().cloned().collect(),
    };
    Json(runbooks)
}

async fn create_runbook(
    State(state): State<Arc<RunbookState>>,
    Json(req): Json<CreateRunbookRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let runbook = Runbook {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        version: req.version.unwrap_or_else(|| "1.0.0".to_string()),
        tags: req.tags.unwrap_or_default(),
        parameters: req.parameters.unwrap_or_default(),
        steps: req.steps,
        timeout_seconds: req.timeout_seconds.unwrap_or(300),
        on_failure: req.on_failure.unwrap_or(FailureAction::Stop),
        is_template: false,
        created_by: req.created_by.unwrap_or_else(|| "api".to_string()),
        created_at: now,
        updated_at: now,
    };
    let mut store = state.store.write().await;
    store.runbooks.insert(runbook.id, runbook.clone());
    (StatusCode::CREATED, Json(runbook))
}

async fn get_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.runbooks.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(r).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "runbook not found" })),
        ),
    }
}

async fn update_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRunbookRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.runbooks.get_mut(&id) {
        Some(existing) => {
            existing.name = req.name;
            existing.description = req.description;
            if let Some(v) = req.version {
                existing.version = v;
            }
            if let Some(tags) = req.tags {
                existing.tags = tags;
            }
            if let Some(params) = req.parameters {
                existing.parameters = params;
            }
            existing.steps = req.steps;
            if let Some(t) = req.timeout_seconds {
                existing.timeout_seconds = t;
            }
            if let Some(f) = req.on_failure {
                existing.on_failure = f;
            }
            existing.updated_at = Utc::now();
            let updated = existing.clone();
            (StatusCode::OK, Json(serde_json::to_value(updated).unwrap()))
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "runbook not found" })),
        ),
    }
}

async fn delete_runbook(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.runbooks.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

async fn list_steps(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.runbooks.get(&id) {
        Some(r) => (StatusCode::OK, Json(serde_json::to_value(&r.steps).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "runbook not found" })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Execution handlers
// ---------------------------------------------------------------------------

async fn execute_runbook(
    State(state): State<Arc<RunbookState>>,
    Json(req): Json<ExecuteRequest>,
) -> impl IntoResponse {
    let runbook = {
        let store = state.store.read().await;
        store.runbooks.get(&req.runbook_id).cloned()
    };

    let runbook = match runbook {
        Some(r) => r,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "runbook not found" })),
            );
        }
    };

    // Validate steps
    let mut all_errors: Vec<String> = vec![];
    for step in &runbook.steps {
        all_errors.extend(crate::steps::validate_step(step));
    }
    if !all_errors.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "step validation failed", "details": all_errors })),
        );
    }

    let triggered_by = req.triggered_by.unwrap_or_else(|| "api".to_string());
    let parameters = req.parameters.unwrap_or_default();

    let mut execution = crate::engine::create_execution(
        &runbook,
        &triggered_by,
        TriggerType::Manual,
        parameters,
    );

    crate::engine::run_execution(&runbook, &mut execution);

    // Create ApprovalRequests for any steps now in WaitingForApproval
    let approval_requests = build_approval_requests(&execution, &runbook);

    let mut store = state.store.write().await;
    for ar in approval_requests {
        store.approval_requests.insert(ar.id, ar);
    }
    store.executions.insert(execution.id, execution.clone());

    (StatusCode::CREATED, Json(serde_json::to_value(&execution).unwrap()))
}

fn build_approval_requests(execution: &Execution, runbook: &Runbook) -> Vec<ApprovalRequest> {
    execution
        .step_results
        .iter()
        .filter(|sr| sr.status == StepStatus::WaitingForApproval)
        .filter_map(|sr| {
            let step = runbook.steps.iter().find(|s| s.id == sr.step_id)?;
            if let StepType::ManualApproval {
                message,
                approvers,
                timeout_seconds,
            } = &step.step_type
            {
                let now = Utc::now();
                Some(ApprovalRequest {
                    id: Uuid::new_v4(),
                    execution_id: execution.id,
                    step_id: sr.step_id.clone(),
                    message: message.clone(),
                    approvers: approvers.clone(),
                    approved_by: None,
                    rejected_by: None,
                    status: ApprovalStatus::Pending,
                    created_at: now,
                    responded_at: None,
                    expires_at: now + chrono::Duration::seconds(*timeout_seconds as i64),
                })
            } else {
                None
            }
        })
        .collect()
}

async fn list_executions(State(state): State<Arc<RunbookState>>) -> impl IntoResponse {
    let store = state.store.read().await;
    let executions: Vec<Execution> = store.executions.values().cloned().collect();
    Json(executions)
}

async fn get_execution(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.executions.get(&id) {
        Some(e) => (StatusCode::OK, Json(serde_json::to_value(e).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "execution not found" })),
        ),
    }
}

async fn cancel_execution(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.executions.get_mut(&id) {
        Some(execution) => {
            crate::engine::cancel_execution(execution);
            let updated = execution.clone();
            (StatusCode::OK, Json(serde_json::to_value(updated).unwrap()))
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "execution not found" })),
        ),
    }
}

async fn get_execution_logs(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.executions.get(&id) {
        Some(execution) => {
            let logs: Vec<&String> = execution
                .step_results
                .iter()
                .flat_map(|sr| sr.logs.iter())
                .collect();
            (StatusCode::OK, Json(serde_json::to_value(logs).unwrap()))
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "execution not found" })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Approval handlers
// ---------------------------------------------------------------------------

async fn list_approvals(
    State(state): State<Arc<RunbookState>>,
    Query(query): Query<ApprovalQuery>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    let requests: Vec<ApprovalRequest> = match query.status.as_deref() {
        Some("pending") => store
            .approval_requests
            .values()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .cloned()
            .collect(),
        Some("approved") => store
            .approval_requests
            .values()
            .filter(|a| a.status == ApprovalStatus::Approved)
            .cloned()
            .collect(),
        Some("rejected") => store
            .approval_requests
            .values()
            .filter(|a| a.status == ApprovalStatus::Rejected)
            .cloned()
            .collect(),
        _ => store.approval_requests.values().cloned().collect(),
    };
    Json(requests)
}

async fn get_approval(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state.store.read().await;
    match store.approval_requests.get(&id) {
        Some(a) => (StatusCode::OK, Json(serde_json::to_value(a).unwrap())),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "approval request not found" })),
        ),
    }
}

async fn approve_request(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ApprovalDecisionRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let (execution_id, step_id) = match store.approval_requests.get_mut(&id) {
        Some(ar) => {
            if ar.status != ApprovalStatus::Pending {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "approval request is not pending" })),
                );
            }
            ar.status = ApprovalStatus::Approved;
            ar.approved_by = Some(req.decided_by.clone());
            ar.responded_at = Some(Utc::now());
            (ar.execution_id, ar.step_id.clone())
        }
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "approval request not found" })),
            );
        }
    };

    // Continue execution from approval step
    if let Some(execution) = store.executions.get_mut(&execution_id) {
        if let Some(sr) = execution
            .step_results
            .iter_mut()
            .find(|r| r.step_id == step_id)
        {
            sr.status = StepStatus::Completed;
            sr.completed_at = Some(Utc::now());
            sr.logs.push(format!(
                "[{}] Approved by {}",
                Utc::now().to_rfc3339(),
                req.decided_by
            ));
        }
        if execution.status == ExecutionStatus::WaitingForApproval {
            execution.status = ExecutionStatus::Running;
            // Resume remaining pending steps
            resume_execution_after_approval(execution);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "approved", "decided_by": req.decided_by })),
    )
}

async fn reject_request(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ApprovalDecisionRequest>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    let execution_id = match store.approval_requests.get_mut(&id) {
        Some(ar) => {
            if ar.status != ApprovalStatus::Pending {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "approval request is not pending" })),
                );
            }
            ar.status = ApprovalStatus::Rejected;
            ar.rejected_by = Some(req.decided_by.clone());
            ar.responded_at = Some(Utc::now());
            ar.execution_id
        }
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "approval request not found" })),
            );
        }
    };

    if let Some(execution) = store.executions.get_mut(&execution_id) {
        execution.status = ExecutionStatus::Failed;
        execution.completed_at = Some(Utc::now());
    }

    (
        StatusCode::OK,
        Json(
            serde_json::json!({ "status": "rejected", "decided_by": req.decided_by, "reason": req.reason }),
        ),
    )
}

/// After an approval, mark remaining pending steps as completed (simulation).
fn resume_execution_after_approval(execution: &mut Execution) {
    for sr in execution.step_results.iter_mut() {
        if sr.status == StepStatus::Pending {
            sr.status = StepStatus::Completed;
            sr.started_at = Some(Utc::now());
            sr.completed_at = Some(Utc::now());
            sr.logs
                .push(format!("[{}] Step completed after approval", Utc::now().to_rfc3339()));
        }
    }
    execution.status = ExecutionStatus::Completed;
    execution.completed_at = Some(Utc::now());
}

// ---------------------------------------------------------------------------
// Trigger handlers
// ---------------------------------------------------------------------------

async fn create_trigger(
    State(state): State<Arc<RunbookState>>,
    Json(req): Json<CreateTriggerRequest>,
) -> impl IntoResponse {
    // Verify runbook exists
    {
        let store = state.store.read().await;
        if !store.runbooks.contains_key(&req.runbook_id) {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "runbook not found" })),
            );
        }
    }

    let trigger = RunbookTrigger {
        id: Uuid::new_v4(),
        runbook_id: req.runbook_id,
        trigger_type: req.trigger_type,
        cron_expression: req.cron_expression,
        alert_source: req.alert_source,
        alert_condition: req.alert_condition,
        parameters: req.parameters.unwrap_or_default(),
        enabled: true,
        last_triggered_at: None,
        created_at: Utc::now(),
    };

    let mut store = state.store.write().await;
    store.triggers.insert(trigger.id, trigger.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(&trigger).unwrap()))
}

async fn list_triggers(State(state): State<Arc<RunbookState>>) -> impl IntoResponse {
    let store = state.store.read().await;
    let triggers: Vec<RunbookTrigger> = store.triggers.values().cloned().collect();
    Json(triggers)
}

async fn delete_trigger(
    State(state): State<Arc<RunbookState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut store = state.store.write().await;
    match store.triggers.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

// ---------------------------------------------------------------------------
// Library & health
// ---------------------------------------------------------------------------

async fn list_library() -> impl IntoResponse {
    let templates = crate::library::builtin_templates();
    Json(templates)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "module": crate::MODULE_NAME,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_approval_requests_empty_when_no_approval_steps() {
        let rb = crate::library::pod_restart_template();
        let exec = crate::engine::create_execution(&rb, "test", TriggerType::Manual, HashMap::new());
        let reqs = build_approval_requests(&exec, &rb);
        // pod restart has no approval steps
        assert!(reqs.is_empty());
    }

    #[test]
    fn test_build_approval_requests_for_rollback() {
        let rb = crate::library::deployment_rollback_template();
        let mut exec =
            crate::engine::create_execution(&rb, "test", TriggerType::Manual, HashMap::new());
        crate::engine::run_execution(&rb, &mut exec);
        let reqs = build_approval_requests(&exec, &rb);
        assert!(!reqs.is_empty());
        assert!(reqs.iter().all(|r| r.status == ApprovalStatus::Pending));
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;
        let state = Arc::new(RunbookState::default());
        let router = create_router(state);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/runbook/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_runbooks_returns_templates() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;
        let state = Arc::new(RunbookState::default());
        let router = create_router(state);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/runbook/runbooks?template=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
