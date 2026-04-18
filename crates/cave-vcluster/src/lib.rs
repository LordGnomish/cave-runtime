//! CAVE vCluster — virtual Kubernetes clusters for PR review environments.
//! ADR-070: 2 vCPU / 4 Gi RAM / 4-hour TTL / max 5 per namespace.

pub mod cluster;
pub mod error;
pub mod models;
pub mod quota;
pub mod routes;
pub mod syncer;
pub mod tenant;

use axum::Router;
use std::sync::Arc;

pub use error::{VClusterError, VClusterResult};

pub const MODULE_NAME: &str = "vcluster";

pub struct VClusterState {
    pub clusters: Arc<cluster::ClusterStore>,
    pub quota: Arc<quota::QuotaManager>,
    pub syncer: Arc<syncer::ResourceSyncer>,
}

impl Default for VClusterState {
    fn default() -> Self {
        Self {
            clusters: Arc::new(cluster::ClusterStore::new()),
            quota: Arc::new(quota::QuotaManager::new()),
            syncer: Arc::new(syncer::ResourceSyncer::new()),
        }
    }
}

pub fn router(state: Arc<VClusterState>) -> Router {
    routes::create_router(state)
}
