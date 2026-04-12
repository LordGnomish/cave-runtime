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
//! CAVE Deploy — GitOps engine, full ArgoCD replacement.
//! ## Feature parity with ArgoCD 2.x
//! - Application CRD model (source, destination, syncPolicy)
//! - Git sync engine: clone/pull, drift detection
//! - Sync operations: manual, auto-sync, prune, self-heal, retry + backoff
//! - Health assessment: Healthy / Progressing / Degraded / Suspended / Missing / Unknown
//! - Sync status: Synced / OutOfSync / Unknown
//! - Sync waves (argocd.argoproj.io/sync-wave) and hooks (PreSync / Sync / PostSync / SyncFail)
//! - ApplicationSet generators: List, Clusters, Git, Matrix, Merge, PullRequest
//! - Multi-cluster deployment with cluster secrets
//! - Resource tracking: label-based and annotation-based
//! - Diff engine: show what would change before applying
//! - Rollback to any previous revision
//! - SSO / Okta integration hooks
//! - RBAC: project-scoped roles with policy engine
//! - Notifications: Slack, email, generic webhook
//! - Admin API: /api/v1/applications, /api/v1/repositories, /api/v1/clusters, /api/v1/projects
//! ## Upstream tracking: ArgoCD
//! - GitHub: https://github.com/argoproj/argo-cd
//! - Parity target: ArgoCD v2.x feature set
//! - Annotations: argocd.argoproj.io/sync-wave, argocd.argoproj.io/hook
pub mod appset;
pub mod cluster;
pub mod diff;
pub mod error;
pub mod notifications;
pub mod rbac;
pub mod store;
pub mod sync;
pub use error::DeployError;
pub use routes::{create_router, DeployState};
pub use store::{DeployStore, MODULE_NAME};
use cave_db::CavePool;
use std::sync::Arc;
/// Module state shared across all request handlers.
pub struct DeployModule {
    pub state: Arc<DeployState>,
impl DeployModule {
    pub async fn new(pool: Arc<CavePool>) -> Result<Self, DeployError> {
        let store = Arc::new(DeployStore::new(pool).await?);
        let state = Arc::new(DeployState { store });
        Ok(Self { state })
    pub fn router(&self) -> Router {
        create_router(self.state.clone())
