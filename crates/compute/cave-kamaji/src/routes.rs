// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::KamajiState;
use crate::components::{
    ControlPlaneInput, DatastoreBinding, NetworkProfile, build_control_plane,
};
use crate::connection::Driver;
use crate::ds_setup::tenant_schema;
use crate::lifecycle;
use crate::models::{CreateTenantRequest, TenantControlPlane, TenantPhase, TenantStatus};
use crate::reconcile::default_pipeline;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

/// Resolve a TenantControlPlane `spec.data_store` alias to a datastore
/// [`Driver`]. Defaults to etcd (the native back-end) for unknown values.
pub fn driver_from_data_store(data_store: &str) -> Driver {
    match data_store {
        "postgres" | "postgresql" => Driver::PostgreSql,
        "mysql" => Driver::MySql,
        "nats" => Driver::Nats,
        "etcd" | "shared-etcd" => Driver::Etcd,
        _ => Driver::Etcd,
    }
}

/// Build the control-plane component input a TCP implies, filling Cave-default
/// network/admission settings the minimal CRD shape does not carry.
fn control_plane_input(tcp: &TenantControlPlane) -> ControlPlaneInput {
    ControlPlaneInput {
        name: tcp.name.clone(),
        version: tcp.spec.kubernetes_version.clone(),
        advertise_address: "0.0.0.0".into(),
        network: NetworkProfile {
            service_cidr: "10.96.0.0/12".into(),
            pod_cidr: "10.244.0.0/16".into(),
            port: 6443,
        },
        datastore: DatastoreBinding {
            driver: driver_from_data_store(&tcp.spec.data_store),
            endpoints: vec!["etcd.cave-system.svc:2379".into()],
            schema: tenant_schema(&tcp.namespace, &tcp.name),
        },
        admission_plugins: vec!["NodeRestriction".into()],
        preferred_address_types: vec!["InternalIP".into(), "Hostname".into()],
    }
}

/// JSON projection of the three per-tenant control-plane containers.
pub fn component_plan_json(tcp: &TenantControlPlane) -> serde_json::Value {
    let components: Vec<serde_json::Value> = build_control_plane(&control_plane_input(tcp))
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "image": c.image,
                "command": c.command,
                "args": c.args,
                "liveness_port": c.liveness_port,
            })
        })
        .collect();
    serde_json::json!({
        "tenant": tcp.name,
        "replicas": tcp.spec.replicas,
        "components": components,
    })
}

/// JSON projection of the tenant's status conditions (Ready / ControlPlane /
/// Kubeconfig / DataStore / Konnectivity), as the reconcile loop reports them.
pub fn status_plan_json(tcp: &TenantControlPlane) -> serde_json::Value {
    let conditions: Vec<serde_json::Value> = crate::status::status_summary(tcp)
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "type": format!("{:?}", c.cond_type),
                "status": format!("{:?}", c.status),
                "reason": c.reason,
                "message": c.message,
            })
        })
        .collect();
    let phase = serde_json::to_value(&tcp.status.phase)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    serde_json::json!({
        "tenant": tcp.name,
        "phase": phase,
        "ready": tcp.status.ready,
        "conditions": conditions,
    })
}

/// JSON projection of the reconcile pipeline + the tenant's isolated datastore.
pub fn reconcile_plan_json(tcp: &TenantControlPlane) -> serde_json::Value {
    let driver = driver_from_data_store(&tcp.spec.data_store);
    let schema = tenant_schema(&tcp.namespace, &tcp.name);
    let phase = serde_json::to_value(&tcp.status.phase)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    serde_json::json!({
        "tenant": tcp.name,
        "phase": phase,
        "pipeline": default_pipeline(),
        "datastore": {
            "driver": driver.as_str(),
            "schema": schema,
            "user": tenant_schema(&tcp.namespace, &tcp.name),
            "supports_multitenancy": driver.supports_multitenancy(),
            "kine": driver.is_kine(),
        },
    })
}

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

pub async fn list_tenants(State(state): State<Arc<KamajiState>>) -> Json<Vec<TenantControlPlane>> {
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

/// `GET /api/kamaji/tenants/{id}/components` — the per-tenant control-plane
/// component plan (kube-apiserver + controller-manager + scheduler).
pub async fn get_components(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let tcp = state.tenants.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(component_plan_json(tcp.value())))
}

/// `GET /api/kamaji/tenants/{id}/reconcile-plan` — the provisioning pipeline +
/// the tenant's isolated datastore binding.
pub async fn get_reconcile_plan(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let tcp = state.tenants.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(reconcile_plan_json(tcp.value())))
}

/// `GET /api/kamaji/tenants/{id}/status` — the tenant's reported status
/// conditions (Ready / ControlPlaneHealthy / KubeconfigReady / DataStoreHealthy
/// / KonnectivityHealthy).
pub async fn get_status(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let tcp = state.tenants.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(status_plan_json(tcp.value())))
}
