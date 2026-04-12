//! CAVE Registry — container image registry and package proxy/cache.
//!
//! Replaces Pulp / Harbor with a Rust-native implementation.
//! Implements the Docker Registry HTTP API V2 / OCI Distribution Spec.

pub mod docker_v2;
pub mod engine;
pub mod error;
pub mod gc;
pub mod migrations;
pub mod models;
pub mod policy;
pub mod replication;
pub mod routes;
pub mod scan;
pub mod store;
pub mod types;
pub mod webhook;

use axum::Router;
use cave_db::CavePool;
use policy::PolicyManager;
use replication::ReplicationManager;
use scan::ScanManager;
use std::sync::Arc;
use store::RegistryStore;
use webhook::WebhookManager;

/// Shared state injected into every request handler.
pub struct AppState {
    pub store: Arc<RegistryStore>,
    pub webhooks: Arc<WebhookManager>,
    pub replication: Arc<ReplicationManager>,
    pub scanner: Arc<ScanManager>,
    pub policy: Arc<PolicyManager>,
}

/// Type alias for consistency with the rest of the workspace.
pub type RegistryState = AppState;
pub type State = AppState;

impl Default for AppState {
    fn default() -> Self {
        let store = Arc::new(RegistryStore::new());
        Self {
            webhooks: Arc::new(WebhookManager::new(Arc::clone(&store))),
            replication: Arc::new(ReplicationManager::new(Arc::clone(&store))),
            scanner: Arc::new(ScanManager::new(Arc::clone(&store))),
            policy: Arc::new(PolicyManager::new(Arc::clone(&store))),
            store,
        }
    }
}

/// Full registry with DB pool (used at startup).
pub struct Registry {
    pub state: Arc<AppState>,
    pub pool: Arc<CavePool>,
}

impl Registry {
    /// Create a new registry, run DB migrations, and return a ready instance.
    pub async fn new(pool: Arc<CavePool>) -> Result<Self, String> {
        migrations::run(&pool).await?;
        let state = Arc::new(AppState::default());
        Ok(Self { state, pool })
    }

    /// Return an axum Router for mounting in the main application.
    pub fn router(&self) -> Router {
        routes::create_router(Arc::clone(&self.state))
    }

    /// Spawn periodic garbage collection (default: every 24 hours).
    pub fn spawn_gc(&self) {
        gc::GarbageCollector::spawn_periodic(
            Arc::clone(&self.state.store),
            std::time::Duration::from_secs(86400),
        );
    }
}

/// Create the axum router from an `AppState` reference.
pub fn router(state: Arc<AppState>) -> Router {
    routes::create_router(Arc::clone(&state))
        .merge(docker_v2::docker_v2_router(state))
}

pub const MODULE_NAME: &str = "registry";
