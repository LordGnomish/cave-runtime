// SPDX-License-Identifier: AGPL-3.0-or-later
//! API spec registry — compatible with Apicurio + openapi-diff
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
