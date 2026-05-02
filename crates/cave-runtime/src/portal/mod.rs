//! Portal-facing handlers for cave-runtime.
//!
//! Provides the user-facing surface: persona login (Keycloak / dev mode),
//! upstream tracker (ADR-aware), ADR browser, and attribution dashboard.
//!
//! All handlers live in this module so the runtime binary stays focused on
//! wiring; each sub-module owns its routes and inline tests.

pub mod auth;
pub mod upstream;

use axum::Router;

/// Build the combined portal router (auth + upstream tracker).
pub fn router() -> Router {
    Router::new().merge(auth::router()).merge(upstream::router())
}

/// Resolve the workspace root used by upstream/ADR/attribution handlers.
/// Honours `CAVE_WORKSPACE_ROOT`, falling back to the current directory.
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var("CAVE_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
