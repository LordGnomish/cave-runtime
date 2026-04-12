//! CAVE Deploy — GitOps continuous delivery engine.
//!
//! Replaces: ArgoCD, Flux
//! Native GitOps sync, canary/blue-green/rolling rollouts, drift detection.

pub mod gitops;
pub mod health;
pub mod models;
pub mod rollout;
pub mod routes;

use axum::Router;
use models::DeployStore;
use std::sync::{Arc, Mutex};

/// Shared state: a single in-memory store protected by a Mutex.
pub struct DeployState {
    pub store: Arc<Mutex<DeployStore>>,
}

impl Default for DeployState {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(DeployStore::default())),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<DeployState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "deploy";
