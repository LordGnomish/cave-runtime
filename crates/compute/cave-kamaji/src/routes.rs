// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::KamajiState;
use crate::lifecycle;
use crate::models::{CreateTenantRequest, TenantControlPlane, TenantPhase, TenantStatus};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
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

/// Return the per-tenant PKI plan: the kubeadm certificate tree, the computed
/// kube-apiserver serving-cert SANs, and the rotation parameters the
/// CertificateLifecycle controller enforces.
pub async fn get_certificates(
    State(state): State<Arc<KamajiState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let tcp = state.tenants.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(cert_plan_json(tcp.value())))
}

/// Build the JSON cert plan for a tenant from the `certs` decision layer.
fn cert_plan_json(tcp: &TenantControlPlane) -> serde_json::Value {
    use crate::certs;

    let tree: Vec<serde_json::Value> = certs::pki_tree()
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "base_name": c.base_name(),
                "is_ca": c.is_ca(),
                "is_keypair": c.is_keypair(),
                "signer": c.signer().map(|s| s.base_name()),
                "ext_key_usage": c.ext_key_usage().map(|u| match u {
                    certs::ExtKeyUsage::ServerAuth => "server-auth",
                    certs::ExtKeyUsage::ClientAuth => "client-auth",
                }),
            })
        })
        .collect();

    // Derive the apiserver control-plane endpoint host from the status, using
    // kubeadm defaults for the in-cluster service IP + DNS domain.
    let cp_host = tcp.status.api_server_endpoint.as_deref().map(|ep| {
        ep.trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or(ep)
            .split(':')
            .next()
            .unwrap_or(ep)
            .to_string()
    });
    let sans = certs::apiserver_cert_sans("cluster.local", "10.96.0.1", cp_host.as_deref(), &[]);

    serde_json::json!({
        "tenant": tcp.name,
        "namespace": tcp.namespace,
        "pki_tree": tree,
        "apiserver_sans": {
            "dns_names": sans.dns_names,
            "ip_addresses": sans.ip_addresses,
        },
        "rotation": {
            "deadline_days": certs::ROTATION_DEADLINE_DAYS,
            "strategies": [
                certs::RotationStrategy::X509.label(),
                certs::RotationStrategy::Kubeconfig.label(),
            ],
        },
    })
}
