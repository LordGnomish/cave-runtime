//! /pulp/api/v3/distributions/ routes.

use crate::models::*;
use crate::store::ArtifactsState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        .route(
            "/pulp/api/v3/distributions/",
            get(list_distributions).post(create_distribution),
        )
        .route(
            "/pulp/api/v3/distributions/{plugin}/{subtype}/{id}/",
            get(get_distribution).delete(delete_distribution),
        )
        .with_state(state)
}

async fn list_distributions(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<Distribution>> {
    Json(PulpPage::of(state.list_distributions().await))
}

async fn create_distribution(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreateDistributionRequest>,
) -> Result<Json<AsyncOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let task = state
        .run_as_task(
            "pulp.tasking.tasks.create_distribution",
            vec![],
            move |s| async move {
                let dist = s.create_distribution(req).await?;
                Ok(vec![dist.pulp_href])
            },
        )
        .await;

    Ok(Json(AsyncOperationResponse { task: task.pulp_href }))
}

async fn get_distribution(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<Json<Distribution>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/distributions/{plugin}/{subtype}/{id}/");
    state
        .get_distribution(&href)
        .await
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "not found" }))))
}

async fn delete_distribution(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/distributions/{plugin}/{subtype}/{id}/");
    state
        .delete_distribution(&href)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (
            StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(serde_json::json!({ "detail": e.to_string() })),
        ))
}
