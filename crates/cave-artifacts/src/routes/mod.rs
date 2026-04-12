//! HTTP route assembly for cave-artifacts.

pub mod content_guards;
pub mod content;
pub mod distributions;
pub mod publications;
pub mod remotes;
pub mod repositories;
pub mod tasks;

use crate::store::ArtifactsState;
use axum::Router;
use std::sync::Arc;

pub fn create_router(state: Arc<ArtifactsState>) -> Router {
    Router::new()
        .merge(repositories::router(Arc::clone(&state)))
        .merge(remotes::router(Arc::clone(&state)))
        .merge(publications::router(Arc::clone(&state)))
        .merge(distributions::router(Arc::clone(&state)))
        .merge(content::router(Arc::clone(&state)))
        .merge(tasks::router(Arc::clone(&state)))
        .merge(content_guards::router(Arc::clone(&state)))
}
