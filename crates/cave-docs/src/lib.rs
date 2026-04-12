//! API spec registry — replaces Apicurio + openapi-diff
//!
//! Upstream tracking: apicurio + openapi-diff
//! Features: OpenAPI/AsyncAPI spec storage, breaking change detection, schema versioning

pub mod routes;
pub mod models;
pub mod engine;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "docs";
