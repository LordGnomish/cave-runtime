//! Developer portal — compatible with Backstage.

pub mod catalog;
pub mod dashboard;
pub mod engine;
pub mod models;
pub mod oauth;
pub mod plugins;
pub mod registry;
pub mod routes;
pub mod ui;
pub mod wraps;

/// Per-module admin views (etcd / cri / apiserver / iam / mesh / pg / vault)
/// + per-tenant dashboard. Pinned to backstage/backstage v1.50.3.
pub mod admin;

use axum::Router;
use cave_kernel::parity::ParityReport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PortalState {
    pub services: RwLock<Vec<models::Service>>,
    /// Parity reports collected from each module, keyed by module name.
    pub parity_cache: RwLock<HashMap<String, ParityReport>>,
    /// Catalog of every upstream UI consolidated into cave-portal — populated
    /// at startup with [`wraps::WrapRegistry::builtin`]. Mutable so deployment
    /// configuration can flag specific wraps as `Proxy`/`Planned` without a
    /// rebuild.
    pub wraps: RwLock<wraps::WrapRegistry>,
    /// Live module registry — discovers the workspace at runtime and serves
    /// /api/portal/modules with a 5-minute TTL.
    pub modules: registry::ModuleRegistry,
}

impl Default for PortalState {
    fn default() -> Self {
        Self::new(default_workspace_root())
    }
}

impl PortalState {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self {
            services: RwLock::new(Vec::new()),
            parity_cache: RwLock::new(HashMap::new()),
            wraps: RwLock::new(wraps::WrapRegistry::builtin()),
            modules: registry::ModuleRegistry::new(workspace_root),
        }
    }
}

/// Resolve the workspace root from environment or fall back to current dir.
/// `CAVE_WORKSPACE_ROOT` overrides; otherwise we use `cwd` which is the
/// repo root when the runtime is launched via `cargo run`.
pub fn default_workspace_root() -> std::path::PathBuf {
    if let Ok(v) = std::env::var("CAVE_WORKSPACE_ROOT") {
        return std::path::PathBuf::from(v);
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

pub type State = PortalState;

pub fn router(state: Arc<PortalState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "portal";
