//! CAVE Deploy — GitOps deployment engine.
//!
//! Replaces: ArgoCD + Flux
//! Implements: Application CRD, ApplicationSet generators, sync engine,
//! health assessment, diff engine, rollback, App of Apps, Helm/Kustomize/YAML,
//! project-level RBAC, notifications, SSO patterns.

pub mod appset;
pub mod diff;
pub mod health;
pub mod models;
pub mod rbac;
pub mod routes;
pub mod sync;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

pub use models::*;

pub struct DeployState {
    pub pool: Arc<CavePool>,
}

impl DeployState {
    pub fn new(pool: Arc<CavePool>) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

pub fn router(state: Arc<DeployState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "deploy";
