//! /pulp/api/v3/publications/ routes.

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
            "/pulp/api/v3/publications/",
            get(list_publications).post(create_publication),
        )
        .route(
            "/pulp/api/v3/publications/{plugin}/{subtype}/{id}/",
            get(get_publication),
        )
        .with_state(state)
}

async fn list_publications(
    State(state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<Publication>> {
    Json(PulpPage::of(state.list_publications().await))
}

async fn create_publication(
    State(state): State<Arc<ArtifactsState>>,
    Json(req): Json<CreatePublicationRequest>,
) -> Result<Json<AsyncOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let task = state
        .run_as_task(
            "pulp.tasking.tasks.publish",
            vec![],
            move |s| async move {
                let pub_ = crate::publication::create_publication(s, req).await?;
                Ok(vec![pub_.pulp_href])
            },
        )
        .await;

    Ok(Json(AsyncOperationResponse { task: task.pulp_href }))
}

async fn get_publication(
    State(state): State<Arc<ArtifactsState>>,
    Path((plugin, subtype, id)): Path<(String, String, String)>,
) -> Result<Json<Publication>, (StatusCode, Json<serde_json::Value>)> {
    let href = format!("/pulp/api/v3/publications/{plugin}/{subtype}/{id}/");
    state
        .get_publication(&href)
        .await
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "detail": "not found" }))))
}
