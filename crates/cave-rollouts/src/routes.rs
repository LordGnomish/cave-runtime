//! HTTP routes for cave-rollouts — progressive delivery API.
//!
//! Endpoints:
//!   GET  /api/v1/rollouts
//!   POST /api/v1/rollouts
//!   GET  /api/v1/rollouts/:id
//!   PUT  /api/v1/rollouts/:id
//!   DELETE /api/v1/rollouts/:id
//!   POST /api/v1/rollouts/:id/promote
//!   POST /api/v1/rollouts/:id/rollback
//!   GET  /api/v1/experiments
//!   POST /api/v1/experiments
//!   GET  /api/v1/experiments/:id
//!   GET  /api/v1/analysistemplates
//!   POST /api/v1/analysistemplates
//!   GET  /api/v1/analysistemplates/:id
//!   GET  /api/v1/analysisruns
//!   GET  /api/v1/analysisruns/:id
//!   GET  /api/rollouts/health

use crate::{
    engine,
    store::RolloutsStore,
    types::{
        AnalysisRun, AnalysisTemplate, Experiment, ExperimentVariant, MetricTemplate,
        Rollout, RolloutStrategy,
    },
    RolloutsState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": msg.into() })))
}

pub fn create_router(state: Arc<RolloutsState>) -> Router {
    Router::new()
        // Rollouts CRUD
        .route("/api/v1/rollouts", get(list_rollouts).post(create_rollout))
        .route(
            "/api/v1/rollouts/:id",
            get(get_rollout).put(update_rollout).delete(delete_rollout),
        )
        .route("/api/v1/rollouts/:id/promote", post(promote_rollout))
        .route("/api/v1/rollouts/:id/rollback", post(rollback_rollout))
        // Experiments
        .route(
            "/api/v1/experiments",
            get(list_experiments).post(create_experiment),
        )
        .route("/api/v1/experiments/:id", get(get_experiment))
        // Analysis Templates
        .route(
            "/api/v1/analysistemplates",
            get(list_templates).post(create_template),
        )
        .route("/api/v1/analysistemplates/:id", get(get_template))
        // Analysis Runs
        .route("/api/v1/analysisruns", get(list_runs))
        .route("/api/v1/analysisruns/:id", get(get_run))
        // Health
        .route("/api/rollouts/health", get(health))
        .with_state(state)
}

// ─── Rollouts ─────────────────────────────────────────────────────────────────

async fn list_rollouts(State(s): State<Arc<RolloutsState>>) -> Json<Vec<Rollout>> {
    Json(s.store.list_rollouts().await)
}

#[derive(Deserialize)]
struct CreateRolloutRequest {
    name: String,
    namespace: String,
    strategy: RolloutStrategy,
    stable_revision: String,
    canary_revision: String,
}

async fn create_rollout(
    State(s): State<Arc<RolloutsState>>,
    Json(req): Json<CreateRolloutRequest>,
) -> ApiResult<Rollout> {
    if req.name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    // Validate canary strategy steps if applicable.
    if let RolloutStrategy::Canary(ref c) = req.strategy {
        engine::validate_canary_steps(c)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    }
    let mut rollout = Rollout::new(
        req.name,
        req.namespace,
        req.strategy,
        req.stable_revision,
        req.canary_revision,
    );
    engine::start(&mut rollout);
    s.store.insert_rollout(rollout.clone()).await;
    Ok(Json(rollout))
}

async fn get_rollout(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Rollout> {
    s.store
        .get_rollout(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "rollout not found"))
}

#[derive(Deserialize)]
struct UpdateRolloutRequest {
    paused: Option<bool>,
}

async fn update_rollout(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRolloutRequest>,
) -> ApiResult<Rollout> {
    let mut rollout = s
        .store
        .get_rollout(id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "rollout not found"))?;

    match req.paused {
        Some(true) => engine::pause(&mut rollout),
        Some(false) => engine::resume(&mut rollout),
        None => {}
    }
    s.store.update_rollout(rollout.clone()).await;
    Ok(Json(rollout))
}

async fn delete_rollout(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<serde_json::Value> {
    if s.store.delete_rollout(id).await {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(err(StatusCode::NOT_FOUND, "rollout not found"))
    }
}

async fn promote_rollout(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Rollout> {
    let mut rollout = s
        .store
        .get_rollout(id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "rollout not found"))?;
    engine::promote(&mut rollout);
    s.store.update_rollout(rollout.clone()).await;
    Ok(Json(rollout))
}

#[derive(Deserialize)]
struct RollbackRequest {
    reason: Option<String>,
}

async fn rollback_rollout(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RollbackRequest>,
) -> ApiResult<Rollout> {
    let mut rollout = s
        .store
        .get_rollout(id)
        .await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "rollout not found"))?;
    let reason = req.reason.unwrap_or_else(|| "manual rollback".into());
    engine::rollback(&mut rollout, reason);
    s.store.update_rollout(rollout.clone()).await;
    Ok(Json(rollout))
}

// ─── Experiments ──────────────────────────────────────────────────────────────

async fn list_experiments(State(s): State<Arc<RolloutsState>>) -> Json<Vec<Experiment>> {
    Json(s.store.list_experiments().await)
}

#[derive(Deserialize)]
struct CreateExperimentRequest {
    name: String,
    namespace: String,
    variants: Vec<ExperimentVariant>,
    duration_seconds: Option<u64>,
}

async fn create_experiment(
    State(s): State<Arc<RolloutsState>>,
    Json(req): Json<CreateExperimentRequest>,
) -> ApiResult<Experiment> {
    if req.variants.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "at least one variant required"));
    }
    let mut experiment = Experiment::new(req.name, req.namespace, req.variants);
    experiment.duration_seconds = req.duration_seconds;
    s.store.insert_experiment(experiment.clone()).await;
    Ok(Json(experiment))
}

async fn get_experiment(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Experiment> {
    s.store
        .get_experiment(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "experiment not found"))
}

// ─── Analysis Templates ───────────────────────────────────────────────────────

async fn list_templates(State(s): State<Arc<RolloutsState>>) -> Json<Vec<AnalysisTemplate>> {
    Json(s.store.list_templates().await)
}

#[derive(Deserialize)]
struct CreateTemplateRequest {
    name: String,
    namespace: String,
    metrics: Vec<MetricTemplate>,
}

async fn create_template(
    State(s): State<Arc<RolloutsState>>,
    Json(req): Json<CreateTemplateRequest>,
) -> ApiResult<AnalysisTemplate> {
    if req.metrics.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "at least one metric required"));
    }
    let template = AnalysisTemplate::new(req.name, req.namespace, req.metrics);
    s.store.insert_template(template.clone()).await;
    Ok(Json(template))
}

async fn get_template(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<AnalysisTemplate> {
    s.store
        .get_template(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "analysis template not found"))
}

// ─── Analysis Runs ────────────────────────────────────────────────────────────

async fn list_runs(State(s): State<Arc<RolloutsState>>) -> Json<Vec<AnalysisRun>> {
    Json(s.store.list_runs().await)
}

async fn get_run(
    State(s): State<Arc<RolloutsState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<AnalysisRun> {
    s.store
        .get_run(id)
        .await
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "analysis run not found"))
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-rollouts",
        "status": "ok",
        "upstream": "Flagger / Argo Rollouts",
        "strategies": ["canary", "blue-green", "a/b-testing", "header-based"],
        "features": [
            "automated-metric-analysis",
            "promotion-rollback-thresholds",
            "traffic-splitting-cave-mesh",
            "prometheus-metric-provider",
            "webhook-metric-provider",
            "reusable-analysis-templates",
            "set-weight-steps",
            "pause-steps",
            "header-routing",
            "mirror-routing",
            "automatic-rollback",
            "manual-rollback",
            "webhook-slack-notifications",
            "a/b-experiments"
        ]
    }))
}
