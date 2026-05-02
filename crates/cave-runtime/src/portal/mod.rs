//! Portal-facing handlers for cave-runtime.
//!
//! Provides the user-facing surface: persona login (Keycloak / dev mode),
//! upstream tracker (ADR-aware), ADR browser, and attribution dashboard.
//!
//! All handlers live in this module so the runtime binary stays focused on
//! wiring; each sub-module owns its routes and inline tests.

pub mod adr;
pub mod attribution;
pub mod auth;
pub mod cloud_controller_manager;
pub mod controller_manager;
pub mod upstream;

use axum::Router;
use std::sync::Arc;

/// Build the combined portal router (auth + upstream + ADR + attribution +
/// controller-manager + cloud-controller-manager).
pub fn router(
    cm_state: Arc<controller_manager::ControllerManagerPortal>,
    ccm_state: Arc<cloud_controller_manager::CcmPortal>,
) -> Router {
    Router::new()
        .merge(auth::router())
        .merge(upstream::router())
        .merge(adr::router())
        .merge(attribution::router())
        .merge(controller_manager::router(cm_state))
        .merge(cloud_controller_manager::router(ccm_state))
}

/// Resolve the workspace root used by upstream/ADR/attribution handlers.
/// Honours `CAVE_WORKSPACE_ROOT`, falling back to the current directory.
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var("CAVE_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
