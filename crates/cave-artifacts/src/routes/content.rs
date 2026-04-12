//! /pulp/api/v3/content/ routes — upload & list content units.

use crate::models::*;
use crate::store::ArtifactsState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        .route("/pulp/api/v3/content/", get(list_content))
        .route("/pulp/api/v3/artifacts/", get(list_artifacts).post(upload_artifact))
        .with_state(state)
}

#[derive(Deserialize)]
struct ContentQuery {
    plugin_type: Option<PluginType>,
    name: Option<String>,
    version: Option<String>,
}

async fn list_content(
    State(state): State<Arc<ArtifactsState>>,
    Query(q): Query<ContentQuery>,
) -> Json<PulpPage<ContentUnit>> {
    let units = state
        .search_content(
            q.name.as_deref(),
            q.plugin_type.as_ref(),
            q.version.as_deref(),
        )
        .await;
    Json(PulpPage::of(units))
}

async fn list_artifacts(
    State(_state): State<Arc<ArtifactsState>>,
) -> Json<PulpPage<serde_json::Value>> {
    // In production: list artifacts from object storage.
    Json(PulpPage::of(vec![]))
}

/// Upload a raw artifact blob (multipart/form-data with `file` field).
/// The caller should then create a content unit referencing the artifact href.
async fn upload_artifact(
    State(state): State<Arc<ArtifactsState>>,
    mut multipart: axum::extract::Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    use sha2::{Digest, Sha256};

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "detail": e.to_string() })),
        )
    })? {
        if field.name() == Some("file") {
            let data = field.bytes().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "detail": e.to_string() })),
                )
            })?;

            let sha256 = hex::encode(Sha256::digest(&data));
            let artifact = Artifact::new(data.to_vec(), sha256);
            let stored = state.store_artifact(artifact).await;

            return Ok((
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "pulp_href": stored.pulp_href,
                    "sha256": stored.sha256,
                    "size": stored.size,
                })),
            ));
        }
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "detail": "no 'file' field in multipart" })),
    ))
}
