//! Release intelligence — git + SBOM diff based release notes
//!
//! Upstream tracking: custom
//! Features: Auto-generated changelogs from git commits + SBOM diffs per deployment

pub mod routes;
pub mod models;
pub mod engine;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "changelog";
