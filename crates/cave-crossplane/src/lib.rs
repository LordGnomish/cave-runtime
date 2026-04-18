//! CAVE Crossplane — Crossplane v2 XRD composition engine.
//! Replaces: crossplane/crossplane
//! Features: XRD parser, composition engine, patch transforms, claims, composites, providers, reconciler.

pub mod claim;
pub mod composition;
pub mod engine;
pub mod error;
pub mod models;
pub mod provider;
pub mod reconciler;
pub mod routes;
pub mod xrd;

use axum::Router;
use std::sync::Arc;

pub use claim::ClaimStore;
pub use composition::CompositionStore;
pub use engine::CompositionEngine;
pub use error::{CrossplaneError, CrossplaneResult};
pub use provider::ProviderStore;
pub use reconciler::ReconcileQueue;
pub use xrd::XrdStore;

pub const MODULE_NAME: &str = "crossplane";

pub struct CrossplaneState {
    pub xrd_store: Arc<XrdStore>,
    pub composition_store: Arc<CompositionStore>,
    pub claim_store: Arc<ClaimStore>,
    pub provider_store: Arc<ProviderStore>,
    pub reconcile_queue: Arc<ReconcileQueue>,
    pub engine: Arc<CompositionEngine>,
}

impl Default for CrossplaneState {
    fn default() -> Self {
        Self {
            xrd_store: Arc::new(XrdStore::new()),
            composition_store: Arc::new(CompositionStore::new()),
            claim_store: Arc::new(ClaimStore::new()),
            provider_store: Arc::new(ProviderStore::new()),
            reconcile_queue: Arc::new(ReconcileQueue::new()),
            engine: Arc::new(CompositionEngine::new()),
        }
    }
}

pub fn router(state: Arc<CrossplaneState>) -> Router {
    routes::create_router(state)
}
