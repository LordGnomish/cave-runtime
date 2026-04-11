//! Release intelligence — git + SBOM diff based release notes
//!
//! Upstream tracking: custom
//! Features: Auto-generated changelogs from git commits + SBOM diffs per deployment

pub mod routes;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "changelog";
