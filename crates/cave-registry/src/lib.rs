//! Container registry — Harbor replacement.
//!
//! Implements Docker Registry V2 + OCI Distribution Spec:
//!   - Complete HTTP V2 API (manifests, blobs, catalog, tags, uploads)
//!   - OCI Image Manifest, Docker Manifest V2 Schema 2, Manifest List
//!   - Content-addressable blob storage with SHA256 digest verification
//!   - Chunked and monolithic blob uploads with session tracking
//!   - Garbage collection for unreferenced blobs
//!   - Vulnerability scanning integration hooks
//!   - Repository-level access control
//!   - Webhook notifications on push/pull/delete events
//!   - Replication to upstream registries
//!   - Tag immutability policies
//!   - cave-db integration via CavePool migrations
//!
//! Replaces: Harbor

<<<<<<< HEAD
pub mod engine;
pub mod models;
=======
pub mod error;
pub mod gc;
pub mod migrations;
pub mod policy;
pub mod replication;
>>>>>>> claude/brave-raman
pub mod routes;
pub mod scan;
pub mod store;
pub mod types;
pub mod webhook;

use axum::Router;
<<<<<<< HEAD
use cave_db::Storage;
=======
use cave_db::CavePool;
use policy::PolicyManager;
use replication::ReplicationManager;
use scan::ScanManager;
>>>>>>> claude/brave-raman
use std::sync::Arc;
use store::RegistryStore;
use webhook::WebhookManager;

<<<<<<< HEAD
/// Module state.
pub struct State {
    pub storage: Arc<dyn Storage>,
=======
pub const MODULE_NAME: &str = "registry";

// ── Application state ─────────────────────────────────────────────────────────

/// Shared state injected into every request handler.
pub struct AppState {
    pub store: Arc<RegistryStore>,
    pub webhooks: Arc<WebhookManager>,
    pub replication: Arc<ReplicationManager>,
    pub scanner: Arc<ScanManager>,
    pub policy: Arc<PolicyManager>,
}

// ── Owned state that also holds the DB pool (used at startup) ─────────────────

pub struct Registry {
    pub state: Arc<AppState>,
    pub pool: Arc<CavePool>,
>>>>>>> claude/brave-raman
}

impl Registry {
    /// Create a new registry, run DB migrations, and return a ready instance.
    pub async fn new(pool: Arc<CavePool>) -> Result<Self, String> {
        migrations::run(&pool).await?;
        let store = Arc::new(RegistryStore::new());
        let state = Arc::new(AppState {
            store: Arc::clone(&store),
            webhooks: Arc::new(WebhookManager::new(Arc::clone(&store))),
            replication: Arc::new(ReplicationManager::new(Arc::clone(&store))),
            scanner: Arc::new(ScanManager::new(Arc::clone(&store))),
            policy: Arc::new(PolicyManager::new(Arc::clone(&store))),
        });
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

/// Convenience: build just an axum Router from an existing pool (for embedding).
pub fn router(_pool: Arc<CavePool>) -> Router {
    let store = Arc::new(RegistryStore::new());
    let state = Arc::new(AppState {
        store: Arc::clone(&store),
        webhooks: Arc::new(WebhookManager::new(Arc::clone(&store))),
        replication: Arc::new(ReplicationManager::new(Arc::clone(&store))),
        scanner: Arc::new(ScanManager::new(Arc::clone(&store))),
        policy: Arc::new(PolicyManager::new(Arc::clone(&store))),
    });
    routes::create_router(state)
}
