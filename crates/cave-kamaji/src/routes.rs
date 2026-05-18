// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::lifecycle;
use crate::models::{CreateTenantRequest, TenantControlPlane, TenantPhase, TenantStatus};
use crate::KamajiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub async fn create_tenant(
    State(state): State<Arc<KamajiState>>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<Json<TenantControlPlane>, StatusCode> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let mut tcp = TenantControlPlane {
        id,
        name: req.name.clone(),
        namespace: req.namespace.clone(),
        spec: req.spec,
        status: TenantStatus {
            phase: TenantPhase::Provisioning,
            api_server_endpoint: None,
            ready: false,
            message: None,
        },
        created_at: now,
        updated_at: now,
    };
    lifecycle::provision(&mut tcp);
    state.tenants.insert(id, tcp.clone());
    Ok(Json(tcp))
}

pub async fn list_tenants(
    State(state): State<Arc<KamajiState>>,
) -> Json<Vec<TenantControlPlane>> {
    let list = state.tenants.iter().map(|e| e.value().clone()).collect();
    Json(list)
}

pub async fn get_tenant(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TenantControlPlane>, StatusCode> {
    state
        .tenants
        .get(&id)
        .map(|e| Json(e.value().clone()))
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn delete_tenant(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    match state.tenants.get_mut(&id) {
        Some(mut entry) => {
            lifecycle::deprovision(entry.value_mut());
            drop(entry);
            state.tenants.remove(&id);
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

pub async fn get_kubeconfig(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let tcp = state.tenants.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    lifecycle::generate_kubeconfig(tcp.value())
        .map(Json)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
