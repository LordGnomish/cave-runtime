//! Status page — auto-generated from cave-uptime probes
//!
//! Upstream tracking: custom
//! Features: Public/internal status page, auto-generation from probes, incident integration

pub mod routes;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "status";
