pub mod lifecycle;
pub mod models;
pub mod routes;

use axum::{routing::{get, post}, Router};
use dashmap::DashMap;
use models::TenantControlPlane;
use std::sync::Arc;
use uuid::Uuid;

pub struct KamajiState {
    pub tenants: DashMap<Uuid, TenantControlPlane>,
}

impl Default for KamajiState {
    fn default() -> Self {
        Self {
            tenants: DashMap::new(),
        }
    }
}

pub fn router(state: Arc<KamajiState>) -> Router {
    Router::new()
        .route("/api/kamaji/tenants", post(routes::create_tenant).get(routes::list_tenants))
        .route("/api/kamaji/tenants/{id}", get(routes::get_tenant).delete(routes::delete_tenant))
        .route("/api/kamaji/tenants/{id}/kubeconfig", post(routes::get_kubeconfig))
        .with_state(state)
}
